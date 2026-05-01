#!/usr/bin/env bash
# Run Miri on extension-sdk-mongodb tests inside Docker (slow; no host nightly required).
# Usage from repo root: ./e2e-tests/run-miri-docker.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

IMAGE="${RUST_TEST_IMAGE:-rust:bookworm}"

echo "==> Miri (nightly) in ${IMAGE} for extension-sdk-mongodb tests..."
docker run --rm \
  -v "${ROOT}:/build" \
  -w /build \
  "${IMAGE}" \
  bash -c 'set -euo pipefail
    rustup toolchain install nightly --profile minimal --no-self-update
    rustup default nightly
    rustup component add miri rust-src
    cargo miri setup
    # Exclude `proptest_byte_buf_roundtrip` (fork-based harness is not Miri-friendly).
    for t in api_coverage map_extension_init map_extension_register_fail map_extension_entry_failures passthrough_extension_init; do
      cargo miri test -p extension-sdk-mongodb --test "${t}"
    done
  '

echo "==> Miri finished."
