#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
COMPOSE=(docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos)
usage() {
  printf '%s\n' \
    'Usage: bash examples/opensearch-mongos/run-demo.sh COMMAND' \
    '' \
    'Commands:' \
    '  build       Build mongos extension, Kafka Connect and test images.' \
    '  up          Start and initialize the cluster, seed all datasets, configure synchronization and start Dashboards (default).' \
    '  test        Run configuration and integration tests against the running demo.' \
    '  query       Run vectorSearch queries through mongos for every demo collection.' \
    '  dashboards  Start or restart OpenSearch Dashboards for the running demo.' \
    '  down        Remove demo containers, networks and volumes.' \
    '  help        Show this help; --help and -h also work.' \
    '' \
    'Endpoints: MongoDB mongodb://localhost:27040; OpenSearch http://localhost:9201;' \
    '           Dashboards http://localhost:5602; Kafka Connect http://localhost:8084.' \
    '' \
    'Examples:' \
    '  bash examples/opensearch-mongos/run-demo.sh build' \
    '  bash examples/opensearch-mongos/run-demo.sh up' \
    '  bash examples/opensearch-mongos/run-demo.sh test' \
    '  bash examples/opensearch-mongos/run-demo.sh query' \
    '  bash examples/opensearch-mongos/run-demo.sh down'
}
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
  help|--help|-h) usage ;;
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
    "${COMPOSE[@]}" up -d opensearch-dashboards
    ;;
  dashboards) "${COMPOSE[@]}" up -d opensearch-dashboards ;;
  test) "${COMPOSE[@]}" run --rm --no-deps tests ;;
  query) "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/demo-queries.js ;;
  down) "${COMPOSE[@]}" --profile test down -v --remove-orphans ;;
  *) usage >&2; exit 2 ;;
esac
