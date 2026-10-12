# Rust SDK for MongoDB Extensions

## MongoDB extensions: a new idea

In 2025 MongoDB made it possible to load extensions into the server: separately
built libraries that plug into MongoDB's query engine. An extension registers
aggregation stages, and those stages go well beyond a single step. A stage can
expand into several stages, including MongoDB's built-in ones; take part in
query optimization; split its work across the servers of a cluster; and report
explain output, metrics, and logs.

MongoDB 8.3 shipped the first version of the server-extension contract, 0.1
([SERVER-117539](https://jira.mongodb.org/browse/SERVER-117539)), with
extensions off by default. MongoDB 9.0 raised it to 1.0
([SERVER-129395](https://jira.mongodb.org/browse/SERVER-129395)) and turned
extensions on by default
([SERVER-108108](https://jira.mongodb.org/browse/SERVER-108108)).

There is no public documentation for extensions yet. The current source of
truth is the server source, including the
[extension overview in the MongoDB repository](https://github.com/mongodb/mongo/blob/master/src/mongo/db/extension/README.md).

## Why Rust

Two goals appear to stand behind the API:

- **Development separate from the server.** An extension can be built, changed,
  and shipped without waiting for a new MongoDB release.
- **Safety by default.** An extension runs inside the database, and the raw C
  contract is unsafe to use directly. The overview names Rust as the language
  for production extensions; Rust rules out many memory errors before the code
  runs.

This repository is an attempt at that Rust layer: the unsafe part sits in one
place, and the extension's own logic stays in safe Rust. Details are in
[ARCHITECTURE.md](ARCHITECTURE.md).

Checking extension signatures is a separate switch,
`featureFlagExtensionsApiSignatureValidation`. In the 9.0 source it is off by
default; in the main development branch, the future 9.1, it is on by default
([SERVER-123691](https://jira.mongodb.org/browse/SERVER-123691)). The
extensions here are unsigned, so the demos turn the check off explicitly and
keep loading once it is on.

## Main example: vector search without mongot

Vector search finds documents by meaning rather than exact words. In MongoDB it
relies on mongot, a separate search service. In these examples MongoDB keeps the
documents, a separate vector database stores the vectors, and an extension adds
the `$vectorSearch` step that connects them. The examples focus on two things:

- **A production-grade search engine.** OpenSearch runs as a cluster with
  replicated data and is offered as a managed service by cloud providers. The
  extension is built for that setting: it accepts several OpenSearch addresses
  and moves to the next one if one fails, limits how long each request may
  take, and stops when MongoDB cancels the query.
- **Any vector database.** Documents reach the vector database through Kafka.
  Switching to another vector database, including one offered by a cloud
  provider, needs two changes: the Kafka connector plugin that writes to it,
  and the extension's query code. The AWS setup shows this with OpenSearch on
  AWS.

| Setup | Guide |
|-------|-------|
| Single server | [examples/opensearch](examples/opensearch/README.md) |
| Cluster | [examples/opensearch-mongos](examples/opensearch-mongos/README.md) |
| AWS | [examples/aws-opensearch](examples/aws-opensearch/README.md) |

Design notes: [docs/design/opensearch-mongot-replacement.md](docs/design/opensearch-mongot-replacement.md).

## Smaller examples

- [Fibonacci](examples/fibonacci/README.md) (`$fibonacci`) produces a sequence
  of Fibonacci numbers, one document per number.
- [HTTP fetch](examples/http-fetch/README.md) (`$httpFetch`) downloads a web
  page and returns it as one document.
- [Data federation](examples/data-federation/README.md) (`$readLocalJsonl`)
  reads records from a JSON Lines file in an allowed folder on the server and
  returns them as documents.

All examples are listed in [examples/README.md](examples/README.md).

## More documentation

| Topic | Where |
|-------|-------|
| How the SDK works | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Shared OpenSearch code | [opensearch-extension-core/README.md](opensearch-extension-core/README.md) |
| How the project is checked | [TESTING.md](TESTING.md) |

## License

Rust crates and other original material in this repository are licensed under the **[MIT License](LICENSE)** (SPDX: `MIT`).

The vendored C header [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) is **not** under MIT: it remains under the **[Server Side Public License, v1 (SSPL-1.0)](https://www.mongodb.com/licensing/server-side-public-license)** as in the upstream MongoDB source. See **[`NOTICE`](NOTICE)** for attribution and compliance when you ship or link that file.
