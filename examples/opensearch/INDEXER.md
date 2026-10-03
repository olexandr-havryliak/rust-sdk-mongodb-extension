# Kafka-to-OpenSearch Indexer

[README.md](README.md) contains the demo HOWTO. See [SYNC.md](SYNC.md) for source
connector/field configuration and [ARCHITECTURE.md](ARCHITECTURE.md) for the full
data and query flow.

## Role and Processing

The indexer is the custom Kafka sink in the Rust SDK for MongoDB Extensions
example. Kafka Connect captures MongoDB documents; the indexer projects fields,
creates OpenSearch mappings and default pipelines, and writes search documents.
The query extension is separate and never consumes Kafka.

At startup the indexer validates one namespace, one topic, and exactly partition
0. It retrieves the real topic UUID using `kafka-python==3.0.8`. The index mapping
stores an immutable `_meta` contract containing that UUID and a SHA-256 digest
of the configuration, resolved model, mappings, and pipeline settings.
Concurrent index creation is safe: the losing worker validates the winner's
contract. A different stream/configuration or legacy index is rejected, not
silently upgraded. Field types and vector dimensions are checked as well.

For each record, the active worker:

1. Verifies that it owns the partition and that the topic UUID is unchanged.
2. Parses the complete document and validates that its key matches `_id`.
3. Projects configured fields, stores the original `_id` JSON in `_mongo_id`, and
   uses that same JSON as the OpenSearch document ID. Integer `1` and string
   `"1"` stay different keys. Updates replace the
   entire indexed document, including newly generated embeddings.
4. Sends `version=offset+1` and `version_type=external`. Offset zero becomes
   positive version one. OpenSearch rejects equal or older versions.
5. Treats a version conflict as completed only when a real-time GET confirms
   an equal/newer version from the same topic UUID.
6. Commits exactly `offset+1` only after a successful write or validated replay.

Auto-commit is disabled. The worker polls one record at a time. Malformed records,
write failures, and commit failures stop processing; later offsets are never
committed past the failure. The OpenSearch client has bounded retries (10-second
request timeout, two retries); unresolved errors exit the process. Compose uses
`restart: unless-stopped`, so recovery starts from the committed Kafka offset.
This is at-least-once processing with fenced, replay-safe writes, not an atomic
Kafka/OpenSearch transaction or an exactly-once delivery claim.

## Active-Standby and Ordering

Kafka guarantees order within a partition, not across an entire multi-partition
topic. The single partition gives a total order to the records published into
this topic. It does not prove that a producer/source connector emitted events
in the correct MongoDB causal order; the source connector remains responsible
for initial-copy/change-stream and retry behavior.

All workers must share the group, topic, indexing configuration, and model.
Kafka assigns partition 0 to one member. Additional members are standby, not
parallel indexers. Multiple namespaces or partitions are deliberately rejected.
This sacrifices throughput for a simple ordered active-standby model.

Rebalance uses `AsyncConsumerRebalanceListener`. Checkpoint reads await the
Kafka client's native coroutine APIs, allowing its IO loop to keep servicing
heartbeats. Version 3.0.8 does not expose those operations as public async
`KafkaConsumer` methods: `read_checkpoints` is a small version-pinned adapter to
its fetcher/coordinator and is covered by Docker failover tests. It must be
rechecked when upgrading the dependency; this runtime is not Python `asyncio`.

On assignment, the worker validates the committed offset against the partition's
retained beginning/end offsets and seeks to that checkpoint. With no checkpoint,
starting at zero is allowed only if zero is still retained. Automatic offset
reset is disabled. Revocation does not commit unfinished work. A graceful signal
stops taking new work, and consumer close never auto-commits.

An old HTTP request can finish after Kafka has reassigned ownership; Kafka cannot
cancel an external request. External versions ensure that the old request cannot
overwrite an already-applied newer record. Commit failure causes safe replay.
`/tmp/indexer-role` records the last local role (`active`, `standby`, `stopped`)
for the failover test. It is diagnostic, not a distributed lease or health check.

## Persistent Delete Fences

Deleting physically from OpenSearch would eventually discard its version fence.
Instead, a Kafka null value replaces the document with a minimal persistent
tombstone using the same external version:

```json
{
  "_mongo_namespace": "search_demo.products",
  "_sync_topic_id": "<Kafka topic UUID>",
  "_sync_deleted": true
}
```

Tombstones bypass embedding with `pipeline=_none`. Live documents carry
`_sync_deleted=false`. Both `$search` and `$vectorSearch` exclude tombstones;
the vector filter is applied inside the neural query alongside the user filter.
Direct OpenSearch GET still returns a tombstone, and raw `_count` includes it.
Direct search/count consumers must also exclude `_sync_deleted=true`.
Do not purge tombstones: doing so removes the stale-write protection. A newer
MongoDB insert with the same ID replaces the fence with a live document.

## Recovery and Limits

Topic UUID is checked before consumption writes and on worker startup. A changed
UUID, added partition, changed configuration/model, missing mapping, or checkpoint
outside retained history requires an explicit rebuild. No automatic reset hides
missing history. A lost consumer checkpoint can replay from zero if all history
remains available; external versions prevent rolling back indexed documents.

For a disposable demo, stop and remove the entire stack, then restart using the
[README commands](README.md#howto). This erases demo data and requires a new copy.
For a deployment, stop every worker and in-flight request before rebuilding;
recreate the search index and connector copy/checkpoint state in a coordinated
resync. Never reset Kafka offsets or recreate its topic independently while
keeping an old index. Automatic production resync is not implemented.

The version contract assumes no other writer changes document versions, mappings,
or pipelines. Kafka must retain unprocessed events; MongoDB must retain usable
change-stream history. Infrastructure HA additionally needs durable replicated
Kafka/OpenSearch storage, MongoDB replication, and workers on separate failure
domains. The provided Docker stack has single-node services and is not production
HA. Poison records block progress intentionally; there is no skip/DLQ policy.

## Verification

From the repository root, run unit tests without starting the stack:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example build indexer-tests
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps indexer-tests
```

Run synchronization tests on a fresh demo, then failover against that running
stack. The failover test temporarily stops a worker and modifies a probe document:

```bash
./examples/opensearch/run-demo.sh test
bash examples/opensearch/tests/run-failover.sh
```

To run two workers manually against an already started stack:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example up -d --no-deps --build --scale indexer=2 indexer
```

Unit tests cover crash-between-write-and-commit replay semantics, rejection of
foreign conflicts, checkpoint expiry, malformed records, and async rebalance.
The deterministic randomized replay test runs 64 seeds with reordered and
duplicated writes/deletes, then verifies that a newer same-ID insert cannot be
overwritten by any older event.
Live tests cover forced worker termination/takeover, ordered replacements,
embeddings, stale-write rejection after delete, and same-ID re-insertion.
They do not certify multi-node broker failover, source-connector failover, or all
possible network partitions and delayed-request timing combinations.

See [Kafka delivery semantics](https://kafka.apache.org/40/design/design/#message-delivery-semantics)
and [OpenSearch external versioning](https://docs.opensearch.org/latest/api-reference/document-apis/index-document/).
