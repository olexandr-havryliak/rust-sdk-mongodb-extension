#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
topology=${1:-all}
case "$topology" in
  all)
    failed=0
    for selected in replica-set mongos; do
      if ! bash "$ROOT/.github/testing/load.sh" "$selected"; then failed=1; fi
    done
    exit "$failed" ;;
  replica-set|mongos) ;;
  --help|-h) printf '%s\n' 'Usage: bash .github/testing/load.sh [all|replica-set|mongos] (default: all, sequential)' \
    'Optional: LOAD_CONCURRENCY=1,4,16,64 LOAD_REQUESTS=8 LOAD_TARGET_RPS=0 LOAD_SOAK_SECONDS=0'; exit 0 ;;
  *) echo "Unknown topology: $topology" >&2; exit 2 ;;
esac
export MONGO_IMAGE="${MONGO_IMAGE:-mongodb/mongodb-community-server:9.0-ubi9}"
export LOAD_TOPOLOGY=$topology
report="$ROOT/reports/performance/load/$topology"
COMPOSE=(docker compose -f "e2e-tests/performance/$topology.json" --project-name sdk-performance-load)
monitor_pid=
cleanup() {
  local rc=$?
  rm -f "$report/monitor-active"
  if [[ -n "$monitor_pid" ]]; then
    if ! wait "$monitor_pid"; then rc=1; fi
  fi
  "${COMPOSE[@]}" logs --no-color > "$report/containers.log" 2>&1 || rc=1
  if ! "${COMPOSE[@]}" down -v --remove-orphans; then rc=1; fi
  if [[ "$rc" -ne 0 ]]; then
    docker run --rm --network none -v "$ROOT:/build" -w /build python:3.11-slim \
      python .github/testing/performance_reports.py "reports/performance/load/$topology" --setup-failure "load launcher exited $rc" || true
  fi
  exit "$rc"
}
if docker ps --filter label=com.docker.compose.project=sdk-performance-load -q | grep -q .; then
  echo 'A performance topology is already running; stop it before starting another.' >&2
  exit 2
fi
mkdir -p "$report"
rm -f "$report/checks.xml" "$report/setup.xml" "$report/load.json" "$report/resources.jsonl" "$report/processes.jsonl"
trap cleanup EXIT
# BuildKit can consume the base image without tagging it locally, so inspect would fail.
docker pull "$MONGO_IMAGE"
"${COMPOSE[@]}" build
wait_for() {
  local service=$1 expression=$2
  for _ in $(seq 1 120); do
    if "${COMPOSE[@]}" exec -T "$service" mongosh --quiet --eval "$expression" 2>/dev/null | grep -qx 1; then return; fi
    sleep 1
  done
  echo "MongoDB service $service did not become ready" >&2
  return 1
}
if [[ "$topology" == replica-set ]]; then
  "${COMPOSE[@]}" up -d mongo
  wait_for mongo 'db.adminCommand({ping:1}).ok'
  "${COMPOSE[@]}" exec -T mongo mongosh --quiet /scripts/init-rs.js
  wait_for mongo 'Number(db.hello().isWritablePrimary)'
  services=(mongo)
else
  services=(config shard0 shard1)
  "${COMPOSE[@]}" up -d "${services[@]}"
  for service in "${services[@]}"; do
    wait_for "$service" 'db.adminCommand({ping:1}).ok'
    "${COMPOSE[@]}" exec -T "$service" mongosh --quiet /scripts/init-rs.js
    wait_for "$service" 'Number(db.hello().isWritablePrimary)'
  done
  "${COMPOSE[@]}" up -d mongos
  wait_for mongos 'db.adminCommand({ping:1}).ok'
  "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/init-cluster.js
  services+=(mongos)
fi
containers=()
for service in "${services[@]}"; do containers+=("$("${COMPOSE[@]}" ps -q "$service")"); done
docker image inspect "$MONGO_IMAGE" --format '{{json .RepoDigests}}' > "$report/mongo-image.json"
docker version --format '{{json .}}' > "$report/docker-version.json"
touch "$report/monitor-active"
bash .github/testing/monitor-load.sh "$report" "${containers[@]}" > "$report/monitor.log" 2>&1 &
monitor_pid=$!
"${COMPOSE[@]}" run --rm --name sdk-performance-load-driver --no-deps driver > "$report/driver.log" 2>&1
sleep "${LOAD_RECOVERY_SECONDS:-5}"
docker run --rm --network none -v "$ROOT:/build" -w /build python:3.11-slim \
  python .github/testing/performance_reports.py "reports/performance/load/$topology"
