# KnowMesh

Local, evidence-backed knowledge workspaces built around a Rust CLI, canonical
Markdown/YAML, SQLite projections, and reviewed proposals.

**Development status:** v0.1 is being implemented. The current executable supports
`knowmesh init [path] --template research`, `knowmesh version`, and operation discovery with `knowmesh schema list` /
`knowmesh schema command <operation>` / `knowmesh schema pack <id>`, plus local
`source add/list/get/content`, confirmed `source remove`, paginated `source impact`, `sync`, `status`,
`doctor`, and `rebuild`;
the complete ingestion, knowledge, search, graph, proposal,
and optional Web workflows are not released yet. The npm bootstrap package does
not contain an executable.

The Core library also provides canonical parsers, recoverable file transactions,
and atomic SQLite projection reconciliation. Doctor supports explicit transaction
repair, including interrupted initialization with missing configuration.
Rebuild preserves runtime state and backs up the old database before replacement.
URL imports store immutable snapshots with address checks, bounded redirects,
download limits, and timeouts. Private targets require explicit local CLI approval
through `--allow-private-network`.
Source removal previews include affected knowledge and preserve the disk index.
Source lists support cursor pagination; content reads verify historical bytes,
with explicit `--raw` output for text and PDFs.
Workspace tests also check public operation registration, dependency direction,
and registered write boundaries;
coverage and limitations are recorded in the [development guide](docs/development.md).

```bash
cargo run -p knowmesh -- init ./my-knowledge --name "My Research"
cargo run -p knowmesh -- version
cargo run -p knowmesh -- --workspace ./my-knowledge sync --dry-run
cargo run -p knowmesh -- --workspace ./my-knowledge status
cargo run -p knowmesh -- --workspace ./my-knowledge rebuild --dry-run
cargo test --workspace
```

The backend builds without Node.js or Web assets. Rust is pinned by
`rust-toolchain.toml`; dependencies are pinned by `Cargo.lock`.

- [Technical specification](docs/KnowMesh_v0.1_Technical_SPEC.md)
- [Development and verification](docs/development.md)
- [Implementation tracking](https://github.com/CaiZongyuan/knowmesh/issues/1)
- [Chinese](READEME-zh.md)
