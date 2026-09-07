# Triage Labels

These labels define the approved parallel workflow. GitHub native blockers and the committed integration base determine eligibility; labels support scanning and assignment.

| Role | Label | Meaning |
| --- | --- | --- |
| needs-triage | `needs-triage` | An incoming request needs assessment |
| needs-info | `needs-info` | Required information is missing |
| ready-for-agent | `ready-for-agent` | Approved, self-contained implementation instructions; check blockers before dispatch |
| ready-for-human | `ready-for-human` | A human action or judgment is needed |
| wontfix | `wontfix` | Will not be implemented |
| Hard | `difficulty:hard` | State, persistence, concurrency, model execution or comparable integration risk |
| Medium | `difficulty:medium` | Bounded functionality using established contracts |
| Easy | `difficulty:easy` | Small, stable read-only behavior, documentation or material preparation |
| Plan | `plan:v0.1` | Part of the approved parallel execution plan |
| Tracking | `tracking-only` | Scope/evidence umbrella, not a worker assignment |
| Ready | `status:ready` | No unfinished code prerequisite; check external inputs before dispatch |
| Blocked | `status:blocked` | At least one prerequisite is not yet integrated |
| Assigned | `status:assigned` | Reserved for a named worktree/agent slot, implementation not claimed started |
| In Progress | `status:in-progress` | Assigned implementation has begun |
| Review | `status:review` | Candidate handed to the coordinator for review/integration |

Each execution ticket has one difficulty label. Difficulty is independent of execution state and priority. A human gold-set review is human work even when preparation is assigned to an easy agent.

The runnable frontier is the approved ticket set whose blockers are verified and merged into the integration base, with no active owner or conflicting shared change reservation. Required human-reviewed inputs, model profiles and available Harnesses must also be present; unresolved `needs-info` or `ready-for-human` prerequisites prevent an agent-only dispatch. Preparation work may proceed only when separately scoped, and does not pass the blocked quality gate. A coordinator records the assigned agent, base SHA and PR on the ticket when it is dispatched. See the [worktree protocol](parallel-development.md).
