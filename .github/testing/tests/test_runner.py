"""Tests for fail-closed check execution and sanitizer controls."""

import tempfile
import unittest
from pathlib import Path
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from runner import execute, structured_cases, native_report_case, clang_runtime_directory
import runner


class RunnerTests(unittest.TestCase):
    def test_fmt_emits_logs_only_and_removes_stale_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "reports/fmt"
            report.mkdir(parents=True)
            (report / "summary.md").write_text("stale")
            (report / "checks.xml").write_text("stale")
            with patch.object(runner, "ROOT", Path(directory)), \
                    patch.object(sys, "argv", ["runner", "fmt"]), \
                    patch.object(runner, "execute", return_value=runner.Case("fmt", True)):
                with self.assertRaises(SystemExit) as stopped:
                    runner.main()
            self.assertEqual(stopped.exception.code, 0)
            self.assertFalse((report / "summary.md").exists())
            self.assertFalse((report / "checks.xml").exists())

    def test_runtime_directory_comes_from_the_actual_shared_library(self):
        with tempfile.TemporaryDirectory() as directory:
            library = Path(directory) / "libclang_rt.asan-x86_64.so"
            library.touch()
            with patch("runner.subprocess.check_output", return_value=str(library) + "\n"):
                self.assertEqual(clang_runtime_directory(), str(library.parent))

    def test_unresolved_runtime_is_a_failure(self):
        with patch("runner.subprocess.check_output", return_value="libclang_rt.asan-x86_64.so\n"):
            with self.assertRaises(FileNotFoundError):
                clang_runtime_directory()

    def test_command_failure_is_not_hidden(self):
        with tempfile.TemporaryDirectory() as directory:
            case = execute("bad command", [sys.executable, "-c", "print('diagnostic'); exit(7)"],
                           Path(directory) / "command.log")
        self.assertFalse(case.passed)
        self.assertIn("diagnostic", case.output)
        self.assertIn("exit 7", case.output)

    def test_expected_failure_requires_the_sanitizer_diagnostic(self):
        with tempfile.TemporaryDirectory() as directory:
            case = execute("negative control", [sys.executable, "-c", "exit(1)"],
                           Path(directory) / "command.log", expected="AddressSanitizer")
        self.assertFalse(case.passed)

    def test_invalid_json_is_a_failure(self):
        cases = structured_cases("audit", "not json", lambda _: [])
        self.assertFalse(cases[0].passed)

    def test_missing_native_report_is_a_failure(self):
        self.assertFalse(native_report_case(Path("/does-not-exist/report.xml")).passed)

    def test_native_test_failures_are_not_hidden(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuites><testsuite tests="1" failures="1"><testcase><failure>bad</failure></testcase></testsuite></testsuites>')
            self.assertFalse(native_report_case(path).passed)

    def test_native_failure_counts_are_not_hidden(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuite tests="1" failures="1" errors="0"/>')
            self.assertFalse(native_report_case(path).passed)


if __name__ == "__main__":
    unittest.main()
