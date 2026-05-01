# HTTP fetch example (`$httpFetch`)

Aggregation stage **`$httpFetch`** runs a blocking **HTTP GET** for a **`url`** string (similar to `curl`) and returns **one document** when the pipeline hits **EOF with zero upstream rows** (empty collection, `$limit: 0`, or an impossible `$match`).

## Output document

| Field | Meaning |
|--------|---------|
| **`httpFetch`** | Always `true` on success path |
| **`url`** | The URL that was requested |
| **`status`** | HTTP status code |
| **`contentType`** | `Content-Type` response header (may be empty) |
| **`body`** | Response body as UTF-8 (lossy); capped by **`maxBytes`** |
| **`bytes`** | Body length before decoding |
| **`error`** | Present on network / size errors instead of a full success payload |

## Stage arguments

| Field | Required? | Default |
|--------|-----------|---------|
| **`url`** | **Yes** | — must start with `http://` or `https://` |
| **`maxBytes`** | No | `262144` (256 KiB), max `2097152` |
| **`timeoutMs`** | No | `15000`, range `100`–`120000` |

## Example query (mongosh)

With **`mongod`** loading this extension (e.g. after `./examples/http-fetch/run-demo.sh up`, connect with **`mongosh --port 27021`**).

**Empty scratch collection** (same pattern as `scripts/demo.js`):

```javascript
use http_fetch_demo;
db.createCollection("_scratch");
db.getCollection("_scratch").aggregate([
  {
    $httpFetch: {
      url: "https://example.com/",
      maxBytes: 65536,
      timeoutMs: 15000,
    },
  },
]).toArray();
```

**Non-empty collection** (nothing must reach `$httpFetch` before EOF — guard with an impossible `$match`):

```javascript
use shop;
db.orders.insertOne({ sku: "demo" });
db.orders.aggregate([
  { $match: { $expr: { $eq: [0, 1] } } },
  {
    $httpFetch: {
      url: "https://example.com/",
      maxBytes: 32768,
    },
  },
]).toArray();
```

On success you get one document with **`httpFetch: true`**, **`status`**, **`body`**, etc.; on failure, **`error`** is set instead.

## Security (read this)

This is a **server-side request forgery (SSRF)** primitive: anyone who can run aggregation with this extension loaded can make **`mongod`** perform HTTP requests. Use **only in trusted demos** or behind strict controls (extension not loaded in production, separate network, allowlisted URLs in a hardened fork, etc.).

## Run the demo

From the **repository root** (needs Docker and outbound HTTPS from the container):

```bash
chmod +x examples/http-fetch/run-demo.sh
./examples/http-fetch/run-demo.sh
```

Leaves MongoDB on host port **27021** when using `run-demo.sh up`.

## Implementation

[`http-fetch-extension/`](http-fetch-extension/) — `ureq` for the GET, `export_map_transform_stage!` with EOF-only emission.
