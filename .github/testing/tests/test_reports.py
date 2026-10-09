"""Tests for JUnit adapters used by CI and local Docker checks."""

import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from reports import Case, write_junit, diagnostic_cases, audit_cases


class ReportTests(unittest.TestCase):
    def test_failure_preserves_diagnostics_and_escapes_xml(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.xml"
            write_junit(path, "ffi", [Case("build", False, "bad <pointer> & error\x00", 0.5)])
            suite = ET.parse(path).getroot()
            self.assertEqual(suite.attrib["failures"], "1")
            self.assertEqual(suite.find("testcase/failure").text, "bad <pointer> & error")

    def test_empty_report_is_not_success(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                write_junit(Path(directory) / "empty.xml", "empty", [])

    def test_clippy_includes_file_and_line(self):
        cases = diagnostic_cases([{"reason": "compiler-message", "message": {
            "level": "error", "message": "unsafe operation", "code": {"code": "clippy::test"},
            "spans": [{"file_name": "src/lib.rs", "line_start": 12, "is_primary": True}],
            "rendered": "unsafe operation at src/lib.rs:12",
        }}])
        self.assertEqual(len(cases), 1)
        self.assertFalse(cases[0].passed)
        self.assertIn("src/lib.rs:12", cases[0].name)

    def test_audit_reports_advisory_and_package(self):
        cases = audit_cases({"vulnerabilities": {"list": [{
            "advisory": {"id": "RUSTSEC-test", "title": "unsafe dependency"},
            "package": {"name": "example", "version": "1.0.0"},
        }]}, "warnings": {}})
        self.assertEqual(len(cases), 1)
        self.assertFalse(cases[0].passed)
        self.assertIn("RUSTSEC-test", cases[0].name)

    def test_audit_clean_report_is_success(self):
        self.assertTrue(audit_cases({"vulnerabilities": {"list": []}, "warnings": {}})[0].passed)


if __name__ == "__main__":
    unittest.main()
