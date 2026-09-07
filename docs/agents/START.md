# Start Three Agents

The approved execution tracker is [#46](https://github.com/CaiZongyuan/knowmesh/issues/46). Start each agent with its assigned worktree as the working directory. Each worktree has a local `.agent-task/START.md` containing the exact issue, branch, base SHA and checks.

| Agent | Task | Working Directory From Repository Root |
| --- | --- | --- |
| Hard | [P10 / #56: Compiler candidates](https://github.com/CaiZongyuan/knowmesh/issues/56) | `.worktrees/hard/issue-56-candidate-extraction/` |
| Medium | [P04 / #50: Node reads](https://github.com/CaiZongyuan/knowmesh/issues/50) | `.worktrees/medium/issue-50-node-reads/` |
| Easy | [P01 / #47: Material baseline](https://github.com/CaiZongyuan/knowmesh/issues/47) | `.worktrees/easy/issue-47-material-baseline/` |

Give each fresh agent this instruction:

> Read `.agent-task/START.md` in this worktree and implement its single assigned GitHub issue. The user already approved the scope and test interface. Follow the pinned base, task-specific notes and verification requirements. Hand off a candidate PR to the coordinator, then stop; do not claim another issue or merge your own work.

The root checkout is the coordinator/integration workspace. All implementation PRs target `feat/knowmesh-v0.1`, whose backend baseline is already implemented. `main` is not the starting point for these workers.

Run complete backend checks through `bash scripts/agent-checks.sh`; the shared lock queues concurrent full checks. Each worktree has its own target and test data. Reviewer agents are scheduled by the coordinator after candidate handoff, not spawned by all three workers at once.

The [worker guide](worker-start.md) owns the common process, and the local packet links the specific notes for its task. The [publication map](../planning/parallel-v0.1/publication.json) maps all P IDs to GitHub numbers. A new task always receives a new issue worktree/session after its prerequisites have been integrated.
