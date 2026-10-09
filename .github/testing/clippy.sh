#!/usr/bin/env bash
set -euo pipefail
bash "$(dirname "$0")/run-docker.sh" clippy
