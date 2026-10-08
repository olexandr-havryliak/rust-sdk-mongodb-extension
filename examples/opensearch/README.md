# OpenSearch-backed MongoDB Vector Search Demo

## HOWTO

Run commands from the repository root. Only Docker is required; MongoDB clients,
connectors, ML inference, and tests all run in containers.
The demo mongod uses `--wiredTigerCacheSizeGB 0.25` (256 MiB of internal cache,
not a total process-memory limit). JVM heap settings are unchanged. Run this
and the [mongos demo](../opensearch-mongos/README.md) separately, stopping one
stack before starting the other.

For an older disposable demo that indexed `name`, use the cleanup command in
section 8 before starting this version. Changing field projection alone does
not remove old mappings or reindex previously consumed documents.

### 1. Build and Start

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example --profile dashboards build
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example --profile dashboards up -d
```

Startup loads 20 products, deploys the model, creates the shared OpenSearch
template/pipelines, and registers both Kafka connectors. Model setup can take
several minutes on the first run. Synchronization is eventually consistent.

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --eval 'printjson(db.adminCommand({ping: 1})); print(db.products.countDocuments())'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.search_demo.products/_count?pretty'
```

Expect `ok: 1` and eventually 20 documents in both systems. Retry the OpenSearch
check if the index does not exist yet or the initial copy is still running.

### 2. Insert a Document

The initial dataset is [datasets/outdoor-products.json](datasets/outdoor-products.json).
The upsert script is safe to repeat: it replaces the document with the same ID.

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --file /scripts/demo-upsert.js
```

### 3. Compare MongoDB and OpenSearch

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --eval 'printjson(db.products.findOne({_id: "demo-shell"}))'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.search_demo.products/_doc/demo-shell?pretty'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.search_demo.products/_mapping?pretty'
```

MongoDB keeps the full document. OpenSearch has the same `_id`, but its `_source`
contains only `description_embedding`: a numeric array with 384 components,
mapped as `knn_vector`. Demo documents contain only `_id`, `title`, and
`description`; `title` is not indexed and description text is replaced by its
vector in OpenSearch. Retry the GET after a few seconds if it returns 404.

Alternatively open [OpenSearch Dashboards](http://localhost:5601), select
**Dev Tools**, and run:

```http
GET /mongodb.search_demo.products/_doc/demo-shell
GET /mongodb.search_demo.products/_mapping
```

### 4. Update and Check Again

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --file /scripts/demo-update.js
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --eval 'printjson(db.products.findOne({_id: "demo-shell"}))'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -fsS 'http://localhost:9200/mongodb.search_demo.products/_doc/demo-shell?pretty'
```

The updated MongoDB document has a winter-parka title and description.
OpenSearch's `_version` increases and `description_embedding` changes after
synchronization. Replacements remove embeddings for fields no longer present.

### 5. Query Through MongoDB

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --file /scripts/demo-queries.js
```

For an interactive query:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec mongo mongosh 'mongodb://mongo:27017/search_demo?replicaSet=rs0'
```

```javascript
db.products.aggregate([
  { $vectorSearch: {
    path: "description",
    query: "warm clothing for freezing mountain camps",
    limit: 5
  } },
  { $set: { score: { $meta: "vectorSearchScore" } } }
]).toArray();

db.products.aggregate([
  { $vectorSearch: {
    path: "description",
    query: "winter clothing",
    filter: { ids: { values: ["demo-shell"] } },
    limit: 1
  } },
  { $set: { score: { $meta: "vectorSearchScore" } } }
]).toArray();
```

Results are full MongoDB documents plus the projected score, including fields
not sent to OpenSearch. No query vector, model ID, or MongoDB search index is
required. Only `path: "description"` is indexed in this demo. The optional filter is OpenSearch DSL, not
MongoDB query syntax; domain scalar fields are unavailable in this vector-only
index. `$search` is not registered by this extension.

### 6. Delete

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' --eval 'printjson(db.products.deleteOne({_id: "demo-shell"}))'
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example exec -T opensearch curl -sS 'http://localhost:9200/mongodb.search_demo.products/_doc/demo-shell?pretty'
```

Eventually OpenSearch returns `found: false`. See the physical-delete/replay
limitation in [CONNECTOR.md](CONNECTOR.md#delivery-and-ha-limits).

### 7. Add, Update, or Delete Connectors On the Fly

Follow [CONNECTOR-LIFECYCLE.md](CONNECTOR-LIFECYCLE.md) for Docker commands that
add `catalog.articles`, copy existing documents, update source/sink configurations,
and remove connectors without restarting the stand. The second namespace uses
the same `title`/`description` shape and indexes only `description`.

Deleting connectors deliberately leaves the OpenSearch index and documents
intact; it stops synchronization, not storage. No additional model, mapping,
or extension setup is needed for the new namespace.

### 8. Stop and Remove Demo Data

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example --profile dashboards --profile test down -v
```

This is a disposable, single-node Docker stand without TLS/auth and without
production durability or infrastructure HA. Extension signature validation is
disabled for the unsigned local test library, not as production guidance.
Do not attach this configuration to production topics or indices.

## Automated Verification

```bash
./examples/opensearch/run-demo.sh test
bash examples/opensearch/tests/run-recovery.sh
./e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-e2e.sh
ITERATIONS=1000 ./e2e-tests/run-fuzz-e2e.sh
```

Run sync tests before changing or deleting seeded products. Test probes clean
up after themselves. Replay tests deliberately rewind this demo sink's offsets;
never run them against a production connector. Recovery tests stop the single
Connect worker and verify writes/deletes made during the outage.
Lifecycle tests also create/update/delete isolated connector pairs and verify
that their OpenSearch indices survive connector deletion without further writes.

## Further Reading

- [ARCHITECTURE.md](ARCHITECTURE.md): data/query flow, embedding model, mappings, and defaults.
- [SYNC.md](SYNC.md): connector configurations and changing selected fields.
- [CONNECTOR.md](CONNECTOR.md): sink processing, ordering, recovery, and HA limits.
- [CONNECTOR-LIFECYCLE.md](CONNECTOR-LIFECYCLE.md): create/update/delete connectors at runtime.
