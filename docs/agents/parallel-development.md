# Retired Parallel Workflow

The user replaced worktree dispatch with [single-agent continuous development](worker-start.md) on 2026-09-07. Read the [current entry](START.md).

The former requirements for one worktree/session per issue, hard/medium/easy agent lanes, separate reviewer agents and stopping after a candidate PR are inactive. One agent now implements, reviews, integrates, updates issue state and continues through the unblocked queue.

This path preserves existing links. Existing branches, PRs, worktrees and private material are retained; changing execution mode does not revert accepted code or authorize destructive cleanup. The old rationale is available in Git history.
