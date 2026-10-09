#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IMAGE="${TESTING_IMAGE:-rust-sdk-testing:local}"
cd "$ROOT"
if [[ "${TESTING_BUILD_IMAGE:-1}" == "1" ]]; then
  bash .github/testing/build-image.sh
fi
docker run --rm --cpus 2 -v "$ROOT:/build" -w /build \
  -e LOCAL_UID="$(id -u)" -e LOCAL_GID="$(id -g)" "$IMAGE" \
  bash -c 'trap '\''chown -R "$LOCAL_UID:$LOCAL_GID" /build/reports'\'' EXIT
    python3 .github/testing/runner.py "$@"' -- "$@"
