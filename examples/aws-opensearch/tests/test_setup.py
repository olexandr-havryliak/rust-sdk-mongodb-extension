import importlib.util
import unittest
from unittest.mock import patch
from pathlib import Path


class RemoteModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[1] / "scripts" / "setup_local.py"
        spec = importlib.util.spec_from_file_location("setup_local", path)
        cls.setup = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.setup)

    def test_connector_batches_text_without_bedrock_stub(self):
        connector = self.setup.local_connector()
        self.assertEqual(connector["protocol"], "http")
        self.assertTrue(connector["credential"])
        action = connector["actions"][0]
        self.assertEqual(action["url"], "http://embeddings:8080/embed")
        self.assertEqual(action["request_body"], "${parameters.input}")
        self.assertEqual(action["pre_process_function"], "connector.pre_process.default.embedding")
        self.assertEqual(action["post_process_function"], "connector.post_process.default.embedding")

    def test_domain_preserves_engine_family_and_single_node(self):
        domain = self.setup.domain_config()
        self.assertEqual(domain["EngineVersion"], "OpenSearch_2.19")
        self.assertEqual(domain["ClusterConfig"]["InstanceCount"], 1)

    def test_floci_uses_image_health_probe_not_missing_curl(self):
        compose = (Path(__file__).resolve().parents[1] / "docker-compose.yml").read_text()
        self.assertIn("/usr/local/bin/healthcheck.sh", compose)

    def test_private_ip_exception_is_local_and_endpoint_restricted(self):
        settings = self.setup.local_settings()["persistent"]
        self.assertIs(settings["plugins.ml_commons.connector.private_ip_enabled"], True)
        self.assertEqual(settings["plugins.ml_commons.trusted_connector_endpoints_regex"],
                         ["^http://embeddings:8080/embed$"])

    def test_repeated_setup_reuses_deployed_remote_model(self):
        response = {"hits": {"hits": [{"_id": "existing", "_source": {"model_state": "DEPLOYED"}}]}}
        with patch.object(self.setup, "request", return_value=response) as request:
            self.assertEqual(self.setup.ensure_model("http://example:9200"), "existing")
        self.assertEqual(request.call_count, 1)


if __name__ == "__main__":
    unittest.main()
