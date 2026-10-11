#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
command=${1:-help}
topology=${2:-replica-set}
case "$topology" in replica-set|mongos) ;; *) echo 'Topology must be replica-set or mongos' >&2; exit 2 ;; esac
COMPOSE=(docker compose -f examples/aws-opensearch/docker-compose.yml -f "examples/aws-opensearch/$topology.yml")
cloud=${AWS_MODE:-false}
if [[ "$cloud" == true ]]; then
  export OPENSEARCH_AWS_REGION=${AWS_DEFAULT_REGION:?Set AWS_DEFAULT_REGION}
  export OPENSEARCH_AWS_ALLOW_HTTP=false
  COMPOSE+=(-f examples/aws-opensearch/aws.yml)
else
  # Never forward a developer's real AWS credentials into the HTTP emulator.
  export AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test AWS_SESSION_TOKEN=
  export OPENSEARCH_AWS_REGION=us-east-1 OPENSEARCH_AWS_ALLOW_HTTP=true
fi
help() {
  printf '%s\n' \
    'Usage: bash examples/aws-opensearch/run-demo.sh COMMAND [replica-set|mongos]' \
    '  build  Build the domain, embedding, MongoDB extension, Connect and tools images.' \
    '  up     Start one topology, seed data and configure the Floci domain and connectors.' \
    '  unit   Run unit/configuration tests without starting the demo.' \
    '  test   Build, start, run integration tests and clean up, also on failure.' \
    '  query  Run MongoDB vectorSearch queries for the selected topology.' \
    '  down   Delete the Floci domain, containers and all demo volumes.' \
    '  help   Show help (--help and -h also work).' \
    'Run topologies sequentially; down is required before switching.'
  printf '%s\n' 'AWS_MODE=true selects a real AWS domain; see AWS.md. down never deletes that domain.'
}
wait_job() {
  local service=$1 id result
  id=$("${COMPOSE[@]}" ps -aq "$service")
  result=$(docker wait "$id")
  if [[ "$result" != 0 ]]; then "${COMPOSE[@]}" logs "$service"; return 1; fi
}
primary() {
  local service=$1
  for _ in $(seq 1 120); do
    if "${COMPOSE[@]}" exec -T "$service" mongosh --quiet --eval 'Number(db.hello().isWritablePrimary)' 2>/dev/null | rg -qx 1; then return; fi
    sleep 1
  done
  "${COMPOSE[@]}" logs "$service"
  return 1
}
cleanup() {
  if [[ "$cloud" == true ]]; then
    "${COMPOSE[@]}" --profile test down --volumes --remove-orphans
    return
  fi
  local domain=floci-aws-opensearch-mongodb-vector-demo
  # Remember any domain-owned volumes before DeleteDomain removes its container.
  local volumes=()
  if docker inspect "$domain" >/dev/null 2>&1; then
    read -r -a volumes <<< "$(docker inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$domain")"
  fi
  if [[ -n "$("${COMPOSE[@]}" ps -q floci)" ]]; then
    "${COMPOSE[@]}" run --rm --no-deps tools python /demo/scripts/delete_domain.py || return 1
  fi
  if docker inspect "$domain" >/dev/null 2>&1; then docker rm -fv "$domain"; fi
  for volume in "${volumes[@]}"; do
    if docker volume inspect "$volume" >/dev/null 2>&1; then docker volume rm "$volume"; fi
  done
  "${COMPOSE[@]}" --profile test down --volumes --remove-orphans
}
build() {
  if [[ "$cloud" != true ]]; then
    docker build -f examples/aws-opensearch/Dockerfile.opensearch -t mongodb-aws-opensearch:2.19.6 .
  fi
  "${COMPOSE[@]}" --profile test build
}
up() {
  # The state volume must never be reused for another topology's identity format.
  local state_volume=aws-opensearch_state
  [[ "$cloud" != true ]] || state_volume=aws-opensearch-cloud_state
  if docker volume inspect "$state_volume" >/dev/null 2>&1; then
    previous=$("${COMPOSE[@]}" run --rm --no-deps tools python -c 'from pathlib import Path; p=Path("/state/topology"); print(p.read_text() if p.exists() else "")')
    if [[ -n "$previous" && "$previous" != "$topology" ]]; then echo 'Run down before switching topology' >&2; return 1; fi
  fi
  "${COMPOSE[@]}" up -d configure kafka connect
  if [[ "$cloud" != true ]]; then wait_job setup; fi
  wait_job configure
  if [[ "$topology" == replica-set ]]; then
    "${COMPOSE[@]}" up -d connect-setup
    wait_job mongo-init
    wait_job mongo-seed
  else
    "${COMPOSE[@]}" up -d --wait config shard0 shard1
    for service in config shard0 shard1; do
      "${COMPOSE[@]}" exec -T "$service" mongosh --quiet /scripts/init-rs.js
      primary "$service"
    done
    "${COMPOSE[@]}" up -d --wait mongos
    "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/init-cluster.js
    "${COMPOSE[@]}" exec -T mongos mongosh --quiet /scripts/seed.js
    "${COMPOSE[@]}" up -d connect-setup
  fi
  wait_job connect-setup
}
case "$command" in
  help|--help|-h) help ;;
  build) build ;;
  up) up ;;
  unit) "${COMPOSE[@]}" build tools; "${COMPOSE[@]}" run --rm --no-deps tools ;;
  test)
    if [[ "$cloud" == true ]]; then echo 'Automated integration tests target the disposable Floci domain only' >&2; exit 2; fi
    trap cleanup EXIT
    build
    up
    "${COMPOSE[@]}" run --rm --no-deps -e RUN_INTEGRATION=true tools
    ;;
  query)
    service=mongo; [[ "$topology" != mongos ]] || service=mongos
    "${COMPOSE[@]}" exec -T "$service" mongosh --quiet /scripts/demo-queries.js
    ;;
  down) cleanup ;;
  *) help >&2; exit 2 ;;
esac
