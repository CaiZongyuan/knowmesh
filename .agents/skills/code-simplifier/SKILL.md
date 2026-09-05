---
name: code-simplifier
description: Simplify recently changed code after its intended behavior is verified, preserving observable behavior and public interfaces. Use for focused cleanup before review, not architecture changes or broad maintenance surveys.
---

# Simplify the Current Diff

Keep the pass within the current task's changes unless the user names a wider scope. Read applicable `AGENTS.md` instructions, interfaces, and relevant tests before editing.

## Workflow

1. Confirm the affected behavior passes its relevant checks. Resolve existing failures before simplifying code whose behavior they cover.
2. Remove redundant branches, duplication, unnecessary indirection, and dead code introduced by the change. Prefer clear control flow and existing local patterns; fewer lines alone is not a reason to edit.
3. Preserve observable behavior, public interfaces, data formats, errors, ordering, side effects, and security checks. Keep non-obvious rationale and useful module responsibilities. Do not weaken tests to make a cleanup pass.
4. Update documentation or comments when internal names, paths, or explanations change, following [doc-standards](../doc-standards/SKILL.md). Inspect the diff for scope growth and run the checks affected by the edits. Report material changes and actual results.

Record opportunities outside this scope for later assessment with [find-simplifications](../find-simplifications/SKILL.md), then continue the scoped cleanup. Removing a public capability or changing behavior needs an implementation task with its own verification.

This workflow is independently adapted from Anthropic's Apache-2.0 [code-simplifier agent](https://github.com/anthropics/claude-plugins-official/blob/main/plugins/code-simplifier/agents/code-simplifier.md).
