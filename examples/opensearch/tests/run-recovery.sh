#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
COMPOSE="$ROOT/examples/opensearch/docker-compose.yml"
PROJECT="${PROJECT_NAME:-opensearch-example}"

compose() { docker compose -f "$COMPOSE" --project-name "$PROJECT" "$@"; }
compose build sync-tests
compose run --rm --no-deps sync-tests python /tests/test_recovery.py before
trap 'compose start connect >/dev/null' EXIT
compose stop --timeout 0 connect
compose run --rm --no-deps sync-tests python /tests/test_recovery.py offline
compose start connect
compose run --rm --no-deps sync-tests python /tests/test_recovery.py after
