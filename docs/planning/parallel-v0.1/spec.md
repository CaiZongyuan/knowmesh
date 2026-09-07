# KnowMesh v0.1 并行交付规格

> Status: Approved。用户已于 2026-09-07 批准任务拆分、依赖、测试接口和首批三个 worktree。
> 已验证代码基线：`5247efeff835fa487f481ab96756b646ab8b6d40`。
> GitHub 执行计划：[#46](https://github.com/CaiZongyuan/knowmesh/issues/46)，56 张执行票；编号见[发布映射](publication.json)。
> 本文拥有交付顺序与工作分配决策；[技术 SPEC](../../KnowMesh_v0.1_Technical_SPEC.md)拥有产品契约。

## Problem Statement

当前开发已经完成可重建的规范知识存储、检索、解析、模型适配、Evidence 验证以及 Proposal 审核、Apply 和幂等。剩余工作仍以较大的技术组件 issue 排队，部分基础 issue 的验收依赖后续 HTTP、Ask 或 Server。完成的组件不能及时组成真实材料闭环，单个 agent 需要持续携带大量历史上下文。

同时进行开发需要可独立验收的任务、明确的运行基线和共享接口协调。仅增加 worktree 数量无法消除业务依赖、数据库迁移冲突、生成客户端冲突或最终审查积压。

## Solution

采用按 issue 创建的独立 worktree 与新 agent 会话，按难、中、简单三档分配实现工作。协调者从依赖已合入共享基线的任务中选择最多三个并发任务，每项只承担一个可验证结果，以小批次 PR 串行集成。

完整待办见[执行票目录](index.md)。首先推进真实材料输入、知识读取与 Compiler 抽取；之后接通可靠的编译审核闭环和 Ask/save。HTTP/Web、维护和打包在其真实依赖就绪后推进。内部阶段成果不声明满足正式 v0.1 发布条件。

## User Stories

1. As a coordinator, I want each task to declare its prerequisites, so that I can dispatch only work that can finish on the current integration baseline.
2. As a coordinator, I want difficulty to describe technical risk, so that I can assign suitable agents without treating difficulty as a dependency.
3. As an implementation agent, I want one issue and one fresh worktree per assignment, so that previous tasks do not consume my working context.
4. As an implementation agent, I want the agreed contract, relevant SPEC sections and acceptance criteria in my task, so that I can implement without reconstructing the planning conversation.
5. As an integrator, I want shared interface and migration changes declared before work begins, so that independently correct branches can be combined safely.
6. As an integrator, I want every completed change reviewed against a fixed base and tested on the integration result, so that branch-local success does not hide integration regressions.
7. As a researcher, I want representative source material imported early, so that parser and retrieval weaknesses are visible before release work dominates.
8. As a researcher, I want to inspect Nodes, Claims, Relations, Evidence and source revisions through the CLI, so that search results lead to verifiable knowledge.
9. As a researcher, I want parsed source chunks to become searchable and recover after rebuilding, so that ingestion produces useful retrieval results.
10. As a researcher, I want a source compiled into a reviewable Proposal, so that model candidates can become knowledge only after explicit review.
11. As a reviewer, I want a Proposal queue and safe relaxed-mode Apply, so that routine review is efficient while strict policy remains enforced.
12. As a researcher, I want compile and Ask runs to preserve inputs, budgets and checkpoints, so that interruption or retry does not silently duplicate results or reset limits.
13. As a researcher, I want to pause, cancel and resume long operations, so that execution remains under my control across CLI and Web.
14. As a researcher, I want source refresh to preserve historical evidence and propose changes, so that a new revision does not erase accepted knowledge.
15. As a researcher, I want graph queries with explicit limits and evidence-bearing edges, so that relationships can be inspected without unbounded traversal.
16. As a researcher, I want grounded answers that expose conflicting evidence and gaps, so that a plausible answer is not mistaken for established fact.
17. As a researcher, I want to save an answer through a Synthesis Proposal with its original dependencies, so that later changes remain traceable.
18. As a CLI user, I want optional vectors and Web to be independently available, so that core use does not require model keys or frontend installation.
19. As a Web user, I want complete search, source, graph, review and Ask journeys, so that useful behavior is delivered beyond an application shell.
20. As a Web user, I want connection, API compatibility and task state to remain explicit, so that refreshing a page does not repeat a mutation.
21. As an Agent Harness user, I want version-matched embedded Skills and a thin Loader, so that a new session can use the CLI without repository knowledge.
22. As a maintainer, I want honest recovery, migration and diagnostics behavior, so that unsupported operations cannot damage canonical files or hide runtime loss.
23. As an evaluator, I want separate fixture, real-model and human-reviewed evidence, so that test success cannot be presented as proven research quality.
24. As a release maintainer, I want independent backend and Web artifacts verified together before v0.1, so that packaging progress cannot be mistaken for a complete release.

## Implementation Decisions

1. **继承已完成工作。** 以已验证的 Proposal 幂等提交为产品基线；先把批准的规划、必要工作流及 tracker 配置纳入共享分支，再派发新任务。历史组件不因改写计划而重新实现，旧 issue 的关闭状态不作为新依赖就绪的唯一证明。
2. **结果里程碑。** A0 为可复现材料与可检查知识；A1 为 compile、review、Apply、检索和可恢复 Run；A2 为 grounded Ask 与保存 Synthesis；W 为完整 Web 旅程；R 为原 SPEC 全部发布门槛。W 的局部工作可早于 A2 开始，A2 不依赖完整 Web 或完整 CLI 总清单。
3. **任务身份。** 一个实现票对应一个分支、worktree、实现会话和 PR。难、中、简单是任务属性和调度池；完成后下一项使用新任务身份。难度不等于优先级、任务大小或固定工时。
4. **派发条件。** 必须同时满足任务已批准、所有阻塞任务已验收并合入共享基线、所需接口及外部输入已存在、资源和共享变更许可可用。人工 gold、实际 Harness 和模型 profile 等前提需要独立核实。兄弟分支完成但未集成时，消费者继续阻塞。只共享修改位置而没有语义依赖的任务采用合并协调，不人为增加产品依赖。
5. **难度与容量。** 高风险事务、状态机、模型执行和恢复交给难档；边界明确的读取、协议和 UI 工作交给中档；资料整理、稳定文档和较小的只读任务交给简单档。初始上限为协调者加三个实现者，审查另行预约空闲 agent 槽位；简单任务不足时允许该档空闲。
6. **延用现有模块。** 使用现有 Application Core、Storage ports、CLI、规范文件与模型适配器。新增 Compiler 拥有候选抽取、临时引用转换和 Proposal 规划；Run 拥有执行所有权、预算、checkpoint 和最终产物发布。Compiler 与 Run 都不获得直接修改规范知识的能力。
7. **分步公开能力。** 候选抽取、实体规划、完整断言规划和请求预算可通过最高层 Core 集成测试先独立验收；公开 compile 在执行器与原子产物发布就绪后接入。每个阶段明示可用能力，未完成的恢复或预算行为不写成正式支持。
8. **持久化边界。** 保留 Proposal 幂等结果与审核历史语义。Run 必须逐请求预留/结算预算，最终 Proposal/Answer、output refs 和完成 checkpoint 原子发布；暂停、取消及旧 attempt 的迟到结果不能发布知识。Runtime 列表的分页一致性不以 canonical generation 代替自身变化版本。
9. **派生内容归属。** 已有 parse/chunk/cache helpers 继续复用；新增独立任务负责 Chunk 的检索投影、失效和 rebuild。向量 Provider/cache 可以先独立实现，向量索引集成才依赖可搜索派生产物。
10. **公共契约对齐。** HTTP 复用已实现的 `sync`、`proposal.get`、edit/revalidate 等 Operation 身份，补齐 source content 与 Relation 读取所需接口。Web 读取和 mutation 使用生成的客户端；每张新增 HTTP 功能票负责同步自身契约及客户端。
11. **设置范围澄清。** v0.1 已列出的 settings/schema/models/diagnostics 页面提供只读配置摘要、Schema、能力、脱敏模型状态和诊断。实际秘密值不返回，配置编辑或秘密管理不作为这些页面的隐含任务。新增读取 DTO 随所属 HTTP/Web 票一同验证。
12. **迁移范围澄清。** 规范格式当前为 version 1；迁移命令应明确报告支持的版本对、当前版本 no-op 与不支持版本错误。未有依据的历史格式不臆造兼容承诺；实际转换启用前验证预览、备份和文件事务。SQLite 索引迁移与规范文件迁移保持区分。
13. **共享变更协调。** CLI/Operation 注册、ports、组装入口、架构权限、依赖清单和生成客户端是合并热点。修改同一契约的任务按依赖执行；独立的追加注册可并行编写，集成时逐个复核。数据库迁移按合入顺序编号，已应用迁移的内容和 checksum 不重写。
14. **票据状态。** GitHub 是实施 tracker。发布新的规划父票及执行子票，以 native blocking links 表达依赖；用户已授权更新现有 open issue 的执行映射，原正文与完成证据保留。本轮不会把旧父票改成已完成，最终交付仍由原发布 tracker 验收。
15. **文档归属。** 技术 SPEC 拥有产品规则，执行票拥有局部验收，协作指南拥有 worktree 操作流程。批准前本地票据是发布草稿；发布后 GitHub 票据拥有实时状态和讨论，本地草稿保留为冻结的规划快照并补充链接。

## Testing Decisions

- **已批准的主验收接口：** CLI 进程调用真实 Application Core、真实 SQLite 和临时规范 workspace；检查命令输出及实际文件/索引结果。既有 Source、Search、Proposal CLI 和 SQLite workflow 测试是先例。沿用这些接口不需要再次请求用户确认。
- Compiler 规划与 Run 请求预算在公开命令就绪前，通过最高层 Core 用例加真实解析/存储集成测试验收。模型边界使用既有 fake provider 或本地 HTTP fixture；不对每个内部 helper 建立新的验收接口。
- Run、Apply、migration 和 rebuild 增量覆盖真实子进程退出、并发领取、输入变化、迟到结果与回执重放。模拟 journal/SQL 失败保留为快速回归，不能代替要求的进程退出证据。
- HTTP 票验证真实响应、鉴权、Schema 和 CLI/Core 一致性；Web 票验证浏览器用户旅程及生成客户端，无手写平行业务接口。资源、空/错/断线状态和键盘使用与对应旅程一并验收。
- 材料清单、模型输出和人工 gold 分开记录。未经人工审核的标注保持候选状态，模型自评不能替代 Compiler/Answer 的人工质量评价。
- 实现中运行相关测试，交付前执行仓库既有完整检查；复用已完成组件的结果，不为每个小步重复进行无关完整检查。共享基线变化后运行受影响回归，PR 的最终提交通过 CI。
- 规划草案检查所有本地链接、任务 ID 唯一性、依赖无环、旧 issue 覆盖和最终验收的依赖覆盖。该检查验证调度计划，不声明产品测试已经通过。

## Out of Scope

- 改用其他语言、数据库、前端框架或绕过 Proposal；原系统不变量持续适用。
- 把内部 alpha 标成正式 v0.1，或降低真实评测、证据定位、崩溃恢复、跨平台和独立分发门槛。
- 常驻通用 agent 调度平台、共享数据库开发环境，或为了并行先做大规模架构重构。
- 本轮自动执行所有实现票、修改已关闭组件的验收结论或发布生产版本。
- 把难度级别绑定为某个永久模型名称；实际执行时选择可用模型和推理强度，并记录分配结果。

## Further Notes

- 首批建议为 P10（难）、P04（中）、P01（简单）。这是有限容量下的优先调度，不表示其他无阻塞票必须等待这三项。
- 第二批由已合入的结果重新计算，例如 P11、P05、P06；Run admission P13 应及早进入中档，支持后续执行器关键路径。
- 现有分支为 `feat/knowmesh-v0.1`；`main` 尚未包含后端基础实现。PR 需要显式指定集成目标，最终发布前再完成向 main 的交付审查。
- 本方案包含一次性的工作流基线整理：新建 worktree 不会继承暂存区，所需 skills 和 tracker 配置必须已在基线中，或在任务说明中明确提供可访问的固定版本来源。
- 具体 worktree 命名、提交/审查顺序、资源限制和交接格式见[并行开发指南](../../agents/parallel-development.md)；不要把整份技术 SPEC 或上一位实现者的会话默认注入每张票。
