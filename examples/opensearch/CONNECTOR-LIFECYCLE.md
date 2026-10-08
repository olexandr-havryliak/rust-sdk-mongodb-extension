# Manage Kafka Connectors On the Fly

Start the stand using [README.md](README.md), then run these commands from the
repository root. Every command runs in Docker. No image rebuild or worker restart
is required: Kafka Connect stores connector configuration in Kafka's internal
topics. Changes can cause task restarts/rebalances and short synchronization gaps.

Use a separate source/sink pair for each namespace. This example adds
`catalog.articles` -> Kafka topic and OpenSearch index `mongodb.catalog.articles`.
Both demo namespaces use `_id`, `title`, and `description`; only `description`
is sent to OpenSearch. `title` remains available in full MongoDB query results.

## 1. Insert Documents Before Creating Connectors

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/catalog?replicaSet=rs0' --file /scripts/demo-articles.js
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T kafka kafka-topics --bootstrap-server kafka:9092 --create --if-not-exists --topic mongodb.catalog.articles --partitions 1 --replication-factor 1
```

The script performs repeatable replacement upserts, not collection drops. Use
one partition to preserve this PoC's ordering assumptions. Replication factor
one is for the single-broker demo, not an HA recommendation.

## 2. Create the Sink, Then the Source

These files are optional examples, not registered by the default startup:

- [opensearch-articles-sink-connector.json](config/opensearch-articles-sink-connector.json)
  selects the new topic and applies `transforms.fields.include=description`.
- [mongo-articles-source-connector.json](config/mongo-articles-source-connector.json)
  selects `database=catalog`, `collection=articles`, routes the topic, and enables
  `copy_existing` for the anchored namespace regex `^catalog\.articles$`.

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS -X PUT -H 'Content-Type: application/json' --data-binary @/config/opensearch-articles-sink-connector.json http://connect:8083/connectors/opensearch-articles-sink/config
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS -X PUT -H 'Content-Type: application/json' --data-binary @/config/mongo-articles-source-connector.json http://connect:8083/connectors/mongo-articles-source/config
```

`PUT /connectors/{name}/config` creates a missing connector or updates an existing
one. Send the **complete configuration**, not a partial JSON patch. A successful
request accepts configuration; check that the connector and its tasks reach
`RUNNING` before assuming synchronization is healthy:

If the REST API returns HTTP 409 during a rebalance, wait for it to finish and
retry the same request.

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS http://connect:8083/connectors/mongo-articles-source/status
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS http://connect:8083/connectors/opensearch-articles-sink/status
```

The fresh source copies existing documents, then follows change streams. The
first sink write automatically creates the index using the existing `mongodb.*`
template, embedding pipeline, and default model. No additional OpenSearch setup
or extension registration is needed. See [ARCHITECTURE.md](ARCHITECTURE.md).

## 3. Check Initial Copy and Live Updates

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/catalog?replicaSet=rs0' --eval 'printjson(db.articles.findOne({_id: "a001"}))'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.catalog.articles/_doc/a001?pretty'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.catalog.articles/_mapping?pretty'
```

Retry after a few seconds if initial copy has not finished. OpenSearch `_source`
contains only the 384-component `description_embedding`, not `title` or text.

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/catalog?replicaSet=rs0' --eval 'printjson(db.articles.updateOne({_id: "a001"}, {$set: {description: "Waterproof tents and rain covers for wet mountain camps."}}))'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.catalog.articles/_doc/a001?pretty'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/catalog?replicaSet=rs0' --eval 'printjson(db.articles.aggregate([{$vectorSearch: {path: "description", query: "rain protection", limit: 2}}, {$set: {score: {$meta: "vectorSearchScore"}}}]).toArray())'
```

Eventually the vector changes and `_version` increases. MongoDB aggregation
returns full documents, including unindexed `title`, plus scores.

## 4. Update Connector Configuration

For a reproducible non-schema update, edit `batch.size` from `"1"` to `"2"` in
the articles sink JSON, keeping `max.in.flight.requests="1"` and the field
projection unchanged. Submit the full file again:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS -X PUT -H 'Content-Type: application/json' --data-binary @/config/opensearch-articles-sink-connector.json http://connect:8083/connectors/opensearch-articles-sink/config
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS http://connect:8083/connectors/opensearch-articles-sink/config
```

Check task status again, then repeat the document update/check above. Source
configuration updates use the same PUT endpoint with the source name and full
source JSON. Editing a local file alone does not change the running connector;
REST changes persist across worker restarts. Default `connect-setup` re-applies
the products configs only, not the optional articles pair.

Changing `transforms.fields.include` affects future writes, **not documents
already consumed**. Projection/model changes require a coordinated rebuild for
a consistent index; do not reset offsets against a retained index independently.

Adding a new namespace to an existing source is not a reliable initial-sync
procedure: `copy_existing` applies when no source offset is available. Prefer a
fresh source name for a new namespace. `topic.namespace.map` only routes topics;
it does not expand the source's watched database/collection. The copy regex
controls initial copy, not the change-stream selection.

## 5. Delete Connectors Without Deleting the Index

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS -X DELETE http://connect:8083/connectors/mongo-articles-source
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps --entrypoint curl connect-setup -fsS -X DELETE http://connect:8083/connectors/opensearch-articles-sink
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.catalog.articles/_doc/a001?pretty'
```

**Keeping the OpenSearch index and documents is expected.** DELETE removes the
connector, not MongoDB data, Kafka topics, or the OpenSearch index. It is not an
offset reset either; do not assume reusing a connector name starts a fresh copy.
Deleting only the source stops new events; the sink can still drain queued events.
Deleting only the sink stops index writes while the source keeps publishing.
Deleting both stops this synchronization path; an already submitted HTTP write
can still complete. For a clean cutoff, stop the source and drain the sink first.

After both are removed, subsequent MongoDB inserts/updates/deletes do not change
the retained index. Queries may see stale vectors; MongoDB ID lookup omits
documents that no longer exist. Reconnecting after a gap is a separate recovery
operation, constrained by offsets and history retention, not a new initial copy.
See [CONNECTOR.md](CONNECTOR.md#delivery-and-ha-limits).

## Automated Verification

```bash
./examples/opensearch/run-demo.sh test
```

Lifecycle tests use isolated namespaces and fresh connector names. They check
initial copy, live inserts, description-only vectors, full MongoDB results with
scores, source/sink configuration updates, and index retention/no propagation
after deletion. Test connectors, MongoDB collections, OpenSearch indices, and
test topics are cleaned up; the default products pair is left intact.

References: [Kafka Connect REST API](https://kafka.apache.org/36/kafka-connect/user-guide/#rest-api),
[MongoDB startup and copy settings](https://www.mongodb.com/docs/kafka-connector/current/source-connector/configuration-properties/startup/).
