# Synchronization

Commands and datasets: [README.md](README.md). Component flow:
[ARCHITECTURE.md](ARCHITECTURE.md). Real AWS setup: [AWS.md](AWS.md).

## Example-Owned Files

`Dockerfile.connect` installs MongoDB's source connector and Aiven's OpenSearch
sink at image build time. There is no custom synchronization consumer.

`scripts/register_connectors.py` reads this example's selected topology through
`scripts/configure.py`, substitutes the domain endpoint and AWS authentication,
then PUTs the sink and source configurations into Kafka Connect.

| Topology | Configurations | Source identity |
| --- | --- | --- |
| Replica set | `config/replica-set/` | `_id`, extracted by the sink SMT |
| Mongos | `config/mongos/` | Complete canonical Extended JSON document key |

Sources use `startup.mode=copy_existing`, then follow change streams. Updates
fetch full documents. The sink projects `description`, fully replaces indexed
documents, and deletes them on tombstones. MongoDB retains `title`; OpenSearch
stores `description_embedding` instead of plaintext. Mongos additionally stores
the serialized identity in `__mongodb.documentKey`, which is not indexed.

`scripts/configure.py` installs the selected ingest/search pipelines and global
`mongodb.*` index template with the deployed model ID. New indices inherit the
pipelines and vector mapping. Local inference uses 384 dimensions; AWS Titan V2
uses 512. Existing indices need rebuilding when the dimension changes.

Mongos fixtures cover `_id: hashed` (`products`), range (`tenant`), hashed
(`tenant`), compound (`location.region`, `tenant`) and unsharded collections.
Initial copy and change streams normalize document keys identically. Preserve
the serialized key exactly; OpenSearch limits it to 512 UTF-8 bytes. Changes to
shard-key values and resharding are unsupported.

## Connector Lifecycle

Run these commands from the repository root against a running replica-set demo.
For mongos, replace `replica-set.yml` with `mongos.yml` and use
`mongo-range-source` (or another configured namespace).

Inspect the active configuration:

```bash
curl -fsS http://localhost:8083/connectors/mongo-products-source/config
curl -fsS http://localhost:8083/connectors/opensearch-aws-sink/config
```

Create or update the example's connectors after editing the selected local JSON
files. Rebuild the tools image first, because it contains those files:

```bash
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml build connect-setup
docker compose -f examples/aws-opensearch/docker-compose.yml -f examples/aws-opensearch/replica-set.yml run --rm --no-deps connect-setup
```

In real AWS mode, use the same environment and `aws.yml` overlay as in
[AWS.md](AWS.md); never submit raw local sink JSON to a real domain, because
registration supplies the hostname, region and credential references.

To add a namespace while running, create a separate source configuration with
its database, collection, namespace map and initial-copy namespace regex. Add
its `mongodb.db.collection` topic to the sink's `topics` list. Submit the sink
first and then the source with `PUT /connectors/NAME/config`. For mongos, adapt
the document-key normalization to that collection's actual shard key; an
unsharded collection needs only typed `_id`. Source names must be unique.

Example REST operation for a complete prepared configuration:

```bash
curl -fsS -X PUT http://localhost:8083/connectors/mongo-new-namespace/config -H 'Content-Type: application/json' --data-binary @source-config.json
curl -fsS -X PUT http://localhost:8083/connectors/opensearch-aws-sink/config -H 'Content-Type: application/json' --data-binary @sink-config.json
```

Delete a source without deleting its OpenSearch index:

```bash
curl -fsS -X DELETE http://localhost:8083/connectors/mongo-products-source
```

Remove that topic from the sink configuration if it should no longer be
consumed. Deleting the sink stops all its subscriptions but likewise preserves
indices. Recreating a connector may reuse stored offsets; deletion does not
mean a fresh initial copy. Preserve topics and offsets during recovery.

## Ordering And Failure Behavior

Kafka orders records within a partition, not globally. Stable keys route each
document's changes to the same partition. Demo sources/sink use one task and
one in-flight sink request; the sink uses Kafka offsets as external versions.
Retries are at-least-once, not a cross-system transaction. Do not recreate topics
with reset offsets and assume their versions remain comparable.

The demo is not HA. Production deployment needs replicated Kafka, distributed
Connect workers, and a suitable multi-node domain. Multiple workers can take over
tasks in the same Connect group; task count and partitioning still determine
parallelism. An unavailable backend fails synchronization rather than silently
discarding records. Search may lag MongoDB by several seconds.
