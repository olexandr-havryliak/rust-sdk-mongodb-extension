#!/usr/bin/env bash
# Run SDK workspace tests + Miri in Docker (no local Rust). E2E + fuzz stay separate (longer).
# From repo root: ./e2e-tests/run-all-checks-docker.sh
# Optional: SKIP_MIRI=1 ./e2e-tests/run-all-checks-docker.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

chmod +x e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-sdk-tests-docker.sh

if [[ -n "${SKIP_MIRI:-}" ]]; then
  echo "==> SKIP_MIRI set; skipping Miri."
  exit 0
fi

chmod +x e2e-tests/run-miri-docker.sh
./e2e-tests/run-miri-docker.sh

echo "==> All Docker checks (workspace tests + Miri) passed."
echo "    E2E + fuzz: ./e2e-tests/run-e2e.sh && ./e2e-tests/run-fuzz-e2e.sh"
