# Issue Tracker

Use GitHub repository `CaiZongyuan/knowmesh` through `gh`. [#46](https://github.com/CaiZongyuan/knowmesh/issues/46) owns the single-agent execution queue; [#1](https://github.com/CaiZongyuan/knowmesh/issues/1) owns final v0.1 acceptance. [publication.json](../planning/parallel-v0.1/publication.json) maps the 56 stable P IDs to GitHub issues. Native sub-issue and blocked-by relationships remain the dependency source.

## Continuous Execution

Read the current issue body/comments and blockers before working. Resume status:in-progress or status:review work first. Otherwise choose an open, approved execution issue whose blockers are complete and whose required code is in `feat/knowmesh-v0.1`. Ignore tracking-only umbrellas and retired worktree assignments.

One agent implements and performs sequential Standards/Spec review, commits, verifies required checks and updates the issue. Once acceptance passes, close it, update the relevant tracker checklist and immediately select the next issue. Per-issue PRs, new sessions and a separate coordinator are not prerequisites. Existing PRs may be completed by the same agent. A non-default branch may require explicit issue closure after integration.

Use [status labels](triage-labels.md) for progress. A closed dependency is insufficient if its code is missing; a ready label is insufficient if required human/model input is missing. Record external blockers precisely, then continue other available work. Keep the original 25 implementation umbrellas and their historical evidence; their execution children are the actual work queue.

## Checkpoint

Record branch/commit, completed acceptance, actual checks, remaining steps and next issue/action before compaction or interruption. Do not report an unrun check as passing, close a blocked issue for convenience, or require the user to redispatch after a normal issue completion. The [execution loop](worker-start.md) defines the stop conditions.

## CLI Compatibility

Read with explicit fields, for example `gh issue view <number> --json title,body,comments,labels,state,url`. This host's default views and `gh pr edit` may fail on deprecated projectCards. PR body updates can use `gh api repos/CaiZongyuan/knowmesh/pulls/<number> --method PATCH --input <request.json>` with structured JSON.

Use body files or structured JSON for multiline updates. Preserve existing discussions and unrelated fields. Keep secrets and restricted source text out of GitHub records. Production release and protected-branch requirements remain in force; switching to continuous execution does not bypass them.
