#!/usr/bin/env bash
# Run every GitHub workflow's checks sequentially, without installing host tools.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
if [[ "${TESTING_BUILD_IMAGE:-1}" == "1" ]]; then
  bash .github/testing/build-image.sh
fi
export TESTING_BUILD_IMAGE=0
export RUST_TEST_IMAGE="${RUST_TEST_IMAGE:-${TESTING_IMAGE:-rust-sdk-testing:local}}"
failed=0
for check in fmt clippy tests audit abi asan sdk e2e miri; do
  echo "==> $check"
  if bash ".github/testing/$check.sh"; then
    echo "==> $check passed"
  else
    echo "==> $check failed" >&2
    failed=1
  fi
done
exit "$failed"
