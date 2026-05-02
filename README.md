# Rust SDK for MongoDB Extensions

Rust workspace that ships **`extension-sdk-mongodb`**, the **Rust SDK for MongoDB Extensions**: libraries and macros for building **MongoDB server extensions**—`cdylib` plugins loaded by `mongod` that register aggregation stages behind the versioned C ABI in [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) (vendored from MongoDB’s public extension API).

The same repository also contains **sample extensions** and a **test harness**; those are documented separately so this file stays focused on the SDK crates.

**MongoDB Extensions ABI:** this tree targets the vendored C API **version 0.1** (`MONGODB_EXTENSION_API_MAJOR_VERSION` **0**, `MONGODB_EXTENSION_API_MINOR_VERSION` **1** in [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h)). Extensions built with the SDK advertise that pair at load time; the server must report a compatible slot in its extension API version vector (see [`extension-sdk-mongodb/src/version.rs`](extension-sdk-mongodb/src/version.rs)).

## Crates

| Crate | Role |
|--------|--------|
| [`extension-sys-mongodb`](extension-sys-mongodb) | `#[repr(C)]` types and constants mirroring the host ABI |
| [`extension-sdk-mongodb`](extension-sdk-mongodb) | Status and byte-buffer helpers, panic-safe FFI shims, macros to export stages, BSON utilities, and typed errors for stage logic |

Depend on **`extension-sdk-mongodb`** from your extension crate; you normally do not need to depend on `extension-sys-mongodb` directly unless you are touching low-level symbols.

## Execution model and lifecycle

MongoDB owns the query, the aggregation plan, and the cursor. Your extension is a **guest inside the server’s call stack**: the host calls into your exported C entry points; you return statuses and (when asked) BSON payloads. There is no separate async runtime inside the SDK—you finish the work for one host call and return.

At a high level, the lifecycle looks like this:

1. **Load** — The server `dlopen`s your library and resolves **`get_mongodb_extension`**. The SDK checks the host’s extension API version vector and registers your stage descriptor when the slot is compatible.
2. **Extension initialize** (optional) — If you use hooks such as **`on_init`** on a map transform, the host may call your extension **initialize** while a host **portal** is valid (e.g. to read extension manifest bytes once).
3. **Parse** — For each stage instance in a pipeline, the host supplies the **full stage BSON** (a single document whose sole top-level key is your stage name). The SDK decodes it, validates the key and inner argument object, and builds whatever parse node the host ABI expects.
4. **Open / bind** — When execution starts, **source** stages run **`open`**: arguments → owned **`State`**. **Transform** stages typically hold parsed args and wait for the first upstream row; the host wires your stage after an upstream executable stage when the pipeline requires it.
5. **Run (`get_next`)** — The host drives the cursor by calling **`get_next`** repeatedly. **Transforms** pull one advanced row from upstream (when present), apply **`transform`**, and return one output row or an error. **Sources** run **`next`** until you return **`Next::Advanced`** or **`Next::Eof`**. When a **source** stage is **not** the first executable stage, the bundled source implementation **forwards** **`get_next`** to the upstream stage (passthrough); the generator path applies when there is **no** upstream executable stage.
6. **Teardown** — Cursor completion or failure leads the host to drop execution objects; the SDK drops your **`State`** through the **`drop_state`** hook for source stages.

## Kinds of stages (what to implement)

- **Passthrough** — No per-document logic. **`export_transform_stage!`** only forwards documents and optionally enforces an empty inner document (`expect_empty`).
- **Transform (map)** — There **is** an upstream stream. For each **advanced** upstream document, you emit **one** output document. **`export_map_transform_stage!`** uses plain functions; **`TransformStage`** + **`export_transform_stage_type!`** gives typed **`parse`** + **`transform`** with **`StageContext`**. You may optionally supply **`on_eof`** when upstream ends **before** any row (empty collection) to synthesize a single row from arguments alone, and **`on_init`** for extension-level setup.
- **Source (generator)** — There is **no** upstream executable stage in front of you (e.g. **`aggregate: 1`**, or your stage is the only executable stage). You implement **`SourceStage`**: **`parse`**, **`open`**, **`next`**. If the pipeline later places another stage upstream of yours, the SDK’s source wrapper **delegates** **`get_next`** to that upstream stage instead of calling your **`next`**.

## Streaming vs “blocking” work

The host uses a **pull** model: each **`get_next`** asks for **at most one** logical advance for that stage boundary (one transformed row, or one **`Next`** result from your generator).

- **Streaming** here means *cursor-oriented*: you may keep internal buffers, file handles, or parsers in **`State`**, but each successful return that emits a row hands **one** document (and optional metadata) to the host for that call. You can read more data internally between calls; the next **`get_next`** continues where you left off.
- **Blocking** from an application perspective: your Rust code runs **synchronously** inside the host’s thread until you return. Long CPU or I/O work inside **`transform`** or **`next`** **blocks** that pipeline slice. Prefer bounded work per call, respect **`StageContext::check_interrupt`** and deadlines when the host binds execution context, and avoid unbounded allocations per row.

There is **no** built-in “async stage” API in this SDK layer—if you need backpressure or async I/O, you coordinate it yourself (e.g. incremental reads inside **`next`**) within the synchronous contract.

## `next` and `Next` semantics (source stages)

**[`Next`](extension-sdk-mongodb/src/stage_output.rs)** is the only way a **`SourceStage::next`** implementation signals progress to the host for the generator path:

- **`Next::Advanced { document, metadata }`** — Emit **exactly one** pipeline document for this **`get_next`** invocation. **`metadata`** is optional BSON carried alongside the row when the host supports it.
- **`Next::Eof`** — The generator is finished; **no** document is emitted for this call. After **`Eof`**, the host should not rely on further rows from this cursor for that stage instance.

You may **skip** work internally (e.g. ignore blank lines) and loop until you have a document to return or hit true end-of-input; the host still sees **one** **`next`** completion per **`get_next`** that advances the source.

## Memory and ownership

- **Opaque stage state** — For **`export_source_stage!`**, **`open`** builds **`State`** owned by a **`Box`**. The SDK converts it to a raw pointer for the host; **`drop_state`** runs when the host tears down the cursor. Do not leak that box outside the hooks the SDK provides.
- **BSON you return** — Output documents are serialized for the host according to the ABI helpers the crate uses internally (including **`byte_buf`**). Treat returned BSON as **handed off** to the host once the call succeeds; keep your own clones in Rust if you still need them.
- **Input slices** — During **parse**, stage BSON arrives as a **view** over host memory (`ByteView`). The SDK copies it into a **`bson::Document`** before your **`parse`** / **`open`** logic runs—do not hold pointers into the original view past the FFI shim.
- **Extension options snapshot** — **`StageContext::extension_options_raw`** returns an **owned** **`Vec<u8>`** when data is available (see module docs).

## Error handling

Stage logic should use **`ExtensionError`** ([`extension-sdk-mongodb/src/error.rs`](extension-sdk-mongodb/src/error.rs)) and the crate’s **`Result<T>`** alias:

- **`BadValue`** — Invalid arguments or disallowed values (user-facing validation).
- **`FailedToParse`** — BSON / serde shape mismatches (including **`parse_args`** failures).
- **`Runtime`** — Internal failures (I/O, invariant violations, etc.).
- **`HostError`** — Propagated from host callbacks (codes and reasons from the extension API).

**`ExtensionError::into_raw_status`** builds a heap **`MongoExtensionStatus`** for **`extern "C"`** returns. **`map_transform`** paths surface **`String`** errors from your callbacks and map them into failures at the boundary. Panics in SDK-wrapped entry points are caught so they **do not unwind across the FFI edge** ([`extension-sdk-mongodb/src/panics.rs`](extension-sdk-mongodb/src/panics.rs)); treat panics as bugs—return **`Result::Err`** for expected failures.

## BSON shape and arguments

Pipeline stages are documents with **exactly one** top-level field: the stage name (including **`$`**) must **match** the static name you registered (e.g. **`TransformStage::NAME`**, **`SourceStage::NAME`**). The **value** of that field is the **argument object** passed to **`parse`** / map callbacks.

- Use **`parse_args::<T>(args_document)`** ([`parse_args`](extension-sdk-mongodb/src/error.rs)) to deserialize the inner object with **serde** when you want typed fields and BSON-friendly scalars.
- **`expect_empty: true`** means the inner object must be **`{}`** (useful for passthrough markers).

Invalid BSON on the wire fails before your business logic runs, with a parse error classification.

## Talking to the host: `StageContext`

**[`StageContext`](extension-sdk-mongodb/src/stage_context.rs)** is passed into **`SourceStage::open`**, **`SourceStage::next`**, **`TransformStage::transform`**, and related hooks. It is the supported way to:

- **Log** at info / warn / error / debug (no-ops if the logger is unavailable).
- Read a cached **extension options** blob (manifest/config) when present.
- Update **operation metrics** (counters and timings serialized for the host).
- Query **deadlines** and call **`check_interrupt`** so long-running **`next`** / **`transform`** work can cooperate with kills and stepdowns.

Until the host binds query execution for a given **`get_next`**, some of these calls are deliberately **no-ops** or return **`None`**—see the type’s module documentation for the exact contract.

## Other contracts worth remembering

- **`Send + 'static`** — **`TransformStage`** and **`SourceStage`** implementations must be sendable and not borrow short-lived stack data across host calls.
- **One exported extension per `cdylib`** — The provided macros emit a single **`get_mongodb_extension`**; the stock layout assumes **one** logical stage registration per shared library.
- **ABI stability** — Follow the vendored header and the version structs in **`extension-sys-mongodb`**; do not assume layout beyond what the header documents.

## API quick reference

| Concept | API / type | Role |
|---------|------------|------|
| **Passthrough** | **`export_transform_stage!`** | Forwards upstream documents; optional empty inner args. |
| **Map transform** | **`export_map_transform_stage!`** | **`transform(row, args)`**; optional **`on_eof`**, **`on_init`**. |
| **Typed transform** | **`TransformStage`** + **`export_transform_stage_type!`** | Typed **`parse`** / **`transform(..., ctx)`**; see rustdoc in [`extension-sdk-mongodb/src/lib.rs`](extension-sdk-mongodb/src/lib.rs). |
| **Source / generator** | **`SourceStage`** + **`export_source_stage!`** | **`parse` → `open` → `next`** with **`Next`**. |
| **Execution context** | **`StageContext`** | Logging, options, metrics, deadlines, interrupts. |
| **Generator output** | **`Next`** | **`Advanced { document, metadata }`** or **`Eof`**. |
| **Planner BSON** | **`StageProperties`** (+ **`StreamType`**, **`StagePosition`**) | Override **`SourceStage::properties`** / **`TransformStage::properties`**; see **Planner static properties** above. |
| **Stage plan (model)** | **`StagePlan`** + **`ExecutionModel`** in **[`stage_model`](extension-sdk-mongodb/src/stage_model.rs)** | Single snapshot: planner properties, streaming vs blocking execution, lifecycle; defaults match the export traits. |
| **Blocking (buffered) logic** | **`BlockingStage`** | **`consume`** per row, **`finish`** → **`Vec<Next>`**; **`properties()`** defaults to the blocking plan; not exported to **`mongod`** yet. |

Typed arguments and errors also use **`ExtensionError`**, **`parse_args`**, and status helpers—see [`extension-sdk-mongodb/src/lib.rs`](extension-sdk-mongodb/src/lib.rs) and the modules under **`extension-sdk-mongodb/src/`**.

## Planner static properties (`get_properties`)

The host calls the AST node’s **`get_properties`** to obtain BSON aligned with MongoDB’s **`MongoExtensionStaticProperties`** IDL (see upstream [`extension_agg_stage_static_properties.idl`](https://github.com/mongodb/mongo/blob/v8.3/src/mongo/db/extension/public/extension_agg_stage_static_properties.idl)). The SDK maps that to Rust types in **[`stage_properties`](extension-sdk-mongodb/src/stage_properties.rs)** and groups planner + execution in **[`stage_model::StagePlan`](extension-sdk-mongodb/src/stage_model.rs)**:

- **`StreamType`** — **`Streaming`** or **`Blocking`** (host field **`streamType`**).
- **`StagePosition`** — **`Anywhere`**, **`First`**, or **`Last`** (host field **`position`**; **`Anywhere`** maps to IDL **`none`**).
- **`StageProperties`** — **`stream_type`**, **`position`**, **`requires_input`** (**`requiresInputDocSource`**); use **`StageProperties::to_document()`** or **`StagePlan::static_properties_document()`** for the BSON sent to **`get_properties`** (other IDL fields use host defaults when omitted).
- **`ExecutionModel`** — **`Streaming`** (pull / per-row) vs **`Blocking`** (buffer then **`finish`**), aligned with **`StageProperties::stream_type`** in the built-in **`StagePlan`** constructors.

**[`SourceStage`](extension-sdk-mongodb/src/source_stage.rs)** defaults match **`StagePlan::source_default()`**. **[`TransformStage`](extension-sdk-mongodb/src/transform_stage.rs)** defaults match **`StagePlan::transform_streaming_default()`** (same as **`StageProperties::default()`** / **`transform_stage_default()`**). For **collection-less-only** sources, override **`properties()`** as before. Map and passthrough macros use **`default_map_stage_static_properties()`**; **`export_transform_stage_type!`** uses **`<YourType as TransformStage>::properties()`**.

Both traits define **`fn expand(&Args) -> Expansion`** ([**`Expansion`**](extension-sdk-mongodb/src/expansion.rs), also re-exported under **`stage_model`**): **`SelfStage`** or **`Pipeline(Vec<Document>)`**. Typed transforms wire **`expand`** through **`export_transform_stage_type!`**; map-only macros keep the single-AST path.

Re-exports at the crate root: **`StageProperties`**, **`StreamType`**, **`StagePosition`**, **`StagePlan`**, **`ExecutionModel`**, **`StageLifecycleShape`**, **`Expansion`**, **`default_map_stage_static_properties`**.

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
| **Tests**, e2e, fuzz, Miri, scripts, debugging | [`e2e-tests/README.md`](e2e-tests/README.md) |

## License

Rust crates and other original material in this repository are licensed under the **[MIT License](LICENSE)** (SPDX: `MIT`).

The vendored C header [`include/mongodb_extension_api.h`](include/mongodb_extension_api.h) is **not** under MIT: it remains under the **[Server Side Public License, v1 (SSPL-1.0)](https://www.mongodb.com/licensing/server-side-public-license)** as in the upstream MongoDB source. See **[`NOTICE`](NOTICE)** for attribution and compliance when you ship or link that file.
