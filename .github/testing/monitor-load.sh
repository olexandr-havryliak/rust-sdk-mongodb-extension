#!/usr/bin/env bash
set -euo pipefail
report=$1
shift
while [[ -e "$report/monitor-active" ]]; do
  docker stats --no-stream --format '{{json .}}' "$@" >> "$report/resources.jsonl"
  if [[ "$(docker inspect --format '{{.State.Running}}' sdk-performance-load-driver 2>/dev/null || true)" == true ]]; then
    # The one-off driver can disappear between inspect and stats; server samples remain mandatory.
    docker stats --no-stream --format '{{json .}}' sdk-performance-load-driver >> "$report/resources.jsonl" || \
      echo 'Driver statistics unavailable (the one-off container may have exited).' >&2
  fi
  for container in "$@"; do
    printf '{"time":%s,"container":"%s","probe":' "$(date +%s)" "$container" >> "$report/processes.jsonl"
    docker exec "$container" sh -c '
      rss=$(awk "/^VmRSS:/ {print \$2}" /proc/1/status)
      threads=$(awk "/^Threads:/ {print \$2}" /proc/1/status)
      fds=$(find /proc/1/fd -mindepth 1 -maxdepth 1 | wc -l)
      printf "{\"rss_kib\":%s,\"threads\":%s,\"fds\":%s}" "${rss:-0}" "${threads:-0}" "$fds"
    ' >> "$report/processes.jsonl"
    printf '}\n' >> "$report/processes.jsonl"
  done
  sleep 1
done
