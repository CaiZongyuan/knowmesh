# 开发配置检查

本文说明仓库中的配置自检脚本及 2026-09-05 的实际检查结果。当前尚无 KnowMesh 应用工程；接口连通不代表产品功能或发布验收已经完成。

## 模型接口

从仓库根目录使用 Node.js 24 执行：

```bash
node scripts/check-model-config.mjs
```

脚本使用 Node.js 的 dotenv 解析器读取本地 `.env`，不会执行文件内容。变量名及示例见 [环境变量示例](../.env.example)。这些变量目前用于自检脚本，尚不是已实现的 KnowMesh CLI 配置。

- `LLM_BASE_URL` 是 API 根路径；脚本补充 `/chat/completions`。
- `EMBEDDING_BASE_URL` 是完整嵌入端点路径。
- `LLM_KEY`、`EMBEDDING_KEY` 只发送给各自配置的 HTTPS 服务，不写入输出。

每项检查输出一行 JSON。基本 JSON、提示词内 Schema 和批量嵌入任一失败时，进程退出码为 1。原生 `json_schema` 是额外能力探测，失败时输出 `ok: false, optional: true`，不使其他可用调用方式失败。

| 检查 | 实测结果 |
| --- | --- |
| `glm-5.3-flash` JSON 模式 | 通过；返回 JSON 对象并通过字段、值及额外字段校验 |
| 仅通过 `response_format.json_schema` 提供约束 | 未通过；返回 Markdown/空对象，未遵循指定 Schema |
| 提示词内提供 Schema，使用 JSON 模式 | 通过；返回内容通过客户端校验 |
| `BAAI/bge-m3` 批量嵌入 | 通过；三条输入均返回有限、非零的 1024 维向量 |
| 中英混合相似度检查 | 通过；同义中英文本余弦相似度约 0.803，无关文本约 0.379 |

原嵌入地址 `/v1/embedding` 返回 HTTP 404，已将本地配置及示例修正为 `/v1/embeddings`。向量维度与规格中的 1024 维示例一致。

后续模型适配器应使用经验证的 JSON 调用方式，独立执行 Schema、证据和引用校验。一次简单结构测试不能证明复杂 Compiler 输出的可靠性；仍需结构化输出 fixtures 和真实材料评测。

## npm 与 GitHub

`NPM_TOKEN` 保存在 GitHub 仓库 Actions Secret 中，不需要放入本地 `.env`。自检只使用 npm 的读取接口，不发布包。

重新执行：

```bash
gh workflow run verify-configuration.yml --repo CaiZongyuan/knowmesh --ref main
gh run list --repo CaiZongyuan/knowmesh --workflow verify-configuration.yml --limit 1
```

[Verify Configuration workflow](../.github/workflows/verify-configuration.yml) 使用 Secret 验证 npm CLI 身份，并运行 [npm 自检脚本](../scripts/check-npm-config.mjs)。脚本只输出身份、HTTP 状态及权限统计，不输出 token。身份认证失败会使检查失败；包列表查询是可选诊断，拒绝访问时仍明确输出 `ok: false, optional: true`，不据此推断能否发布。

本次 npm 实测结果：

- `npm whoami` 和 npm 身份接口均通过，账号为 `airickc1999`。
- npm CLI 的组织包列表接口及直接调用的用户包列表接口均返回 HTTP 403。
- 初版 [只读检查](https://github.com/CaiZongyuan/knowmesh/actions/runs/33973657802) 因列表查询的 403 而失败；实际发布成功后再次查询仍为 403，因此已将列表查询调整为可选诊断。
- 用户确认包名为 `knowmesh` 后，[首次发布](https://github.com/CaiZongyuan/knowmesh/actions/runs/33974083891) 成功，实际验证了现有 token 的包创建和发布权限，无需复制到本地 `.env`。
- `knowmesh@0.0.0-bootstrap.0` 是初始化版本，仅包含项目说明、包元数据和 MIT 许可证，没有可运行的 CLI。正式 v0.1 尚未发布。
- registry 中的包可下载，大小为 1,463 bytes，SHA-512 integrity 与本地打包结果一致；GitHub Actions 发布携带 provenance。

初始化发布显式使用 `bootstrap` 标签，但 npm 首次发布还自动添加了 `latest`。[标签清理运行](https://github.com/CaiZongyuan/knowmesh/actions/runs/33974281414) 调用 `npm dist-tag rm knowmesh latest` 时返回 HTTP 403，因此目前两个标签都指向初始化版本。这项管理权限尚未通过验证；包创建和发布本身已成功。

[初始化 workflow](../.github/workflows/npm-bootstrap.yml) 提供 `cleanup_only=true` 输入，可在标签管理权限可用后重新执行清理；它仅删除指向 `0.0.0-bootstrap.0` 的 `latest`，不影响未来正式版本。正式版本以后由后端 release 流程交付；不应重复发布这个不可变的初始化版本。

发布状态可通过以下只读命令核实：

```bash
npm view knowmesh@0.0.0-bootstrap.0 name version dist-tags dist.integrity --json
```

GitHub 仓库已使用 [MIT License](../LICENSE)，并已启用 GitHub Pages 的 Actions 部署模式及 HTTPS。预定文档地址为 `https://caizongyuan.github.io/knowmesh/`。VitePress 工程和部署 workflow 尚未建立，因此本轮没有站点构建或上线验证。

## 论文材料检查

独立 subagent 只读检查了本机相邻仓库的 `../virtual-cell-2026/docs/references`：

- 17 个 Markdown 文件、44 个 JPG，共约 4.10 MiB；Markdown 均可读，未发现 UTF-8 替换字符。
- 没有 PDF 文件，无法用此目录验证 PDF 文本层、扫描件或加密文件处理。
- STATE、scGPT 有中文阅读材料；Geneformer、Tahoe-100M、Perturb-seq 尚缺独立一手来源。Virtual Cell Challenge 有索引摘要，同仓库 `docs/Official-website/` 有补充页面材料。
- Lingshu-Cell 阅读材料有 6 处本地图片引用缺失；227 处外链图片尚未验证可达性。
- 素材量足以准备至少 20 个抽取片段和 10 个研究问题，但尚未生成或审核 gold 标注集。独立 subagent 的材料检查不能记为已完成规格要求的人工标注验收。

后续可用现有 Markdown 启动导入与检索测试，再补充权威原文、PDF fixtures 和证据标注。来源目录没有被修改，阅读版内容也没有作为本次接口自检的输入发送给模型服务。
