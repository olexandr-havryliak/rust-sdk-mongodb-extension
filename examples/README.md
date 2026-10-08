# Example extensions

This directory holds **self-contained demos**: each example is a workspace member `cdylib` plus Docker assets and a `run-demo.sh` wrapper. They show how to use the **Rust SDK for MongoDB Extensions** against a real **MongoDB 9.0** image with the Extensions API enabled—not the SDK API reference (see the [repository root README](../README.md)).

All example images default to **`mongodb/mongodb-community-server:9.0-ubi9`**. Their Dockerfiles write extension manifests under **`/etc/mongo/extensions`** and start **`mongod`** with **`featureFlagExtensionsAPI=true`**, **`--extensionsConfigPath /etc/mongo/extensions`**, and **`--loadExtensions ...`**. Because these demo libraries are unsigned local builds, the scripts set **`featureFlagExtensionsApiSignatureValidation=false`** for the demo containers.

Each example’s **README** follows the same section order: summary → what the demo shows → stage shape → behaviour → requirements → run → keep stack running → image overrides → manual Compose → try in `mongosh`.

## Index

| Example | Stage | Host port | README |
|---------|--------|-----------|--------|
| **fibonacci** | `$fibonacci` (source / generator) | **27018** | [fibonacci/README.md](fibonacci/README.md) |
| **http-fetch** | `$httpFetch` (map + EOF) | **27021** | [http-fetch/README.md](http-fetch/README.md) |
| **data-federation** | `$readLocalJsonl` (JSONL under `allowedRoot`) | **27022** | [data-federation/README.md](data-federation/README.md) |
| **opensearch** | `$vectorSearch` backed by OpenSearch, with MongoDB -> Kafka -> OpenSearch connectors | **27030**, **8083**, **9200**, **5601** | [opensearch/README.md](opensearch/README.md) |

Use a **different host port per stack** so several examples (or `e2e-tests`) can run at once.

## Quick try (`mongosh`)

After **`./examples/<name>/run-demo.sh up`**, connect on that example’s port (see table above), then:

```javascript
// Fibonacci — port 27018
use fibonacci_demo;
db.createCollection("n");
db.n.aggregate([{ $fibonacci: { n: 10 } }]);

// HTTP fetch — port 27021 (empty `n` → EOF fetch path)
use http_fetch_demo;
db.createCollection("n");
db.n.aggregate([{ $httpFetch: { url: "https://example.com/", maxBytes: 65536 } }]);

// Data federation — port 27022 (fixtures → `/federation-data`; `allowedRoot` in extension conf)
use data_federation_demo;
db.createCollection("n");
db.n.aggregate([{ $readLocalJsonl: { path: "sample.ndjson" } }]);

// OpenSearch — full sync + extension query path
// Run `./examples/opensearch/run-demo.sh test` from the repository root.
```

## Layout (every example)

```text
examples/<name>/
  <crate>/              # cdylib; path dependency on ../../extension-sdk-mongodb
  Dockerfile            # build context = repository root
  docker-compose.yml
  scripts/              # mongosh scripts used by the demo
  run-demo.sh           # run from repository root
  README.md
```

To add a new example: copy this layout, register the crate in the root [`Cargo.toml`](../Cargo.toml) workspace `members`, and keep the README structure aligned with the existing examples.
