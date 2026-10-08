#!/usr/bin/env sh
set -eu
curl -fsS -X PUT -H 'Content-Type: application/json' --data-binary @/config/opensearch-sink.json http://connect:8083/connectors/opensearch-sharded-sink/config
for name in range hashed compound unsharded; do
  curl -fsS -X PUT -H 'Content-Type: application/json' --data-binary @/config/mongo-$name-source.json http://connect:8083/connectors/mongo-$name-source/config
done
