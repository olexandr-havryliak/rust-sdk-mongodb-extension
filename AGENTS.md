# Agent Guide — Rust SDK for MongoDB Extensions

This repository contains the **Rust SDK for MongoDB Extensions**: Rust crates,
examples, and Docker tooling for MongoDB server extensions loaded by `mongod` as
shared libraries.

These instructions apply to any automated or human-assisted coding agent working
in this repository.

---

## Product Naming

Use the name **Rust SDK for MongoDB Extensions** in documentation, comments meant
for users, release notes, and PR descriptions.

Do **not** use variants such as “MongoDB Extension SDK (Rust)” unless quoting an
external source.

---

## Working Principles

- Before implementation, always write a concise plan and wait for explicit user
  confirmation. Do not start editing or coding until the user has approved the
  plan.
- Explain intended changes clearly before performing them so the user can
  confirm the direction with enough context.
- If implementation reveals multiple reasonable approaches, unclear
  requirements, or uncertainty that could materially affect the result, stop and
  ask the user which path to take before continuing.
- Preserve existing user or agent changes. Do not revert unrelated files.
- Keep changes scoped to the requested behavior and the surrounding ownership
  boundary.
- Prefer repository patterns over new abstractions.
- Use structured parsers and APIs for structured data when available.
- Add comments only when they explain non-obvious behavior or invariants.
- Keep generated or mechanical churn out of commits unless it is required.
- Always clean up after implementation: remove obsolete code, stale docs,
  dead configuration, unused files, and leftovers from previous approaches when
  they are no longer part of the current design.

---

## Test-Driven Development

TDD is mandatory in this repository.

1. Add a failing test that captures the intended behavior or bug.
2. Run the relevant test and confirm it fails for the expected reason.
3. Implement the smallest change that makes the test pass.
4. Refactor only while keeping tests green.

Every new feature or bug fix should include focused test coverage. Prefer several
small tests over one broad test.

---

## Verification

Do not assume a local Rust toolchain is installed. This project standardizes Rust
verification in Docker.

After meaningful Rust changes, run from the repository root:

```bash
./e2e-tests/run-sdk-tests-docker.sh
```

Agents must execute this script when verifying Rust tests unless Docker is
unavailable in the current environment. If Docker is unavailable, say so clearly
and still leave the change test-complete.

Broader checks:

- End-to-end MongoDB extension flow: `./e2e-tests/run-e2e.sh`
- Miri / undefined behavior checks: `./e2e-tests/run-miri-docker.sh`
- Additional compose flows: see `e2e-tests/README.md`

For documentation-only changes, `git diff --check` is usually sufficient unless
the documentation change affects commands, scripts, Dockerfiles, or expected
runtime behavior.

---

## Rust Guidelines

- Use `#[test]` unit tests in crate `src/` when behavior is local.
- Use crate `tests/` integration tests when testing public API, FFI surface, or a
  separate test binary is useful.
- Prefer real behavior over mocks when practical.
- Use BSON documents in tests so shapes match what MongoDB sends and receives.
- Cover FFI boundaries when feasible, including panic boundaries, status handling,
  vtable layout, and buffer ownership.

---

## MongoDB Extension Guidelines

Treat each aggregation stage as a testable unit.

Prefer tests that cover:

- Stage parsing: stage document, stage name, and arguments.
- Stage execution: `get_next`, EOF, passthrough, and error semantics.
- BSON input/output shape.
- ABI compatibility when changing `extension-sys-mongodb` or
  `include/mongodb_extension_api.h`.

For source stages, assert full output sequences, including order and document
contents.

For transform or map stages, assert explicit input-to-output mappings.

When updating the MongoDB Extensions ABI:

- Sync Rust `#[repr(C)]` types with the vendored C header.
- Keep `include/mongodb_extension_api.h` and `extension-sys-mongodb/src/abi.rs`
  consistent.
- Add or update tests that fail on vtable slot drift or version mismatch.
- Run both SDK tests and the Docker e2e flow when runtime ABI behavior can be
  affected.

---

## Docker And MongoDB Images

The repository currently targets the official MongoDB community-server image used
by the scripts and examples. Keep README files, Dockerfiles, compose files, and
shell scripts aligned when changing the MongoDB image or server flags.

Local e2e/demo images may disable extension signature validation for unsigned
development libraries. Document that as a local testing choice, not as production
guidance.

---

## Anti-Patterns

Avoid:

- Shipping implementation without tests.
- Adding tests only after implementation.
- Skipping tests because a change seems simple.
- Ignoring failing tests.
- Relying on `println!` debugging instead of assertions.
- Reformatting unrelated code.
- Hiding ABI or Docker behavior changes in documentation-only commits.

---

## Default Loop When Unsure

1. Read the relevant code and docs.
2. Present a short plan and wait for explicit user confirmation.
3. Write the smallest failing test that describes the desired behavior.
4. Run the relevant Docker verification.
5. Implement the minimal fix.
6. Clean up obsolete code and stale artifacts introduced or exposed by the
   change.
7. Re-run verification.
8. Summarize what changed and what was verified.
