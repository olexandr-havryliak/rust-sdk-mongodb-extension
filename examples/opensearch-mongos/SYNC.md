# Sharded MongoDB to OpenSearch Synchronization

See [README](README.md) for commands and [Architecture](ARCHITECTURE.md) for diagrams.

## Standard connectors only

No custom consumer, indexer, connector, or SMT runs in this example. The shared
[Kafka Connect Dockerfile](../opensearch/Dockerfile.connect) installs the MongoDB
Kafka source connector and Aiven OpenSearch sink connector (3.2.0). Source
connectors connect to **mongos**, never directly to a shard.

Source configurations are explicit per namespace and shard-key definition:

| Namespace | Shard key | Configuration |
| --- | --- | --- |
| `search_demo.range` | `{tenant: 1}` | [range source](config/mongo-range-source.json) |
| `search_demo.hashed` | `{tenant: "hashed"}` | [hashed source](config/mongo-hashed-source.json) |
| `search_demo.compound` | `{"location.region": 1, tenant: 1}` | [compound source](config/mongo-compound-source.json) |
| `search_demo.unsharded` | None; key is only `_id` | [unsharded source](config/mongo-unsharded-source.json) |

`startup.mode=copy_existing` starts initial copy and then streams subsequent
changes. `change.stream.full.document=updateLookup` supplies complete documents
for updates; replacements and inserts also use complete documents. The sink
reindexes the whole projected document, not individual changed fields.

## One key for copy, writes, and deletes

MongoDB connector initial-copy events initially contain only `_id` in
`documentKey`. Live sharded change events include the shard key too. Sending
these unchanged would create different Kafka/OpenSearch identities.

Each source's regular `pipeline` normalizes `documentKey` into a fixed field
order: `_id` first, then every configured shard-key field. It reads original
values from `fullDocument` for writes/copy, and from `documentKey` for deletes.
`$arrayToObject` permits literal dotted field names such as `location.region`.
Missing shard-key values normalize to null. The same regular pipeline runs on
synthetic copy events and live change-stream events.

For an unsharded collection, both copy and live events already identify the
document by `_id` alone. Its source pipeline keeps exactly that field; do not
reuse a sharded namespace's configuration and add artificial shard-key fields.
The same sink, template, ingest pipeline, and mongos extension handle these keys
without special code or a separate extension.

The source pipeline changes only `documentKey`. It never manufactures a
`fullDocument` for delete events, which would suppress tombstones and leave
deleted documents in OpenSearch. Inspection metadata is created by OpenSearch
after embedding, not by the source connector.

`change.stream.document.key.as.key=true` selects the normalized key.
`ExtendedJson` preserves BSON types. Both source converters publish JSON strings;
the sink key converter is **StringConverter**, so the complete serialized key
becomes OpenSearch `_id` unchanged, including formatter whitespace. The sink
value converter parses JSON and projects only `description`.

The default ingest pipeline vectorizes the projected text, removes raw text,
then sets `__mongodb.documentKey` to an exact string copy of `ctx._id`. That
canonical Extended JSON string preserves BSON types and literal dotted names.
It is inspection metadata, stored in `_source` under an object with `enabled:
false`, not indexed or embedded. Creating it after `text_embedding` avoids that
processor's removal of nested string values. Any stale incoming metadata is
removed before embedding and replaced with the actual record key afterward.
The extension still decodes candidates from OpenSearch `_id`.

Delete tombstones carry the same normalized key and a null value.
`behavior.on.null.values=delete` sends an OpenSearch delete with that exact ID.
Hashing/changing `_id` in an ingest pipeline would break this path because deletes
do not run the text ingest pipeline. No such rewriting occurs here.

## Key-size limit and errors

The complete serialized key must be at most **512 UTF-8 bytes**. This includes
names, JSON escaping, type wrappers, and whitespace; it is not 512 characters.
Long string `_id` values and compound shard keys can exceed this even when
MongoDB accepts them. Keep key values small enough for the chosen formatter.

The [sink configuration](config/opensearch-sink.json) uses `errors.tolerance=none`
and the connector's fail-on-malformed-document default. OpenSearch rejects
oversized IDs, causing the sink task to fail. Documents are never truncated or
discarded as a workaround. Monitor task failure and fix the invalid input before
resuming; this is not an automatic recovery strategy. Resetting source offsets
or recreating topics is not a safe substitute for repairing data.

## Ordering, replay, and HA boundaries

Kafka guarantees ordering **within a partition**, not globally across partitions.
The demo has one partition per namespace and `tasks.max=1`; the sink also uses
`batch.size=1` and `max.in.flight.requests=1`. Aiven uses the Kafka offset as the
external OpenSearch document version when record keys are document IDs, and
version conflicts on replay are ignored.

Delivery is at least once. Retrying a record with the same key is idempotent
within these ordering/versioning assumptions. Full document keys keep all
operations for one document on the same Kafka key. Do not increase existing
topic partition counts: a key can move to another partition, whose independent
offset cannot be compared safely with the previous external version.

Additional Kafka Connect workers with the same distributed worker group allow
task reassignment; they do not intentionally run duplicate active tasks for one
assignment. This is task-level failover, not a custom active/standby consumer.
In-flight requests during reassignment are constrained by connector/version
semantics; this PoC does not certify lossless HA under every failure scenario.
Kafka/OpenSearch replication and replica-set node redundancy must be configured
separately for production. The demo's CSRS and shards are each single-node.

## Connector lifecycle

Configurations can be created or updated while the cluster runs. `PUT` creates
the connector if absent and updates it otherwise:

```bash
curl -fsS -X PUT -H 'Content-Type: application/json' --data-binary @examples/opensearch-mongos/config/mongo-range-source.json http://localhost:8084/connectors/mongo-range-source/config
curl -fsS -X PUT -H 'Content-Type: application/json' --data-binary @examples/opensearch-mongos/config/opensearch-sink.json http://localhost:8084/connectors/opensearch-sharded-sink/config
curl -fsS -X DELETE http://localhost:8084/connectors/mongo-range-source
```

Deleting a connector does **not** delete its OpenSearch index or Kafka topic.
Recreating a connector under an existing name can reuse stored offsets;
`copy_existing` does not force a new snapshot if stored offsets already exist.

For another namespace, create a source config with its database, collection,
topic mapping, copy regex, and **complete shard-key fields in `pipeline`**. Add
the new topic to the sink's `topics` list and update the sink configuration.
The global `mongodb.*` template handles the new index automatically. The sink
projection selects embedding inputs only; reserved `__mongodb` metadata is
generated by the ingest pipeline, not included in this projection.

Do not change the normalized key format, field order, or shard-key field set
on an already indexed namespace without an explicit rebuild/migration plan.
Shard-key value/type changes, refinement, and resharding are outside this PoC.
The source watches document changes through mongos; synchronization does not
track chunk placement. Chunk migration is not covered by this integration suite.
