# Kafka OpenSearch Sink

See [README.md](README.md) for the demo, [SYNC.md](SYNC.md) for connector settings,
and [ARCHITECTURE.md](ARCHITECTURE.md) for OpenSearch mappings and inference.
Runtime create/update/delete commands are in [CONNECTOR-LIFECYCLE.md](CONNECTOR-LIFECYCLE.md).
Deleting a connector intentionally retains its OpenSearch index and documents;
connector removal is not data removal or an offset reset.

## Processing

The ready-made Aiven OpenSearch Sink **3.2.0** replaces the custom Python indexer.
It runs as a Kafka Connect task, decodes JSON, applies the configured field/key
SMTs, and writes documents using MongoDB's string `_id`.

`index.write.method=insert` uses full replacement, not partial merge. Each live
write runs OpenSearch's default embedding pipeline. Null values become physical
DELETE requests. The connector uses external versions based on Kafka offsets
when `key.ignore=false`; duplicate/older conflicts are ignored using
`behavior.on.version.conflict=ignore`. Other write errors are not deliberately
skipped: `errors.tolerance=none`, with no DLQ configuration.

Collection/database DDL (for example `drop`) is not supported by this PoC.
The source may publish a non-document-key event that fails the sink instead of
deleting an entire OpenSearch index. The demo seeder uses keyed replacement
upserts, never `drop`, so restarting the stand does not introduce such events.

`batch.size=1` and `max.in.flight.requests=1` limit batching/concurrency in the
PoC. Kafka Connect manages task lifecycle, retries, and committed offsets. The
worker uses a one-second commit interval for the demo. This is at-least-once
delivery, not an atomic MongoDB/Kafka/OpenSearch transaction.

## Ordering and Active-Standby

Kafka orders records within a partition, not globally across multiple partitions.
This stand uses one partition and one sink task. Running extra distributed
Connect workers in the same worker group makes task reassignment possible when
the owning worker fails; the sink remains a single active task, not concurrent
writers for this topic. Source and sink tasks may live on different workers.

Additional workers require the same plugins, shared Connect internal topics and
group ID, and unique reachable advertised REST addresses. The demo starts one
worker; blindly scaling this compose service is not a complete HA deployment.
Infrastructure HA also requires replicated Kafka/internal topics, OpenSearch
replicas, MongoDB replication, durable storage, and independent failure domains.

Kafka ordering does not cancel an HTTP request already sent by a former task.
External versioning protects live documents from older writes, subject to the
physical-delete limitation below. It does not prove MongoDB source initial-copy
and change-stream causal ordering under every failure scenario.

## Delivery and HA Limits

**Accepted PoC limitation:** OpenSearch physically deletes documents and retains
their version fence only temporarily (`index.gc_deletes`, default 60 seconds).
After that fence expires, an old replayed/delayed write can recreate a deleted
document. Increasing the interval does not provide permanent protection. The
old persistent tombstone contract is intentionally not retained in this PoC.

The standard sink also has no custom binding between an OpenSearch index and a
Kafka topic UUID or configuration/model digest. Do not recreate the topic, reset
offsets, change models, or change projections independently while retaining an
existing index. Such changes require a coordinated rebuild. Generic version
conflicts are ignored, not checked against a stream identity.

Kafka history and MongoDB change-stream history must remain available through
outages. The stand does not certify checkpoint expiry handling, node/network
partitions, or indefinite delayed requests. It must not be described as having
the previous custom indexer's full stale-write/failover guarantees.

## Recovery and Verification

A restarted Connect worker recovers its connector configuration and offsets
from Kafka. A sink restarted after an acknowledged write but before commit can
read the same record again; live-document external versions reject that replay.
The synchronization tests explicitly rewind demo sink offsets and verify this.

```bash
./examples/opensearch/run-demo.sh test
bash examples/opensearch/tests/run-recovery.sh
```

The second script forcibly stops the only Connect worker, changes MongoDB
documents while it is unavailable, and restarts it to verify update/insert/delete
recovery. It is a restart test, not a multi-worker failover certification.

For a disposable rebuild, remove the complete stand with the README cleanup
command and restart. Never reset a production stream based on this demo test.

Primary references: [Aiven connector source](https://github.com/Aiven-Open/opensearch-connector-for-apache-kafka/tree/v3.2.0),
[Kafka Connect offset management](https://kafka.apache.org/36/kafka-connect/user-guide/),
and [OpenSearch delete version retention](https://docs.opensearch.org/latest/api-reference/document-apis/delete-document/).
