#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export MONGO_IMAGE="${MONGO_IMAGE:-mongo:8.3-rc-noble}"

COMPOSE=(docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e)

echo "==> Building e2e image (Rust extension + MongoDB ${MONGO_IMAGE})..."
"${COMPOSE[@]}" build

cleanup() {
  echo "==> Stopping containers..."
  "${COMPOSE[@]}" down -v 2>/dev/null || true
}
trap cleanup EXIT

echo "==> Starting mongod..."
"${COMPOSE[@]}" up -d

echo "==> Waiting for mongod to accept connections..."
ok=
for _ in $(seq 1 120); do
  if "${COMPOSE[@]}" exec -T mongo mongosh --quiet --eval 'db.adminCommand({ping:1}).ok' 2>/dev/null | grep -qx 1; then
    ok=1
    break
  fi
  sleep 1
done
if [[ -z "${ok:-}" ]]; then
  echo "Timed out waiting for MongoDB. Logs:"
  "${COMPOSE[@]}" logs mongo
  exit 1
fi

echo "==> Running aggregation against \$rustSdkE2e (extension YAML param + pipeline checks)..."
set +e
out="$("${COMPOSE[@]}" exec -T mongo mongosh --quiet /scripts/aggregate_e2e.js 2>&1)"
rc=$?
set -e
if [[ "$rc" -ne 0 ]]; then
  echo "$out"
  echo "mongosh exited with code $rc"
  exit "$rc"
fi
if ! grep -q '^E2E_OK$' <<<"$out"; then
  echo "$out"
  echo "Expected E2E_OK in mongosh output"
  exit 1
fi

echo "==> E2E passed."
