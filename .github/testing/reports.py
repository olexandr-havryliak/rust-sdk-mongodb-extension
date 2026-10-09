"""JUnit adapters for checks that do not natively emit test reports."""

from dataclasses import dataclass
from pathlib import Path
import json
import xml.etree.ElementTree as ET


@dataclass
class Case:
    name: str
    passed: bool
    output: str = ""
    seconds: float = 0.0


def xml_text(value):
    return "".join(c for c in str(value) if c in "\t\n\r" or
                   0x20 <= ord(c) <= 0xD7FF or 0xE000 <= ord(c) <= 0xFFFD or
                   0x10000 <= ord(c) <= 0x10FFFF)


def write_junit(path, name, cases):
    if not cases:
        raise ValueError("an empty report cannot establish success")
    suite = ET.Element("testsuite", name=name, tests=str(len(cases)),
                       failures=str(sum(not c.passed for c in cases)), errors="0",
                       time=str(sum(c.seconds for c in cases)))
    for case in cases:
        test = ET.SubElement(suite, "testcase", name=xml_text(case.name), classname=name,
                             time=str(case.seconds))
        if not case.passed:
            ET.SubElement(test, "failure", message="check failed").text = xml_text(case.output)
        ET.SubElement(test, "system-out").text = xml_text(case.output)
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(suite).write(path, encoding="utf-8", xml_declaration=True)


def diagnostic_cases(messages):
    cases = []
    for entry in messages:
        if entry.get("reason") != "compiler-message":
            continue
        message = entry["message"]
        if message["level"] not in ("error", "warning"):
            continue
        span = next((s for s in message["spans"] if s.get("is_primary")), {})
        code = (message.get("code") or {}).get("code", "compiler")
        cases.append(Case(f"{code}: {span.get('file_name', '?')}:{span.get('line_start', 0)}",
                          False, message.get("rendered") or message["message"]))
    return cases


def audit_cases(report):
    cases = []
    for finding in report["vulnerabilities"]["list"]:
        advisory, package = finding["advisory"], finding["package"]
        cases.append(Case(f"{advisory['id']}: {package['name']} {package['version']}",
                          False, json.dumps(finding, indent=2)))
    for kind, warnings in report.get("warnings", {}).items():
        for finding in warnings:
            cases.append(Case(f"{kind}: {finding['package']['name']}", False,
                              json.dumps(finding, indent=2)))
    return cases or [Case("dependency audit", True)]
