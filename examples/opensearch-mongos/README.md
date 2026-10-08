# OpenSearch Vector Search on mongos

Separate sharded-cluster example for the **Rust SDK for MongoDB Extensions**.
The extension is loaded **only on mongos**. The replica-set example in
[../opensearch](../opensearch/README.md) remains unchanged.

## Build and start

Run from the repository root. Only Docker is required; nothing is installed on
the host. This demo uses one mongos, one single-node CSRS, two single-node shards,
one Kafka broker/Connect worker, and one OpenSearch node. It is not an HA deployment.
Each mongod (CSRS and both shards) uses `--wiredTigerCacheSizeGB 0.25` to bound
its internal WiredTiger cache to 256 MiB. This is not a total process-memory cap.
JVM heap settings are unchanged. Run this and the replica-set example separately,
stopping one stack before starting the other.

```bash
bash examples/opensearch-mongos/run-demo.sh build
bash examples/opensearch-mongos/run-demo.sh up
```

Startup initializes the cluster, inserts the demo documents **before** registering
the source connectors, deploys the embedding model, and installs the pipelines
and index template. Initial copy indexes the pre-existing documents; change
streams handle subsequent writes. Wait a few seconds for indexing to finish.

## Insert and inspect

The seed contains four documents per collection (`range`, `hashed`, `compound`),
including two documents with `_id: "shared"` on different shards. An additional
`unsharded` collection contains three documents with string, ObjectId, and Int64
IDs, and is never passed to `shardCollection`. `title` is
MongoDB-only; `description` is vectorized. Routing fields are `tenant` and, for
the compound key, `location.region`. ObjectId and Int64 identifiers are included.

Reinsert/upsert the dataset:

```bash
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet /scripts/seed.js
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet --eval 'db.getSiblingDB("search_demo").range.find().forEach(printjson)'
curl -fsS -X POST 'http://localhost:9201/mongodb.search_demo.range/_refresh'
curl -fsS 'http://localhost:9201/mongodb.search_demo.range/_search?pretty&size=10'
```

OpenSearch `_source` contains `description_embedding` (384 numbers) instead of
`description`, plus unindexed `__mongodb.documentKey` metadata. `title` is absent.
OpenSearch `_id` is the complete typed MongoDB document key serialized as
canonical Extended JSON, not the original `_id` alone.
`__mongodb.documentKey` is an exact string copy of that `_id`, created **after**
embedding. Parsing it as Extended JSON restores BSON types and literal dotted
shard-key names. Metadata is neither indexed nor embedded; Kafka sends only
`description` in the projected document value.

## Update and delete

```bash
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet /scripts/demo-update.js
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet --eval 'printjson(db.getSiblingDB("search_demo").range.findOne({_id:"shared",tenant:-1}))'
curl -fsS -X POST 'http://localhost:9201/mongodb.search_demo.range/_refresh'
curl -fsS 'http://localhost:9201/mongodb.search_demo.range/_search?pretty&size=10'
```

Allow a few seconds after writes. The embedding for tenant `-1` changes; the
document with the same `_id` and tenant `1` remains separate.

```bash
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet --eval 'db.getSiblingDB("search_demo").range.deleteOne({_id:"shared",tenant:-1})'
curl -fsS -X POST 'http://localhost:9201/mongodb.search_demo.range/_refresh'
curl -fsS 'http://localhost:9201/mongodb.search_demo.range/_search?pretty&size=10'
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet /scripts/seed.js
```

## Query through mongos

```bash
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos exec -T mongos mongosh --quiet /scripts/demo-queries.js
```

The script runs this pipeline for all three shard-key types:

```javascript
db.getSiblingDB("search_demo").range.aggregate([
  {$vectorSearch: {path: "description", query: "waterproof hiking jacket", limit: 4}},
  {$set: {score: {$meta: "vectorSearchScore"}}}
]).toArray()
```

Results are current full MongoDB documents, ordered by score. No model ID or
collection name is passed to the stage. `$search` is not registered.
The same script also queries `unsharded`; its OpenSearch document key contains
only `_id`, with no shard-key fields. Synchronization and lookup still go through
mongos. To inspect its vectors:

```bash
curl -fsS 'http://localhost:9201/mongodb.search_demo.unsharded/_search?pretty&size=10'
```

## Tests and shutdown

```bash
bash examples/opensearch-mongos/run-demo.sh test
./e2e-tests/run-sdk-tests-docker.sh
bash examples/opensearch-mongos/run-demo.sh down
```

The integration suite checks initial copy, typed/dotted composite keys, CRUD,
duplicate IDs across shards, score metadata, Mongo-only updates,
extension absence on shards, the OpenSearch 512-byte boundary, and metadata
creation after embedding (including replacement of stale metadata). The unsharded
tests cover initial copy, typed `_id`-only keys, insert/update/replace/delete,
full documents, scores, and fresh Mongo-only titles through mongos. The Rust suite
checks key decoding, argument validation, generator order/EOF, metadata, partial
responses, and endpoint failover.

Optional OpenSearch Dashboards (not Kibana):

```bash
docker compose -f examples/opensearch-mongos/docker-compose.yml --project-name opensearch-mongos --profile dashboards up -d opensearch-dashboards
```

Open <http://localhost:5602>. MongoDB is available at `mongodb://localhost:27040`,
OpenSearch at <http://localhost:9201>, and Kafka Connect at <http://localhost:8084>.

## Scope and limits

- The **entire serialized Kafka key** must fit within **512 UTF-8 bytes**, including
  JSON syntax, field names, BSON type wrappers, and formatter whitespace. This is
  not a character limit or a limit on `_id` alone. Larger keys fail the sink task;
  they are not truncated, hashed, or silently discarded.
- Shard-key values and BSON types must remain stable. Changing a shard key,
  refining it, and resharding are outside this PoC. Chunk migration is not covered
  by this integration suite.
- Native lookup checks the full key but may fan out across shards. This PoC does
  not guarantee targeted routing for dynamic shard-key predicates.
- Synchronization is eventual and at least once, not transactional with MongoDB.
- Projected embedding inputs are flat, nonempty text fields. `__mongodb` is reserved.
- Namespaces must produce valid OpenSearch index names (in particular lowercase).
- Local Docker uses no TLS/auth and disables signature validation for the unsigned
  development extension. Neither is production guidance.

See [Architecture](ARCHITECTURE.md) and [Synchronization](SYNC.md) for implementation
details, model setup, connector configuration, and ordering/HA boundaries.
