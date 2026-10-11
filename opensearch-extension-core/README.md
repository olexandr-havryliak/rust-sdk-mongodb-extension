# OpenSearch Extension Core

Shared OpenSearch implementation for the **Rust SDK for MongoDB Extensions**.
This crate is outside `examples/`: no example owns or imports another example.

- `src/lib.rs`: query parsing, configuration, neural queries, response decoding
  and replica-set ID lookup expansion.
- `src/transport.rs`: bounded HTTP requests, HTTPS and optional AWS SigV4.
- `src/mongodb.rs`: source-stage registration and native ID lookup.
- `src/mongos.rs`: router-only stage, typed full document keys and catalog-bound
  lookup for sharded and unsharded collections.

Each example owns its ABI wrapper, library name, Docker build, deployment,
configuration, scripts, datasets and tests. The core owns the shared behavior
tests, moved with their implementations; it has no dependency on example files.

Run workspace tests from the repository root, without a local Rust toolchain:

```bash
./e2e-tests/run-sdk-tests-docker.sh
```

See [TESTING.md](../TESTING.md) for the remaining SDK checks.
