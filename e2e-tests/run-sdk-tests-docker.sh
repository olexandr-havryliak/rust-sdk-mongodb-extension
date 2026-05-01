#!/usr/bin/env bash
# Run Rust crate tests inside Docker (no local `cargo` / rustup required).
# From repo root: ./e2e-tests/run-sdk-tests-docker.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

IMAGE="${RUST_TEST_IMAGE:-rust:bookworm}"

echo "==> Running Rust tests in ${IMAGE} (mounted ${ROOT})..."
docker run --rm \
  -v "${ROOT}:/build" \
  -w /build \
  "${IMAGE}" \
  bash -c 'set -euo pipefail
    cargo test -p extension-sdk-mongodb --tests
    cargo test -p e2e_extension --lib
    cargo test -p http_fetch_extension --lib
    cargo check -p mongo_extension_fuzz
  '

echo "==> SDK Rust tests passed (Docker)."
echo "    Optional Miri (slow): ./e2e-tests/run-miri-docker.sh"
