# Single-Agent Execution Loop

The user selected single-agent continuous development on 2026-09-07. This replaces three-worktree dispatch. Use this loop when executing the implementation plan; completing one issue or creating a PR is not a stopping point.

## Start Or Resume

Work in the primary repository checkout on `feat/knowmesh-v0.1`. Read AGENTS.md, [tracker #46](https://github.com/CaiZongyuan/knowmesh/issues/46), and the current issue's body/comments. [publication.json](../planning/parallel-v0.1/publication.json) maps P IDs to GitHub issues. Existing worktrees and private files are recovery material; old one-ticket stop instructions are inactive.

Inspect Git status and commits first. Resume unfinished implementation or outstanding review/CI corrections before selecting new work. Preserve unrelated changes. The accepted CLI/Core/real-SQLite test interfaces remain approved and need no repeated confirmation.

## Repeat Until Complete

1. **Select.** Read live issue state and native blockers. Choose one approved open execution issue whose prerequisites are verified in the development branch. Prefer the earliest useful product loop and work that unblocks it. Difficulty is a risk estimate, not an assignment lane. Do not implement a tracking-only umbrella as a duplicate task.
2. **Record.** Set status:in-progress and record the starting commit, acceptance criteria and real prerequisites. Read owning SPEC sections and existing code/tests rather than the entire previous conversation.
3. **Implement.** Stay within the current issue, reuse established contracts and use TDD for behavioral code at the approved interface. Resolve routine implementation choices directly. Update owning documentation and record significant design decisions. A blocked issue does not prevent other unblocked work.
4. **Verify And Review.** Run focused tests during implementation and complete checks required by the delivered scope. `bash scripts/agent-checks.sh` runs backend checks. Perform Standards and Spec review yourself as two sequential passes, fix findings and keep the results distinguishable. No separate reviewer agent or coordinator is required.
5. **Commit And Integrate.** Make an issue-referenced commit, push the development branch and verify required CI. Per-issue PRs are optional. If a PR or branch rule is required, the same agent handles allowed review and integration without handing control back merely because a PR exists. Integrate existing PRs before using their changes as prerequisites; preserve branch protections.
6. **Close And Continue.** Once acceptance and required checks pass, record commit/PR, actual tests, limitations and newly unblocked work; close the issue and update tracking checklists. Immediately return to step 1. Do not ask whether to continue or require a new worktree/session.

CI waiting is not a human handoff. Poll it, diagnose failures and do useful preparation while waiting. An issue awaiting required CI stays status:review and is not a completed prerequisite. Keep one active implementation at a time.

A ticket's "stop at" or "Scope and handoff" wording bounds that ticket's feature scope. It does not end the continuous queue: after acceptance, update the issue and select the next one.

## State And Recovery

Use status:ready, status:in-progress, status:review and status:blocked for the phase; GitHub closed state represents completed acceptance. Remove obsolete status:assigned labels. Use needs-info or ready-for-human for a precise missing external input, then look for another executable issue.

Before compaction or interruption, checkpoint the current issue: branch/HEAD, work done, checks, unfinished steps and next command. Resume from that record after recovery. Context management preserves continuity and does not require redispatch.

## Completion Boundary

Continue the authorized queue until all applicable work and checks are complete, the user explicitly pauses, or every remaining issue depends on external input the agent cannot obtain or perform. A single blocked issue, passing tests, a commit or a PR does not justify stopping while other work is executable.

Human scientific gold/quality judgments, unavailable credentials and explicit production-release authorization remain real prerequisites. Do not manufacture those results or weaken provenance, Proposal, recovery or release gates. If only external blockers remain, report their exact issues and required input. The technical SPEC and tracker #1 still own final product acceptance.
