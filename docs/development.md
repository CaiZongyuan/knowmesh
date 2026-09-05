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
- `init [path] --name <name> --template research` creates a portable workspace;
  `--dry-run` reports planned paths without creating the destination. Repeating
  identical initialization preserves the workspace ID and existing content.
- Workspace loading validates config versions, confines an optional Purpose to
  the workspace, caps it at 16 KiB, and resolves model secrets only when requested.
  The `general` template has no research Purpose. Initialization preflights
  existing paths, including `.gitignore`, before creating canonical files.
- `schema pack <id-or-id@version>` reads a configured pack through the same
  workspace resolution rules. Base, Research, and Clinical Preview are embedded;
  `init --template clinical` selects strict review defaults without a research
  Purpose. Clinical Preview is not a production clinical capability.
- Schema validation rejects cycles, ambiguous overrides, undefined relation
  endpoints, and invalid properties. Pack order does not change the schema hash.
  Detailed merge and property rules live in SPEC section 7.3.
- Core Source Library plans managed/referenced/snapshot imports and soft removal.
  Historical revisions cannot be rewritten; importing an already recorded hash
  returns that revision without moving the current head. Reading content verifies
  its size and SHA-256, including referenced files. Unchanged manifests round-trip
  byte-for-byte. Source CLI/API and the real URL fetch adapter are still pending.

Initialization now uses a durable file journal under `.knowmesh/transactions/`
and verified staging under `.knowmesh/staging/`. The Core coordinator can roll
forward after any file replacement; external edits or staged corruption stop
recovery and preserve its materials. Pending transactions block new writes.
The doctor command and SQLite reconciliation/rebuild integration remain under
KM-023, so the recovery workflow is not yet available through the CLI.

## TDD Evidence

| Issues | Red Evidence | Green Verification |
| --- | --- | --- |
| [KM-001 / #2](https://github.com/CaiZongyuan/knowmesh/issues/2), [KM-002 / #3](https://github.com/CaiZongyuan/knowmesh/issues/3) | Commit `3da473e`: core tests fail on missing domain/error modules; all three CLI tests fail on empty output and missing validation errors | `cargo +stable test --workspace`: 8 tests pass using installed Rust 1.96.0 |
| [KM-003 / #4](https://github.com/CaiZongyuan/knowmesh/issues/4) | Commit `d8c80d7`: registry tests fail on missing Application Core module | `cargo +stable test --workspace`: 11 tests pass, including schema discovery and unknown-operation errors |
| [KM-010 / #6](https://github.com/CaiZongyuan/knowmesh/issues/6) | Commits `658b109`, `6465a39`: missing workspace module and CLI init; invalid ignore paths cause partial writes | `cargo +stable test --workspace --locked`: 22 tests pass, including CLI dry-run/repeatability and workspace confinement |
| [KM-011 / #7](https://github.com/CaiZongyuan/knowmesh/issues/7) | Commits `fb1ee40`, `cfe3307`: missing schema module, pack CLI, and clinical template | `cargo +stable test --workspace --locked`: 33 tests pass, including DAG/override errors, relation/property constraints, deterministic hash, and strict Clinical Preview |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), file transaction layer | Commits `9d021cd`, `7a151f5`: missing coordinator, changed staging installed after preflight, reserved path aliases accepted | `cargo +stable test --workspace --locked`: 40 tests pass, including interruption after each replacement, recovery conflict preservation, staging revalidation, and writer exclusion |
| [KM-012 / #8](https://github.com/CaiZongyuan/knowmesh/issues/8), Source Library | Commits `794e5a8`, `e477c9c`: missing source model/store, unrelated files break enumeration, interrupted cleanup and case aliases fail | `cargo +stable test --workspace --locked`: 51 tests pass, including all storage modes, immutable revisions, soft removal, content integrity, portable transaction paths, and repeated cleanup |

The [foundation CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33976876366)
passed formatting, clippy, tests, and CLI version smoke checks on Linux, macOS,
and Windows for commit `77fc2ec`.
The [workspace CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33978958983)
also passed on all three operating systems for commit `2c2ca76`.

The foundation evidence does not validate the remaining SPEC workflows, supported
platform matrix, model quality, or release packages. Those gates remain tracked
by their implementation issues.
