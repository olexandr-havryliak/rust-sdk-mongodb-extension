"""Examples must not read files or documentation from sibling examples."""
import re
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
EXAMPLES = ROOT / "examples"


class ExampleIsolationTests(unittest.TestCase):
    def test_examples_have_no_sibling_file_references(self):
        names = sorted(path.name for path in EXAMPLES.iterdir() if path.is_dir())
        violations = []
        for name in names:
            for path in (EXAMPLES / name).rglob("*"):
                if not path.is_file() or any(part in (".terraform", "__pycache__", "target") for part in path.parts):
                    continue
                if path.suffix not in (".md", ".toml", ".yml", ".yaml", ".py", ".sh", ".rs", ".json") and not path.name.startswith("Dockerfile"):
                    continue
                text = path.read_text()
                for other in names:
                    if other == name:
                        continue
                    pattern = rf"(?:examples/|\.\./){re.escape(other)}(?:/|[\"')\s])"
                    if re.search(pattern, text):
                        violations.append(f"{path.relative_to(ROOT)} references {other}")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_shared_core_lives_outside_examples(self):
        self.assertTrue((ROOT / "opensearch-extension-core/Cargo.toml").is_file())
        self.assertFalse((EXAMPLES / "opensearch/extension-core").exists())

    def test_core_mongodb_module_is_used_by_both_wrappers(self):
        core = ROOT / "opensearch-extension-core"
        self.assertTrue((core / "src/mongodb.rs").is_file())
        self.assertFalse((core / "src/replica_set.rs").exists())
        self.assertIn("pub mod mongodb;", (core / "src/lib.rs").read_text())
        for wrapper in ("opensearch/extension", "aws-opensearch/extensions/replica-set"):
            source = (EXAMPLES / wrapper / "src/lib.rs").read_text()
            self.assertIn("opensearch_extension_core::mongodb::", source)
            self.assertNotIn("opensearch_extension_core::replica_set::", source)
        readme = (core / "README.md").read_text()
        self.assertIn("src/mongodb.rs", readme)
        self.assertNotIn("src/replica_set.rs", readme)

    def test_aws_has_its_own_extension_packages_and_resources(self):
        example = EXAMPLES / "aws-opensearch"
        for name in ("replica-set", "mongos"):
            self.assertTrue((example / "extensions" / name / "Cargo.toml").is_file())
            self.assertTrue((example / "config" / name / "opensearch-index-template.json").is_file())
        self.assertTrue((example / "datasets/outdoor-products.json").is_file())

    def test_tools_image_does_not_copy_terraform_state_or_provider_cache(self):
        dockerfile = (EXAMPLES / "aws-opensearch/Dockerfile.tools").read_text()
        self.assertNotIn("COPY examples/aws-opensearch /demo", dockerfile)
        ignored = (ROOT / ".dockerignore").read_text().splitlines()
        for pattern in ("**/.terraform", "**/*.tfstate", "**/*.tfstate.*", "**/*.tfvars", "**/*.tfvars.json"):
            self.assertIn(pattern, ignored)

    def test_example_documentation_links_resolve(self):
        missing = []
        for path in EXAMPLES.rglob("*.md"):
            if ".terraform" in path.parts:
                continue
            for target in re.findall(r"\]\(([^)]+)\)", path.read_text()):
                if "://" in target or target.startswith("#"):
                    continue
                target = target.split("#", 1)[0]
                if not (path.parent / target).exists():
                    missing.append(f"{path.relative_to(ROOT)}: {target}")
        self.assertEqual(missing, [], "\n".join(missing))

    def test_small_demo_builds_and_mongod_bound_resources(self):
        for example in ("fibonacci", "http-fetch", "data-federation"):
            dockerfile = (EXAMPLES / example / "Dockerfile").read_text()
            self.assertIn("ENV CARGO_BUILD_JOBS=2", dockerfile)
            command = json.loads(next(line[4:] for line in dockerfile.splitlines()
                                      if line.startswith("CMD ")))
            self.assertEqual(command[command.index("--wiredTigerCacheSizeGB") + 1], "0.25")

    def test_aws_topologies_build_own_libraries_and_preserve_image_override(self):
        example = EXAMPLES / "aws-opensearch"
        for topology, service in (("replica-set", "mongo"), ("mongos", "mongos")):
            compose = json.loads((example / f"{topology}.yml").read_text())
            build = compose["services"][service]["build"]
            self.assertEqual(build["dockerfile"], "examples/aws-opensearch/Dockerfile.mongo")
            self.assertEqual(build["target"], topology)
            self.assertEqual(build["args"]["MONGO_IMAGE"],
                             "${MONGO_IMAGE:-mongodb/mongodb-community-server:9.0-ubi9}")

    def test_aws_mongo_image_assigns_data_directories_to_runtime_user(self):
        dockerfile = (EXAMPLES / "aws-opensearch/Dockerfile.mongo").read_text()
        self.assertIn("chown -R 999:999 /data/db /data/configdb", dockerfile)

    def test_terraform_documents_standard_credential_exports(self):
        terraform = EXAMPLES / "aws-opensearch/terraform"
        readme = (terraform / "README.md").read_text()
        environment = (terraform / "docker-compose.yml").read_text().splitlines()
        for variable in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"):
            self.assertIn(f"export {variable}='YOUR_{variable.removeprefix('AWS_')}'", readme)
            self.assertIn(f"      - {variable}", environment)
        self.assertNotIn("read -r", readme)

    def test_federation_e2e_instructions_use_its_own_demo(self):
        readme = (EXAMPLES / "data-federation/README.md").read_text()
        self.assertNotIn("./e2e-tests/run-e2e.sh", readme)
        self.assertIn("scripts/demo.js", readme)


if __name__ == "__main__":
    unittest.main()
