import math
import time
import unittest

import requests
from pymongo import MongoClient


MONGO_URI = "mongodb://mongo:27017/?replicaSet=rs0"
CONNECT_URL = "http://connect:8083"
OPENSEARCH_URL = "http://opensearch:9200"
INDEX = "mongodb.search_demo.products"


def wait_until(label, predicate, timeout=180):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except Exception as exc:
            last_error = exc
        time.sleep(1)
    raise AssertionError(f"timed out waiting for {label}; last error: {last_error}")


def os_request(method, path, body=None):
    response = requests.request(method, OPENSEARCH_URL + path, json=body, timeout=30)
    response.raise_for_status()
    return response.json()


def os_doc(document_id):
    response = requests.get(f"{OPENSEARCH_URL}/{INDEX}/_doc/{document_id}", timeout=10)
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return response.json()


def connector_running(name):
    status = requests.get(f"{CONNECT_URL}/connectors/{name}/status", timeout=10)
    if status.status_code != 200:
        return False
    payload = status.json()
    return (payload["connector"]["state"] == "RUNNING" and bool(payload.get("tasks"))
            and all(task["state"] == "RUNNING" for task in payload["tasks"]))


class SyncTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mongo = MongoClient(MONGO_URI, serverSelectionTimeoutMS=10000)
        cls.products = cls.mongo.search_demo.products
        for name in ("mongo-products-source", "opensearch-products-sink"):
            wait_until(name, lambda name=name: connector_running(name))
        wait_until("initial copy", lambda: os_doc("p001") is not None)

    @classmethod
    def tearDownClass(cls):
        cls.mongo.close()

    def assert_vectors(self, source, fields=("description",)):
        self.assertEqual(set(source), {f + "_embedding" for f in fields})
        for vector in source.values():
            self.assertEqual(len(vector), 384)
            self.assertTrue(all(isinstance(x, (int, float)) and math.isfinite(x) for x in vector))

    def test_initial_copy_and_global_mapping(self):
        wait_until("all seed documents", lambda: all(os_doc(f"p{i:03}") for i in range(1, 21)))
        self.assert_vectors(os_doc("p001")["_source"])
        mapping = os_request("GET", f"/{INDEX}/_mapping")[INDEX]["mappings"]["properties"]
        self.assertEqual(set(mapping), {"description_embedding"})
        for field in mapping.values():
            self.assertEqual(field["type"], "knn_vector")
            self.assertEqual(field["dimension"], 384)
        templates = os_request("GET", "/_index_template/mongodb-vectors")["index_templates"]
        self.assertEqual(templates[0]["index_template"]["index_patterns"], ["mongodb.*"])

    def test_insert_update_replace_delete_and_reinsert(self):
        key = "sync-vector-probe"
        self.products.delete_one({"_id": key})
        wait_until("probe cleanup", lambda: os_doc(key) is None)
        try:
            self.products.insert_one({"_id": key, "title": "Hiking pack", "description": "A red backpack"})
            wait_until("insert", lambda: os_doc(key) is not None)
            first = os_doc(key)
            self.assert_vectors(first["_source"])
            self.products.update_one({"_id": key}, {"$set": {"description": "A warm sleeping bag"}})
            wait_until("new embedding", lambda: os_doc(key)["_source"]["description_embedding"]
                       != first["_source"]["description_embedding"])
            updated = os_doc(key)
            self.assertGreater(updated["_version"], first["_version"])
            self.products.update_one({"_id": key}, {"$set": {"title": "Not indexed"}})
            wait_until("title update processed", lambda: os_doc(key)["_version"] > updated["_version"])
            self.assertEqual(os_doc(key)["_source"], updated["_source"])
            self.products.replace_one({"_id": key}, {"_id": key, "title": "Only a title"})
            wait_until("replace removes old fields", lambda: os_doc(key)["_source"] == {})
            self.products.delete_one({"_id": key})
            wait_until("physical delete", lambda: os_doc(key) is None)
            self.products.insert_one({"_id": key, "description": "Same ID, new document"})
            wait_until("same ID reinsert", lambda: os_doc(key) is not None)
            self.assert_vectors(os_doc(key)["_source"], ("description",))
        finally:
            self.products.delete_one({"_id": key})
            wait_until("probe removed", lambda: os_doc(key) is None)

    def test_default_query_model_and_full_mongo_document_with_score(self):
        body = {"size": 3, "_source": False, "query": {"neural": {
            "description_embedding": {"query_text": "waterproof backpack", "k": 3}
        }}}
        wait_until("default model query", lambda: bool(os_request("POST", f"/{INDEX}/_search", body)["hits"]["hits"]))
        for path in ("description",):
            stage = {"$vectorSearch": {"path": path, "query": "camp mug", "limit": 1,
                                      "filter": {"ids": {"values": ["p007"]}}}}
            expected = self.products.find_one({"_id": "p007"})
            wait_until("Mongo host ID lookup", lambda: list(self.products.aggregate([stage])) == [expected])
            hits = list(self.products.aggregate([stage, {"$set": {"score": {"$meta": "vectorSearchScore"}}}]))
            self.assertEqual(len(hits), 1)
            score = hits[0].pop("score")
            self.assertIsInstance(score, (int, float))
            self.assertGreater(score, 0)
            self.assertEqual(hits[0], expected)

    def test_replay_live_records_and_reject_stale_version(self):
        before = os_doc("p001")
        response = requests.put(f"{OPENSEARCH_URL}/{INDEX}/_doc/p001",
                                params={"version": before["_version"], "version_type": "external"},
                                json={"description": "stale data"}, timeout=30)
        self.assertEqual(response.status_code, 409)
        base = f"{CONNECT_URL}/connectors/opensearch-products-sink"

        def checkpoint():
            response = requests.get(base + "/offsets", timeout=10)
            response.raise_for_status()
            return response.json()["offsets"][0]["offset"]["kafka_offset"]

        wait_until("committed sink offset", lambda: checkpoint() > before["_version"])
        committed = checkpoint()
        requests.put(base + "/stop", timeout=10).raise_for_status()
        try:
            wait_until("stopped sink", lambda: requests.get(base + "/status", timeout=10)
                       .json()["connector"]["state"] == "STOPPED")
            requests.patch(base + "/offsets", json={"offsets": [{
                "partition": {"kafka_topic": INDEX, "kafka_partition": 0},
                "offset": {"kafka_offset": 0},
            }]}, timeout=30).raise_for_status()
        finally:
            requests.put(base + "/resume", timeout=10).raise_for_status()
        wait_until("sink resumed", lambda: connector_running("opensearch-products-sink"))
        wait_until("same offsets replayed", lambda: checkpoint() >= committed)
        after = os_doc("p001")
        self.assertEqual(before["_version"], after["_version"])
        self.assertEqual(before["_source"], after["_source"])
        wait_until("replayed deleted probe is absent", lambda: os_doc("sync-vector-probe") is None)


if __name__ == "__main__":
    unittest.main(verbosity=2)
