# MongoDB-to-OpenSearch Synchronization

See [README.md](README.md) for Docker demo commands,
[ARCHITECTURE.md](ARCHITECTURE.md) for mappings and embedding pipelines, and
[INDEXER.md](INDEXER.md) for consumer operation, ordering, failover, and recovery.

## Source Connector Configuration

[Dockerfile.connect](Dockerfile.connect) installs the MongoDB Source Connector
into the Kafka Connect image during build. The registration service runs
[scripts/register-source-connector.sh](scripts/register-source-connector.sh),
which configures `mongo-products-source` from
[config/mongo-source-connector.json](config/mongo-source-connector.json).

```json
{
  "tasks.max": "1",
  "startup.mode": "copy_existing",
  "publish.full.document.only": "true",
  "publish.full.document.only.tombstone.on.delete": "true",
  "change.stream.document.key.as.key": "true"
}
```

The connector copies existing documents and captures MongoDB change streams.
It publishes full JSON documents, not update patches, to `search_demo.products`.
The MongoDB document key is the Kafka record key. Deletes have a null value.
[docker-compose.yml](docker-compose.yml) explicitly creates this topic with one
partition before registering the source connector. The current single broker
uses replication factor 1; this is not a replicated Kafka deployment.

## Indexing Configuration

[indexer/indexer.py](indexer/indexer.py) reads
[config/indexing.yml](config/indexing.yml). This configuration controls field
projection and automatically generated OpenSearch mappings:

```yaml
namespaces:
  search_demo.products:
    index: search_demo.products
    vector:
      dimension: 384
      modelId: "${OPENSEARCH_MODEL_ID:-}"
    fields:
      description:
        sourcePath: description
        tags: [search, vectorSearch]
      category:
        sourcePath: category
        tags: [filter]
        type: keyword
```

`search` selects text mappings; `vectorSearch` additionally creates an embedding
field. Scalar fields use their declared type. An empty model ID is resolved from
the shared model-ID file written by model setup. Source text remains in the
projected document alongside generated vectors. Unconfigured fields are omitted.

The indexer, rather than Kafka Connect, creates mappings and pipelines and writes
OpenSearch documents. This active-standby implementation supports exactly one
namespace/topic with partition 0. Index names should match MongoDB namespaces.
Reserved `_sync_*`, `_mongo_*`, and `_id` output fields cannot be configured.

## Consumer and HA/FT

Workers share a consumer group. With one partition, one worker processes records
and other members wait for takeover. Auto-commit is disabled; successful
OpenSearch writes precede explicit Kafka commits. Kafka offsets provide external
document versions, and persistent OpenSearch tombstones prevent delayed writes
from resurrecting deleted documents.

The complete processing contract, UUID/configuration checks, async rebalance,
retention requirements, and migration procedure are in [INDEXER.md](INDEXER.md).
Replicating indexer processes does not make the single-node demo's MongoDB,
Kafka, or OpenSearch infrastructure highly available.

## Test Coverage

[indexer/test_indexer.py](indexer/test_indexer.py) covers UUID/topology validation,
bootstrap races, version conflicts, malformed records, write/commit ordering,
checkpoint validation, async assignment, revocation, and shutdown.
[tests/test_sync.py](tests/test_sync.py) checks initial copy, insert, replace,
update, tombstones, embeddings, full MongoDB lookup, and both score types.
[tests/run-failover.sh](tests/run-failover.sh) starts two workers, forcibly stops
the active, waits for standby takeover, checks document updates, rejects a stale
write after deletion, and checks re-insertion. Commands are in
[INDEXER.md#verification](INDEXER.md#verification).
