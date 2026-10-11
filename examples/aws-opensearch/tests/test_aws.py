import importlib.util
import json
import os
import subprocess
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from botocore.credentials import Credentials


class AwsSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("setup_aws", Path(__file__).resolve().parents[1] / "scripts" / "setup_aws.py")
        cls.setup = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.setup)

    def test_titan_v2_uses_role_and_builtin_embedding_functions(self):
        connector = self.setup.bedrock_connector("us-east-1", "arn:aws:iam::123456789012:role/model")
        self.assertEqual(connector["credential"], {"roleArn": "arn:aws:iam::123456789012:role/model"})
        self.assertEqual(connector["parameters"]["dimensions"], 512)
        self.assertEqual(connector["parameters"]["model"], "amazon.titan-embed-text-v2:0")
        self.assertEqual(connector["actions"][0]["pre_process_function"], "connector.pre_process.bedrock.embedding")
        self.assertNotIn("private_ip_enabled", json.dumps(connector))

    def test_provisioned_domain_is_single_node_encrypted_and_principal_restricted(self):
        config = self.setup.provision_request("us-east-1", "123456789012", "mongodb-demo", "arn:aws:iam::123456789012:role/client")
        self.assertEqual(config["ClusterConfig"]["InstanceCount"], 1)
        self.assertTrue(config["DomainEndpointOptions"]["EnforceHTTPS"])
        self.assertTrue(config["EncryptionAtRestOptions"]["Enabled"])
        policy = json.loads(config["AccessPolicies"])
        self.assertNotEqual(policy["Statement"][0]["Principal"], "*")
        self.assertEqual(policy["Statement"][0]["Resource"], "arn:aws:es:us-east-1:123456789012:domain/mongodb-demo/*")

    def test_existing_endpoint_must_be_https_origin(self):
        self.assertEqual(self.setup.validate_endpoint("https://search-demo.us-east-1.es.amazonaws.com/"), "https://search-demo.us-east-1.es.amazonaws.com")
        for endpoint in ("http://localhost:9200", "https://user:secret@example", "https://example/_search", "https://example?query=1"):
            with self.assertRaises(ValueError):
                self.setup.validate_endpoint(endpoint)

    def test_signed_http_uses_exact_body_session_token_and_no_redirects(self):
        session = Mock()
        session.get_credentials.return_value = Credentials("test-key", "test-secret", "test-token")
        response = Mock(status_code=200, content=b"{}")
        response.json.return_value = {"ok": True}
        with patch("requests.request", return_value=response) as send:
            result = self.setup.signed_request(session, "us-east-1", "https://example", "POST", "/_search", {"size": 3})
        self.assertEqual(result, {"ok": True})
        self.assertEqual(send.call_args.kwargs["data"], b'{"size": 3}')
        self.assertIn("/us-east-1/es/aws4_request", send.call_args.kwargs["headers"]["Authorization"])
        self.assertEqual(send.call_args.kwargs["headers"]["X-Amz-Security-Token"], "test-token")
        self.assertFalse(send.call_args.kwargs["allow_redirects"])
        self.assertEqual(send.call_args.kwargs["timeout"], 30)

    def test_http_errors_do_not_echo_credentials_or_response_body(self):
        session = Mock()
        session.get_credentials.return_value = Credentials("test-key", "do-not-print")
        with patch("requests.request", return_value=Mock(status_code=403, text="do-not-print")):
            with self.assertRaisesRegex(RuntimeError, "HTTP 403") as caught:
                self.setup.signed_request(session, "us-east-1", "https://example", "POST", "/_search", {})
        self.assertNotIn("do-not-print", str(caught.exception))

    def test_local_launcher_never_forwards_ambient_aws_credentials(self):
        env = dict(os.environ, AWS_MODE="false", AWS_ACCESS_KEY_ID="do-not-use", AWS_SECRET_ACCESS_KEY="do-not-use", AWS_SESSION_TOKEN="do-not-use", DEMO_SCRIPT=str(Path(__file__).resolve().parents[1] / "run-demo.sh"))
        result = subprocess.run(["bash", "-c", 'set -- help; source "$DEMO_SCRIPT"; [[ "$AWS_ACCESS_KEY_ID" == test && "$AWS_SECRET_ACCESS_KEY" == test && -z "$AWS_SESSION_TOKEN" && "$OPENSEARCH_AWS_ALLOW_HTTP" == true ]]'], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_cloud_launcher_preserves_credentials_and_forces_https(self):
        env = dict(os.environ, AWS_MODE="true", AWS_DEFAULT_REGION="eu-west-1", AWS_ACCESS_KEY_ID="cloud-key", AWS_SECRET_ACCESS_KEY="cloud-secret", AWS_SESSION_TOKEN="cloud-token", OPENSEARCH_AWS_ALLOW_HTTP="true", DEMO_SCRIPT=str(Path(__file__).resolve().parents[1] / "run-demo.sh"))
        result = subprocess.run(["bash", "-c", 'set -- help; source "$DEMO_SCRIPT"; [[ "$AWS_ACCESS_KEY_ID" == cloud-key && "$AWS_SECRET_ACCESS_KEY" == cloud-secret && "$AWS_SESSION_TOKEN" == cloud-token && "$OPENSEARCH_AWS_REGION" == eu-west-1 && "$OPENSEARCH_AWS_ALLOW_HTTP" == false ]]'], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
