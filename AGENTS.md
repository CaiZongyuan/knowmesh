# Project Workflow

## Documentation

For documentation work or code changes affecting documented behavior, use [doc-standards](.agents/skills/doc-standards/SKILL.md).

1. Locate the existing document and verify facts against the relevant source, tests, or configuration.
2. Update that document in the same change. Keep one home per fact, link to details, and distinguish proposals from implemented behavior.
3. Synchronize affected links, examples, navigation, and existing translations. Edit the source of generated content, then regenerate.
4. Run existing project checks. If none exist, inspect links and the diff. Report actual results and anything unverified.

## As Needed

- Existing documentation site affected: use [doc-site-sync](.agents/skills/doc-site-sync/SKILL.md).
- Cleanup after behavior is verified: use [code-simplifier](.agents/skills/code-simplifier/SKILL.md), scoped to the current task.
- Requested maintenance survey: use [find-simplifications](.agents/skills/find-simplifications/SKILL.md).
- Significant decisions: preserve the rationale and tradeoffs using [lightweight decision records](.agents/skills/doc-standards/SKILL.md#lightweight-decision-records). Small fixes need no separate record.

Follow existing directories, tools, and conventions. Add a skill only when a recurring workflow needs guidance that these skills do not cover.
