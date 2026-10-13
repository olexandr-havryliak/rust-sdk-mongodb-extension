#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
mode=${1:-all}
case "$mode" in
  timing|instructions|all) ;;
  --help|-h) printf '%s\n' 'Usage: bash .github/testing/bench.sh [timing|instructions|all]' \
    'Optional: BASE_REF=<git commit/ref> BENCH_QUICK=1 BENCH_BUILD_IMAGE=0'; exit 0 ;;
  *) echo "Unknown benchmark mode: $mode" >&2; exit 2 ;;
esac
image=${BENCH_IMAGE:-sdk-performance-tools:local}
report="$ROOT/reports/performance/bench"
mkdir -p "$report"
archive=
benchmark_started=
cleanup() {
  local rc=$?
  if [[ -n "$archive" ]]; then rm -f "$archive"; fi
  if [[ "$rc" -ne 0 && ! -e "$report/checks.xml" ]]; then
    docker run --rm --network none -v "$ROOT:/build" -w /build python:3.11-slim \
      python .github/testing/performance_reports.py reports/performance/bench --setup-failure "benchmark launcher exited $rc" || true
  fi
  # Valgrind writes Callgrind output mode 0600 as root. Artifact upload runs as the host user.
  if [[ -n "$benchmark_started" ]]; then
    if ! docker run --rm --network none \
        -e LOCAL_UID="$(id -u)" -e LOCAL_GID="$(id -g)" \
        -v "$ROOT:/build" "$image" \
        bash -c 'chmod -R a+rX /build/reports/performance/bench; chown -R "$LOCAL_UID:$LOCAL_GID" /build/reports/performance/bench'; then
      echo "Failed to make benchmark reports readable by the host user" >&2
      if [[ "$rc" -eq 0 ]]; then rc=1; fi
    fi
  fi
  exit "$rc"
}
rm -f "$report/checks.xml" "$report/setup.xml"
trap cleanup EXIT
if [[ "${BENCH_BUILD_IMAGE:-1}" == 1 ]]; then
  docker build -f .github/testing/Dockerfile.benchmarks -t "$image" .
fi
args=()
mounts=()
if [[ -n "${BASE_REF:-}" ]]; then
  BASE_REF=$(git rev-parse --verify --end-of-options "${BASE_REF}^{commit}")
  archive=$(mktemp /tmp/sdk-benchmark-base.XXXXXX.tar)
  git archive --format=tar "$BASE_REF" extension-sdk-mongodb extension-sys-mongodb > "$archive"
  mounts=(-v "$archive:/baseline.tar:ro")
  args=(--base-archive /baseline.tar)
fi
HEAD_REF=$(git rev-parse HEAD)
benchmark_started=1
docker run --rm --cpus 2 --memory 3g -e CARGO_BUILD_JOBS=2 \
  -e "BENCH_QUICK=${BENCH_QUICK:-0}" -e "BASE_REF=${BASE_REF:-}" -e "HEAD_REF=$HEAD_REF" \
  -v "$ROOT:/build" "${mounts[@]}" -w /build "$image" \
  python3 .github/testing/bench_run.py "$mode" "${args[@]}"
