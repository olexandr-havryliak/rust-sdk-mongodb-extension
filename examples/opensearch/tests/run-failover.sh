#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
COMPOSE="$ROOT/examples/opensearch/docker-compose.yml"
PROJECT="${PROJECT_NAME:-opensearch-example}"

compose() { docker compose -f "$COMPOSE" --project-name "$PROJECT" "$@"; }
role() { docker exec "$1" python -c 'from pathlib import Path; print(Path("/tmp/indexer-role").read_text())' 2>/dev/null || true; }

compose up -d --no-deps --build --scale indexer=2 indexer
compose build sync-tests
active=""
standby=""
for attempt in $(seq 1 90); do
  active=""
  standby=""
  for container in $(compose ps -q indexer); do
    case "$(role "$container")" in
      active) active="$container" ;;
      standby) standby="$container" ;;
    esac
  done
  if [[ -n "$active" && -n "$standby" ]]; then break; fi
  sleep 2
done
[[ -n "$active" && -n "$standby" ]] || { echo "Expected one active and one standby"; exit 1; }
compose run --rm --no-deps sync-tests python /tests/test_failover.py before
# An explicit stop suppresses restart-policy recovery, isolating standby takeover.
trap 'docker start "$active" >/dev/null' EXIT
docker stop --time 0 "$active" >/dev/null
for attempt in $(seq 1 60); do
  if [[ "$(role "$standby")" == active ]]; then break; fi
  sleep 2
done
[[ "$(role "$standby")" == active ]] || { echo "Standby did not take over"; exit 1; }
compose run --rm --no-deps sync-tests python /tests/test_failover.py after
echo "Active-standby failover verified; restarting former active as a group member."
