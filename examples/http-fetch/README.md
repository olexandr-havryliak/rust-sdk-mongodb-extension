# HTTP fetch example (`$httpFetch`)

Aggregation extension that implements **`$httpFetch`**: a blocking **HTTP GET** (similar to **`curl`**) for a **`url`** in the stage document. It returns **one document** when the pipeline reaches **EOF with zero upstream rows** (empty collection, **`$limit: 0`**, or an impossible **`$match`**). Runs against **MongoDB 8.3** in Docker.

## What this demo shows

- How to use **`export_map_transform_stage!`** with an **EOF-with-no-rows** handler so the stage emits a single synthetic document after upstream exhaustion.
- How to bundle **`ureq`** inside an extension for outbound HTTP (**demo only**—see **Behaviour**).

## Stage shape

**Arguments**

| Field | Required? | Default |
|--------|-----------|---------|
| **`url`** | **Yes** | — must start with **`http://`** or **`https://`** |
| **`maxBytes`** | No | `262144` (256 KiB), hard cap `2097152` |
| **`timeoutMs`** | No | `15000`, allowed range `100`–`120000` |

**Success document (fields)**

| Field | Meaning |
|--------|---------|
| **`httpFetch`** | Always **`true`** on the success path |
| **`url`** | URL that was requested |
| **`status`** | HTTP status code |
| **`contentType`** | `Content-Type` header (may be empty) |
| **`body`** | Response body as UTF-8 (lossy), truncated by **`maxBytes`** |
| **`bytes`** | Body length before decoding |
| **`error`** | Present on network / validation errors instead of the success payload |

## Behaviour

- The GET runs only on the **EOF / zero rows** path for the map stage; design your pipeline so **`$httpFetch`** sees that condition (see **Try in mongosh**).
- **Security:** this is a **server-side request forgery (SSRF)** primitive. Anyone who can run aggregation with this extension loaded can make **`mongod`** perform HTTP requests. Use **only in trusted demos** or behind strict controls (do not load in production without hardening: allowlists, separate networks, etc.).

Implementation: [`http-fetch-extension/`](http-fetch-extension/) (`cdylib`, **`ureq`**).

## Requirements

- Docker with Compose v2 (`docker compose`)
- Network access to pull **`mongo:8.3-rc-noble`** (or override), the Rust builder image, and **outbound HTTPS** from the container (the default demo fetches a public URL)

## Run (from repository root)

```bash
chmod +x examples/http-fetch/run-demo.sh
./examples/http-fetch/run-demo.sh
```

**`mongod`** is published on host port **27021** when the stack is up.

### Keep MongoDB running

```bash
./examples/http-fetch/run-demo.sh up
mongosh --port 27021
./examples/http-fetch/run-demo.sh down
```

Usage for **`run-demo.sh`**: **`./examples/http-fetch/run-demo.sh --help`**.

### Override the MongoDB image

```bash
MONGO_IMAGE=mongo:8.3.0-rc5-noble ./examples/http-fetch/run-demo.sh up
```

## Manual Compose

From the **repository root**:

```bash
docker compose -f examples/http-fetch/docker-compose.yml --project-name http-fetch-example up --build
docker compose -f examples/http-fetch/docker-compose.yml --project-name http-fetch-example exec mongo mongosh /scripts/demo.js
```

## Try in mongosh

Connect with **`mongosh --port 27021`** after **`run-demo.sh up`**.

**Empty collection `n`** (EOF path — same as `scripts/demo.js`):

```javascript
use http_fetch_demo;
db.createCollection("n");
db.n.aggregate([{ $httpFetch: { url: "https://example.com/", maxBytes: 65536 } }]);
```

**Non-empty collection** (force zero rows before **`$httpFetch`**), e.g.:

```javascript
use shop;
db.orders.insertOne({ sku: "demo" });
db.orders.aggregate([
  { $match: { $expr: { $eq: [0, 1] } } },
  { $httpFetch: { url: "https://example.com/", maxBytes: 32768 } },
]).toArray();
```

On success you get one document with **`httpFetch: true`**, **`status`**, **`body`**, and related fields; on failure **`error`** is set.
