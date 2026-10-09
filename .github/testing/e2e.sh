#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
cleanup() {
  docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e down -v --remove-orphans
}
trap cleanup EXIT
bash e2e-tests/run-e2e.sh
ITERATIONS="${ITERATIONS:-1500}" bash e2e-tests/run-fuzz-e2e.sh
