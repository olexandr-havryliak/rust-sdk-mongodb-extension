# OpenSearch-backed MongoDB Search Demo

## HOWTO

Run all commands from the repository root. Everything runs in Docker, including
`mongosh` and `curl`.

### 1. Build

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  --profile dashboards build
```

### 2. Start

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  --profile dashboards up -d
```

Startup loads 20 products and deploys the embedding model. The first run can
take several minutes.

Check MongoDB and the OpenSearch document count:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' \
  --eval 'printjson(db.adminCommand({ping: 1}))'

docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T opensearch curl -fsS 'http://localhost:9200/search_demo.products/_count?pretty'
```

MongoDB should return `ok: 1`; the OpenSearch count should become `20`.
Retry the count check while initial indexing completes.

### 3. Insert a Document

Startup loads [datasets/outdoor-products.json](datasets/outdoor-products.json).
Add a document using [scripts/demo-upsert.js](scripts/demo-upsert.js):

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' \
  --file /scripts/demo-upsert.js
```

This inserts `demo-shell` with price `159` and `inStock: true`. Repeating the
command replaces the same document instead of creating duplicates.

### 4. Check MongoDB and OpenSearch

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' \
  --eval 'printjson(db.products.findOne({_id: "demo-shell"}))'

docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T opensearch curl -fsS 'http://localhost:9200/search_demo.products/_doc/demo-shell?pretty'

docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T opensearch curl -fsS 'http://localhost:9200/search_demo.products/_mapping?pretty'
```

Synchronization takes a few seconds. Repeat the OpenSearch GET if it initially
returns 404. Expect the same `_id`, price, and stock flag in both systems.
OpenSearch keeps `description` as text and adds a numeric
`description_embedding` array with 384 components. Its mapping is `knn_vector`.
`internalNotes` exists only in MongoDB.

For a visual check, open [OpenSearch Dashboards](http://localhost:5601), select
**Dev Tools**, and run:

```http
GET /search_demo.products/_doc/demo-shell
GET /search_demo.products/_mapping
```

### 5. Update the Document

Run [scripts/demo-update.js](scripts/demo-update.js):

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' \
  --file /scripts/demo-update.js
```

The script changes the description to a winter expedition parka, price to
`179`, and stock to `false`. Check OpenSearch again:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T opensearch curl -fsS 'http://localhost:9200/search_demo.products/_doc/demo-shell?pretty'
```

After a few seconds, expect the new description, price `179`, and
`inStock: false`. OpenSearch also regenerates `description_embedding` from the
new text. To repeat the demo, run the upsert script again before the update.

### 6. Search Through MongoDB

Run prepared text and vector queries, which print documents and scores:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec -T mongo mongosh --quiet 'mongodb://mongo:27017/search_demo?replicaSet=rs0' \
  --file /scripts/demo-queries.js
```

Or open `mongosh` inside the MongoDB container:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  exec mongo mongosh 'mongodb://mongo:27017/search_demo?replicaSet=rs0'
```

After running the update in step 5, execute:

```javascript
db.products.aggregate([
  { $search: { path: "description", query: "winter expedition parka", limit: 5 } },
  { $project: { _id: 1, name: 1, description: 1, score: { $meta: "searchScore" } } }
]).toArray();

db.products.aggregate([
  { $vectorSearch: {
    path: "description",
    query: "warm clothing for freezing mountain camps",
    limit: 5
  } },
  { $project: { _id: 1, name: 1, description: 1, score: { $meta: "vectorSearchScore" } } }
]).toArray();

// Verify full MongoDB document lookup for one specific OpenSearch ID.
db.products.aggregate([
  { $vectorSearch: {
    path: "description",
    query: "warm clothing for freezing mountain camps",
    filter: { ids: { values: ["demo-shell"] } },
    limit: 1
  } }
]).toArray();
```

The last query returns `demo-shell`, including its MongoDB-only `internalNotes`.
Neither query needs a model ID or a query vector. Exit `mongosh` with `exit`.

### 7. Stop and Remove the Demo

This removes the containers and demo volumes:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example \
  --profile dashboards --profile test down -v
```

## Automated Tests

Run synchronization and search tests on a fresh stack, before manual inserts.
This command removes the Compose project's volumes first:

```bash
./examples/opensearch/run-demo.sh test
```

The tests modify and delete catalog documents. See [SYNC.md](SYNC.md#test-coverage)
for covered scenarios.
For indexer unit tests and the two-worker failover check, see
[INDEXER.md](INDEXER.md#verification).

## Further Reading

- [ARCHITECTURE.md](ARCHITECTURE.md): components, data flow, mappings, embedding model, and search defaults.
- [SYNC.md](SYNC.md): connector and field configuration.
- [INDEXER.md](INDEXER.md): consumer processing, active-standby, ordering, recovery, and HA limits.
