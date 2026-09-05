# KnowMesh

Local, evidence-backed knowledge workspaces built around a Rust CLI, canonical
Markdown/YAML, SQLite projections, and reviewed proposals.

**Development status:** v0.1 is being implemented. The current executable supports
`knowmesh init [path] --template research`, `knowmesh version`, and operation discovery with `knowmesh schema list` /
`knowmesh schema command <operation>`; the complete ingestion, knowledge, search, graph, proposal,
and optional Web workflows are not released yet. The npm bootstrap package does
not contain an executable.

```bash
cargo run -p knowmesh -- init ./my-knowledge --name "My Research" --dry-run
cargo run -p knowmesh -- version
cargo test --workspace
```

The backend builds without Node.js or Web assets. Rust is pinned by
`rust-toolchain.toml`; dependencies are pinned by `Cargo.lock`.

- [Technical specification](docs/KnowMesh_v0.1_Technical_SPEC.md)
- [Development and verification](docs/development.md)
- [Implementation tracking](https://github.com/CaiZongyuan/knowmesh/issues/1)
- [Chinese](READEME-zh.md)
