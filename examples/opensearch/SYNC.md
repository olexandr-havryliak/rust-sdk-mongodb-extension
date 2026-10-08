# MongoDB-to-OpenSearch Synchronization

[README.md](README.md) contains runnable commands. See [ARCHITECTURE.md](ARCHITECTURE.md)
for mappings/model defaults and [CONNECTOR.md](CONNECTOR.md) for delivery limits.
See [CONNECTOR-LIFECYCLE.md](CONNECTOR-LIFECYCLE.md) for on-the-fly namespace and
connector management, including expected OpenSearch index retention on deletion.

## Source Connector

[mongo-source-connector.json](config/mongo-source-connector.json) configures
`com.mongodb.kafka.connect.MongoSourceConnector` with one task:

- `startup.mode=copy_existing` copies existing documents and then follows change streams.
- `database=search_demo`, `collection=products` select the demo namespace.
- `topic.namespace.map` routes it to `mongodb.search_demo.products`.
- `publish.full.document.only=true` publishes complete documents rather than update patches.
- `change.stream.document.key.as.key=true` preserves the document key for inserts and deletes.
- `publish.full.document.only.tombstone.on.delete=true` emits keyed null values for deletes.

Source keys/values are serialized as JSON strings with `SimplifiedJson`. The sink
uses schemaless JSON converters to decode them into Kafka Connect maps. No
additional custom transformer or consumer is installed.

## Sink and Field Selection

[opensearch-sink-connector.json](config/opensearch-sink-connector.json) selects
the topic and configures two standard Kafka SMTs:

```json
{
  "transforms": "key,fields",
  "transforms.key.type": "org.apache.kafka.connect.transforms.ExtractField$Key",
  "transforms.key.field": "_id",
  "transforms.fields.type": "org.apache.kafka.connect.transforms.ReplaceField$Value",
  "transforms.fields.include": "description"
}
```

`ExtractField` makes the string MongoDB `_id` the OpenSearch document ID.
`ReplaceField` selects only fields to vectorize and preserves null-valued delete
records. Projection happens in the Kafka sink before OpenSearch receives a
document; Kafka still retains the full source payload. This is not field
redaction from Kafka storage.

Both demo namespaces use `_id`, `title`, and `description`. Only description is
projected; title stays in MongoDB and is returned through the host ID lookup.

To select another flat text field, change `transforms.fields.include`, then
re-register the connector configuration:

```bash
docker compose -f examples/opensearch/docker-compose.yml --project-name opensearch-example run --rm --no-deps connect-setup
```

No per-field mapping/pipeline change is needed. New or updated documents get
`<field>_embedding`. Changing projection does not reindex old documents already
consumed: perform a coordinated full rebuild for a consistent catalog. Numeric,
boolean, object, array, null, and blank values are not vectorized; selecting them
causes a write failure. There are no field tags or text-search fields.

For a new namespace, configure the source namespace/topic route and sink topic
as `mongodb.db.collection`, and provision a one-partition Kafka topic. The index
uses the shared template automatically and has the topic name. Prefer a fresh
source/sink pair per namespace to isolate initial copy and projection settings.
The complete commands and optional articles configurations are in
[CONNECTOR-LIFECYCLE.md](CONNECTOR-LIFECYCLE.md). All documents with the same ID
must stay in one ordered partition; this demo deliberately does not scale
partitions or tasks.

## Test Coverage

- Configuration unit tests: Kafka projection/key extraction, delete mode,
  replacement mode, scoped template, vector dimensions, and task/request limits.
- Seed regression test: repeated loading preserves non-dataset documents and
  does not drop the collection or duplicate seed IDs.
- Real-model pipeline tests: arbitrary fields, distinct vectors, consistent
  field association, text removal, empty projection, and invalid input rejection.
- Synchronization tests: initial copy, insert/update/replace/delete, removed
  embeddings, same-ID reinsert, default query model, full MongoDB document/score,
  stale-version rejection, and deliberate same-offset replay.
- Worker recovery test: forced Connect stop, MongoDB writes/deletes during the
  outage, and resumed propagation without a custom consumer.
- Connector lifecycle tests: dynamic initial copy/live inserts, source/sink
  configuration updates, description-only projection, MongoDB documents/scores,
  and retained index/documents with no propagation after connector deletion.

These tests do not certify multi-node HA, indefinite stale-write safety after
physical deletes, or topic recreation. See [CONNECTOR.md](CONNECTOR.md).
