#!/usr/bin/env bash
# Random aggregation fuzz against the same Docker stack as run-e2e.sh (requires Docker only).
# From repo root: ITERATIONS=3000 ./e2e-tests/run-fuzz-e2e.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export MONGO_IMAGE="${MONGO_IMAGE:-mongo:8.3-rc-noble}"
export ITERATIONS="${ITERATIONS:-5000}"

COMPOSE=(docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e)

echo "==> Building mongod image (Rust extension + MongoDB ${MONGO_IMAGE})..."
"${COMPOSE[@]}" build mongo

echo "==> Starting mongo..."
"${COMPOSE[@]}" up -d mongo

echo "==> Waiting for mongod..."
ok=
for _ in $(seq 1 120); do
  if "${COMPOSE[@]}" exec -T mongo mongosh --quiet --eval 'db.adminCommand({ping:1}).ok' 2>/dev/null | grep -qx 1; then
    ok=1
    break
  fi
  sleep 1
done
if [[ -z "${ok:-}" ]]; then
  echo "Timed out waiting for MongoDB."
  "${COMPOSE[@]}" logs mongo
  exit 1
fi

echo "==> Running mongo_extension_fuzz (ITERATIONS=${ITERATIONS}, stage=\$rustSdkE2e)..."
"${COMPOSE[@]}" --profile fuzz run --rm fuzz

echo "==> Mongo aggregation fuzz finished (mongod still running; use: docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e down)."
