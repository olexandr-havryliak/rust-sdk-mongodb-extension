"""Run release benchmarks with the identical HEAD harness against HEAD and optional base SDK."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

from reports import Case, write_junit
from runner import execute

ROOT = Path(__file__).resolve().parents[2]


def benchmark_command(mode, common, label, has_base):
    command = ["cargo", "bench", *common, "--bench", mode, "--"]
    if mode == "timing":
        command += ["--baseline", "base"] if label == "head" and has_base else ["--save-baseline", label]
    else:
        command += ["--allow-aslr=yes", "--save-summary=json", f"--save-baseline={label}"]
        if label == "head" and has_base:
            command += ["--baseline=base"]
    return command


def prepare_base(directory, archive):
    directory = Path(directory)
    with tarfile.open(archive) as bundle:
        bundle.extractall(directory, filter="data")
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    for relative in manifest["workspace"]["members"]:
        if relative not in ("extension-sdk-mongodb", "extension-sys-mongodb"):
            shutil.copytree(ROOT / relative, directory / relative,
                            ignore=shutil.ignore_patterns("target", "__pycache__"))
    shutil.copyfile(ROOT / "Cargo.lock", directory / "Cargo.lock")
    shutil.copyfile(ROOT / "Cargo.toml", directory / "Cargo.toml")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["timing", "instructions", "all"])
    parser.add_argument("--base-archive")
    args = parser.parse_args()
    report = ROOT / "reports/performance/bench"
    report.mkdir(parents=True, exist_ok=True)
    for name in ("checks.xml", "setup.xml"):
        (report / name).unlink(missing_ok=True)
    env = dict(os.environ, CARGO_BUILD_JOBS="2", BENCH_CRITERION_DIR=str(report / "criterion"),
               GUNGRAUN_HOME=str(report / "gungraun"))
    env.pop("RUSTFLAGS", None)
    env.pop("RUSTDOCFLAGS", None)
    cases = []
    modes = ["timing", "instructions"] if args.mode == "all" else [args.mode]
    try:
        with tempfile.TemporaryDirectory(prefix="sdk-benchmark-base-") as temporary:
            workspaces = []
            if args.base_archive:
                prepare_base(temporary, args.base_archive)
                workspaces.append(("base", Path(temporary)))
            workspaces.append(("head", ROOT))
            for label, workspace in workspaces:
                environment = dict(env, CARGO_TARGET_DIR=str(ROOT / f"target/performance/{label}"),
                                   GUNGRAUN_WORKSPACE_ROOT=str(workspace))
                common = ["--locked", "--manifest-path", str(workspace / "Cargo.toml"), "-p", "sdk_benchmarks"]
                contract = execute(f"{label} ABI benchmark contracts", ["cargo", "test", *common, "--test", "abi"],
                                   report / f"{label}-contracts.log", env=environment)
                cases.append(contract)
                if not contract.passed:
                    break
                for mode in modes:
                    command = benchmark_command(mode, common, label, bool(args.base_archive))
                    cases.append(execute(f"{label} {mode} benchmark execution", command,
                                         report / f"{label}-{mode}.log", env=environment))
    except Exception as error:
        cases.append(Case("benchmark setup/execution", False, str(error)))
    metadata = {"rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
                "architecture": os.uname().machine, "profile": "bench (release, debug=1)",
                "cpus": 2, "base_ref": env.get("BASE_REF", ""), "head_ref": env.get("HEAD_REF", ""),
                "quick": env.get("BENCH_QUICK", "0"), "aslr": "enabled",
                "performance_thresholds": "none"}
    (report / "environment.json").write_text(json.dumps(metadata, indent=2))
    write_junit(report / "checks.xml", "SDK benchmark execution", cases)
    raise SystemExit(0 if all(case.passed for case in cases) else 1)


if __name__ == "__main__":
    main()
