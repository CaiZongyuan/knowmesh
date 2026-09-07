# Parallel Development

Status: approved on 2026-09-07 with the [execution plan](../planning/parallel-v0.1/spec.md). Use this document when starting, integrating or handing off an issue worktree. Workers start with [Assigned Worker](worker-start.md) and their local dispatch packet. Product contracts remain in the technical SPEC; [development checks](../development.md#checks) remain the verification command source.

## Shared Baseline

The verified product baseline is `5247efeff835fa487f481ab96756b646ab8b6d40` on `feat/knowmesh-v0.1`. `main` does not yet contain the current backend. Record the actual integration SHA at dispatch rather than assuming that a moving branch name identifies the reviewed code.

Before starting the first implementation worktree:

1. Use the approved planning/spec breakdown and its test interfaces; approval is already recorded for this plan.
2. Review a path-limited bootstrap change containing the approved planning documents, this workflow configuration, the worktree/local-packet ignore rules and the skills used by the implementation/review flow, including referenced resources. Keep unrelated staged imports out of that commit; a path-limited commit must preserve the rest of the index.
3. Verify those files and links from the committed tree, run the existing baseline checks if executable behavior changed, and record the shared starting SHA. All workers must see the same workflow version. A worktree at HEAD does not inherit the root index or working copy.
4. Publish the approved spec/tickets and native dependencies using the [tracker workflow](issue-tracker.md). Resolve source links to the committed planning documents, record the published issue mapping, then dispatch the initial frontier.

The coordinator keeps the root checkout as the integration checkout. Workers commit only their issue changes and use a PR with explicit base `feat/knowmesh-v0.1`. The integration target can change after deliberate consolidation into main; record that change once in this protocol and in newly dispatched tickets.

## Worktree Identity

Difficulty is a directory grouping and assignment category. Every task still gets a separate branch/worktree:

```text
.worktrees/
  hard/issue-<number>-<slug>/
  medium/issue-<number>-<slug>/
  easy/issue-<number>-<slug>/
```

Create a worktree only for a dispatched ticket, from the recorded integration SHA. For example, after replacing the placeholders with the published ticket and verified base:

```bash
git worktree add -b feat/issue-<number>-<slug> .worktrees/hard/issue-<number>-<slug> <base-sha>
```

Worktree isolation provides different files and branches. Start a new agent session with that worktree as its working directory to obtain context isolation. Reusing a conversation in a new directory does not discard its context.

Use a new task identity for the next issue. Remove a finished worktree only after its work is integrated, the working copy is clean, and no process still uses it. Preserve unmerged branches, untracked artifacts and diagnostics until deliberately handled; do not use forced cleanup to make the queue appear empty.

## Dispatch Packet

Provide each worker only:

- The approved GitHub issue body/comments, its local draft mapping and explicit acceptance criteria.
- The worktree, branch, integration target and pinned base SHA.
- The owning SPEC sections and relevant current implementation/test families, found from the issue's references.
- The approved test interface: normally CLI through Core and real SQLite/temp workspace; documented Core-only stages use the named integration use case.
- The assigned capability and any reserved shared interfaces/migration or dependency changes.
- Required human-reviewed inputs, actual Harness availability and authorized model configuration; a missing prerequisite keeps the corresponding evaluation blocked.
- Required check commands through the development guide, and the instruction to report blockers before widening scope.

A worker may read more of the SPEC when a referenced rule requires it. It does not inherit the coordinator's entire planning history or another worker's previous implementation session.

## Difficulty And Capacity

| Assignment | Use For | Escalate When |
| --- | --- | --- |
| Hard | Canonical/runtime atomicity, Compiler/Run state, cancellation/budgets, security boundaries, recovery, complex graph layout | A required contract or state transition is undefined |
| Medium | Read operations, defined HTTP/client/UI journeys, fixed-contract adapters and packaging | Work changes write authorization, transaction ownership or published interfaces |
| Easy | Material inventories, stable Skills/examples and small established read surfaces | Scientific gold judgment, migration or concurrency behavior becomes necessary |

Choose an available model and reasoning level appropriate to the assignment, and record the actual choice. These classes do not assert that a particular model is installed or assign work based only on cost. Do not turn a hard task into an easy assignment by reducing its required checks.

Start with one coordinator and at most three implementation agents. There are four available agent slots in this environment. A two-axis review needs two independent reviewers, so pause dispatch and free slots before review; three workers plus their reviewers cannot all run concurrently. Empty easy capacity is acceptable when only harder tasks are ready.

The current machine has approximately 15 GiB RAM. Use independent target directories in each worktree and initially set `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Run `bash scripts/agent-checks.sh` for complete backend checks: its filesystem lock serializes the full passes across independently launched agents, even while the coordinator is idle. Targeted tests may run concurrently within memory limits. Sharing a single target directory introduces Cargo lock contention and is not the default. Test workspaces, databases, caches and network ports must also be independent; existing fixtures use temporary directories and ephemeral loopback ports.

## Shared Changes

| Shared Area | Coordination Rule |
| --- | --- |
| Core ports, Operation registry, CLI command mapping, runtime assembly | Declare additions before dispatch. Independent additions may be developed concurrently; a consumer of a new contract waits for it to merge. Review the combined registrations at integration. |
| SQLite migrations and registry | One migration-bearing PR lands at a time. Assign the next contiguous number after rebasing; never modify an applied migration to resolve overlap. |
| Runtime table inventories and copy/delete order | A new persisted runtime object includes rebuild preservation/discard and rollback coverage in its owning ticket. |
| Architecture policy | Review new write authority and its transaction tests. Register only the intended capability; passing the static guard alone is not authorization. |
| Cargo manifests/lockfile | Declare dependency changes. Resolve manifests first and regenerate the lockfile with Cargo on the combined tree; inspect unexpected changes and rerun locked checks. |
| pnpm root workspace/lockfile | The initial Web ticket establishes the root setup. Later dependencies use the same pinned toolchain; serialized regeneration handles overlaps. |
| OpenAPI/generated client | Each HTTP change updates its contract and generated consumer; regenerate on the final combined base, never hand-edit the generated code to resolve conflicts. |
| SPEC/development docs | Edit the owning section only. A task's full local progress history belongs in its issue, not in every shared document. |

Shared-file overlap is not itself a business dependency. The coordinator distinguishes additive registration edits from incompatible changes to the same contract. If the latter is discovered, name the prerequisite and update the dispatch plan before another worker relies on an unmerged interface.

## Integrate And Review

1. Implement one behavioral slice using the ticket's test interface. Run focused checks while changing it, then the existing complete checks for the delivered scope.
2. Create a reviewable candidate commit on the issue branch and hand it to the coordinator with a PR. The coordinator schedules `/code-review` with the pinned integration base and originating issue once reviewer slots are available. The current review skill compares committed `<base>...HEAD`; running it against only uncommitted edits would omit the candidate. Candidate commits are not a merge or approval.
3. Address assigned review findings, keep the review diff complete, and update the PR with an explicit integration base. Report the completed behavior, actual verification and any remaining limitations. Workers stop after their handoff and do not independently begin a second issue.
4. Integrate PRs serially. Refresh the candidate against the latest target, resolve conflicts by the intended contracts, regenerate generated artifacts as needed and run affected checks. The final submitted commit must pass CI; earlier branch checks do not validate a later merge result.
5. Once the verified result is in the target, record the merge SHA and close only that execution ticket. Recalculate the runnable frontier. An approved or green sibling branch is not a substitute for a merged prerequisite.

Final v0.1 acceptance remains the technical SPEC release gate and original delivery tracker. Small PRs may land in the integration branch before final product acceptance, but production release and parent closure remain distinct operations.

## Handoff

When a task ends or needs a new session, record its branch/HEAD, base SHA, delivered behavior, test results, unresolved blockers and artifact locations. A resumed worker reads that record plus the original issue. An unfinished task is not marked done to release an agent slot.
