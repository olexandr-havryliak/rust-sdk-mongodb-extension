# Tests and integration harness

This tree builds and runs **automated checks** for the workspace: **Rust unit and integration tests** (often in Docker), **end-to-end** smoke checks against a real **`mongod`** with the OpenSearch extension loaded, optional **aggregation fuzz**, and **Miri** over FFI-heavy tests in the **Rust SDK for MongoDB Extensions** (`extension-sdk-mongodb`).

The Docker e2e image is intentionally focused on the current OpenSearch-backed
`$search` / `$vectorSearch` PoC. Older example extensions such as Fibonacci and
Data federation remain in the repository as examples, but are not loaded into
this image.

---

## Prerequisites

- **Docker** with Compose v2 (`docker compose`)
- Network access to pull **`mongodb/mongodb-community-server:9.0-ubi9`** (or your `MONGO_IMAGE` override) and **`rust:bookworm`** (or `RUST_TEST_IMAGE` for Rust-in-Docker commands)

Extensions are **Linux-only** in upstream MongoDB; these scripts default to the official UBI9 community-server image.

---

## Building

### Extension `cdylib` (workspace crates)

- **E2E image**: [`Dockerfile`](Dockerfile) uses the **repository root** as Docker build context. It compiles **`opensearch_extension`** into one shared library, then writes its extension config under **`/etc/mongo/extensions`**.
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

Builds the e2e image, starts **`mongod`** from **`mongodb/mongodb-community-server:9.0-ubi9`** with **`featureFlagExtensionsAPI`**, **`--extensionsConfigPath /etc/mongo/extensions`**, and **`--loadExtensions opensearch,e2e,fibonacci,datafederation`**. It runs a load smoke test for **`$search`** and **`$vectorSearch`**, then successful aggregations for **`$rustSdkE2e`**, **`$fibonacci`**, and **`$readLocalJsonl`**. The OpenSearch smoke test expects connection errors because this stack does not start OpenSearch; that still verifies that MongoDB loaded and registered those stages. The local test libraries are unsigned, so the harness also sets **`featureFlagExtensionsApiSignatureValidation=false`**; production deployments should use the server’s expected signing/validation flow.

```bash
chmod +x e2e-tests/run-e2e.sh
./e2e-tests/run-e2e.sh
```

Override MongoDB base image:

```bash
MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9 ./e2e-tests/run-e2e.sh
```

### Random aggregation fuzz (Docker + live `mongod`)

Sends bounded random pipelines mixing **`$search`** and **`$vectorSearch`** on the same Compose stack as end-to-end. That image also loads **`$rustSdkE2e`**, **`$fibonacci`**, and **`$readLocalJsonl`**, which this driver does not call. Occasionally appends **`$match`** / **`$project`**. Not LLVM libFuzzer; uses **`maxTimeMS`** per aggregate and alternates empty vs non-empty collections. The stack does not start OpenSearch, so connection errors from those search stages are expected.

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

Scripts live under **`e2e-tests/scripts/`**. With the stack running, exec into the **`mongo`** service and run them by path (e.g. **`mongosh /scripts/opensearch_extension_load_e2e.js`**) or copy the pipeline into an interactive shell.

### AddressSanitizer on `opensearch_extension` (Linux, nightly)

Sanitizes the OpenSearch extension **`cdylib`** build (you still need a matching **`mongod`** to load it):

```bash
docker run --rm -v "$PWD:/build" -w /build rust:bookworm bash -lc '
  rustup toolchain install nightly --profile minimal --no-self-update
  rustup default nightly
  RUSTFLAGS="-Zsanitizer=address" cargo build -p opensearch_extension --release
'
```

Only use the produced **`libopensearch_extension.so`** in a test image or local **`mongod`** if you understand sanitizer runtime requirements.

### Common issues

- If **`mongod`** rejects **`--loadExtensions`**, **`--extensionsConfigPath`**, or stages are unknown, the server build may omit the Extensions API—use an image/build that matches [MongoDB source](https://github.com/mongodb/mongo) expectations for your branch.
- If a local unsigned `*.so` is rejected for signature validation, use this repository’s scripts as-is for demos/e2e; they explicitly disable signature validation only for local test images.
- **`aggregate: 1`** behaviour for extension-only pipelines can differ by server version; prefer named collections when scripts assume a collection exists.
