---
name: doc-standards
description: Create, update, organize, or review project documentation, including documentation affected by code changes. Keep facts accurate, give each subject one home, and verify related links and outputs.
---

# Maintain Project Documentation

Follow the project's existing structure and applicable `AGENTS.md` instructions. Use the requested scope or the current task's changes. A review reports findings; an editing task applies fixes.

## Workflow

1. **Locate.** Read the affected document and verify its claims against the owning code, tests, configuration, or decision record. Identify the reader and what they need to do or look up. Prefer updating an existing page.
2. **Write.** Describe current behavior in plain language. For a guide, include prerequisites, actions, and observable success; for a reference, make facts easy to find. Preserve limitations, failures, conditions, and important rationale when shortening. Remove repetition and authoring-session narration. Keep proposals clearly separate from implemented behavior.
3. **Synchronize.** Update affected references, examples, and existing translations. Before moving or removing a page, search for inbound paths and anchors, including in hidden directories; repair links and navigation in the same change. For generated content, edit its source and regenerate. When a documentation site is affected, follow [doc-site-sync](../doc-site-sync/SKILL.md).
4. **Verify.** Discover checks from actual project instructions, scripts, and CI. Run the required checks and safely exercise changed examples where practical. If no docs checks exist, inspect local links, anchors, images, and the final diff. Report what changed, checks actually run, and anything unverified; do not invent commands or claim an unrun check passed.

## One Home Per Fact

Use these roles within the existing layout; create a page only when the content needs one.

| Content | Home |
| --- | --- |
| Project entry and quick start | Root README |
| Module usage, configuration, and limitations | Documentation nearest that module |
| Cross-module guides and architecture | Project documentation directory |
| Important decisions and tradeoffs | Existing ADR or notes location |
| Standing agent instructions / reusable procedures | `AGENTS.md` / skills |

Keep the full explanation at its owner and link there from other pages. Include essential local usage facts where needed. Link generated references instead of maintaining parallel catalogs. Follow existing language and format conventions; extra templates, metadata, translations, and validation infrastructure are not prerequisites for a documentation edit.

## Lightweight Decision Records

Unless project rules require more, add a record only for a significant choice whose rationale will help future work. Search existing records first. Capture the problem, decision or proposal, real alternatives and tradeoffs, and status; link the current implementation or documentation. Small fixes need no separate record.

Reuse the project's location and lifecycle. If a new record is needed and no convention exists, one Markdown file under `docs/decisions/` is enough. After implementation, update its status and affected current documentation. When a decision changes, cross-link its replacement and preserve useful rationale; follow existing rules for frozen records and archives.
