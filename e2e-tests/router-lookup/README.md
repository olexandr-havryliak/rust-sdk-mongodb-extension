# Router-only lookup proof of concept

Experimental, not the sharded OpenSearch integration. This harness isolates
post-bind distributed planning in the Rust SDK for MongoDB Extensions. It does
not change MongoDB server code and does not use a separate MongoDB client inside
the extension.

## Run

From the repository root, with Docker and Compose available:

```bash
./e2e-tests/run-sdk-tests-docker.sh
bash e2e-tests/run-router-lookup-poc.sh
```

The second command builds the library, initializes the cluster, runs assertions,
and removes its containers, network, and anonymous volumes on exit. No local
Rust toolchain or mongosh is needed. `MONGO_IMAGE` overrides the default official
`mongodb/mongodb-community-server:9.0-ubi9` image.

## Topology and assertions

- One mongos with only the test extension loaded.
- One single-node config-server replica set, `csrs`.
- Two single-node shard replica sets, `shard0` and `shard1`, without extensions.
- Fixtures in `sdk_router_poc.products` and `sdk_router_other.articles`, each
  split at `_id: 0`, with two documents physically stored on each shard.
- No OpenSearch, Kafka, TLS, or authentication. Extension signature validation
  is disabled on mongos only for this unsigned development library.

The test stage accepts candidate rows, not a collection name:

```javascript
db.products.aggregate([
  {$routerLookupPoc: {candidates: [{_id: 2, score: 0.95}, {_id: -2, score: 0.9}]}}
])
```

AST bind supplies the namespace. `SourceStage::merging_pipeline()` then builds a
native `$lookup`, `$unwind`, and `$replaceWith` suffix for the merger. The SDK
prepends an owned logical clone of the source and prevents recursive DPL on
that clone. For explicit generators, the shard component is a native
`$match: {$expr: false}`: MongoDB optimizes it to EOF, avoiding a collection scan
before candidate generation. Native stages are passed through the existing host parse-node API.
The API header and ABI layout are unchanged.

Expected output consists of current full MongoDB documents in candidate order
with each candidate's score, omitting missing IDs and preserving repeated IDs.
For this isolated PoC, score is an ordinary field, not `$meta: "vectorSearchScore"`.
The assertions also cover empty candidates, updated documents, and namespace
selection, but stop at the first failure.

## Verified result

On MongoDB **9.0.2**, the runtime test passes. Confirmed:

- Both shards contain data and reject the unknown `$routerLookupPoc` stage.
- The router's explain reports `mergeType: "router"` and a merger pipeline
  containing the extension followed by the automatically generated `$lookup`.
- `$lookup.from` is obtained from bind; the caller does not supply it.
- Both namespaces return current full documents in candidate order, preserving
  scores and repeated IDs while omitting missing IDs.
- Empty candidates produce no documents; updates are visible on the next lookup.
- The candidate-generation shard plans contain no `COLLSCAN`.

The host still inserts `$mergeCursors` before the source. The SDK now honors
`requiresInputDocSource: false`, so the source generates candidates without
reading this upstream. Input-requiring sources retain the existing passthrough
and empty-input generator fallback. Unit tests cover both router and replica-set
catalog contexts, upstream errors, EOF, and panic containment.

The existing OpenSearch extension for a plain replica set is unchanged. Its
Docker regression suite passes, including full-document `$vectorSearch` with
`$meta: "vectorSearchScore"`, CRUD synchronization, and connector lifecycle.
That extension does not enable DPL, so it keeps its native ID lookup expansion.

This confirms late native lookup on the router without server changes. It does
not yet implement OpenSearch querying from mongos: any such integration will
use a separate extension, leaving the replica-set extension separate. Other
out-of-scope cases include views, transactions, authentication, and production
HA; this PoC uses single-node replica sets.

See the [test overview](../README.md) and
[test extension](../router-lookup-extension/src/lib.rs).
