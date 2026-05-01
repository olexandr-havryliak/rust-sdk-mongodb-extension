# Docker end-to-end tests

These tests build the sample `e2e-tests/e2e-extension` `cdylib`, bake it into the official
**MongoDB 8.3** image family (`mongo:8.3-rc-noble` by default), start `mongod` with
`featureFlagExtensionsAPI` and `--loadExtensions e2e`, then run `mongosh` checks against **`$rustSdkE2e`**.

The extension reads **`e2eExtensionParam`** from the extension YAML in [`Dockerfile`](Dockerfile) (`e2e.conf`) at **`initialize`**, stores it for the process lifetime, and merges **`rustSdkE2eExtensionParam`** into each output document (and into the synthetic document produced on an **empty** collection via `on_eof_no_rows`). That verifies **parametrized extensions** end-to-end against a real `mongod`.

The script also covers non-empty stage args, explain, downstream `$match`, and a small batch cursor.

**Rust unit tests** (API coverage + `e2e_extension` YAML parser tests, plus a **`cargo check`** of the Mongo fuzz driver) run **inside Docker** — no local `cargo` needed:

```bash
chmod +x e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-sdk-tests-docker.sh
```

Override the builder image with `RUST_TEST_IMAGE=...` if required.

## MongoDB aggregation fuzz (Docker + live `mongod`)

The binary crate **`mongo_extension_fuzz`** sends random aggregation pipelines to a running server. It targets **`$rustSdkE2e`** (matches the e2e image).

This is **not** LLVM libFuzzer; it uses bounded random BSON, alternates empty vs non-empty source collections, and sets server **`maxTime`** per aggregate to avoid hangs.

```bash
chmod +x e2e-tests/run-fuzz-e2e.sh
ITERATIONS=5000 ./e2e-tests/run-fuzz-e2e.sh
```

Optional environment: `ITERATIONS`, `SEED`, `PER_ITER_TIMEOUT_MS`, `FUZZ_DATABASE`, `MONGO_IMAGE`.

## Miri (undefined-behavior checks on `unsafe`)

Slow, but useful for the SDK’s FFI-heavy paths (mocked integration tests, not `proptest`):

```bash
chmod +x e2e-tests/run-miri-docker.sh
./e2e-tests/run-miri-docker.sh
```

Scheduled / manual workflow: [`.github/workflows/miri.yml`](../.github/workflows/miri.yml).

## Optional: AddressSanitizer on `e2e_extension` (Linux, nightly)

Sanitizes the sample `cdylib` (host still involved when you run `mongod`). Example:

```bash
docker run --rm -v "$PWD:/build" -w /build rust:bookworm bash -lc '
  rustup toolchain install nightly --profile minimal --no-self-update
  rustup default nightly
  RUSTFLAGS="-Zsanitizer=address" cargo build -p e2e_extension --release
'
```

Use the produced `libe2e_extension.so` in your own image or local `mongod` only if you understand sanitizer runtime requirements.

## Requirements

- Docker with Compose v2 (`docker compose`)
- Network access to pull `mongo:8.3-rc-noble` and `rust:bookworm` (builder stage)

## Run

From the repository root:

```bash
chmod +x e2e-tests/run-e2e.sh
./e2e-tests/run-e2e.sh
```

Use another 8.3 tag if needed:

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./e2e-tests/run-e2e.sh
```

## Manual compose

```bash
docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e up --build
docker compose -f e2e-tests/docker-compose.yml --project-name rust-sdk-mongo-e2e exec mongo mongosh
```

## Notes

- Extensions are **Linux-only** in upstream MongoDB; the image must be a Linux variant (`*-noble`, etc.).
- If your `mongod` build omits the Extensions API, startup will fail or the stage will be unknown; use a build that matches the server in [`percona-server-mongodb`](https://github.com/percona/percona-server-mongodb) / MongoDB source tree if the official image does not match expectations.
