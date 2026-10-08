import json
import time
import unittest
import uuid
from pathlib import Path

import requests
from bson import json_util, Int64, ObjectId
from pymongo import MongoClient
from pymongo.errors import OperationFailure

MONGO = MongoClient("mongodb://mongos:27017", serverSelectionTimeoutMS=5000)
ROOT = Path("/demo")


def api(method, path, body=None, base="http://opensearch:9200"):
    response = requests.request(method, base + path, json=body, timeout=30)
    response.raise_for_status()
    return response.json() if response.content else {}


def eventually(check, seconds=120):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (AssertionError, requests.RequestException) as exc:
            last = exc
        time.sleep(1)
    raise AssertionError(f"condition did not converge: {last}")


def hits(collection):
    index = "mongodb.search_demo." + collection
    api("POST", f"/{index}/_refresh")
    return api("POST", f"/{index}/_search", {"size": 100, "query": {"match_all": {}}})["hits"]["hits"]


def decoded_key(hit):
    return json_util.loads(hit["_id"])


def candidate_hit(collection, key):
    return next((h for h in hits(collection) if decoded_key(h) == key), None)


class ShardedFlowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        eventually(lambda: all(len(hits(name)) == 4 for name in ("range", "hashed", "compound")))

    def test_01_initial_copy_preserves_full_keys_and_embeds_only_description(self):
        for name in ("config", "shard0", "shard1"):
            with MongoClient(f"mongodb://{name}:27017/?directConnection=true") as direct:
                cache = direct.admin.command("serverStatus")["wiredTiger"]["cache"]
                self.assertEqual(cache["maximum bytes configured"], 256 * 1024 * 1024)
        for shard in ("shard0", "shard1"):
            with MongoClient(f"mongodb://{shard}:27017/?directConnection=true") as direct:
                for name in ("range", "hashed", "compound"):
                    self.assertEqual(direct.search_demo[name].count_documents({"_id": "shared"}), 1)
                    with self.assertRaises(OperationFailure) as failure:
                        list(direct.search_demo[name].aggregate([{"$vectorSearch": {"path": "description", "query": "rain"}}]))
                    self.assertEqual(failure.exception.code, 31082, "shards must use the native disabled-search stage, not the extension")
                self.assertNotIn("--loadExtensions", direct.admin.command("getCmdLineOpts")["argv"])
        for name, fields in (("range", ["tenant"]), ("hashed", ["tenant"]),
                             ("compound", ["location.region", "tenant"])):
            rows = hits(name)
            self.assertEqual(len({h["_id"] for h in rows}), 4)
            for hit in rows:
                source = hit["_source"]
                self.assertEqual(set(source), {"description_embedding", "__mongodb"})
                self.assertEqual(len(source["description_embedding"]), 384)
                key = decoded_key(hit)
                self.assertEqual(set(key), {"_id", *fields})
                metadata = source["__mongodb"]["documentKey"]
                self.assertIsInstance(metadata, str)
                self.assertEqual(metadata, hit["_id"], "metadata must be the exact canonical record key")
                self.assertEqual(key, json_util.loads(metadata))
                self.assertIsNotNone(MONGO.search_demo[name].find_one(key))
            self.assertEqual(sum(decoded_key(h)["_id"] == "shared" for h in rows), 2)

    def test_02_copy_live_and_delete_key_normalization(self):
        for name in ("range", "hashed", "compound"):
            config = json.loads((ROOT / "config" / f"mongo-{name}-source.json").read_text())
            pipeline = json.loads(config["pipeline"])
            doc = {"_id": ObjectId(), "tenant": Int64(7), "location": {"region": "eu"},
                   "title": "Not indexed", "description": "Rainproof jacket"}
            document_key = {"tenant": doc["tenant"], "_id": doc["_id"]}
            if name == "compound":
                document_key["location.region"] = "eu"
            events = MONGO.pipeline_tests[name]
            events.drop()
            events.insert_many([
                {"_id": 1, "operationType": "insert", "documentKey": {"_id": doc["_id"]}, "fullDocument": doc},
                {"_id": 2, "operationType": "update", "documentKey": document_key, "fullDocument": doc},
                {"_id": 3, "operationType": "delete", "documentKey": document_key},
            ])
            normalized = list(events.aggregate([{"$sort": {"_id": 1}}, *pipeline]))
            self.assertEqual(normalized[0]["documentKey"], normalized[1]["documentKey"])
            self.assertEqual(normalized[0]["documentKey"], normalized[2]["documentKey"])
            self.assertEqual(list(normalized[0]["documentKey"]), list(normalized[2]["documentKey"]))
            self.assertNotIn("fullDocument", normalized[2], "delete must remain a tombstone")

    def test_03_crud_reindexes_without_cross_shard_id_collisions(self):
        for name, extra in (("range", {}), ("hashed", {}), ("compound", {"location.region": "eu"})):
            coll = MONGO.search_demo[name]
            key = {"_id": "shared", "tenant": -1, **extra}
            other = {"_id": "shared", "tenant": 1, **extra}
            before = candidate_hit(name, key)["_source"]["description_embedding"]
            untouched = candidate_hit(name, other)["_source"]["description_embedding"]
            self.assertEqual(coll.update_one(key, {"$set": {"description": "A lightweight camping tent for summer"}}).matched_count, 1)
            eventually(lambda: candidate_hit(name, key)["_source"]["description_embedding"] != before)
            hit = candidate_hit(name, key)
            self.assertEqual(hit["_source"]["__mongodb"]["documentKey"], hit["_id"])
            self.assertEqual(candidate_hit(name, other)["_source"]["description_embedding"], untouched)
            replacement = {"_id": "shared", "tenant": -1, "title": "Replacement", "description": "Insulated winter sleeping bag"}
            if name == "compound":
                replacement["location"] = {"region": "eu"}
            updated = candidate_hit(name, key)["_source"]["description_embedding"]
            coll.replace_one(key, replacement)
            eventually(lambda: candidate_hit(name, key)["_source"]["description_embedding"] != updated)
            hit = candidate_hit(name, key)
            self.assertEqual(hit["_source"]["__mongodb"]["documentKey"], hit["_id"])
            coll.delete_one(key)
            eventually(lambda: candidate_hit(name, key) is None)
            self.assertIsNotNone(candidate_hit(name, other))
            coll.insert_one(replacement)
            hit = eventually(lambda: candidate_hit(name, key))
            self.assertEqual(hit["_source"]["__mongodb"]["documentKey"], hit["_id"])

    def test_04_vector_search_returns_current_mongo_docs_and_score_metadata(self):
        for name in ("range", "hashed", "compound"):
            stage = {"$vectorSearch": {"path": "description", "query": "waterproof hiking jacket", "limit": 10}}
            rows = list(MONGO.search_demo[name].aggregate([stage, {"$set": {"score": {"$meta": "vectorSearchScore"}}}]))
            self.assertEqual(len(rows), 4)
            self.assertEqual(sum(row["_id"] == "shared" for row in rows), 2)
            scores = [row.pop("score") for row in rows]
            self.assertEqual(scores, sorted(scores, reverse=True))
            for row in rows:
                query = {"_id": row["_id"], "tenant": row["tenant"]}
                if name == "compound":
                    query["location.region"] = row["location"]["region"]
                self.assertEqual(row, MONGO.search_demo[name].find_one(query))
                self.assertNotIn("__mongodb", row)
            coll = MONGO.search_demo[name]
            doc = coll.find_one({"_id": "shared", "tenant": 1})
            coll.update_one({"_id": "shared", "tenant": 1}, {"$set": {"title": "Fresh Mongo-only title"}})
            rows = list(coll.aggregate([stage]))
            self.assertEqual(next(r for r in rows if r["_id"] == "shared" and r["tenant"] == 1)["title"], "Fresh Mongo-only title")
            coll.update_one({"_id": "shared", "tenant": 1}, {"$set": {"title": doc["title"]}})

    def test_05_opensearch_accepts_512_bytes_and_rejects_513(self):
        source = {"description": "Waterproof outdoor jacket", "__mongodb": {"documentKey": {"_id": "boundary"}}}
        for length, status in ((512, 201), (513, 400)):
            body = json.dumps({"index": {"_index": "mongodb.boundary.documents", "_id": "x" * length}}) + "\n" + json.dumps(source) + "\n"
            response = requests.post("http://opensearch:9200/_bulk", data=body,
                                     headers={"Content-Type": "application/x-ndjson"}, timeout=30)
            if length == 513:
                self.assertEqual(response.status_code, 400, response.text)
                self.assertIn("512", response.text)
            else:
                response.raise_for_status()
                result = response.json()["items"][0]["index"]
                self.assertIn(result["status"], (200, status), result)
        api("DELETE", "/mongodb.boundary.documents")

    def test_06_oversized_composite_key_fails_sink_task_without_silent_drop(self):
        name = "limit_" + uuid.uuid4().hex
        namespace = "sdk_mongos_test." + name
        topic = "mongodb." + namespace
        source_name, sink_name = "mongo-" + name, "opensearch-" + name
        MONGO.admin.command({"enableSharding": "sdk_mongos_test", "primaryShard": "shard0"})
        MONGO.admin.command({"shardCollection": namespace, "key": {"tenant": 1}})
        coll = MONGO.sdk_mongos_test[name]
        coll.insert_one({"_id": "x" * 500, "tenant": "t" * 100,
                         "title": "Not indexed", "description": "An oversized identity"})
        source = json.loads((ROOT / "config" / "mongo-range-source.json").read_text())
        source.update({"database": "sdk_mongos_test", "collection": name,
                       "topic.namespace.map": json.dumps({namespace: topic}),
                       "startup.mode.copy.existing.namespace.regex": r"sdk_mongos_test\." + name})
        sink = json.loads((ROOT / "config" / "opensearch-sink.json").read_text())
        sink["topics"] = topic
        try:
            api("PUT", f"/connectors/{sink_name}/config", sink, base="http://connect:8083")
            api("PUT", f"/connectors/{source_name}/config", source, base="http://connect:8083")

            def failed_task():
                status = api("GET", f"/connectors/{sink_name}/status", base="http://connect:8083")
                return next((t for t in status["tasks"] if t["state"] == "FAILED"), None)

            task = eventually(failed_task)
            self.assertIn("512", task["trace"])
            self.assertIsNotNone(coll.find_one({"_id": "x" * 500}))
        finally:
            for connector in (source_name, sink_name):
                response = requests.delete(f"http://connect:8083/connectors/{connector}", timeout=30)
                self.assertIn(response.status_code, (204, 404))
            coll.drop()
            response = requests.delete("http://opensearch:9200/" + topic, timeout=30)
            self.assertIn(response.status_code, (200, 404))

    def test_07_ingest_creates_metadata_after_embedding_and_overwrites_stale_metadata(self):
        key = json_util.dumps({"_id": ObjectId("000000000000000000000009"),
                               "location.region": "eu", "tenant": Int64(7)},
                              json_options=json_util.CANONICAL_JSON_OPTIONS)
        result = api("POST", "/_ingest/pipeline/mongodb-auto-embed/_simulate?verbose=true", {
            "docs": [{"_index": "mongodb.ingest.test", "_id": key, "_source": {
                "description": "Rainproof hiking jacket",
                "__mongodb": {"documentKey": "stale metadata must not be embedded"},
            }}]
        })
        processors = result["docs"][0]["processor_results"]
        self.assertEqual([p["processor_type"] for p in processors], ["script", "text_embedding", "script"])
        for processor in processors:
            self.assertNotIn("error", processor)
        self.assertNotIn("__mongodb", processors[0]["doc"]["_source"])
        self.assertNotIn("__mongodb", processors[1]["doc"]["_source"])
        source = processors[2]["doc"]["_source"]
        self.assertEqual(set(source), {"description_embedding", "__mongodb"})
        self.assertEqual(len(source["description_embedding"]), 384)
        self.assertEqual(source["__mongodb"], {"documentKey": key})
        decoded = json_util.loads(source["__mongodb"]["documentKey"])
        self.assertIsInstance(decoded["_id"], ObjectId)
        self.assertIsInstance(decoded["tenant"], Int64)
        self.assertEqual(decoded["location.region"], "eu")

    def test_08_unsharded_initial_copy_preserves_typed_id_only(self):
        coll = MONGO.search_demo.unsharded
        catalog = MONGO.config.collections.find_one({"_id": coll.full_name})
        self.assertTrue(catalog is None or catalog.get("unsplittable") is True,
                        "fixture must not be a sharded collection")
        rows = eventually(lambda: hits("unsharded") if len(hits("unsharded")) == 3 else None)
        ids = []
        for hit in rows:
            key = decoded_key(hit)
            self.assertEqual(list(key), ["_id"])
            ids.append(key["_id"])
            self.assertIsNotNone(coll.find_one(key))
            source = hit["_source"]
            self.assertEqual(set(source), {"description_embedding", "__mongodb"})
            self.assertEqual(len(source["description_embedding"]), 384)
            self.assertEqual(source["__mongodb"]["documentKey"], hit["_id"])
        self.assertEqual({type(value) for value in ids}, {str, ObjectId, Int64})

    def test_09_unsharded_insert_update_replace_delete(self):
        coll = MONGO.search_demo.unsharded
        for identifier in ("live-unsharded", ObjectId(), Int64(99)):
            with self.subTest(identifier=identifier):
                key = {"_id": identifier}
                coll.insert_one({**key, "title": "Mongo-only title",
                                 "description": "Waterproof hiking jacket"})
                hit = eventually(lambda: candidate_hit("unsharded", key))
                record_id = hit["_id"]
                vector = hit["_source"]["description_embedding"]
                coll.update_one(key, {"$set": {"description": "Lightweight summer camping tent"}})
                eventually(lambda: candidate_hit("unsharded", key)["_source"]["description_embedding"] != vector)
                hit = candidate_hit("unsharded", key)
                self.assertEqual(hit["_id"], record_id)
                vector = hit["_source"]["description_embedding"]
                coll.replace_one(key, {**key, "title": "Replacement",
                                       "description": "Insulated winter sleeping bag"})
                eventually(lambda: candidate_hit("unsharded", key)["_source"]["description_embedding"] != vector)
                hit = candidate_hit("unsharded", key)
                self.assertEqual(hit["_id"], record_id)
                self.assertEqual(hit["_source"]["__mongodb"]["documentKey"], record_id)
                self.assertEqual(sum(decoded_key(h) == key for h in hits("unsharded")), 1)
                coll.delete_one(key)
                eventually(lambda: candidate_hit("unsharded", key) is None)
        self.assertEqual(len(hits("unsharded")), 3)

    def test_10_unsharded_vector_search_returns_full_current_docs_and_score(self):
        coll = MONGO.search_demo.unsharded
        stage = {"$vectorSearch": {"path": "description", "query": "waterproof hiking jacket", "limit": 10}}
        rows = list(coll.aggregate([stage, {"$set": {"score": {"$meta": "vectorSearchScore"}}}]))
        self.assertEqual(len(rows), 3)
        scores = [row.pop("score") for row in rows]
        self.assertTrue(all(isinstance(score, (int, float)) for score in scores))
        self.assertEqual(scores, sorted(scores, reverse=True))
        for row in rows:
            self.assertEqual(row, coll.find_one({"_id": row["_id"]}))
        key = {"_id": "unsharded-jacket"}
        original = coll.find_one(key)["title"]
        try:
            coll.update_one(key, {"$set": {"title": "Fresh unindexed title"}})
            rows = list(coll.aggregate([stage]))
            self.assertEqual(next(r for r in rows if r["_id"] == key["_id"])["title"], "Fresh unindexed title")
        finally:
            coll.update_one(key, {"$set": {"title": original}})


if __name__ == "__main__":
    unittest.main(verbosity=2)
