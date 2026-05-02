# Data federation example (`$readLocalJsonl`)

Proof-of-concept **SourceStage** that streams **JSON Lines** (JSONL) from the **MongoDB server host** filesystem into an aggregation pipeline. Paths in the stage document are **relative** and are always resolved under an **`allowedRoot`** directory supplied only via **extension configuration** (never from the client query alone). This is **not** MongoDB Data Federation; it shows how extensions can expose external file-backed sources.

## What this demo shows

- **`$readLocalJsonl`** with **[`SourceStage`](../../extension-sdk-mongodb/src/source_stage.rs)** / **[`export_source_stage!`](../../extension-sdk-mongodb/src/lib.rs)**.
- Typed arguments with **serde** (`path`, optional `maxDocuments`) and **extension options** parsed from the extension `*.conf` blob (see **Security model**).
- **Line-by-line** limits (`maxLineBytes`, `maxDocumentBytes`), optional **`maxDocuments`**, and **operation metrics** (`bytes_read`, `lines_read`, `documents_returned`, `empty_lines_skipped`, `parse_errors`) plus **`StageContext`** logging (file open, EOF, parse errors, rejected paths where applicable).

## Stage shape

**Arguments (aggregation stage document)**

| Field | Required? | Notes |
|--------|-----------|--------|
| **`path`** | **Yes** | Relative path under **`allowedRoot`** (POSIX `/`; no `..`, no leading `/`, no `\\`). |
| **`maxDocuments`** | No | Cap on emitted documents; pipeline returns **EOF** once the cap is reached. |

**Extension options** (same file as `sharedLibraryPath`, YAML-style lines or a single JSON object)

| Key | Meaning |
|-----|--------|
| **`allowedRoot`** | Absolute directory on the **server**; all `path` values resolve under this tree after canonicalization. |
| **`allowSymlinks`** | If **`false`**, symlinks in the resolution path are rejected and the file is opened with **`O_NOFOLLOW`** on Unix. |
| **`maxLineBytes`** | Max bytes per physical line (excluding the trailing newline) before parsing. |
| **`maxDocumentBytes`** | Max BSON-encoded size per emitted document. |

Example options (JSON):

```json
{
  "allowedRoot": "/var/lib/mongodb-extension-data",
  "allowSymlinks": false,
  "maxLineBytes": 1048576,
  "maxDocumentBytes": 16777216
}
```

The Docker images in this repo set **`allowedRoot: /federation-data`** in the extension `*.conf` and bind-mount **`examples/data-federation/fixtures`** there. If your **`mongod`** build does not forward custom keys from the manifest into the extension-options blob (only **`sharedLibraryPath`**, etc.), the stage still defaults **`allowedRoot`** to **`/federation-data`** so the demo and e2e stacks keep working—**set `allowedRoot` explicitly in production** when your server supports it.

## Behaviour

- **Empty collection**: the stage acts as a **generator**; each non-empty JSONL line that parses to a **JSON object** becomes one BSON document. Empty lines are skipped.
- **Non-empty collection**: same passthrough behaviour as other **`SourceStage`** examples (upstream rows pass through unchanged).

**Paths are always resolved on the server** where `mongod` runs, not on the machine running `mongosh`.

## Security model

- Arbitrary **absolute** paths from the query are **not** allowed; only **relative** `path` under **`allowedRoot`**.
- **`..`** and symlink escapes are rejected (see **`allowSymlinks`**).
- Treat **`allowedRoot`** as a dedicated read-only volume in demos; **never** point it at secrets or the whole host disk for untrusted workloads.

## Requirements

- Docker with Compose v2 (`docker compose`)
- Network access to pull **`mongo:8.3-rc-noble`** (or your override) and the Rust builder image

## Run (from repository root)

```bash
chmod +x examples/data-federation/run-demo.sh
./examples/data-federation/run-demo.sh
```

**`mongod`** listens on host port **27022** while the stack is up.

### Keep MongoDB running

```bash
./examples/data-federation/run-demo.sh up
mongosh --port 27022
./examples/data-federation/run-demo.sh down
```

### Override the MongoDB image

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./examples/data-federation/run-demo.sh up
```

## Manual Compose

From the **repository root**:

```bash
docker compose -f examples/data-federation/docker-compose.yml --project-name data-federation-example up --build
docker compose -f examples/data-federation/docker-compose.yml --project-name data-federation-example exec mongo mongosh /scripts/demo.js
```

## Try in `mongosh`

**Named collection** (generator path):

```javascript
use data_federation_demo;
db.createCollection("n");
db.n.aggregate([
  { $readLocalJsonl: { path: "sample.ndjson" } },
  { $match: { _fed: "orders" } },
]);
```

**Same pattern for `events.jsonl`** (empty collection **`n`**; on MongoDB **8.3-rc**, **`{ aggregate: 1, … }`** without a collection is rejected for this stage — use **`db.<coll>.aggregate`** instead):

```javascript
use data_federation_demo;
db.createCollection("n");
db.n.aggregate([
  { $readLocalJsonl: { path: "events.jsonl", maxDocuments: 1000 } },
  { $match: { level: "error" } },
  { $limit: 10 },
]).toArray();
```

Edit files under **`examples/data-federation/fixtures/`** on the host; the container sees them under **`/federation-data`**.

## Limitations

- **No** remote storage (S3, HTTP, etc.).
- **No** index, **no** predicate pushdown, **no** projection pushdown into the file reader.
- **JSONL only** (one JSON **object** per line); no CSV, Parquet, globs, directories-as-sources, or compression in this example.
- Intended for **demo and testing**, not production.

## Tests

- **Unit tests:** `cargo test -p data_federation_extension` — includes **`stage_extension_parameters`** (BSON **`path`** / **`maxDocuments`**) and **`extension_options_parameters`** (JSON and YAML **`allowedRoot`**, **`allowSymlinks`**, **`maxLineBytes`**, **`maxDocumentBytes`**, defaults, invalid UTF-8), plus path rules, JSONL parsing, and line limits.
- **E2E:** `./e2e-tests/run-e2e.sh` runs [`../e2e-tests/scripts/data_federation_e2e.js`](../e2e-tests/scripts/data_federation_e2e.js) against the shared stack (varied **`path`** / **`maxDocuments`** and downstream **`$match`**).

Implementation: [`data-federation-extension/`](data-federation-extension/) (`cdylib`).
