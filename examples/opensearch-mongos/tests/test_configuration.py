import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ConfigurationTests(unittest.TestCase):
    def test_unsharded_source_keeps_only_typed_id_and_sink_subscribes(self):
        source = json.loads((ROOT / "config" / "mongo-unsharded-source.json").read_text())
        self.assertEqual(source["connection.uri"], "mongodb://mongos:27017")
        self.assertEqual(source["collection"], "unsharded")
        self.assertEqual(source["startup.mode"], "copy_existing")
        self.assertEqual(source["change.stream.document.key.as.key"], "true")
        self.assertEqual(source["publish.full.document.only.tombstone.on.delete"], "true")
        self.assertTrue(source["output.json.formatter"].endswith(".ExtendedJson"))
        self.assertEqual(json.loads(source["pipeline"]), [
            {"$set": {"documentKey": {"_id": "$documentKey._id"}}}
        ])
        sink = json.loads((ROOT / "config" / "opensearch-sink.json").read_text())
        self.assertIn("mongodb.search_demo.unsharded", sink["topics"].split(","))

    def test_mongod_cache_is_minimal_and_jvm_heap_is_unchanged(self):
        services = json.loads((ROOT / "docker-compose.yml").read_text())["services"]
        for name in ("config", "shard0", "shard1"):
            command = services[name]["command"]
            self.assertEqual(command[command.index("--wiredTigerCacheSizeGB") + 1], "0.25")
        self.assertNotIn("--wiredTigerCacheSizeGB", services["mongos"]["command"])
        self.assertEqual(services["opensearch"]["environment"]["OPENSEARCH_JAVA_OPTS"], "-Xms2g -Xmx2g")

    def test_typed_full_key_and_initial_copy_use_same_source_pipeline(self):
        for name in ("range", "hashed", "compound"):
            config = json.loads((ROOT / "config" / f"mongo-{name}-source.json").read_text())
            self.assertEqual(config["connection.uri"], "mongodb://mongos:27017")
            self.assertEqual(config["startup.mode"], "copy_existing")
            self.assertEqual(config["change.stream.document.key.as.key"], "true")
            self.assertEqual(config["publish.full.document.only.tombstone.on.delete"], "true")
            self.assertTrue(config["output.json.formatter"].endswith(".ExtendedJson"))
            pipeline = json.loads(config["pipeline"])
            self.assertIn("documentKey", pipeline[0]["$set"])
            self.assertIn("$arrayToObject", pipeline[0]["$set"]["documentKey"])
            self.assertEqual(len(pipeline), 1, "metadata is generated after embedding, not by the source")
            self.assertNotIn("__mongodb", config["pipeline"])

    def test_sink_uses_entire_key_and_keeps_only_embedding_inputs(self):
        sink = json.loads((ROOT / "config" / "opensearch-sink.json").read_text())
        self.assertEqual(sink["key.converter"], "org.apache.kafka.connect.storage.StringConverter")
        self.assertEqual(sink["key.ignore"], "false")
        self.assertEqual(sink["behavior.on.null.values"], "delete")
        self.assertEqual(sink["errors.tolerance"], "none")
        self.assertEqual(sink["transforms.fields.include"], "description")
        self.assertNotIn("transforms.key.type", sink)
        self.assertEqual(sink["max.in.flight.requests"], "1")

    def test_metadata_is_not_indexed_or_embedded(self):
        template = json.loads((ROOT / "config" / "opensearch-index-template.json").read_text())
        self.assertEqual(template["index_patterns"], ["mongodb.*"])
        self.assertFalse(template["template"]["mappings"]["properties"]["__mongodb"]["enabled"])
        pipeline = json.loads((ROOT / "config" / "opensearch-ingest-pipeline.json").read_text())
        self.assertIn("ctx.remove('__mongodb')", pipeline["processors"][0]["script"]["source"])
        self.assertIn("ctx.__mongodb = ['documentKey': ctx._id]", pipeline["processors"][-1]["script"]["source"])


if __name__ == "__main__":
    unittest.main()
