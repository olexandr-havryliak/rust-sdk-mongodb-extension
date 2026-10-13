"""Performance checks must fail closed and keep timing separate from correctness."""
import importlib.util
import json
from pathlib import Path
import unittest
import sys
import os
import subprocess
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench_run

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("performance_reports", ROOT / ".github/testing/performance_reports.py")
reports = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reports)


class PerformanceReportTests(unittest.TestCase):
    def report(self, passed=True):
        return {"schema_version": 1, "topology": "replica-set", "cases": [
            {"name": "source/c4", "passed": passed, "detail": "checked documents",
             "summary": {"successful": 8, "failed": 0 if passed else 1, "documents": 1024,
                         "seconds": 1.0, "queries_per_second": 8.0, "documents_per_second": 1024.0,
                         "p50_us": 10, "p95_us": 20, "p99_us": 30, "max_us": 40}}]}

    def test_successful_correctness_checks_become_junit_cases(self):
        cases = reports.load_cases(self.report())
        self.assertEqual(len(cases), 1)
        self.assertTrue(cases[0].passed)

    def test_failure_cannot_be_hidden_by_a_success_flag(self):
        value = self.report()
        value["cases"][0]["summary"]["failed"] = 1
        self.assertFalse(reports.load_cases(value)[0].passed)

    def test_empty_unknown_schema_and_invalid_quantiles_are_rejected(self):
        for value in [{}, {"schema_version": 1, "cases": []}, {"schema_version": 2, "cases": []}]:
            with self.assertRaises(ValueError):
                reports.load_cases(value)
        value = self.report()
        value["cases"][0]["summary"]["p99_us"] = 5
        with self.assertRaises(ValueError):
            reports.load_cases(value)

    def test_nonfinite_measurements_are_rejected(self):
        value = self.report()
        value["cases"][0]["summary"]["queries_per_second"] = float("nan")
        with self.assertRaises(ValueError):
            reports.load_cases(value)

    def test_boolean_is_not_a_numeric_measurement(self):
        for key in ("seconds", "queries_per_second", "documents_per_second"):
            value = self.report()
            value["cases"][0]["summary"][key] = True
            with self.assertRaises(ValueError):
                reports.load_cases(value)

    def test_nonobject_reports_are_rejected(self):
        for value in ([], None, "invalid"):
            with self.assertRaises(ValueError):
                reports.load_cases(value)


class PerformanceStackTests(unittest.TestCase):
    def test_monitor_survives_driver_removal_but_not_server_sampling_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            docker = directory / "docker"
            docker.write_text('''#!/bin/sh
case "$1" in
  inspect) echo true ;;
  stats)
    case "$*" in
      *sdk-performance-load-driver*) echo EOF >&2; exit 1 ;;
      *) if [ "$SERVER_FAIL" = 1 ]; then exit 1; fi; echo '{}' ;;
    esac ;;
  exec) echo '{"rss_kib":1,"threads":1,"fds":1}'; rm "$REPORT/monitor-active" ;;
esac
''')
            docker.chmod(0o755)
            for fails in (False, True):
                (directory / "monitor-active").touch()
                env = dict(os.environ, PATH=str(directory) + ":" + os.environ["PATH"],
                           REPORT=str(directory), SERVER_FAIL=str(int(fails)))
                result = subprocess.run(["bash", str(ROOT / ".github/testing/monitor-load.sh"),
                                         str(directory), "server"], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, not fails, result.stderr)

    def test_load_build_preserves_the_locked_workspace(self):
        dockerfile = (ROOT / "e2e-tests/performance/Dockerfile").read_text()
        self.assertIn("COPY Cargo.toml Cargo.lock", dockerfile)
        self.assertNotIn("members =", dockerfile)

    def test_instructions_allow_aslr_without_relaxing_docker_permissions(self):
        command = bench_run.benchmark_command("instructions", [], "head", True)
        self.assertIn("--allow-aslr=yes", command)
        self.assertIn("--baseline=base", command)
        self.assertNotIn("seccomp=unconfined", (ROOT / ".github/testing/bench.sh").read_text())

    def test_only_router_loads_extensions_in_sharded_stack(self):
        config = json.loads((ROOT / "e2e-tests/performance/mongos.json").read_text())
        for name in ("config", "shard0", "shard1"):
            command = config["services"][name]["command"]
            self.assertNotIn("--loadExtensions", command)
            self.assertEqual(command[command.index("--wiredTigerCacheSizeGB") + 1], "0.25")
            self.assertEqual(command[command.index("--port") + 1], "27017")
        self.assertIn("--loadExtensions", config["services"]["mongos"]["command"])

    def test_launchers_cleanup_and_have_no_latency_gate(self):
        script = (ROOT / ".github/testing/load.sh").read_text()
        self.assertIn("trap cleanup EXIT", script)
        self.assertIn("down -v --remove-orphans", script)
        self.assertIn("replica-set|mongos", script)
        self.assertNotIn("LATENCY_THRESHOLD", script)


if __name__ == "__main__":
    unittest.main()
