import importlib.util
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory


class ConfigurationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[1] / "scripts" / "configure.py"
        spec = importlib.util.spec_from_file_location("configure", path)
        cls.config = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.config)

    def test_replica_set_key_and_projection_are_unchanged(self):
        sink, sources = self.config.connector_configs("replica-set", "http://example:9200")
        self.assertEqual(sink["transforms.key.field"], "_id")
        self.assertEqual(sink["transforms.fields.include"], "description")
        self.assertEqual(sources["mongo-products-source"]["startup.mode"], "copy_existing")
        self.assertNotIn("aws.region", sink)

    def test_mongos_preserves_composite_key_and_all_namespaces(self):
        sink, sources = self.config.connector_configs("mongos", "http://example:9200")
        self.assertEqual(sink["key.converter"], "org.apache.kafka.connect.storage.StringConverter")
        self.assertEqual(len(sources), 5)
        self.assertEqual(sink["batch.size"], "1")
        self.assertTrue(all(config["connection.uri"] == "mongodb://mongos:27017" for config in sources.values()))

    def test_aws_sink_uses_hostname_and_environment_credentials(self):
        sink, _ = self.config.connector_configs("replica-set", "https://search-demo.us-east-1.es.amazonaws.com", aws_region="us-east-1")
        self.assertEqual(sink["connection.url"], "search-demo.us-east-1.es.amazonaws.com")
        self.assertEqual(sink["aws.region"], "us-east-1")
        self.assertEqual(sink["aws.service.signing.name"], "es")
        self.assertEqual(sink["aws.secret_access_key"], "${env:AWS_SECRET_ACCESS_KEY}")
        with self.assertRaises(ValueError):
            self.config.connector_configs("replica-set", "http://example", aws_region="us-east-1")

    def test_mapping_and_validation_follow_remote_model_dimensions(self):
        ingest, search, template = self.config.pipeline_configs("mongos", "model-test", 512)
        self.assertEqual(ingest["processors"][1]["text_embedding"]["model_id"], "model-test")
        self.assertIn("vector.size() != 512", ingest["processors"][2]["script"]["source"])
        self.assertEqual(search["request_processors"][0]["neural_query_enricher"]["default_model_id"], "model-test")
        self.assertEqual(template["template"]["mappings"]["dynamic_templates"][0]["embeddings"]["mapping"]["dimension"], 512)
        self.assertEqual(template["index_patterns"], ["mongodb.*"])

    def test_aws_assumed_role_does_not_embed_basic_credentials(self):
        sink, _ = self.config.connector_configs("mongos", "https://search-demo.example", aws_region="us-east-1", role_arn="arn:aws:iam::123456789012:role/writer")
        self.assertEqual(sink["aws.sts.role.arn"], "arn:aws:iam::123456789012:role/writer")
        self.assertEqual(sink["aws.sts.role.session.name"], "mongodb-opensearch-demo")
        self.assertNotIn("aws.access_key_id", sink)
        self.assertNotIn("aws.secret_access_key", sink)

    def test_extension_manifests_keep_both_topology_specific_libraries(self):
        with TemporaryDirectory() as tmp:
            state = Path(tmp)
            self.config.write_extension_configs(state, "https://search-demo.example", "mongos")
            self.assertEqual((state / "topology").read_text(), "mongos")
            for name in ("opensearch", "opensearch_mongos"):
                text = (state / f"{name}.conf").read_text()
                self.assertIn(f"libaws_{name}_extension.so", text)
                self.assertIn("endpoints: https://search-demo.example", text)
                self.assertNotIn("secret", text)

    def test_invalid_topology_is_rejected(self):
        with self.assertRaises(ValueError):
            self.config.connector_configs("typo", "http://example:9200")

    def test_both_topologies_read_only_example_owned_config(self):
        root = Path(__file__).resolve().parents[1]
        for topology in ("replica-set", "mongos"):
            self.assertEqual(self.config.directory(topology), root / "config" / topology)
            self.assertTrue(self.config.directory(topology).is_dir())


if __name__ == "__main__":
    unittest.main()
