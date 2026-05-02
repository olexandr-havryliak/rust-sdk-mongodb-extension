# Fibonacci example (`$fibonacci`)

Aggregation extension that implements **`$fibonacci: { n: <count> }`**: it streams documents **`{ i, value }`** along the Fibonacci sequence for indices `0 .. n` (with **`n`** capped at 10 000). Runs against **MongoDB 8.3** in Docker.

## What this demo shows

- How to implement a **source / generator** stage with **[`SourceStage`](../../extension-sdk-mongodb/src/source_stage.rs)** and **[`export_source_stage!`](../../extension-sdk-mongodb/src/lib.rs)**.
- How stage arguments are decoded with **serde** via **`parse_args`** and **`ExtensionError`** in the example crate.

## Stage shape

```javascript
{ $fibonacci: { n: <non-negative integer> } }
```

Emitted documents look like:

```text
{ i: 0, value: 0 }, { i: 1, value: 1 }, { i: 2, value: 1 }, …
```

## Behaviour

- **Empty collection** (no upstream rows): the stage acts as a **generator** and emits **`n`** rows as above.
- **Non-empty collection**: the stage **passthrough**s upstream documents unchanged (no Fibonacci merge). Use an empty collection (or a later pipeline) when you want only the generated sequence.

`aggregate: 1` with only **`$fibonacci`** is not accepted on MongoDB 8.3-rc for this stage (“a collection is required”). Prefer an **empty named collection** and **`db.n.aggregate([{ $fibonacci: { n: 10 } }])`**, or follow newer server releases.

Implementation: [`fibonacci-extension/`](fibonacci-extension/) (`cdylib`).

## Requirements

- Docker with Compose v2 (`docker compose`)
- Network access to pull **`mongo:8.3-rc-noble`** (or your override) and the Rust builder image used in the Dockerfile

## Run (from repository root)

```bash
chmod +x examples/fibonacci/run-demo.sh
./examples/fibonacci/run-demo.sh
```

This builds the image, starts MongoDB, runs `scripts/demo.js`, then tears the stack down. **`mongod`** is published on host port **27018** (leaves **`e2e-tests`** on **27017** free).

### Keep MongoDB running

```bash
./examples/fibonacci/run-demo.sh up
mongosh --port 27018
./examples/fibonacci/run-demo.sh down
```

Usage for **`run-demo.sh`**: **`./examples/fibonacci/run-demo.sh --help`**.

### Override the MongoDB image

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./examples/fibonacci/run-demo.sh up
```

## Manual Compose

From the **repository root**:

```bash
docker compose -f examples/fibonacci/docker-compose.yml --project-name fibonacci-example up --build
docker compose -f examples/fibonacci/docker-compose.yml --project-name fibonacci-example exec mongo mongosh /scripts/demo.js
```

## Try in mongosh

```bash
mongosh --port 27018
```

**Generator** (empty **`n`**):

```javascript
use fibonacci_demo;
db.n.aggregate([{ $fibonacci: { n: 10 } }]);
```

If **`n`** does not exist yet: **`db.createCollection("n")`** (leave it empty).

**Passthrough** (non-empty **`n`**): **`db.n.insertOne({ x: 1 }); db.n.aggregate([{ $fibonacci: { n: 99 } }])`** returns upstream rows unchanged (no Fibonacci fields).
