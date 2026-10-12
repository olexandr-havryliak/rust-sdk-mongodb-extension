# OpenSearch Extension Core

Shared OpenSearch implementation for the **Rust SDK for MongoDB Extensions**.
This crate is outside `examples/`: no example owns or imports another example.

- `src/lib.rs`: query parsing, configuration, neural queries, response decoding
  and replica-set ID lookup expansion.
- `src/transport.rs`: bounded HTTP requests, HTTPS and optional AWS SigV4.
- `src/mongodb.rs`: source-stage registration and native ID lookup.
- `src/mongos.rs`: router-only stage, typed full document keys and catalog-bound
  lookup for sharded and unsharded collections.

## Query Safety Limits

The optional `$vectorSearch` `filter` is forwarded to OpenSearch as query DSL, so
only a closed set of clauses is accepted; anything else is rejected at parse
time:

- `bool` with `must`, `filter`, `should`, `must_not`, `minimum_should_match`
- `term` (scalar or `{value: <scalar>}`), `terms` (array of scalars, no lookup),
  `range` (`gt`, `gte`, `lt`, `lte`, `format`), `exists`, `ids`

Field names must not start with `_`. Nesting is limited to 8 levels and 256
clauses. Scripts, terms lookups, `more_like_this`, and pattern queries such as
`regexp`, `wildcard`, and `query_string` are not available.

Each OpenSearch attempt checks for MongoDB interrupts before sending the request
and again before using its result, and uses a timeout of at most 2 seconds,
shortened to the query deadline (`maxTimeMS`). A killed operation fails with the
host's error code. Responses larger than 4 MiB, or with more hits than `limit`,
are rejected.

Each example owns its ABI wrapper, library name, Docker build, deployment,
configuration, scripts, datasets and tests. The core owns the shared behavior
tests, moved with their implementations; it has no dependency on example files.

Run workspace tests from the repository root, without a local Rust toolchain:

```bash
./e2e-tests/run-sdk-tests-docker.sh
```

See [TESTING.md](../TESTING.md) for the remaining SDK checks.
Repository overview: [README](../README.md). Host lifecycle and stage contracts:
[ARCHITECTURE.md](../ARCHITECTURE.md). Example index:
[examples/README.md](../examples/README.md). OpenSearch stands:
[replica set](../examples/opensearch/README.md),
[mongos](../examples/opensearch-mongos/README.md),
[AWS](../examples/aws-opensearch/README.md).
Design notes: [opensearch-mongot-replacement.md](../docs/design/opensearch-mongot-replacement.md).
