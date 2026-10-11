import json
import math
import os
import time
import unittest
from pathlib import Path
from urllib.parse import quote

import requests
from bson import json_util
from pymongo import MongoClient


def eventually(check, timeout=180):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (AssertionError, requests.RequestException) as exc:
            last = exc
        time.sleep(1)
    raise AssertionError(f"Synchronization did not converge: {last}")


@unittest.skipUnless(os.environ.get("RUN_INTEGRATION") == "true", "requires a running demo")
class FlowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.topology = os.environ["TOPOLOGY"]
        cls.endpoint = Path("/state/endpoint").read_text().strip()
        host = "mongos" if cls.topology == "mongos" else "mongo"
        uri = f"mongodb://{host}:27017" + ("" if host == "mongos" else "/?replicaSet=rs0")
        cls.mongo = MongoClient(uri, serverSelectionTimeoutMS=5000)

    @classmethod
    def tearDownClass(cls):
        cls.mongo.close()

    def api(self, method, path, body=None):
        response = requests.request(method, self.endpoint + path, json=body, timeout=30)
        response.raise_for_status()
        return response.json() if response.content else {}

    def stored(self, key, collection="products"):
        raw = key if self.topology == "replica-set" else json_util.dumps({"_id": key}, json_options=json_util.CANONICAL_JSON_OPTIONS)
        response = requests.get(f"{self.endpoint}/mongodb.search_demo.{collection}/_doc/{quote(raw, safe='')}", timeout=10)
        if response.status_code == 404:
            return None
        response.raise_for_status()
        return response.json()["_source"]

    def assert_vector(self, source):
        expected = {"description_embedding"}
        if self.topology == "mongos":
            expected.add("__mongodb")
        self.assertEqual(set(source), expected)
        vector = source["description_embedding"]
        self.assertEqual(len(vector), 384)
        self.assertTrue(all(isinstance(x, (int, float)) and math.isfinite(x) for x in vector))

    def test_01_remote_model_default_pipeline_and_initial_copy(self):
        template = self.api("GET", "/_index_template/mongodb-vectors")["index_templates"][0]["index_template"]
        self.assertEqual(template["index_patterns"], ["mongodb.*"])
        model_id = Path("/state/model-id").read_text().strip()
        model = self.api("GET", f"/_plugins/_ml/models/{model_id}")
        self.assertEqual(model["algorithm"], "REMOTE")
        eventually(lambda: all(self.stored(f"p{i:03}") for i in range(1, 21)))
        self.assert_vector(self.stored("p001"))
        self.assertEqual(self.mongo.search_demo.products.count_documents({}), 20)
        for host in (("config", "shard0", "shard1") if self.topology == "mongos" else ("mongo",)):
            with MongoClient(f"mongodb://{host}:27017/?directConnection=true") as direct:
                self.assertEqual(direct.admin.command("serverStatus")["wiredTiger"]["cache"]["maximum bytes configured"], 256 * 1024 * 1024)

    def test_02_crud_reembeds_and_delete_reinsert(self):
        products = self.mongo.search_demo.products
        key = "aws-sync-probe"
        try:
            products.insert_one({"_id": key, "title": "Not indexed", "description": "Waterproof red hiking backpack"})
            first = eventually(lambda: self.stored(key))
            self.assert_vector(first)
            products.update_one({"_id": key}, {"$set": {"description": "Insulated winter sleeping bag", "title": "Updated"}})
            updated = eventually(lambda: (source := self.stored(key)) and source["description_embedding"] != first["description_embedding"] and source)
            self.assert_vector(updated)
            products.replace_one({"_id": key}, {"_id": key, "title": "Replacement", "description": "Lightweight two person camping tent"})
            replaced = eventually(lambda: (source := self.stored(key)) and source["description_embedding"] != updated["description_embedding"] and source)
            self.assert_vector(replaced)
            products.delete_one({"_id": key})
            eventually(lambda: self.stored(key) is None)
            products.insert_one({"_id": key, "title": "Reinserted", "description": "Waterproof red hiking backpack"})
            self.assert_vector(eventually(lambda: self.stored(key)))
        finally:
            products.delete_one({"_id": key})
            eventually(lambda: self.stored(key) is None)

    def test_03_text_query_matches_opensearch_ids_scores_and_full_documents(self):
        names = ("products", "range", "hashed", "compound", "unsharded") if self.topology == "mongos" else ("products",)
        for name in names:
            with self.subTest(collection=name):
                index = "mongodb.search_demo." + name
                query = {"path": "description", "query": "waterproof hiking jacket", "limit": 3}
                self.api("POST", f"/{index}/_refresh")
                native = self.api("POST", f"/{index}/_search?allow_partial_search_results=false", {
                    "size": 3, "_source": False,
                    "query": {"neural": {"description_embedding": {"query_text": query["query"], "k": 3}}},
                })["hits"]["hits"]
                rows = list(self.mongo.search_demo[name].aggregate([
                    {"$vectorSearch": query}, {"$set": {"score": {"$meta": "vectorSearchScore"}}},
                ]))
                self.assertEqual(len(rows), len(native))
                self.assertTrue(rows)
                for row, hit in zip(rows, native):
                    key = json_util.loads(hit["_id"]) if self.topology == "mongos" else {"_id": hit["_id"]}
                    original = self.mongo.search_demo[name].find_one(key)
                    score = row.pop("score")
                    self.assertAlmostEqual(score, hit["_score"], places=6)
                    self.assertEqual(row, original)


if __name__ == "__main__":
    unittest.main()
