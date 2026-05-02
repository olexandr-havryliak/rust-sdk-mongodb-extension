# Tests and integration harness

This tree builds and runs **automated checks** for the workspace: **Rust unit and integration tests** (often in Docker), **end-to-end** pipelines against a real **`mongod`** with extensions loaded, optional **aggregation fuzz**, and **Miri** over FFI-heavy tests in the **Rust SDK for MongoDB Extensions** (`extension-sdk-mongodb`).

Sample extension code for e2e lives in **`e2e-extension/`**; the **Fibonacci** example `cdylib` is also baked into the same image where the compose file references it. See individual scripts for exact stage names and assertions.

---

## Prerequisites

- **Docker** with Compose v2 (`docker compose`)
- Network access to pull **`mongo:8.3-rc-noble`** (or your `MONGO_IMAGE` override) and **`rust:bookworm`** (or `RUST_TEST_IMAGE` for Rust-in-Docker commands)

Extensions are **Linux-only** in upstream MongoDB; use Linux image variants (`*-noble`, etc.).

---

## Building

### Extension `cdylib` (workspace crates)

- **E2E image**: [`Dockerfile`](Dockerfile) uses the **repository root** as Docker build context. It compiles **`e2e_extension`** (and bundles **Fibonacci** per compose/build args).  
  Trigger a build with **`./e2e-tests/run-e2e.sh`** or by running Compose **up --build** (see **Executing**).

### Rust crates without a full Mongo stack

From the repo root, either use a local **`cargo`** or the helper script that runs **`cargo test --workspace`** inside **`rust:bookworm`** (see **Executing → Rust workspace tests in Docker**).

### Fuzz binary

The **`mongo_extension_fuzz`** crate is a normal workspace member; **`cargo test`** builds it. The **fuzz driver** against a live server is a **binary**, not libFuzzer—build is implied when you run **`./e2e-tests/run-fuzz-e2e.sh`** after the stack is up.

---

## Executing

### Rust workspace tests (Docker, no host `cargo`)

Runs **`cargo test --workspace`** in a container with the repo mounted:

```bash
chmod +x e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-sdk-tests-docker.sh
```

**Workspace tests + Miri (no E2E):** `chmod +x e2e-tests/run-all-checks-docker.sh && ./e2e-tests/run-all-checks-docker.sh`  
Skip Miri: **`SKIP_MIRI=1 ./e2e-tests/run-all-checks-docker.sh`**.

Override the toolchain image: **`RUST_TEST_IMAGE=my-registry/rust:nightly ./e2e-tests/run-sdk-tests-docker.sh`**.

### End-to-end: `mongod` + `mongosh` scripts

Builds the e2e image, starts **`mongod`** with **`featureFlagExtensionsAPI`** and **`--loadExtensions`**, then runs the bundled checks (including **`$rustSdkE2e`**, **`$fibonacci`**, **`$readLocalJsonl`**, extension YAML **`e2eExtensionParam`**, explain, non-empty args, downstream **`$match`**, empty-collection EOF path, batch cursor). **`$fibonacci`** ([`fibonacci_source_e2e.js`](scripts/fibonacci_source_e2e.js)) covers **SourceStage**: **`aggregate: 1`** when the server allows it (no collection / no upstream), **empty collection** (scan EOF → generator), and **non-empty collection** (passthrough). **`$readLocalJsonl`** ([`data_federation_e2e.js`](scripts/data_federation_e2e.js)) reads JSONL from **`allowedRoot`** (**`/federation-data`**, fixtures bind-mount): full read + **`$match`**, **`maxDocuments`** caps on **`sample.ndjson`** / **`events.jsonl`**, a nested **`path`**, and **`maxDocuments: 0`** (empty cursor).

```bash
chmod +x e2e-tests/run-e2e.sh
./e2e-tests/run-e2e.sh
```

Override MongoDB base image:

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./e2e-tests/run-e2e.sh
```

### Random aggregation fuzz (Docker + live `mongod`)

Sends bounded random pipelines mixing **`$rustSdkE2e`**, **`$fibonacci`**, and **`$readLocalJsonl`** (same stack and extensions as e2e). Occasionally appends **`$match`** / **`$project`**. Not LLVM libFuzzer; uses **`maxTimeMS`** per aggregate and alternates empty vs non-empty collections.

```bash
chmod +x e2e-tests/run-fuzz-e2e.sh
ITERATIONS=5000 ./e2e-tests/run-fuzz-e2e.sh
```

Optional environment: **`ITERATIONS`**, **`SEED`**, **`PER_ITER_TIMEOUT_MS`**, **`FUZZ_DATABASE`**, **`MONGO_IMAGE`**.

### Miri (undefined behaviour checks on `unsafe`)

Slow; runs **`cargo miri test -p extension-sdk-mongodb --lib`** plus integration tests (**`proptest_byte_buf_roundtrip`** is skipped — fork harness). On PRs and pushes to **`main`**, see [`.github/workflows/miri.yml`](../.github/workflows/miri.yml).

```bash
chmod +x e2e-tests/run-miri-docker.sh
./e2e-tests/run-miri-docker.sh
```

CI reference: [`.github/workflows/miri.yml`](../.github/workflows/miri.yml).

---

## Debugging

### Manual Compose (keep containers up)

```bash
docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e up --build
docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e exec mongo mongosh
```

Inspect logs:

```bash
docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e logs -f mongo
```

### Re-run individual `mongosh` scripts

Scripts live under **`e2e-tests/scripts/`**. With the stack running, exec into the **`mongo`** service and run them by path (e.g. **`mongosh /scripts/<file>.js`**) or copy the pipeline into an interactive shell.

### AddressSanitizer on `e2e_extension` (Linux, nightly)

Sanitizes the sample **`cdylib`** build (you still need a matching **`mongod`** to load it):

```bash
docker run --rm -v "$PWD:/build" -w /build rust:bookworm bash -lc '
  rustup toolchain install nightly --profile minimal --no-self-update
  rustup default nightly
  RUSTFLAGS="-Zsanitizer=address" cargo build -p e2e_extension --release
'
```

Only use the produced **`libe2e_extension.so`** in a test image or local **`mongod`** if you understand sanitizer runtime requirements.

### Common issues

- If **`mongod`** rejects **`--loadExtensions`** or stages are unknown, the server build may omit the Extensions API—use an image/build that matches [MongoDB source](https://github.com/mongodb/mongo) expectations for your branch.
- **`aggregate: 1`** behaviour for extension-only pipelines can differ by server version; prefer named collections when scripts assume a collection exists.
