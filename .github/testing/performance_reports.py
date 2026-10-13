"""Publish correctness verdicts as JUnit; preserve performance numbers as JSON/CSV."""
import argparse
import csv
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from reports import Case, write_junit


def load_cases(report):
    if not isinstance(report, dict) or report.get("schema_version") != 1 or not report.get("cases"):
        raise ValueError("missing cases or unsupported performance report schema")
    cases = []
    names = set()
    for entry in report["cases"]:
        name = entry["name"]
        if not isinstance(name, str) or not name or name in names or type(entry["passed"]) is not bool:
            raise ValueError("invalid or duplicate case")
        names.add(name)
        summary = entry.get("summary")
        passed = entry["passed"]
        if summary is not None:
            for key in ("successful", "failed", "documents", "p50_us", "p95_us", "p99_us", "max_us"):
                if type(summary[key]) is not int or summary[key] < 0:
                    raise ValueError(f"invalid {key}")
            for key in ("seconds", "queries_per_second", "documents_per_second"):
                if type(summary[key]) not in (int, float) or not math.isfinite(summary[key]) or summary[key] < 0:
                    raise ValueError(f"invalid {key}")
            if summary["seconds"] <= 0 or summary["successful"] + summary["failed"] == 0:
                raise ValueError("empty measurements")
            if not summary["p50_us"] <= summary["p95_us"] <= summary["p99_us"] <= summary["max_us"]:
                raise ValueError("invalid percentile ordering")
            passed = passed and summary["failed"] == 0
        cases.append(Case(name, passed, json.dumps(entry, indent=2),
                          summary["seconds"] if summary is not None else 0.0))
    return cases


def publish(directory):
    directory = Path(directory)
    try:
        report = json.loads((directory / "load.json").read_text())
        cases = load_cases(report)
        rows = [{"case": entry["name"], **entry["summary"]} for entry in report["cases"] if entry.get("summary")]
        if rows:
            with (directory / "measurements.csv").open("w", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
    except (OSError, ValueError, KeyError, TypeError) as error:
        cases = [Case("load report integrity", False, str(error))]
    write_junit(directory / "checks.xml", "SDK load", cases)
    return all(case.passed for case in cases)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory")
    parser.add_argument("--setup-failure")
    args = parser.parse_args()
    if args.setup_failure:
        write_junit(Path(args.directory) / "setup.xml", "SDK performance setup",
                    [Case("Docker performance setup/run", False, args.setup_failure)])
        raise SystemExit(1)
    raise SystemExit(0 if publish(args.directory) else 1)
