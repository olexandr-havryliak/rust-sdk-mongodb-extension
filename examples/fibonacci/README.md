# Fibonacci example

Rust aggregation extension **`$fibonacci: { n: <count> }`** with **Docker Compose** and **MongoDB 8.3**.

The stage adds:

- `fibonacci`: array of the first `n` Fibonacci numbers (`0, 1, 1, 2, …`), capped at 10 000 terms
- `fibonacci_n`: the effective `n` that was used

**With input documents**, those fields are merged into each row (map over the collection).

**With an empty collection** (upstream EOF before the first document), the stage emits **one** document with only `fibonacci` and `fibonacci_n` (generator-style, similar to many C++ extensions):

```javascript
db.runCommand({
  aggregate: "emptyColl",
  pipeline: [{ $fibonacci: { n: 10 } }],
  cursor: {},
});
```

Implementation: [`fibonacci-extension/`](fibonacci-extension/) (`cdylib`) using [`export_map_transform_stage!`](../../extension-sdk-mongodb/src/lib.rs) with the optional **fourth** callback (`on_eof_no_rows`).

## Requirements

- Docker with Compose v2
- Network access to pull `mongo:8.3-rc-noble` and `rust:bookworm` (builder stage)

## Run the demo

From the **repository root**:

```bash
chmod +x examples/fibonacci/run-demo.sh
./examples/fibonacci/run-demo.sh          # build, start, run demo.js, then tear down
```

MongoDB listens on host port **27018** (avoids clashing with `e2e-tests` on 27017).

### Start the container only

```bash
./examples/fibonacci/run-demo.sh up
mongosh --port 27018
./examples/fibonacci/run-demo.sh down
```

See `./examples/fibonacci/run-demo.sh --help`.

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./examples/fibonacci/run-demo.sh up
```

## Manual compose

From the **repository root**:

```bash
docker compose -f examples/fibonacci/docker-compose.yml --project-name fibonacci-example up --build
docker compose -f examples/fibonacci/docker-compose.yml --project-name fibonacci-example exec mongo mongosh /scripts/demo.js
```

## Try in mongosh

```bash
mongosh --port 27018
```

```javascript
use fibonacci_demo;
db.n.insertOne({ x: 1 });
db.n.aggregate([{ $fibonacci: { n: 10 } }]).toArray();

db.createCollection("empty");
db.runCommand({
  aggregate: "empty",
  pipeline: [{ $fibonacci: { n: 6 } }],
  cursor: {},
});
```

Use `.toArray()` (or iterate the cursor) so mongosh prints results. The **three-argument** `export_map_transform_stage!` (no `on_eof_no_rows`) still yields **no** rows on an empty collection.
