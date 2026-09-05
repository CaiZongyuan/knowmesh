---
name: doc-site-sync
description: Synchronize documentation source changes with an existing documentation site's routes, navigation, assets, and build. Use when adding, updating, moving, removing, or debugging site pages.
---

# Synchronize a Documentation Site

Use the existing site's source and build conventions. If no site exists, report that site synchronization does not apply; create one only when that is part of the user's request.

## Workflow

1. **Find the source.** Read project instructions and site configuration. Identify authored content, routing and navigation rules, generated output, and the actual preview/build commands. Do not assume a framework, manifest, or directory name.
2. **Update the page.** Edit the authoritative content source. For generated pages, change the input or generator and regenerate; follow the project's policy on committing output. Keep one maintained copy of the content.
3. **Connect it.** Use the site's existing manifest, frontmatter, configuration, or file-based routing to expose the page. For moves and removals, update inbound links, navigation, assets, and existing translations together. Preserve public URLs or add redirects when the project requires it. Respect source-link conventions and the build's link rewriting.
4. **Check the result.** Run existing documentation checks and the site build. Preview affected routes when rendering or navigation changes; check links, anchors, images, and code blocks. Report the source files, affected URLs, and actual validation results.

Content placement and writing follow [doc-standards](../doc-standards/SKILL.md). Skip configuration changes when a content edit needs none. A successful local build does not mean a deployment occurred; deploy when it is within the user's requested scope.
