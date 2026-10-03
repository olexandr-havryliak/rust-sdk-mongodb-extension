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
  test   Reset volumes, build/start the stack, and run the Docker-only sync tests.
  query  Run prepared MongoDB $search / $vectorSearch demo queries.
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
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" --profile dashboards --profile test down -v
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" build indexer-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps indexer-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" up -d --build indexer
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" build sync-tests
    docker compose -f "$COMPOSE_FILE" --project-name "$PROJECT_NAME" run --rm --no-deps sync-tests
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
