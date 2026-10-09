#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
mkdir -p reports/build
docker build -f .github/testing/Dockerfile -t "${TESTING_IMAGE:-rust-sdk-testing:local}" . 2>&1 | tee reports/build/container-build.log
