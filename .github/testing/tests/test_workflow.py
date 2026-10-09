"""Contracts for independent GitHub CI gates and shared local entrypoints."""

from pathlib import Path
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[3]
CHECKS = ("fmt", "clippy", "tests", "audit", "abi", "asan")
ALL_CHECKS = (*CHECKS, "sdk", "e2e", "miri")


def load(path):
    return yaml.load(path.read_text(), Loader=yaml.BaseLoader)


class WorkflowTests(unittest.TestCase):
    def test_six_independent_workflows_use_local_entrypoints(self):
        self.assertEqual(
            {path.stem for path in (ROOT / ".github/workflows").glob("*.yml")},
            {*CHECKS, "e2e", "miri"},
        )
        for check in CHECKS:
            with self.subTest(check=check):
                workflow = load(ROOT / f".github/workflows/{check}.yml")
                self.assertEqual(workflow["permissions"], {"contents": "read"})
                self.assertEqual(set(workflow["on"]), {"push", "pull_request", "workflow_dispatch"})
                self.assertEqual(workflow["on"]["push"]["branches"], ["main"])
                self.assertEqual(workflow["on"]["pull_request"]["branches"], ["main"])
                job = workflow["jobs"]["check"]
                self.assertEqual(job["runs-on"], "ubuntu-24.04")
                self.assertNotIn("strategy", job)
                self.assertNotIn("continue-on-error", job)
                self.assertLessEqual(int(job["timeout-minutes"]), 60)
                action = job["steps"][1]
                self.assertEqual(action["uses"], "./.github/actions/run-check")
                self.assertEqual(action["with"]["check"], check)
                self.assertIn(f'run-docker.sh" {check}', (ROOT / f".github/testing/{check}.sh").read_text())

    def test_ready_junit_publisher_is_read_only_and_runs_after_failure(self):
        action = load(ROOT / ".github/actions/run-check/action.yml")
        steps = {step["id"]: step for step in action["runs"]["steps"]}
        publisher = steps["junit"]
        self.assertEqual(publisher["uses"], "EnricoMi/publish-unit-test-result-action@v2")
        self.assertIn("!cancelled()", publisher["if"])
        self.assertIn("inputs.check != 'fmt'", publisher["if"])
        self.assertEqual(publisher["with"]["check_run"], "false")
        self.assertEqual(publisher["with"]["comment_mode"], "off")
        self.assertEqual(publisher["with"]["compare_to_earlier_commit"], "false")
        self.assertEqual(publisher["with"]["action_fail"], "true")
        self.assertEqual(publisher["with"]["action_fail_on_inconclusive"], "true")
        self.assertEqual(steps["toolchain"]["env"]["TESTING_IMAGE"], "rust-sdk-testing:ci")
        self.assertEqual(steps["check"]["env"]["TESTING_BUILD_IMAGE"], "0")
        self.assertEqual(steps["artifacts"]["with"]["name"], "sdk-testing-${{ inputs.check }}")
        self.assertEqual(steps["artifacts"]["if"], "${{ always() }}")
        for step in steps.values():
            self.assertNotIn("continue-on-error", step)
            self.assertNotIn("GITHUB_STEP_SUMMARY", step.get("run", ""))

    def test_obsolete_infrastructure_is_removed(self):
        for path in ("e2e-tests/abi-harness/Cargo.toml",
                     ".config/nextest.toml"):
            self.assertFalse((ROOT / path).exists(), path)
        self.assertIn('".github/testing/abi-harness"', (ROOT / "Cargo.toml").read_text())

    def test_readme_links_central_testing_document(self):
        readme = (ROOT / "README.md").read_text()
        self.assertIn("(TESTING.md)", readme)
        self.assertNotIn("e2e-tests/", readme)
        testing = (ROOT / "TESTING.md").read_text()
        self.assertIn("bash ./run-workflows-local.sh", testing)
        self.assertIn("run-sdk-tests-docker.sh", testing)

    def test_legacy_checks_also_bound_container_resources(self):
        for script in ("run-sdk-tests-docker.sh", "run-miri-docker.sh"):
            self.assertIn("--cpus 2", (ROOT / "e2e-tests" / script).read_text())
        dockerfile = (ROOT / "e2e-tests/Dockerfile").read_text()
        command = json.loads(next(line[4:] for line in dockerfile.splitlines()
                                  if line.startswith("CMD ")))
        position = command.index("--wiredTigerCacheSizeGB")
        self.assertEqual(command[position + 1], "0.25")
        compose = load(ROOT / "e2e-tests/docker-compose.yml")
        self.assertEqual(compose["services"]["fuzz"]["environment"]["CARGO_BUILD_JOBS"], "2")
        for service in ("mongo", "fuzz"):
            self.assertEqual(compose["services"][service]["cpus"], "2")


class LocalWorkflowTests(unittest.TestCase):
    def run_all(self, failing=None, build_failure=False):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            log = directory / "calls"
            shutil.copyfile(ROOT / "run-workflows-local.sh", directory / "run-workflows-local.sh")
            scripts = directory / ".github/testing"
            scripts.mkdir(parents=True)
            (scripts / "build-image.sh").write_text(
                'printf "build\\n" >> "$CALLS"\n'
                'if [[ "$BUILD_FAILURE" == 1 ]]; then exit 19; fi\n')
            for check in ALL_CHECKS:
                (scripts / f"{check}.sh").write_text(
                    f'printf "{check}\\n" >> "$CALLS"\n'
                    '[[ "$TESTING_BUILD_IMAGE" == 0 ]] || exit 23\n'
                    f'if [[ "$FAIL_CHECK" == "{check}" ]]; then exit 7; fi\n')
            env = dict(os.environ, PATH=str(directory) + ":" + os.environ["PATH"],
                       CALLS=str(log), FAIL_CHECK=failing or "none",
                       BUILD_FAILURE="1" if build_failure else "0", TESTING_BUILD_IMAGE="1")
            result = subprocess.run(["bash", str(directory / "run-workflows-local.sh")],
                                    cwd=directory, env=env, capture_output=True, text=True)
            calls = log.read_text().splitlines() if log.exists() else []
            return result, calls

    def test_build_once_then_run_all_checks_in_order(self):
        result, calls = self.run_all()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, ["build", *ALL_CHECKS])

    def test_failure_does_not_skip_later_checks_or_return_success(self):
        result, calls = self.run_all(failing="clippy")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, ["build", *ALL_CHECKS])

    def test_build_failure_stops_before_any_container_run(self):
        result, calls = self.run_all(build_failure=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, ["build"])

    def test_legacy_workflows_share_local_wrappers(self):
        e2e = load(ROOT / ".github/workflows/e2e.yml")
        self.assertIn(".github/testing/sdk.sh", e2e["jobs"]["sdk-tests"]["steps"][1]["run"])
        self.assertIn(".github/testing/e2e.sh", e2e["jobs"]["docker-e2e"]["steps"][1]["run"])
        miri = load(ROOT / ".github/workflows/miri.yml")
        self.assertIn(".github/testing/miri.sh", miri["jobs"]["miri-sdk"]["steps"][1]["run"])
        script = (ROOT / ".github/testing/e2e.sh").read_text()
        self.assertIn("trap cleanup EXIT", script)
        self.assertIn("down -v --remove-orphans", script)


if __name__ == "__main__":
    unittest.main()
