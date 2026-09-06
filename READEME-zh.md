# KnowMesh

基于 Rust CLI、规范 Markdown/YAML、SQLite 派生索引与 Proposal 审核的本地知识工作空间。

**开发状态：** v0.1 正在实现。目前支持 `knowmesh init [path] --template research`、`knowmesh version`、`knowmesh schema list`、`knowmesh schema command <operation>`、`knowmesh schema pack <id>`，以及 `source add/list/get/content`、需确认的 `source remove`、分页 `source impact`、`sync`、`status`、`doctor`、`rebuild` 和 `search`；完整导入、知识、搜索、图谱、Proposal 和可选 Web 工作流尚未发布。npm 初始化包不包含可执行程序。

Core 库已提供规范文件解析、可恢复文件事务和原子的 SQLite 投影同步。Doctor 支持显式事务修复，包括配置尚未落盘的初始化中断；Rebuild 保留运行状态，先备份旧数据库再原子替换。URL 导入会保存不可变快照，并检查地址、重定向、大小和超时；私网目标需本地 CLI 显式传入 `--allow-private-network`。

来源移除预览会返回受影响的知识，且不改写磁盘索引。
来源列表支持游标分页，内容读取会校验历史字节，文本和 PDF 均可显式使用 `--raw` 输出。
Core 错误映射和 JSON envelope 已有契约快照，HTTP 服务尚待实现。
Search 已支持中英召回、筛选、可配置 RRF、分数解释、稳定游标分页和证据 freshness；可选向量与 Graph paths 仍待接入。
Core 已支持 Markdown/TXT/HTML 的结构化来源解析；`source add --encoding <label>` 可显式导入旧编码文本，保留原始快照字节。原生 PDF 文本提取已包含页码映射和质量门；OCR 与完整编译流水线仍待实现。
Core 已提供结构感知分块和带校验的阶段缓存，继续接入后续 Compiler 工作流。
模型适配器已提供有界结构化生成与本地 Schema 校验；完整 compile/review/apply 流程仍在实现中。
Core Evidence verifier 已支持原文区间验证与有界定位修复，拒绝歧义引用；Proposal review/apply 的强制接入仍待完成。
实体消歧已支持保守的标识符、名称、别名匹配、SQLite 候选召回与受限模型建议；Compiler/Proposal 集成仍待接入。
Workspace 测试也检查公共 Operation 注册、依赖方向及已登记的写入边界，覆盖范围与限制见[开发文档](docs/development.md)。

```bash
cargo run -p knowmesh -- init ./my-knowledge --name "My Research"
cargo run -p knowmesh -- version
cargo run -p knowmesh -- --workspace ./my-knowledge sync --dry-run
cargo run -p knowmesh -- --workspace ./my-knowledge status
cargo run -p knowmesh -- --workspace ./my-knowledge search "virtual cell" --explain
cargo run -p knowmesh -- --workspace ./my-knowledge rebuild --dry-run
cargo test --workspace
```

后端构建不依赖 Node.js 或 Web 资源。Rust 版本由 `rust-toolchain.toml` 固定，依赖由 `Cargo.lock` 固定。

- [技术规格](docs/KnowMesh_v0.1_Technical_SPEC.md)
- [开发与验证](docs/development.md)
- [实施跟踪](https://github.com/CaiZongyuan/knowmesh/issues/1)
- [English](READEME.md)
