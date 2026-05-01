# rust-sdk-mongodb-extension

Rust crates for building **MongoDB server extensions** (dynamic `*.so` plugins) against the
versioned C ABI in [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) (vendored
from MongoDB / Percona Server `src/mongo/db/extension/public/api.h`).

## Crates

| Crate | Role |
|--------|------|
| `extension-sys-mongodb` | `#[repr(C)]` definitions mirroring the public API |
| `extension-sdk-mongodb` | BSON helpers, status/byte-buffer adapters, `export_transform_stage!`, `export_map_transform_stage!` |

## Usage

1. Depend on `extension-sdk-mongodb` and set `[lib] crate-type = ["cdylib"]` in your extension crate.
2. Export exactly one unmangled symbol `get_mongodb_extension` via the macro:

```rust
extension_sdk_mongodb::export_transform_stage!("$myRustPass", true);
```

Use a unique stage name. With `true`, the stage document must be empty (e.g. `{ "$myRustPass": {} }`).

3. Install the shared library and a matching `*.conf` under the server’s extension config directory
   (see MongoDB extension host docs).

The bundled **passthrough** implementation forwards input documents from the upstream stage
unchanged (similar in spirit to the C++ `TestExecStage` used in server tests).

## Building

With a local toolchain:

```bash
cargo build -p extension-sdk-mongodb --release
```

Or only Docker (no host Rust):

```bash
docker run --rm -v "$PWD:/build" -w /build rust:bookworm \
  cargo build -p extension-sdk-mongodb --release
```

Rust **1.85+** is recommended when using a host toolchain (current `bson` / `time` dependency graph).

## End-to-end tests (Docker + MongoDB 8.3)

See [`e2e-tests/README.md`](e2e-tests/README.md). Quick run:

```bash
chmod +x e2e-tests/run-e2e.sh
./e2e-tests/run-e2e.sh
```

This builds a sample `cdylib`, layers it on `mongo:8.3-rc-noble` (override with `MONGO_IMAGE=...`), starts `mongod` with the Extensions API flag, and runs `$rustSdkE2e` in an aggregation pipeline (including **extension YAML** `e2eExtensionParam`, explain, non-empty stage args, a downstream stage, and an empty-collection EOF path).

Rust unit tests (SDK `api_coverage` + `e2e_extension` lib tests, plus a compile check of the Mongo fuzz driver) run **in Docker** so you do not need a host Rust toolchain:

```bash
chmod +x e2e-tests/run-sdk-tests-docker.sh
./e2e-tests/run-sdk-tests-docker.sh
```

Optional **random aggregation fuzz** against the same Docker `mongod`: [`e2e-tests/run-fuzz-e2e.sh`](e2e-tests/run-fuzz-e2e.sh) (see [`e2e-tests/README.md`](e2e-tests/README.md)).

**Miri** (memory / `unsafe` checks on SDK tests in Docker): [`e2e-tests/run-miri-docker.sh`](e2e-tests/run-miri-docker.sh) and [`.github/workflows/miri.yml`](.github/workflows/miri.yml).

## Example: Fibonacci stage (Docker + 8.3)

See [`examples/fibonacci/README.md`](examples/fibonacci/README.md) (index: [`examples/README.md`](examples/README.md)). Builds a `$fibonacci` map stage and runs it against `mongo:8.3-rc-noble` on port **27018**:

```bash
chmod +x examples/fibonacci/run-demo.sh
./examples/fibonacci/run-demo.sh          # automated demo then teardown
./examples/fibonacci/run-demo.sh up       # start MongoDB on port 27018 and keep it running
./examples/fibonacci/run-demo.sh down     # stop the example stack
```

## Example: HTTP fetch (`$httpFetch`, curl-like GET)

See [`examples/http-fetch/README.md`](examples/http-fetch/README.md). Fetches a URL over HTTP(S) inside the extension and returns the response in one EOF document (**demo only** — SSRF risk). Docker demo on port **27021**:

```bash
chmod +x examples/http-fetch/run-demo.sh
./examples/http-fetch/run-demo.sh
```

More examples live under [`examples/`](examples/README.md).

## License

The vendored `api.h` header remains under the [Server Side Public License (SSPL)](https://www.mongodb.com/licensing/server-side-public-license)
as in the upstream MongoDB source. Treat your combined extension artifact as subject to that
header’s license terms where applicable.
