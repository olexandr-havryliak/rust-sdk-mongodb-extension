#!/usr/bin/env sh
set -eu

CONNECT_URL="${CONNECT_URL:-http://connect:8083}"
CONNECTOR_NAME="${CONNECTOR_NAME:-mongo-products-source}"
CONFIG_PATH="${CONFIG_PATH:-/config/mongo-source-connector.json}"

until curl -fsS "$CONNECT_URL/connectors" >/dev/null; do
  echo "waiting for Kafka Connect at $CONNECT_URL"
  sleep 2
done

curl -fsS -X PUT \
  -H "Content-Type: application/json" \
  --data-binary "@$CONFIG_PATH" \
  "$CONNECT_URL/connectors/$CONNECTOR_NAME/config"

echo
echo "registered MongoDB source connector $CONNECTOR_NAME"
