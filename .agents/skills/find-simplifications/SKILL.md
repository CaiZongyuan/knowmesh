---
name: find-simplifications
description: Investigate simplification opportunities when the user requests a maintenance survey or broader cleanup assessment. Produce evidence-backed proposals and identify behavior changes, risks, and affected documentation.
---

# Find Simplifications

Inspect the requested scope and applicable project instructions. For routine cleanup of recently changed code, use [code-simplifier](../code-simplifier/SKILL.md).

## Workflow

1. Look for unused capabilities, duplicated facts or logic, unnecessary indirection, and custom code that maintained tooling could replace. Start with a few plausible candidates.
2. Verify usage with symbol, export, configuration, and route searches. Inspect runtime registration and entry points. Distinguish production consumers from tests and fixtures. Absence of local callers does not prove a public API is unused.
3. Compare the actual reduction with compatibility, stored data, failure handling, and maintenance costs. Include dependency and integration costs for replacements. Reject candidates that only move complexity elsewhere or weaken required safeguards.
4. Report each supported proposal with evidence, the change, any behavior given up, affected tests and documentation, risks, and how to validate it. State uncertainties instead of presenting them as proven removals. Finding no worthwhile candidate is a valid result.

## Follow Through

A survey produces findings. Implement proposals when implementation is already within the user's request; otherwise leave code unchanged. Record durable decisions only when useful, following [doc-standards](../doc-standards/SKILL.md#lightweight-decision-records). A small suggestion can remain in the report.
