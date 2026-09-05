# Development

The [SPEC](KnowMesh_v0.1_Technical_SPEC.md) defines the full v0.1 target.
[Tracking issue #1](https://github.com/CaiZongyuan/knowmesh/issues/1) owns the delivery
checklist. A completed foundation or a green subset of tests is not a v0.1 release.

## Prerequisites

- Rust 1.96.0 with rustfmt and clippy, selected by `rust-toolchain.toml`.
- Backend development does not require Node.js, a browser, or a database server.
- Dependency versions are recorded in `Cargo.lock`.

## Checks

Run from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo run -p knowmesh -- version
```

Core owns the domain, use cases, and wire contracts. The SQLite crate implements
storage ports. The executable owns CLI/HTTP adapters and dependency assembly.

## Verified Behavior

- Typed IDs reject another object's prefix, malformed payloads, and ULID overflow;
  valid IDs survive JSON round trips without changing identity.
- Content digests are SHA-256; timestamps serialize as RFC 3339 UTC with `Z`.
- `version` works outside a workspace and emits one JSON value on stdout.
- Invalid CLI arguments and unsupported formats produce typed JSON on stderr,
  an empty stdout, and the corresponding nonzero exit code.
- `schema list` and `schema command <operation>` expose the Core operation
  registry, including input/output JSON schemas, policy, effect, and support flags.

## TDD Evidence

| Issues | Red Evidence | Green Verification |
| --- | --- | --- |
| [KM-001 / #2](https://github.com/CaiZongyuan/knowmesh/issues/2), [KM-002 / #3](https://github.com/CaiZongyuan/knowmesh/issues/3) | Commit `3da473e`: core tests fail on missing domain/error modules; all three CLI tests fail on empty output and missing validation errors | `cargo +stable test --workspace`: 8 tests pass using installed Rust 1.96.0 |
| [KM-003 / #4](https://github.com/CaiZongyuan/knowmesh/issues/4) | Commit `d8c80d7`: registry tests fail on missing Application Core module | `cargo +stable test --workspace`: 11 tests pass, including schema discovery and unknown-operation errors |

The [foundation CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33976876366)
passed formatting, clippy, tests, and CLI version smoke checks on Linux, macOS,
and Windows for commit `77fc2ec`.

The foundation evidence does not validate the remaining SPEC workflows, supported
platform matrix, model quality, or release packages. Those gates remain tracked
by their implementation issues.
