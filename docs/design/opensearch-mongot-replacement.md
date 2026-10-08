# OpenSearch-backed Vector Search Design

The Rust SDK for MongoDB Extensions PoC currently targets only `$vectorSearch`.
The earlier combined text/vector and custom Python sink design has been replaced.
`$search` is not registered. This document records the current design rather
than the original implementation plan; runnable commands and diagrams live in
the implementation documentation linked below.

## Decisions

- MongoDB remains the document source of truth; no MongoDB search/vector index
  or search-specific collection metadata is created.
- MongoDB Kafka Source uses `startup.mode=copy_existing` and change streams.
- Aiven OpenSearch Sink 3.2.0 handles synchronization without a custom consumer.
- Kafka sink SMT configuration selects only flat, non-empty text fields to
  vectorize. Projection happens before OpenSearch receives the document;
  Kafka still retains the full MongoDB source payload.
- Index/topic naming is `mongodb.<database>.<collection>`; one OpenSearch
  template matching `mongodb.*` provides automatic vector mappings.
- A generic Painless ingest adapter uses the standard OpenSearch embedding
  processor, creates `<field>_embedding`, and removes original text.
- OpenSearch's deployed `huggingface/sentence-transformers/paraphrase-MiniLM-L3-v2`
  model (version 1.0.2, ONNX, 384 dimensions) embeds documents and text queries.
  The shared search pipeline supplies its default ID; clients do not pass a
  model ID or generate query vectors.
- The extension queries by original field path, emits IDs and score metadata,
  and expands to MongoDB host ID lookup for complete source documents.
- The default Docker stand has one Connect worker, one source task, one sink
  task, and one topic partition. Runtime additions use a separate connector
  pair and one-partition topic per namespace. No TLS/auth or infrastructure
  HA is implemented in this stand.

## Separate Data and Query Flows

The architecture is shown as two vertical diagrams in
[ARCHITECTURE.md](../../examples/opensearch/ARCHITECTURE.md#components):

1. **CRUD synchronization:** MongoDB -> MongoDB Kafka Source -> Kafka -> Aiven
   OpenSearch Sink -> embedding pipeline/model -> vector-only index. Inserts,
   updates, and replacements write complete projected documents and re-embed
   their text. Keyed delete events become OpenSearch DELETE requests and bypass
   embedding. Synchronization is eventually consistent; a few seconds of delay
   between a MongoDB write and searchable vectors is acceptable.
2. **Aggregation pipeline:** a client submits `$vectorSearch` with `path`, text
   `query`, optional `limit`, and an optional OpenSearch-DSL `filter`. The extension
   derives the namespace index and queries `<path>_embedding`. OpenSearch returns
   IDs and scores; MongoDB's `$_internalSearchIdLookup` fetches full documents.
   `$set` or `$project` exposes `$meta: "vectorSearchScore"` in client results.

## Demo Data and Namespaces

The default namespace is `search_demo.products` with 20 seeded products. The
optional on-the-fly example is `catalog.articles` with two documents; its
connectors are not registered by default. Both use only these document fields:

| Field | MongoDB | OpenSearch |
| --- | --- | --- |
| `_id` | String document ID | Same document ID, not a projected source field |
| `title` | Original text; returned in aggregation results | Not indexed or stored |
| `description` | Original text | Only `description_embedding`, a 384-component `knn_vector` |

The embedding pipeline remains generic: additional flat text fields can be
selected through Kafka sink configuration without per-field pipeline changes.
The demo indexes only `description`, so queries use `path: "description"`.
Seed scripts use repeatable replacement upserts, not collection drops.

## Connector Lifecycle

Namespaces can be added without rebuilding images or restarting the worker.
Create a one-partition topic, register the sink, then register a fresh source
for that namespace through Kafka Connect REST API. The existing `mongodb.*`
template automatically configures the new index on its first write; no new
model, mapping, or extension registration is required.

`PUT /connectors/{name}/config` creates or updates a connector using its complete
configuration. Configuration persists in Kafka's internal topics; updates may
restart tasks or trigger a rebalance. Changing selected fields does not reindex
already consumed documents. Projection/model changes require a coordinated
rebuild for a consistent index.

Use a fresh source name for a new namespace: `copy_existing` applies when there
is no source offset. Expanding an existing connector's namespace selection does
not guarantee an initial copy. Topic routing and the initial-copy regex do not
replace the source's database/collection change-stream selection.

**Deleting connectors intentionally retains the OpenSearch index and documents.**
It does not delete MongoDB data or Kafka topics, and is not an offset reset.
Deleting both connectors stops this synchronization path, although submitted
writes can still finish. Removing only the source lets the sink drain queued
events; removing only the sink lets the source continue publishing. Recreating
a connector with the same name must not be treated as a guaranteed fresh copy.

Commands, optional configurations, and operational caveats are in
[CONNECTOR-LIFECYCLE.md](../../examples/opensearch/CONNECTOR-LIFECYCLE.md).

## Delivery and HA Boundaries

Delivery is at least once. Kafka ordering is per partition, not global FIFO.
Full replacement and offset-based external versions reject older/duplicate
live-document writes; they do not make MongoDB/Kafka/OpenSearch transactional.
Physical deletes retain their version fence only temporarily, so an old replay
can later resurrect a deleted document. Permanent delete fences and topic
UUID/configuration/model identity validation from the custom sink are absent.
Do not independently recreate topics or reset offsets against a retained index.
Collection/database DDL, including `drop`, is not supported by this PoC.

Additional distributed Connect workers can reassign a single active task within
the same group. They require shared internal topics/plugins and distinct
reachable REST addresses; source and sink tasks can have different owners.
Infrastructure HA additionally needs replicated Kafka/internal topics,
OpenSearch replicas, MongoDB replication, durable storage, and independent
failure domains. Current tests verify worker restart recovery, not multi-node
HA or permanent stale-write safety.

The extension accepts multiple OpenSearch endpoints and tries them with bounded
request timeouts. Failure of every endpoint returns an error. Background
heartbeats are not implemented. See [CONNECTOR.md](../../examples/opensearch/CONNECTOR.md)
for detailed ordering and recovery limits.

## Verification

Docker-only tests cover configuration/dataset shape, repeatable demo scripts,
real-model vectorization and text removal, CRUD propagation, full MongoDB
documents/scores, same-offset replay, and Connect outage recovery. Connector
lifecycle tests additionally exercise runtime creation, source/sink updates,
and retained OpenSearch data with no subsequent propagation after deletion.
Test resources are isolated and cleaned up; they do not certify production HA.

## Implementation Documentation

- [README](../../examples/opensearch/README.md): complete runnable demo and tests.
- [Architecture](../../examples/opensearch/ARCHITECTURE.md): components, automatic
  mappings, deployed model, default-query-model setup, and separate vertical
  CRUD synchronization and aggregation pipeline diagrams.
- [Synchronization](../../examples/opensearch/SYNC.md): source/sink configuration,
  namespace routing, and field selection.
- [Connector](../../examples/opensearch/CONNECTOR.md): processing, ordering,
  recovery, accepted stale-write risk, and requirements for future HA work.
- [Connector lifecycle](../../examples/opensearch/CONNECTOR-LIFECYCLE.md): runtime
  creation, updates, deletion, and the optional namespace demo.

The shared OpenSearch template/pipelines/model are bootstrapped once from
declarative files. Per-namespace and field selection configuration lives in
Kafka Connect. Registering ML resources entirely inside Kafka is not implemented.
