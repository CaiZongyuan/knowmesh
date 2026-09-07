# Issue Tracker

Tracker: GitHub, repository `CaiZongyuan/knowmesh`. Use `gh` for issue and PR operations. Repository identity is also available from `git remote -v`.

The published execution parent is [#46](https://github.com/CaiZongyuan/knowmesh/issues/46), with 56 native sub-issues and 86 blocked-by edges. [publication.json](../planning/parallel-v0.1/publication.json) maps P IDs to GitHub URLs. The original 25 open implementation issues plus delivery tracker #1 now link to their execution replacements and remain `tracking-only`.

Read the complete issue body, comments, labels and blocking relationships before implementation. The original [delivery tracker #1](https://github.com/CaiZongyuan/knowmesh/issues/1) owns final v0.1 acceptance. The [parallel execution plan](../planning/parallel-v0.1/spec.md) was approved on 2026-09-07; P IDs are stable planning identities, mapped to GitHub issues when published.

After the user approves the breakdown, publish one planning issue and one child issue per approved ticket in dependency order. Use GitHub native sub-issue and blocked-by relationships, and include readable links in each body. Maintain an explicit local-ID-to-GitHub-URL mapping during publication so retries update the created issue rather than duplicating it. Re-read relationships after writing them. If native relationships are unavailable for this repository, retain explicit `Blocked by` links and report the limitation.

The old issues remain the historical scope/evidence record. For this reorganization the user explicitly authorized adding the new execution mapping to existing open issues and the original delivery tracker. Preserve their original bodies and evidence, label them `tracking-only`, and do not close them. Existing closed components remain unchanged. Workers update only their assigned execution issue; the approved new ticket set is the dispatch list, so do not dispatch both a legacy umbrella and its replacement.

Apply labels according to [triage labels](triage-labels.md). `ready-for-agent` means a ticket is self-contained; native blockers and the integration base still determine whether it may start. A blocked ticket is not runnable merely because it has that label.

Use an exact body file for multiline `gh issue create/edit` and `gh pr create/edit` operations. Keep publication drafts outside active implementation worktrees. Never put secrets or raw private source material in issue bodies.

Implementation PRs currently target `feat/knowmesh-v0.1` explicitly. A task is integrated only after the verified change is in that target, not merely when a branch exists or an issue is closed. Explicitly close a delivered execution ticket after integration; do not rely on closing keywords for a non-default PR base. Parent closure and production release are separate actions.
