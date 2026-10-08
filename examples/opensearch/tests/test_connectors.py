import json
import time
import unittest
import uuid
from pathlib import Path

import requests

from test_sync import CONNECT_URL, MONGO_URI, OPENSEARCH_URL, MongoClient, connector_running, wait_until


class ConnectorLifecycleTests(unittest.TestCase):
    def setUp(self):
        token = uuid.uuid4().hex
        self.namespace = f"connector_test.{token}"
        self.index = f"mongodb.{self.namespace}"
        self.source_name = f"lifecycle-{token}-source"
        self.sink_name = f"lifecycle-{token}-sink"
        self.client = MongoClient(MONGO_URI, serverSelectionTimeoutMS=10000)
        self.collection = self.client.connector_test[token]
        self.addCleanup(self.cleanup)
        self.source = json.loads(Path("/config/mongo-source-connector.json").read_text())
        self.source.update({"database": "connector_test", "collection": token,
                            "topic.namespace.map": json.dumps({self.namespace: self.index}),
                            "startup.mode.copy.existing.namespace.regex": f"^connector_test\\.{token}$"})
        self.sink = json.loads(Path("/config/opensearch-sink-connector.json").read_text())
        self.sink["topics"] = self.index
        self.collection.insert_one({"_id": "before", "title": "Not indexed",
                                    "description": "A red hiking backpack"})
        self.put(self.sink_name, self.sink)
        self.put(self.source_name, self.source)
        wait_until("new namespace initial copy", lambda: self.document("before") is not None)

    def put(self, name, config):
        response = requests.put(f"{CONNECT_URL}/connectors/{name}/config", json=config, timeout=30)
        response.raise_for_status()
        wait_until(name, lambda: connector_running(name))

    def delete(self, name):
        requests.delete(f"{CONNECT_URL}/connectors/{name}", timeout=30).raise_for_status()
        wait_until("connector removed", lambda: requests.get(
            f"{CONNECT_URL}/connectors/{name}/status", timeout=10).status_code == 404)

    def document(self, key):
        response = requests.get(f"{OPENSEARCH_URL}/{self.index}/_doc/{key}", timeout=10)
        if response.status_code == 404:
            return None
        response.raise_for_status()
        return response.json()

    def cleanup(self):
        # Stop writers before removing disposable test data; never reset demo offsets.
        for name in (self.source_name, self.sink_name):
            response = requests.delete(f"{CONNECT_URL}/connectors/{name}", timeout=30)
            self.assertIn(response.status_code, (204, 404))
            wait_until("test connector removed", lambda name=name: requests.get(
                f"{CONNECT_URL}/connectors/{name}/status", timeout=10).status_code == 404)
        self.collection.drop()
        response = requests.delete(f"{OPENSEARCH_URL}/{self.index}", timeout=30)
        self.assertIn(response.status_code, (200, 404))
        self.client.close()

    def test_create_copies_existing_and_follows_changes_with_description_only(self):
        source = self.document("before")["_source"]
        self.assertEqual(set(source), {"description_embedding"})
        self.assertEqual(len(source["description_embedding"]), 384)
        self.collection.insert_one({"_id": "after", "title": "Also not indexed",
                                    "description": "A winter sleeping bag"})
        wait_until("new namespace insert", lambda: self.document("after") is not None)
        self.assertEqual(set(self.document("after")["_source"]), {"description_embedding"})
        pipeline = [
            {"$vectorSearch": {"path": "description", "query": "backpack", "limit": 1,
                               "filter": {"ids": {"values": ["before"]}}}},
            {"$set": {"score": {"$meta": "vectorSearchScore"}}}]
        wait_until("new namespace searchable", lambda: bool(list(self.collection.aggregate(pipeline))))
        result = list(self.collection.aggregate(pipeline))
        self.assertEqual(len(result), 1)
        score = result[0].pop("score")
        self.assertGreater(score, 0)
        self.assertEqual(result[0], self.collection.find_one({"_id": "before"}))

    def test_update_source_and_sink_config_keeps_sync_running(self):
        before = self.document("before")
        self.source["connection.uri"] += "&appName=connector-lifecycle-test"
        self.put(self.source_name, self.source)
        self.sink["batch.size"] = "2"
        self.put(self.sink_name, self.sink)
        for name, config in ((self.source_name, self.source), (self.sink_name, self.sink)):
            response = requests.get(f"{CONNECT_URL}/connectors/{name}/config", timeout=10)
            response.raise_for_status()
            for key, value in config.items():
                self.assertEqual(response.json()[key], value)
        self.collection.update_one({"_id": "before"}, {"$set": {
            "description": "A warm insulated winter sleeping bag"}})
        wait_until("updated connectors propagate", lambda: self.document("before")["_source"] != before["_source"])
        self.assertEqual(set(self.document("before")["_source"]), {"description_embedding"})

    def test_delete_retains_index_and_documents_but_stops_sync(self):
        self.delete(self.source_name)
        self.delete(self.sink_name)
        # DELETE completion is not an HTTP-write fence. Allow an already submitted write to settle.
        time.sleep(2)
        retained = self.document("before")
        self.assertIsNotNone(retained)
        self.collection.update_one({"_id": "before"}, {"$set": {"description": "Changed after deletion"}})
        self.collection.insert_one({"_id": "offline", "title": "Not indexed", "description": "Never sent"})
        self.collection.delete_one({"_id": "before"})
        for _ in range(5):
            self.assertEqual(self.document("before"), retained)
            self.assertIsNone(self.document("offline"))
            time.sleep(1)
        response = requests.get(f"{OPENSEARCH_URL}/{self.index}/_mapping", timeout=10)
        response.raise_for_status()
        self.assertEqual(set(response.json()[self.index]["mappings"]["properties"]), {"description_embedding"})


if __name__ == "__main__":
    unittest.main(verbosity=2)
