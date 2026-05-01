# Examples

Self-contained demos under **`examples/<name>/`**: each has its own Rust crate (workspace member), Docker assets, and optional `run-demo.sh`.

| Example | Description |
|--------|-------------|
| [**fibonacci**](fibonacci/README.md) | `$fibonacci` stage: Fibonacci sequence (map + empty-collection generator), MongoDB **8.3** via Docker |
| [**http-fetch**](http-fetch/README.md) | `$httpFetch` stage: HTTP GET for a `url` (curl-like; **SSRF-sensitive** demo, host **27021**) |

Add a new example by copying the `fibonacci/` layout: workspace member `examples/<name>/<crate>/`, then register the path in the root [`Cargo.toml`](../Cargo.toml).

## Shared layout (per example)

```
examples/<name>/
  <crate>/              # cdylib, depends on extension-sdk-mongodb with path ../../../extension-sdk-mongodb
  Dockerfile            # build context = repository root (see compose)
  docker-compose.yml
  scripts/              # mongosh tests, etc.
  run-demo.sh           # optional wrapper (run from repo root)
  README.md
```

Use a distinct **host port** per example so several stacks can run side by side (Fibonacci **27018**, http-fetch **27021**).
