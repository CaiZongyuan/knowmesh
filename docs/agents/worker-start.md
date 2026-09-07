# Assigned Worker

Read your worktree's `.agent-task/START.md` first. The user approved the parallel plan, its test interfaces and the initial P10/P04/P01 assignments on 2026-09-07. Implementing those assigned behaviors is authorized; a skill's request to confirm the same test interface does not require another question.

## Start

1. Verify the recorded branch and base SHA, then read the linked GitHub issue body/comments. The current GitHub issue owns acceptance; the local ticket is its publication snapshot. A blocker must be merged into the recorded integration branch before it is considered satisfied.
2. Read the task-specific note and only the referenced SPEC sections. Read the owning code and test families before changing them. The root `AGENTS.md` and documentation workflow apply inside the worktree.
3. Work in this worktree with an independent target directory, temporary workspace/database and model fixtures. Use `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Do not copy the integration checkout's `.env` or use its knowledge workspace as a fixture.
4. Change the assigned issue from `status:assigned` to `status:in-progress` when actual implementation begins. Record the agent identity/model if known; do not infer a model name. No other issue is claimed automatically.

## Implement

Use the committed `implement` and `tdd` skills for code work at the already approved interface. A documentation/material-only change needs the checks appropriate to its output, not artificial RED tests. The exact new test target and private helper design are implementation choices; adding a different public capability or changing a shared contract requires reporting the scope change to the coordinator.

Make only the declared task's changes. Necessary additive module registration is allowed. The initial reservations are:

| Task | Owned Capability | Shared Changes |
| --- | --- | --- |
| P10 | Compiler candidates, prompt, cache identity and extraction tests | One additive Compiler module export; current parser/model contracts remain usable |
| P04 | Node reads, Schema entity discovery, SQLite/CLI read wiring and tests | Additive read port, operations/CLI/runtime registration and required architecture policy |
| P01 | Material inventory, reproducible import/search baseline and unreviewed annotation candidates | One evaluation data/documentation home; no production Rust changes |

P10 and P04 may each add independent exports to Core's module listing. Keep those edits narrow; the coordinator resolves the combined ordering at integration. Neither initial task needs to renumber migrations or redesign a shared registry. Before changing existing port semantics, model request behavior, dependency versions or a migration, report the concrete need and wait for a coordinated integration order. Continue independent assigned work while that dependency is resolved.

## Verify And Handoff

Run relevant tests while implementing. Before handing off a code change, run `bash scripts/agent-checks.sh`; this holds one shared filesystem lock across the full backend checks, so independent agents can queue without a live coordinator. The script is for this Linux development host; the existing CI still verifies all three operating systems. Preserve actual failures and report missing prerequisites.

Commit only your task changes, with its GitHub issue reference, and open a PR with explicit base `feat/knowmesh-v0.1`. Supply the pinned base SHA, completed acceptance criteria, test results and any limitations. Set `status:review` when the candidate is ready.

Stop after that handoff. The coordinator schedules the two independent review axes against the committed candidate, resolves findings, validates the latest combined base and merges serially. Workers do not merge their own PRs, close tracking parents, publish a release or start another ticket in the same context. The next task receives a new worktree/session.

An unexpected external input, unavailable human gold or changed contract is a recorded blocker, not permission to mark the task complete. An unavailable Harness cannot be replaced by a fabricated smoke result.
