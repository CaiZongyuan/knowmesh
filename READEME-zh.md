# KnowMesh

基于 Rust CLI、规范 Markdown/YAML、SQLite 派生索引与 Proposal 审核的本地知识工作空间。

**开发状态：** v0.1 正在实现。目前支持 `knowmesh init [path] --template research`、`knowmesh version`、`knowmesh schema list`、`knowmesh schema command <operation>`、`knowmesh schema pack <id>`，以及本地 `source add`、需确认的 `source remove`、`sync`、`status`、`doctor` 和 `rebuild`；完整导入、知识、搜索、图谱、Proposal 和可选 Web 工作流尚未发布。npm 初始化包不包含可执行程序。

Core 库已提供规范文件解析、可恢复文件事务和原子的 SQLite 投影同步。Doctor 支持显式事务修复；Rebuild 保留运行状态，先备份旧数据库再原子替换。URL 抓取仍待接入。

```bash
cargo run -p knowmesh -- init ./my-knowledge --name "My Research"
cargo run -p knowmesh -- version
cargo run -p knowmesh -- --workspace ./my-knowledge sync --dry-run
cargo run -p knowmesh -- --workspace ./my-knowledge status
cargo run -p knowmesh -- --workspace ./my-knowledge rebuild --dry-run
cargo test --workspace
```

后端构建不依赖 Node.js 或 Web 资源。Rust 版本由 `rust-toolchain.toml` 固定，依赖由 `Cargo.lock` 固定。

- [技术规格](docs/KnowMesh_v0.1_Technical_SPEC.md)
- [开发与验证](docs/development.md)
- [实施跟踪](https://github.com/CaiZongyuan/knowmesh/issues/1)
- [English](READEME.md)
