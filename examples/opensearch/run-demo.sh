#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/examples/opensearch/docker-compose.yml"
PROJECT_NAME="${PROJECT_NAME:-opensearch-example}"

usage() {
  cat <<'USAGE'
Usage: ./examples/opensearch/run-demo.sh [up|test|query|down|logs]

Commands:
  up     Build and start the MongoDB -> Kafka -> OpenSearch stack.
  test   Build/start the stack and run the Docker-only sync tests.
  query  Run prepared MongoDB $vectorSearch demo queries.
  down   Stop and remove the stack volumes.
  logs   Follow stack logs.

Default: test
USAGE
}

cmd="${1:-test}"

case "$cmd" in
  up)
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" --profile dashboards up -d --build
    ;;
  test)
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps configuration-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" up -d --build connect-setup
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps mongo-seed mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' /scripts/test-seed.js
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps mongo-seed mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' /scripts/test-demo-scripts.js
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps pipeline-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" build sync-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps sync-tests
    trap 'docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" exec -T kafka kafka-topics --bootstrap-server kafka:9092 --delete --topic "mongodb\.connector_test\..*"' EXIT
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps sync-tests python -m unittest -v test_connectors
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" exec -T kafka kafka-topics --bootstrap-server kafka:9092 --delete --topic 'mongodb\.connector_test\..*'
    trap - EXIT
    ;;
  query)
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" exec -T mongo mongosh --quiet "mongodb://mongo:27017/search_demo?replicaSet=rs0" /scripts/demo-queries.js
    ;;
  down)
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" --profile dashboards --profile test down -v
    ;;
  logs)
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" logs -f
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
