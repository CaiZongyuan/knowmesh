# Project Workflow

Development uses one agent working continuously through the GitHub issue queue. Follow [the execution loop](docs/agents/worker-start.md): finish and verify one issue, update its state, then automatically select the next unblocked issue. Work in the primary checkout on `feat/knowmesh-v0.1`; a separate worktree, fresh session, per-issue PR, or coordinator handoff is not required.

The same agent performs Standards and Spec review sequentially. Do not spawn implementation or review agents unless the user explicitly changes this mode. Older `.agent-task/START.md` packets and skill clauses requiring parallel reviewers or stopping after one ticket are superseded by this workflow. Preserve unfinished work and resume it from the issue record.

## Documentation

For documentation work or code changes affecting documented behavior, use [doc-standards](.agents/skills/doc-standards/SKILL.md).

1. Locate the existing document and verify facts against the relevant source, tests, or configuration.
2. Update that document in the same change. Keep one home per fact, link to details, and distinguish proposals from implemented behavior.
3. Synchronize affected links, examples, navigation, and existing translations. Edit the source of generated content, then regenerate.
4. Run existing project checks. If none exist, inspect links and the diff. Report actual results and anything unverified.

## As Needed

- Continuous issue-driven development: use [the single-agent entry](docs/agents/START.md) and [tracker workflow](docs/agents/issue-tracker.md).
- Existing documentation site affected: use [doc-site-sync](.agents/skills/doc-site-sync/SKILL.md).
- Cleanup after behavior is verified: use [code-simplifier](.agents/skills/code-simplifier/SKILL.md), scoped to the current task.
- Requested maintenance survey: use [find-simplifications](.agents/skills/find-simplifications/SKILL.md).
- Significant decisions: preserve the rationale and tradeoffs using [lightweight decision records](.agents/skills/doc-standards/SKILL.md#lightweight-decision-records). Small fixes need no separate record.

Follow existing directories, tools, and conventions. Add a skill only when a recurring workflow needs guidance that these skills do not cover.
