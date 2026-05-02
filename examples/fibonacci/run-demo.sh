#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

export MONGO_IMAGE="${MONGO_IMAGE:-mongo:8.3-rc-noble}"
COMPOSE=(docker compose -f examples/fibonacci/docker-compose.yml --project-name fibonacci-example)

usage() {
  cat <<'EOF'
Usage: ./examples/fibonacci/run-demo.sh [COMMAND]

  (default) run   Build image, start MongoDB, run demo.js, then stop containers.
  up | start      Build image, start MongoDB, wait until ready — leaves containers running.
  down | stop     Stop and remove containers (docker compose down -v).

Environment:
  MONGO_IMAGE     MongoDB base image (default: mongo:8.3-rc-noble)

Examples:
  ./examples/fibonacci/run-demo.sh              # full automated demo
  ./examples/fibonacci/run-demo.sh up           # keep mongod on localhost:27018
  mongosh --port 27018
  ./examples/fibonacci/run-demo.sh down
EOF
}

wait_for_mongo() {
  echo "==> Waiting for mongod to accept connections..."
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
}

cmd="${1:-run}"
case "$cmd" in
  -h|--help|help)
    usage
    exit 0
    ;;
  up|start)
    echo "==> Building Fibonacci example image (MongoDB ${MONGO_IMAGE})..."
    "${COMPOSE[@]}" build
    echo "==> Starting mongod (containers stay up; use '$0 down' to remove)..."
    "${COMPOSE[@]}" up -d
    wait_for_mongo
    echo "==> Ready. Connect:"
    echo "    mongosh --port 27018"
    echo "  Example:"
    echo "    use fibonacci_demo; db.createCollection(\"n\"); db.n.aggregate([{ \\\$fibonacci: { n: 10 } }])"
    echo "  Stop:"
    echo "    $0 down"
    exit 0
    ;;
  down|stop)
    echo "==> Stopping containers..."
    "${COMPOSE[@]}" down -v 2>/dev/null || true
    exit 0
    ;;
  run)
    echo "==> Building Fibonacci example image (MongoDB ${MONGO_IMAGE})..."
    "${COMPOSE[@]}" build

    cleanup() {
      echo "==> Stopping containers..."
      "${COMPOSE[@]}" down -v 2>/dev/null || true
    }
    trap cleanup EXIT

    echo "==> Starting mongod..."
    "${COMPOSE[@]}" up -d
    wait_for_mongo

    echo "==> Running Fibonacci demo script..."
    set +e
    out="$("${COMPOSE[@]}" exec -T mongo mongosh --quiet /scripts/demo.js 2>&1)"
    rc=$?
    set -e
    if [[ "$rc" -ne 0 ]]; then
      echo "$out"
      echo "mongosh exited with code $rc"
      exit "$rc"
    fi
    if ! grep -q '^DEMO_OK$' <<<"$out"; then
      echo "$out"
      echo "Expected DEMO_OK in mongosh output"
      exit 1
    fi

    echo "==> Demo passed. Try: mongosh --port 27018"
    echo "    use fibonacci_demo; db.n.aggregate([{ \\\$fibonacci: { n: 10 } }])"
    exit 0
    ;;
  *)
    echo "Unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
