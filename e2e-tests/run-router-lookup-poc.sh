#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export MONGO_IMAGE="${MONGO_IMAGE:-mongodb/mongodb-community-server:9.0-ubi9}"
COMPOSE=(docker compose -f e2e-tests/router-lookup/docker-compose.yml --project-name rust-sdk-router-lookup-poc)
cleanup() {
  local rc=$?
  if [[ "$rc" -ne 0 ]]; then "${COMPOSE[@]}" logs --tail 80 mongos; fi
  "${COMPOSE[@]}" down -v >/dev/null 2>&1 || true
}
trap cleanup EXIT

"${COMPOSE[@]}" build mongos
"${COMPOSE[@]}" up -d config shard0 shard1
wait_for() {
  local service=$1 expression=$2
  for _ in $(seq 1 120); do
    if "${COMPOSE[@]}" exec -T "$service" mongosh --quiet --eval "$expression" 2>/dev/null | grep -qx 1; then return; fi
    sleep 1
  done
  "${COMPOSE[@]}" logs "$service"
  return 1
}
for service in config shard0 shard1; do
  wait_for "$service" 'db.adminCommand({ping:1}).ok'
  "${COMPOSE[@]}" exec -T "$service" mongosh --quiet /scripts/init-rs.js
  wait_for "$service" 'Number(db.hello().isWritablePrimary)'
done
"${COMPOSE[@]}" up -d mongos
wait_for mongos 'db.adminCommand({ping:1}).ok'
"${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/init-cluster.js
for service in shard0 shard1; do
  "${COMPOSE[@]}" exec -T "$service" mongosh --quiet --eval '
    const a = require("node:assert/strict");
    a.equal(db.getSiblingDB("sdk_router_poc").products.countDocuments({}), 2);
    a.equal(db.getSiblingDB("sdk_router_other").articles.countDocuments({}), 2);
    let rejected = false;
    try {
      db.getSiblingDB("sdk_router_poc").products.aggregate([{$routerLookupPoc:{candidates:[]}}]).toArray();
    } catch (err) {
      a.match(String(err), /Unrecognized pipeline stage name/);
      rejected = true;
    }
    a.equal(rejected, true, "extension must not be registered on a shard");
    print("SHARD_WITHOUT_EXTENSION_OK");'
done
"${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/verify.js
