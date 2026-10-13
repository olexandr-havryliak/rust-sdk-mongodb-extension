# Performance And Load Testing

These suites measure the **Rust SDK for MongoDB Extensions**, independently of
OpenSearch, embeddings, Kafka or AWS. Everything builds and runs in Docker;
no host Rust, Valgrind or Python installation is required.

## Run Locally

From the repository root:

```bash
# Criterion timing and Gungraun instruction/allocation measurements:
bash .github/testing/bench.sh all

# Identical HEAD harness against an existing baseline SDK commit and HEAD:
BASE_REF=HEAD~1 bash .github/testing/bench.sh all

# Fast toolchain/harness smoke; not a performance baseline:
BENCH_QUICK=1 bash .github/testing/bench.sh all

# MongoDB topologies must run sequentially. Each command removes its containers/volumes:
bash .github/testing/load.sh replica-set
bash .github/testing/load.sh mongos

# Both topologies, sequentially (also the default when no argument is given):
bash .github/testing/load.sh all
```

The first benchmark command builds the pinned tools image. Set
`BENCH_BUILD_IMAGE=0` on subsequent runs to reuse it. `timing` and `instructions`
can be run separately instead of `all`. Each load command builds the fixture and
driver images, then starts only its selected topology. `--help` lists the options.

For a smaller load smoke run:

```bash
LOAD_CONCURRENCY=1,4 LOAD_REQUESTS=2 bash .github/testing/load.sh replica-set
LOAD_CONCURRENCY=1,4 LOAD_REQUESTS=2 bash .github/testing/load.sh mongos
```

For a 30-minute soak, or a scheduled-arrival workload:

```bash
LOAD_SOAK_SECONDS=1800 bash .github/testing/load.sh replica-set
LOAD_TARGET_RPS=100 LOAD_CONCURRENCY=16 LOAD_REQUESTS=100 bash .github/testing/load.sh replica-set
```

Do not run a load topology and microbenchmarks simultaneously. Docker fixture
libraries are unsigned: signature validation is disabled only for these isolated
tests. The stacks have no TLS/auth and are not production configurations.

## What The Microbenchmarks Measure

- BSON document construction, encoding/decoding and owned-buffer allocation/free
  at payload sizes 256 B, 4 KiB, 64 KiB and 1 MiB. BSON framing adds bytes.
- Source execution with short results, full cursors, metadata and repeated EOF.
- SDK parse, expansion, AST bind, compile, open, get-next and destruction.
- Status allocation/free, error cleanup and host-owned metrics lifecycle.

`SdkCursor` invokes the real public SDK vtables, including their ownership
contract. `RawCursor` is a benchmark-only raw Rust C-ABI executor. Both use the
same deterministic source algorithm and SDK BSON-buffer/status helpers; their
documents and metadata are checked for equivalence before measurement.

The full-lifecycle comparison includes SDK planning/adapter/state-management
overhead. The raw reference is not a feature-equivalent MongoDB extension and is
not a claim about a zero-allocation or C++ baseline. End-to-end times include the
work described in the benchmark name, including output-buffer destruction.
One-time host registration is outside measurement.

Criterion provides wall-clock statistics and HTML charts. Gungraun uses
Callgrind instruction counts and DHAT allocation profiles; these are useful on
noisy shared runners but do not substitute for latency measurements. No hardware
`perf` privileges are required. Sanitizer/Miri runs are separate: their times are
not used as performance measurements.

Gungraun runs with `--allow-aslr=yes` under standard Docker permissions, without
relaxing seccomp. Reports record that ASLR is enabled. Address-layout-dependent
cache measurements can vary between runs; repeat comparisons before interpreting
small differences.

With `BASE_REF`, the current fixture/harness and dependency lock are held fixed;
only `extension-sdk-mongodb` and `extension-sys-mongodb` come from the baseline.
Baseline SDK APIs must be compatible with the harness. Both versions run
sequentially in the same pinned tool image with the same CPU limit. A compilation
failure is reported, not treated as a performance result. Worktree changes to SDK
files are included in the HEAD measurement; `HEAD_REF` identifies its parent Git
commit, not a guarantee that the worktree is clean.

## What The Load Suite Measures

The source fixture emits ordered `_id` values and deterministic payloads, with
optional vector score metadata. The map fixture adds `mapped: true` or passes the
document through unchanged. Neither performs external I/O.

The Rust driver measures full MongoDB requests, including client decoding and
document validation. The eight workloads cover short sources, multi-batch cursors,
64 KiB/1 MiB payloads, metadata, transforms, passthrough-map and a native MongoDB
map reference. The native reference is context, not isolated SDK overhead.

Default concurrency is `1,4,16,64`, with eight requests per worker in each phase.
`LOAD_REQUESTS` changes that count; `LOAD_WARMUP` controls unmeasured warmup
requests. Each result checks document contents, ordering, count and EOF.
Throughput counts only successful queries/documents; unexpected failures fail
the suite. Latency percentiles include failed attempts as well.

`LOAD_TARGET_RPS=0` uses an unpaced producer with bounded admission. A positive
value schedules request arrival times; the admission queue remains bounded and
overdue requests include queue/admission delay in their measured latency.
Inspect achieved throughput and load-generator CPU before attributing a limit
to MongoDB. This is not an unbounded arrival generator that silently drops work.

Correctness/stress controls cover early cursor close, repeated error/panic
paths followed by a healthy request, real `maxTimeMS` and `killOp` error codes,
and cancellation of an in-flight client future with bounded server recovery.
Client-future cancellation is not presented as a raw TCP-disconnect test.
Optional soak repeats bounded metadata/cursor rounds for the configured duration.

The operation probe uses `$currentOp` with `localOps: true`, so `killOp` targets
the router where the fixture executes rather than unrelated shard operations.
The same option has no effect on mongod. See the
[MongoDB operation-probe contract](https://www.mongodb.com/docs/manual/reference/operator/aggregation/currentop/).
Early-close checks wait for open server cursors to return to their baseline;
an idle operation list alone is not sufficient.

The replica-set stack has one single-node replica set. The mongos stack has one
router, one single-node CSRS and two single-node shards, with a hashed `_id`
input collection. Only mongos loads extensions in that topology. Every mongod
uses `--wiredTigerCacheSizeGB 0.25`; each service has CPU/memory limits. The
dedicated `rust_sdk_performance.input` dataset is replaced for each isolated run.

## Reports And Interpretation

Reports are under `reports/performance/`:

- `bench/checks.xml`: execution/contracts pass/fail, not latency thresholds.
- `bench/criterion/`: statistical results and HTML charts, including baseline comparisons.
- `bench/gungraun/`: instruction summaries and DHAT allocation profiles.
- `bench/environment.json`: toolchain, architecture, profile and reference identifiers.
- `load/<topology>/load.json` and `measurements.csv`: throughput and latency percentiles.
- `load/<topology>/checks.xml`: correctness/stress JUnit verdicts.
- `load/<topology>/resources.jsonl`: Docker CPU/memory statistics, including the driver while running.
- `load/<topology>/processes.jsonl`: server RSS, thread and FD samples through recovery.
- Server-status snapshots, container logs and image/Docker metadata accompany the results.

An empty/malformed report, unexpected error, monitor failure or setup failure
cannot establish success. Test reports are published separately from numeric
measurements. Shared GitHub runners do not provide a stable latency baseline;
there is deliberately no hard wall-clock regression gate. First calibrate noise,
repeat runs and examine base/head instruction/allocation changes.

RSS need not return to its cold-start value: allocator and database caches may
retain memory. A memory-leak conclusion needs sustained growth and allocation
evidence, not a single RSS delta. Resource snapshots are diagnostic, not a proof
of leak absence. Keep longer soak and sanitizer checks alongside these suites.

Server sampling failures fail the run. Driver samples are best-effort because
the one-off container can exit between inspection and collection; the monitor
records that race without failing otherwise successful correctness checks.

## GitHub Runs

[Benchmarks](.github/workflows/bench.yml) and [MongoDB load tests](.github/workflows/load.yml)
are independent workflows with read-only permissions. Both run on PRs, main
pushes and manual dispatch; Markdown-only changes skip them.

PR benchmarks use the PR base commit and quick Criterion settings. Other runs
use full settings; manual dispatch can select `base_ref`. The weekly Monday
benchmark uses `HEAD~1` by default. A baseline incompatible with the current
harness fails compilation instead of silently changing the experiment.

PR load checks use concurrency `1,4` and two requests per worker; other runs use
`1,4,16,64` and eight requests. The weekly Tuesday run adds a 900-second soak
per topology; manual dispatch accepts `soak_seconds` (0-7200). Topologies always
run sequentially, with cleanup in between.

The ready-made `EnricoMi/publish-unit-test-result-action@v2` publishes JUnit
correctness/execution results without PR comments or extra check runs. Numeric
results and logs are retained for 14 days as artifacts, also on failure.
Hosted execution/publication must still be verified after pushing the workflows.

General verification: [TESTING.md](TESTING.md).
