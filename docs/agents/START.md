# Start Continuous Development

Use one agent in the primary repository directory on `feat/knowmesh-v0.1`. The execution queue is [#46](https://github.com/CaiZongyuan/knowmesh/issues/46); final product/release acceptance belongs to [#1](https://github.com/CaiZongyuan/knowmesh/issues/1).

Instruction for the agent:

> Read AGENTS.md and docs/agents/worker-start.md. Execute the approved unfinished issues under #46 in dependency order, one at a time. Resume existing work first; implement, verify, review, commit and update GitHub state yourself. After completing an issue, automatically continue to the next unblocked issue. Do not stop at a commit or PR or wait for another agent/coordinator. Checkpoint progress before context compaction and continue after recovery. Stop only when the authorized work is complete, the user pauses, or all remaining work requires unavailable external input.

The [execution loop](worker-start.md) owns detailed rules. [Tracker instructions](issue-tracker.md) explain state and GitHub compatibility. Existing task acceptance and native blockers remain valid; the [publication map](../planning/parallel-v0.1/publication.json) preserves issue identities.

Old worktrees are retained for history, diagnostics and privately acquired material. They are not active development lanes. Their assignment packets do not limit the current agent to one issue or require it to stop after a PR.
