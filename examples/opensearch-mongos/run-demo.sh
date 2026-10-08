#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
COMPOSE=(docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos)
wait_for() {
  local service=$1 expression=$2
  for _ in $(seq 1 120); do
    if "${COMPOSE[@]}" exec -T "$service" mongosh --quiet --eval "$expression" 2>/dev/null | grep -qx 1; then return; fi
    sleep 1
  done
  "${COMPOSE[@]}" logs "$service"
  return 1
}
case "${1:-up}" in
  build) "${COMPOSE[@]}" --profile test build mongos connect tests ;;
  up)
    "${COMPOSE[@]}" up -d --wait config shard0 shard1 kafka opensearch
    for service in config shard0 shard1; do
      "${COMPOSE[@]}" exec -T "$service" mongosh --quiet /scripts/init-rs.js
      wait_for "$service" 'Number(db.hello().isWritablePrimary)'
    done
    "${COMPOSE[@]}" up -d --wait mongos connect
    "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/init-cluster.js
    "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/seed.js
    "${COMPOSE[@]}" up -d connect-setup
    for service in opensearch-model opensearch-setup connect-setup; do
      id=$("${COMPOSE[@]}" ps -aq "$service")
      result=$(docker wait "$id")
      if [[ "$result" != 0 ]]; then "${COMPOSE[@]}" logs "$service"; exit 1; fi
    done
    ;;
  test) "${COMPOSE[@]}" run --rm --no-deps tests ;;
  query) "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/demo-queries.js ;;
  down) "${COMPOSE[@]}" down ;;
  *) echo 'Usage: bash examples/opensearch-mongos/run-demo.sh {build|up|test|query|down}' >&2; exit 2 ;;
esac
