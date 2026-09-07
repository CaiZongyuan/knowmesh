# KnowMesh v0.1 单 Agent 持续交付规格

> Status: Approved。2026-09-07 用户改为单 agent 连续开发，替代原三 worktree 派发方式。
> 执行队列：[#46](https://github.com/CaiZongyuan/knowmesh/issues/46)；56 张 P 票及原生依赖保留，编号见[发布映射](publication.json)。
> 本文拥有执行模式；[技术 SPEC](../../KnowMesh_v0.1_Technical_SPEC.md)拥有产品契约。目录名沿用旧路径以保留已有链接。

## Problem Statement

原流程要求多个 agent 在独立 worktree 完成一票后停止，再由协调者审查、合并和派发。用户需要反复启动会话和衔接状态，操作成本高于当前项目需要。

## Solution

一个 agent 在主开发目录持续执行 GitHub 队列，自己完成选题、实现、验证、两路自审、提交、必要集成和状态更新。一票完成后自动处理下一张已满足依赖的票，不因 PR、提交或上下文压缩而结束整个开发任务。

## User Stories

1. As the developer, I want one agent to execute the approved issue queue, so that I do not repeatedly dispatch separate workers.
2. As the developer, I want issue state and commits to show actual progress, so that a resumed session can continue unfinished work.
3. As the implementation agent, I want explicit acceptance and dependencies, so that I can finish one coherent behavior before moving on.
4. As the developer, I want implementation and review in one continuous loop, so that a PR is not an automatic handoff barrier.
5. As the implementation agent, I want to record external blockers and select other work, so that one missing input does not stop all development.
6. As the researcher, I want the agreed provenance and recovery guarantees preserved, so that simpler execution does not weaken the product.
7. As the maintainer, I want existing commits and source material retained, so that switching modes does not discard completed work.
8. As the developer, I want an explicit completion boundary, so that the agent continues until the authorized queue is complete or truly externally blocked.

## Implementation Decisions

1. **保留任务图。** 现有 56 张执行票、原生依赖、已完成成果及 A0/A1/A2/W/R 验收目标继续有效。难度仅供估计，不再分配三个 agent 档位，也不重新拆一轮票。
2. **单一开发位置。** 默认在主目录的 `feat/knowmesh-v0.1` 连续开发，一次实现一张票。旧 worktree 保留历史与材料，不再作为活动队列；旧本地派发包不能要求当前 agent 一票后停止。
3. **同一 agent 闭环。** 依次实现、验证、顺序执行 Standards/Spec 自审、修复、提交和跟进 CI。不要求额外审查 agent、每票新会话或每票 PR。若已有 PR 或分支规则需要集成，由同一个 agent 完成允许的流程。
4. **据实完成。** 只有本票验收和必要检查通过、代码已在开发分支时才关闭 issue。等待必要 CI 时保持 review；失败由同一 agent 处理。不能用旧绿色提交替代当前组合结果。
5. **自动继续。** 优先恢复 in-progress/review 的工作，再从真实已解除阻塞的集合选择一票。完成并更新 GitHub 后自动继续，不询问是否继续、不等待协调者派发。
6. **上下文恢复。** 在 issue 记录分支/提交、检查结果、未完成步骤与下一动作后压缩或恢复上下文。不要把上下文管理当成任务结束。
7. **真实阻塞。** 某票缺少人工 gold、实际 Harness、凭据或其他必要外部输入时准确标记，再选择可执行任务。所有剩余工作都外部阻塞、全部授权工作完成或用户明确暂停才是停止条件。
8. **原有架构保持。** Core/CLI/HTTP、规范文件、SQLite 派生索引、Proposal、Run 预算和证据规则以技术 SPEC 为准。格式迁移仍需明确支持版本与备份；设置页面仍为既定只读范围。

## Testing Decisions

- 已批准接口继续有效：CLI 经过真实 Core/SQLite/临时 workspace；候选规划和预算阶段用最高 Core 集成接口及 fake provider；HTTP/Web 按所属用户流程验收。
- 行为代码在该接口做 TDD；资料/文档验证实际产物。相关测试随实现执行，本票完成前运行其要求的完整检查。
- Standards 与 Spec 由同一 agent 顺序自审，保留各自结论。当前用户要求优先于旧 skill 中必须创建审查子 agent 的流程。
- 原有多文件恢复、幂等、不可变 Evidence、生成契约、真实评测及分发门槛不降低。人工科研判断不得用 agent 自评代替。

## Execution Order

先收尾已有 PR/修复和必要 CI。随后优先推进已解除依赖的 Compiler/Run 闭环：P11、P12、P13、P14、P15，并补齐 P05、P02/P03、P16/P17 所需的证据读取、审核及恢复操作。再推进 Graph、Ask/Synthesis 与其余界面、维护和发布任务。

这是一条优先建议，不覆盖原生阻塞关系。实际输入缺失时选择其他已就绪票；不按难度分配 agent，也不为了顺序等待一个可绕开的外部阻塞。

## Out of Scope

- 更换技术栈、绕过 Proposal 或降低正式 v0.1 验收。
- 自动删除旧 worktree、原始材料或未合入提交。
- 绕过受保护分支、代签人工评测或在缺少必要授权时发布生产版本。

## Further Notes

具体持续执行循环以[单 agent 工作流程](../../agents/worker-start.md)为准，启动入口为[START](../../agents/START.md)。旧并行规则已退役。GitHub 正文中的当前执行规则覆盖历史派发评论和旧固定版本链接中的停止指令。
