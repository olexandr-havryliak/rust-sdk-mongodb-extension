#!/usr/bin/env sh
set -eu

CONNECT_URL="${CONNECT_URL:-http://connect:8083}"

until curl -fsS "$CONNECT_URL/connectors" >/dev/null; do
  echo "waiting for Kafka Connect at $CONNECT_URL"
  sleep 2
done

for connector in opensearch-products-sink mongo-products-source; do
  case "$connector" in
    opensearch-products-sink) config=/config/opensearch-sink-connector.json ;;
    mongo-products-source) config=/config/mongo-source-connector.json ;;
  esac
  curl -fsS -X PUT \
    -H "Content-Type: application/json" \
    --data-binary "@$config" \
    "$CONNECT_URL/connectors/$connector/config"
  echo
  echo "registered connector $connector"
done
