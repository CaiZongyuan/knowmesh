# KnowMesh

Local, evidence-backed knowledge workspaces built around a Rust CLI, canonical
Markdown/YAML, SQLite projections, and reviewed proposals.

**Development status:** v0.1 is being implemented. The current executable supports
`knowmesh init [path] --template research`, `knowmesh version`, and operation discovery with `knowmesh schema list` /
`knowmesh schema command <operation>` / `knowmesh schema pack <id>`, plus local
`source add/list/get/content`, confirmed `source remove`, paginated `source impact`, `sync`, `status`,
`doctor`, `rebuild`, and `search`;
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
Core error mappings and JSON envelopes have contract snapshots; HTTP serving is
still pending.
Search supports English/Chinese retrieval, filters, configurable RRF, score
explanations, stable cursor pagination, and evidence freshness. Optional vectors
and Graph paths are still pending.
Core also parses Markdown/TXT/HTML into structured source blocks. Explicit
`source add --encoding <label>` supports legacy text while preserving original
snapshot bytes. Native PDF text extraction includes page mapping and quality gates;
OCR and the full compilation pipeline are still pending.
Core includes structure-aware chunking and validated stage caches; these are ready
for the remaining Compiler integration.
Model adapters now provide bounded structured generation with local Schema
validation; the full compile/review/apply workflow remains in development.
Core Evidence verification checks quotes against bounded source spans and rejects
ambiguous repairs; the Proposal Builder rechecks source bytes before review.
Entity resolution includes conservative identifier/name/alias matching and SQLite
candidate retrieval plus bounded model advice; Compiler/Proposal integration is pending.
Canonical conflict groups survive projection and rebuild. Exact assertion/Evidence
deduplication preserves scientific symbols. Bounded semantic comparison produces
reviewable conflict plans; Compiler/Proposal/Run integration remains in development.
Proposal state/review contracts, typed payload/Evidence validation, and read-only
canonical previews are in place. Accepted subsets are revalidated against current
files and workspace policy. Controlled summary editing preserves surrounding
Markdown. SQLite preserves complete review revisions; public Proposal workflows
and coordinated Apply remain in development.
Workspace tests also check public operation registration, dependency direction,
and registered write boundaries;
coverage and limitations are recorded in the [development guide](docs/development.md).

```bash
cargo run -p knowmesh -- init ./my-knowledge --name "My Research"
cargo run -p knowmesh -- version
cargo run -p knowmesh -- --workspace ./my-knowledge sync --dry-run
cargo run -p knowmesh -- --workspace ./my-knowledge status
cargo run -p knowmesh -- --workspace ./my-knowledge search "virtual cell" --explain
cargo run -p knowmesh -- --workspace ./my-knowledge rebuild --dry-run
cargo test --workspace
```

The backend builds without Node.js or Web assets. Rust is pinned by
`rust-toolchain.toml`; dependencies are pinned by `Cargo.lock`.

- [Technical specification](docs/KnowMesh_v0.1_Technical_SPEC.md)
- [Development and verification](docs/development.md)
- [Implementation tracking](https://github.com/CaiZongyuan/knowmesh/issues/1)
- [Chinese](READEME-zh.md)
