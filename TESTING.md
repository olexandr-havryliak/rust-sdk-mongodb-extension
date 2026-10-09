# Testing

All checks for the **Rust SDK for MongoDB Extensions** run in Docker.
Only Docker with Compose and Bash are needed on the host.

## Run Everything Locally

From the repository root:

```bash
bash ./run-workflows-local.sh
```

This runs every GitHub workflow's checks in order: formatting, Clippy,
unit/property/doc tests, dependency audit, C ABI/UBSan, ASan, the standard SDK
test script, MongoDB e2e plus aggregation fuzz, and Miri. Checks never overlap.
The pinned testing image is built once. A failed check does not skip later
checks; the final exit status is nonzero if any check failed. An image build
failure stops the run before checks start.

Compilation and nextest use two jobs/threads. Check containers use two CPUs;
the MongoDB e2e server uses `--wiredTigerCacheSizeGB 0.25`.
The e2e wrapper removes the MongoDB stack and its volumes after the smoke/fuzz
checks, including on failure. Miri is slow and runs last.
Nothing is installed on the host.

To reuse the already-built testing image:

```bash
TESTING_BUILD_IMAGE=0 bash ./run-workflows-local.sh
```

This executes the workflows' underlying checks, not GitHub Actions itself.
GitHub-only publication and artifact upload occur on GitHub, not locally.

## Individual Checks

The same entrypoints are used locally and by workflows:

```bash
bash .github/testing/fmt.sh
bash .github/testing/clippy.sh
bash .github/testing/tests.sh
bash .github/testing/audit.sh
bash .github/testing/abi.sh
bash .github/testing/asan.sh
bash .github/testing/sdk.sh
bash .github/testing/e2e.sh
bash .github/testing/miri.sh
```

Each of the first six commands builds/reuses the pinned image. Set
`TESTING_BUILD_IMAGE=0` to skip rebuilding it, or `TESTING_IMAGE` to change its tag.
The older SDK and Miri scripts use `RUST_TEST_IMAGE`; the all-check runner defaults
that to the testing image. Aggregation fuzz defaults to 1,500 iterations in the
workflow wrapper; override with `ITERATIONS`.

| Check | Scope | Output |
|---|---|---|
| Formatting | Entire Rust workspace, including examples | Result and rustfmt diff in logs; no JUnit |
| Clippy | SDK, sys, ABI fixture; all targets; warnings are errors | Cargo JSON diagnostics and JUnit |
| Unit/property | Entire workspace, doc tests, runner/report/workflow regression tests | Native nextest JUnit and command-level JUnit |
| Audit | Workspace lockfile against RustSec advisories | Audit JSON and JUnit |
| C ABI/UBSan | C header versus Rust layouts and C-to-Rust lifecycle | JUnit and C UBSan diagnostics |
| ASan | SDK/sys/fixture tests and C/Rust FFI harness | Native nextest JUnit, harness JUnit and ASan diagnostics |
| Standard SDK | `cargo test --workspace`, including doc tests | Console logs |
| E2e/fuzz | MongoDB extension load and randomized aggregation driver | Console logs |
| Miri | SDK library and selected integration tests | Console logs |

Testing logs and reports are in `reports/<check>/`. Missing or malformed native
JUnit and failed commands are failures, never empty passes. Property-test
counterexamples and shrinking diagnostics appear in the individual test output.
Doc tests and Python infrastructure tests currently have command-level cases.
Old native XML is removed before a new run so failures cannot reuse stale results.

## ABI And Sanitizers

The fixture in [.github/testing/abi-harness](.github/testing/abi-harness)
parses Rust bindings with `syn`, generating checks for sizes, alignments, field
offsets (including every vtable slot), enum values and API constants.
Clang independently evaluates the matching expressions against the vendored
C header. Opaque C forward declarations have no public layout and are excluded.
A deliberate layout mismatch must be detected.

The C harness loads the Rust library and checks version negotiation,
registration, parsing, cloning, expansion ownership, AST binding, compilation,
EOF, reopening/closing, buffer ownership and status disposal.
This is a minimal host fixture, not a replacement for MongoDB integration tests.

UBSan instruments the C harness only. ASan instruments Rust SDK tests and the
C/Rust boundary in separate build directories. The FFI build shares Clang's
ASan runtime through `-Zexternal-clangrt`, avoiding two runtimes in one process.
The pinned nightly builds the Rust standard library with instrumentation.

Negative controls run in isolated processes: signed integer overflow for C
UBSan, C heap overflow and Rust use-after-free for ASan. They pass only on a
nonzero exit with the expected diagnostic; an unrelated crash is not a pass.
Never use these deliberate memory-safety probes in application code.

No Clippy warning or audit advisory is silently suppressed. These checks do not
prove compatibility with every CPU architecture, glibc version or MongoDB setup.

## GitHub Workflows And JUnit

Six independent testing workflows run on PRs to `main`, pushes to `main`
and manual dispatch:

- [Formatting](.github/workflows/testing-fmt.yml)
- [Clippy](.github/workflows/testing-clippy.yml)
- [Unit/property tests](.github/workflows/testing-tests.yml)
- [Dependency audit](.github/workflows/testing-audit.yml)
- [C ABI/UBSan](.github/workflows/testing-abi.yml)
- [ASan](.github/workflows/testing-asan.yml)

Each has its own status and job, using the shared
[composite action](.github/actions/run-check/action.yml) for Docker execution,
publication and artifact retention. A failing workflow does not cancel other
types. New runs cancel obsolete runs of the same type for the same PR/branch.
The tool image is built once per job; build failures retain their logs.

JUnit is published by
[EnricoMi/publish-unit-test-result-action](https://github.com/EnricoMi/publish-unit-test-result-action)
in the job summary, also when a check fails. PR comments and additional check
runs are disabled, so fork PRs need no write permissions or secrets.
The action also fails for failed or inconclusive results. Formatting does not
invoke the publisher and produces no JUnit or custom Markdown report.

Artifacts `sdk-testing-<check>` retain logs, original JSON and JUnit for 14 days,
including failed runs; harness binaries are excluded. Actual hosted publication
requires a pushed commit/PR and cannot be reproduced without GitHub's context.

The existing [MongoDB e2e/fuzzer workflow](.github/workflows/e2e.yml) and
[Miri workflow](.github/workflows/miri.yml) remain independent. Their entrypoints
are also under `.github/testing`; domain fixtures and standalone scripts remain
in `e2e-tests`.

## Server Integration And Debugging

The standard Rust verification command remains:

```bash
./e2e-tests/run-sdk-tests-docker.sh
```

See [e2e-tests/README.md](e2e-tests/README.md) for server smoke tests, randomized
aggregation fuzz, Miri, logs and manual debugging. The e2e image uses official
`mongodb/mongodb-community-server:9.0-ubi9` with extension API flags.
Signature validation is disabled only for unsigned local test libraries,
not as production guidance.

Additional server scenarios:

- [Router-only lookup PoC](e2e-tests/router-lookup/README.md)
- [OpenSearch replica-set demo](examples/opensearch/README.md)
- [OpenSearch mongos demo](examples/opensearch-mongos/README.md)

Run service scenarios sequentially and stop/remove their containers and volumes
before starting another stack.
