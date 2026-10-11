"""Terraform Docker entrypoint resolves region without overriding explicit inputs."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[3] / "examples/aws-opensearch/terraform/entrypoint.sh"


class TerraformEntrypointTests(unittest.TestCase):
    def region(self, variables):
        with tempfile.TemporaryDirectory() as directory:
            terraform = Path(directory) / "terraform"
            terraform.write_text('#!/bin/sh\nprintf "%s\\n" "$TF_VAR_aws_region" "$@"\n')
            terraform.chmod(0o755)
            environment = {"PATH": f"{directory}:{os.environ['PATH']}", **variables}
            result = subprocess.run(["sh", str(SCRIPT), "plan", "-out=demo.tfplan"],
                                    env=environment, capture_output=True, text=True, check=True)
            lines = result.stdout.splitlines()
            self.assertEqual(lines[1:], ["plan", "-out=demo.tfplan"])
            return lines[0]

    def test_default_region(self):
        self.assertEqual(self.region({}), "us-east-1")

    def test_standard_aws_region(self):
        self.assertEqual(self.region({"AWS_DEFAULT_REGION": "eu-west-1"}), "eu-west-1")

    def test_explicit_terraform_region_has_priority(self):
        self.assertEqual(self.region({"AWS_DEFAULT_REGION": "eu-west-1", "TF_VAR_aws_region": "eu-central-1"}), "eu-central-1")

    def test_empty_region_uses_default(self):
        self.assertEqual(self.region({"AWS_DEFAULT_REGION": "", "TF_VAR_aws_region": ""}), "us-east-1")


if __name__ == "__main__":
    unittest.main()
