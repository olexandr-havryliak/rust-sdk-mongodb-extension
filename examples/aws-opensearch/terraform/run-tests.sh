#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
export LOCAL_UID="$(id -u)" LOCAL_GID="$(id -g)"
COMPOSE=(docker compose -f examples/aws-opensearch/terraform/docker-compose.yml)
"${COMPOSE[@]}" run --rm -T terraform init -backend=false -lockfile=readonly -no-color
"${COMPOSE[@]}" run --rm -T terraform fmt -check -recursive
"${COMPOSE[@]}" run --rm -T terraform validate -no-color
"${COMPOSE[@]}" run --rm -T terraform test -no-color
