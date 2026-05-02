# Rust SDK for MongoDB Extensions

Rust workspace that ships **`extension-sdk-mongodb`**, the **Rust SDK for MongoDB Extensions**: libraries and macros for building **MongoDB server extensions**—`cdylib` plugins loaded by `mongod` that register aggregation stages behind the versioned C ABI in [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) (vendored from MongoDB’s public extension API).

The same repository also contains **sample extensions** and a **test harness**; those are documented separately so this file stays focused on the SDK crates.

**MongoDB Extensions ABI:** this tree targets the vendored C API **version 0.1** (`MONGODB_EXTENSION_API_MAJOR_VERSION` **0**, `MONGODB_EXTENSION_API_MINOR_VERSION` **1** in [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h)). Extensions built with the SDK advertise that pair at load time; the server must report a compatible slot in its extension API version vector (see [`extension-sdk-mongodb/src/version.rs`](extension-sdk-mongodb/src/version.rs)).

## Crates

| Crate | Role |
|--------|------|
| [`extension-sys-mongodb`](extension-sys-mongodb) | `#[repr(C)]` types and constants mirroring the host ABI |
| [`extension-sdk-mongodb`](extension-sdk-mongodb) | Status and byte-buffer helpers, panic-safe FFI shims, macros to export stages, BSON utilities, and typed errors for stage logic |

Depend on **`extension-sdk-mongodb`** from your extension crate; you normally do not need to depend on `extension-sys-mongodb` directly unless you are touching low-level symbols.

## Capabilities

The **Rust SDK for MongoDB Extensions** exposes macros that generate a single exported symbol, **`get_mongodb_extension`**, for the host to `dlsym`:

- **`export_transform_stage!`** — passthrough transform: documents pass upstream unchanged (optional empty stage document).
- **`export_map_transform_stage!`** — per-document map with optional **EOF-with-no-rows** path and optional **`initialize`** hook (extension YAML / portal).
- **`export_source_stage!`** — **generator** source stage when there is no upstream executable stage (e.g. empty collection with only your stage).

Typed arguments and errors are built around **`ExtensionError`**, **`parse_args`** (serde + BSON), and conversion to host status objects. See the crate root in [`extension-sdk-mongodb/src/lib.rs`](extension-sdk-mongodb/src/lib.rs) and module-level docs under `extension-sdk-mongodb/src/`.

## Using the Rust SDK for MongoDB Extensions in your extension

1. Add a path or crates.io dependency on **`extension-sdk-mongodb`**.
2. Set **`[lib] crate-type = ["cdylib"]`** so the compiler produces a shared library the server can load.
3. Invoke exactly **one** of the `export_*` macros so the unmangled **`get_mongodb_extension`** entry point exists.
4. Install the produced `*.so` and matching extension **`*.conf`** according to MongoDB’s extension host documentation for your server build.

Minimal passthrough example:

```rust
extension_sdk_mongodb::export_transform_stage!("$myRustPass", true);
```

Use a **unique** stage name. With `true`, the inner stage document must be empty, e.g. `{ "$myRustPass": {} }`.

## Building the Rust SDK for MongoDB Extensions

With a host toolchain (Rust **1.85+** recommended for the current dependency graph):

```bash
cargo build -p extension-sdk-mongodb --release
```

Without installing Rust locally, you can compile from a container (repository root mounted at `/build`):

```bash
docker run --rm -v "$PWD:/build" -w /build rust:bookworm \
  cargo build -p extension-sdk-mongodb --release
```

## Outside this README

| Topic | Where |
|--------|--------|
| Runnable **demo extensions** (Docker, mongosh, ports) | [`examples/README.md`](examples/README.md) |
| **Tests**, CI-style Rust runs, e2e against real `mongod`, fuzz, Miri, debugging | [`e2e-tests/README.md`](e2e-tests/README.md) |

Quick **`mongosh`** smoke after each example’s **`run-demo.sh up`** (see [`examples/README.md`](examples/README.md) for ports): Fibonacci **`use fibonacci_demo; db.createCollection("n"); db.n.aggregate([{ $fibonacci: { n: 10 } }])`**. HTTP fetch **`use http_fetch_demo; db.createCollection("n"); db.n.aggregate([{ $httpFetch: { url: "https://example.com/", maxBytes: 65536 } }])`** (outbound HTTPS from the container). Data federation **`use data_federation_demo; db.createCollection("n"); db.n.aggregate([{ $readLocalJsonl: { path: "sample.ndjson" } }])`** (fixtures at **`/federation-data`**; **`allowedRoot`** in extension **`*.conf`**).

## License

Rust crates and other original material in this repository are licensed under the **[MIT License](LICENSE)** (SPDX: `MIT`).

The vendored C header [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) is **not** under MIT: it remains under the **[Server Side Public License, v1 (SSPL-1.0)](https://www.mongodb.com/licensing/server-side-public-license)** as in the upstream MongoDB source. See **[`NOTICE`](NOTICE)** for attribution and compliance when you ship or link that file.
