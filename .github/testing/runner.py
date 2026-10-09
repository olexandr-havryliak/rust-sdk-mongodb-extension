"""Run shared CI/local Docker checks; GitHub's action publishes the JUnit."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import traceback
import xml.etree.ElementTree as ET

from reports import Case, audit_cases, diagnostic_cases, write_junit

ROOT = Path(__file__).resolve().parents[2]
PACKAGES = ["-p", "extension-sdk-mongodb", "-p", "extension-sys-mongodb", "-p", "sdk_abi_harness"]
NIGHTLY = "+nightly-2026-10-01"
TARGET = "x86_64-unknown-linux-gnu"
# nextest's default store is workspace-relative, independent of CARGO_TARGET_DIR.
NEXTEST_REPORT = ROOT / "target/nextest/ci/junit.xml"


def clang_runtime_directory():
    library = Path(subprocess.check_output(
        ["clang", "--print-file-name=libclang_rt.asan-x86_64.so"], text=True).strip())
    if not library.is_absolute() or not library.is_file():
        raise FileNotFoundError(f"Clang shared ASan runtime is unavailable: {library}")
    return str(library.parent)


def execute(name, command, log, *, expected=None, env=None):
    started = time.monotonic()
    try:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, errors="replace", check=False)
        output = result.stdout
        passed = result.returncode == 0 if expected is None else (
            result.returncode != 0 and expected in output)
        output += f"\nexit {result.returncode}\n"
    except OSError as error:
        passed, output = False, str(error)
    log = Path(log)
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text(output)
    print(f"{'PASS' if passed else 'FAIL'} {name}", flush=True)
    if not passed:
        print(output, flush=True)
    return Case(name, passed, output, time.monotonic() - started)


def structured_cases(name, output, adapter):
    try:
        return adapter(json.loads(output))
    except (ValueError, KeyError, TypeError) as error:
        return [Case(f"{name} report", False, f"invalid structured output: {error}\n{output}")]


def native_report_case(path):
    try:
        root = ET.parse(path).getroot()
        suites = [root] if root.tag == "testsuite" else list(root.iter("testsuite"))
        count = sum(int(suite.attrib.get("tests", "0")) for suite in suites)
        failed = next(root.iter("failure"), None) is not None or next(root.iter("error"), None) is not None
        failed = failed or any(int(suite.attrib.get("failures", "0")) > 0 or
                               int(suite.attrib.get("errors", "0")) > 0 for suite in suites)
        return Case("native JUnit report", count > 0 and not failed, Path(path).read_text())
    except (OSError, ValueError, ET.ParseError) as error:
        return Case("native JUnit report", False, str(error))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("check", choices=["fmt", "clippy", "audit", "tests", "abi", "asan"])
    args = parser.parse_args()
    report = ROOT / "reports" / args.check
    report.mkdir(parents=True, exist_ok=True)
    for stale in ("summary.md", "checks.xml", "nextest.xml", "setup-error.log"):
        (report / stale).unlink(missing_ok=True)
    env = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(ROOT / "target/testing/normal"))
    cases = []

    def run(name, command, *, expected=None, custom_env=None):
        case = execute(name, command, report / f"{len(cases):02d}.log", expected=expected,
                       env=custom_env or env)
        cases.append(case)
        return case

    if args.check == "fmt":
        run("workspace rustfmt", ["cargo", "fmt", "--all", "--", "--check"])
    elif args.check == "clippy":
        result = run("Clippy", ["cargo", "clippy", "--locked", *PACKAGES, "--all-targets",
                                 "--message-format=json", "--", "-D", "warnings"])
        messages = []
        for line in result.output.splitlines():
            if line.startswith("{"):
                try:
                    messages.append(json.loads(line))
                except ValueError:
                    cases.append(Case("Clippy JSON", False, line))
        cases.extend(diagnostic_cases(messages))
        (report / "diagnostics.json").write_text(json.dumps(messages, indent=2))
    elif args.check == "audit":
        # Keep stdout independently parseable: compiler/network errors remain in the command log.
        started = time.monotonic()
        result = subprocess.run(["cargo", "audit", "--json"], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=False)
        (report / "audit.json").write_text(result.stdout)
        (report / "audit.log").write_text(result.stderr)
        cases.extend(structured_cases("audit", result.stdout, audit_cases))
        cases.append(Case("cargo audit exit", result.returncode == 0,
                          result.stderr + f"\nexit {result.returncode}", time.monotonic() - started))
    elif args.check == "tests":
        run("report adapter tests", ["python3", "-m", "unittest", "discover", "-s",
                                    ".github/testing/tests"])
        native = NEXTEST_REPORT
        native.unlink(missing_ok=True)
        run("workspace unit and property tests", ["cargo", "nextest", "run", "--locked",
                                                  "--config-file", ".github/testing/nextest.toml",
                                                  "--workspace", "--profile", "ci", "--no-fail-fast"])
        cases.append(native_report_case(native))
        if native.exists():
            shutil.copyfile(native, report / "nextest.xml")
        run("workspace doc tests", ["cargo", "test", "--locked", "--workspace", "--doc"])
    elif args.check in ("abi", "asan"):
        address = args.check == "asan"
        flags = ["-fsanitize=address", "-shared-libasan"] if address else ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
        env.update(UBSAN_OPTIONS="halt_on_error=1:print_stacktrace=1",
                   ASAN_OPTIONS="halt_on_error=1:abort_on_error=1",
                   ASAN_SYMBOLIZER_PATH=shutil.which("llvm-symbolizer") or "/usr/bin/llvm-symbolizer")
        if address:
            runtime = clang_runtime_directory()
            env["LD_LIBRARY_PATH"] = runtime + (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
            env["CARGO_TARGET_DIR"] = str(ROOT / "target/testing/asan-tests")
            env["RUSTFLAGS"] = "-Zsanitizer=address"
            native = NEXTEST_REPORT
            native.unlink(missing_ok=True)
            run("ASan SDK unit and property tests", ["cargo", NIGHTLY, "nextest", "run", "--locked",
                *PACKAGES, "--target", TARGET, "-Zbuild-std",
                "--config-file", ".github/testing/nextest.toml", "--profile", "ci", "--no-fail-fast"])
            cases.append(native_report_case(native))
            if native.exists():
                shutil.copyfile(native, report / "nextest.xml")
            env["CARGO_TARGET_DIR"] = str(ROOT / "target/testing/asan-ffi")
            env["RUSTFLAGS"] = "-Zsanitizer=address -Zexternal-clangrt"
        command = ["cargo"] + ([NIGHTLY] if address else []) + ["build", "--locked", "-p", "sdk_abi_harness",
                                                                  "--message-format=json"]
        if address:
            command += ["--target", TARGET, "-Zbuild-std"]
        built = run("build Rust C ABI fixture", command)
        out = None
        for line in built.output.splitlines():
            if line.startswith("{"):
                entry = json.loads(line)
                if entry.get("reason") == "build-script-executed" and "sdk_abi_harness" in entry["package_id"]:
                    out = Path(entry["out_dir"])
        if built.passed and out is not None:
            debug = Path(env["CARGO_TARGET_DIR"]) / (TARGET if address else "") / "debug"
            library = debug / "libsdk_abi_harness.so"
            binary = report / "harness"
            compiled = run("compile C ABI harness", ["clang", "-std=c2x", "-Wall", "-Wextra", "-Werror",
                "-g", "-O1", "-fno-omit-frame-pointer", *flags, "-Iinclude", "-I" + str(out),
                ".github/testing/abi-harness/harness.c", "-ldl", "-o", str(binary)])
            if compiled.passed:
                for name in ("layout", "lifecycle", "malformed-bson", "ownership"):
                    run(name, [str(binary), str(library), name])
                run("layout mismatch control", [str(binary), str(library), "negative-layout"], expected="ABI mismatch")
                controls = [("negative-c-asan", "heap-buffer-overflow"),
                            ("negative-rust-asan", "heap-use-after-free")] if address else [
                                ("negative-ubsan", "runtime error: signed integer overflow")]
                for name, diagnostic in controls:
                    run(name, [str(binary), str(library), name], expected=diagnostic)
        elif built.passed:
            cases.append(Case("generated ABI manifest", False, "Cargo did not report the fixture build output"))
    passed = all(case.passed for case in cases)
    if args.check != "fmt":
        write_junit(report / "checks.xml", args.check, cases)
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        diagnostic = traceback.format_exc()
        print(diagnostic, file=sys.stderr)
        check = sys.argv[1] if len(sys.argv) > 1 else "setup"
        if check in {"clippy", "audit", "tests", "abi", "asan"}:
            report = ROOT / "reports" / check
            report.mkdir(parents=True, exist_ok=True)
            (report / "setup-error.log").write_text(diagnostic)
            write_junit(report / "checks.xml", check, [Case("check setup", False, diagnostic)])
        raise SystemExit(1)
