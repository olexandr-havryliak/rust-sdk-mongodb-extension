# OpenSearch-backed mongot replacement design

This document describes the current proof-of-concept design for replacing the
`mongot` search path with an OpenSearch-backed MongoDB extension and an external
synchronization pipeline.

The runnable demo instructions are in
[`examples/opensearch/README.md`](../../examples/opensearch/README.md).
See [ARCHITECTURE.md](../../examples/opensearch/ARCHITECTURE.md) for component
diagrams and [INDEXER.md](../../examples/opensearch/INDEXER.md) for
consumer behavior and the current HA/FT limitations.

## Goals

- Reproduce the first useful subset of `mongot` behavior for `$search` and
  `$vectorSearch`.
- Keep MongoDB free of search/vector index definitions; search indexes live in
  OpenSearch.
- Preserve MongoDB `_id` as the OpenSearch document `_id`.
- Derive the OpenSearch index name from the MongoDB namespace.
- Return full MongoDB documents by combining OpenSearch candidate results with
  MongoDB host ID lookup.
- Keep the local proof of concept Docker-only and unauthenticated.
- Leave room for high availability by allowing multiple OpenSearch endpoints in
  extension config and by using Kafka/OpenSearch as separately scalable systems.

## Architecture

```text
MongoDB collection
  -> MongoDB Kafka Source Connector
  -> Kafka topic named after the MongoDB namespace
  -> Python indexing service
  -> OpenSearch index named after the MongoDB namespace

MongoDB aggregation
  -> Rust extension stage ($search or $vectorSearch)
  -> OpenSearch query
  -> candidate rows: { _id, $searchScore }
  -> host-created $_internalSearchIdLookup
  -> full MongoDB documents
```

The example stack uses:

- `mongodb/mongodb-community-server:9.0-ubi9`
- MongoDB Kafka Source Connector with `startup.mode=copy_existing`
- Kafka
- OpenSearch 2.x with ML Commons enabled
- OpenSearch Dashboards for manual inspection
- one Rust shared library, `libopensearch_extension.so`

## Synchronization

The connector publishes full MongoDB documents to Kafka. The indexing service:

1. reads namespace-specific Kafka topics;
2. creates OpenSearch mappings from `config/indexing.yml`;
3. creates ingest and search pipelines for autoembeddings when a model ID is
   available;
4. projects only configured fields into OpenSearch;
5. writes OpenSearch documents using MongoDB `_id` as OpenSearch `_id`;
6. handles inserts, replacements, updates, and deletes.

`publish.full.document.only=true` makes update events carry a full document, so
updates are handled as full reindex operations. This intentionally matches the
desired mongot-like behavior for the proof of concept.

See [`examples/opensearch/SYNC.md`](../../examples/opensearch/SYNC.md) for the
operational details.

## Index Configuration

`config/indexing.yml` maps MongoDB namespaces to OpenSearch indexes and field
behavior.

Supported field tags:

| Tag | Behavior |
| --- | --- |
| `search` | Create a text field for OpenSearch `match` queries. |
| `vectorSearch` | Create a source text field plus `<field>_embedding` as a `knn_vector`. |
| `filter` | Create a scalar field for filtering. |
| `sort` | Keep scalar mapping suitable for sorting. |

The proof of concept currently creates mappings in the indexing service. That
keeps MongoDB free of search index metadata while allowing the Kafka/OpenSearch
side to own indexing concerns.

## Extension API Surface

The SDK now supports registering multiple source stages from one shared library.
The OpenSearch extension uses that to register:

- `$search`
- `$vectorSearch`

Both stages parse a user-friendly shape:

```javascript
{ $search: { path: "description", query: "waterproof shell", limit: 5 } }
{ $vectorSearch: { path: "description", query: "warm sleep system", limit: 5 } }
```

The extension derives the OpenSearch index from the MongoDB catalog namespace.
It sends the OpenSearch request, emits candidate documents shaped like
`{ _id, $searchScore }`, and expands to a host-created
`$_internalSearchIdLookup` stage so MongoDB restores fresh full documents.

The extension emits score metadata for the candidate rows. MongoDB 9.0 host ID
lookup restores full documents and preserves scores for downstream
`$meta: "searchScore"` / `$meta: "vectorSearchScore"` projection. Both search
stages declare `requiresInputDocSource: false` to generate candidates from
OpenSearch rather than passing through a collection scan.

## OpenSearch Queries

`$search` sends a native OpenSearch text query:

- `match` on the requested field;
- optional raw OpenSearch filter document;
- `_source: false`;
- result score copied into `$searchScore`.

`$vectorSearch` sends a native OpenSearch neural query:

- target field is `<path>_embedding`;
- query input is `query_text`;
- the query omits `model_id`;
- OpenSearch supplies the model through the configured search pipeline.

## Extension Config

The extension options contain only OpenSearch endpoints. The host config nests
them under `extensionOptions`:

```yaml
sharedLibraryPath: /usr/local/lib/mongo-extensions/libopensearch_extension.so
extensionOptions:
  endpoints: http://opensearch:9200,http://opensearch-2:9200
```

The extension tries endpoints in order with short request timeouts. If all
endpoints fail, the stage returns an error instead of hanging on connection
timeouts.

## Demo Data

The demo uses a stable synthetic outdoor retail catalog in
[`examples/opensearch/datasets/outdoor-products.json`](../../examples/opensearch/datasets/outdoor-products.json).
The dataset is small enough for deterministic tests and realistic enough for
manual search/vector-search demos.

## Current Limits

- Docker-only proof of concept.
- Single-node OpenSearch in the local compose stack.
- No TLS/auth in Docker.
- No production hardening for OpenSearch security, Kafka ACLs, or connector
  credentials.
- `$search` supports text query and an optional raw filter.
- `$vectorSearch` supports text-to-vector neural query, optional raw filter, and
  OpenSearch-side scoring.
- Score metadata is preserved through host `$_internalSearchIdLookup` and can
  be projected with `$meta`.
- Advanced Atlas Search semantics are intentionally outside the current subset.

## Verification

Primary checks:

```bash
./e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-e2e.sh
./examples/opensearch/run-demo.sh test
./examples/opensearch/run-demo.sh query
```

The demo stack is intentionally left inspectable with:

```bash
./examples/opensearch/run-demo.sh up
./examples/opensearch/run-demo.sh logs
```
