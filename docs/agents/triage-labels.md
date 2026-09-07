# Issue State

Labels describe one agent's work phase. Native blockers and verified commits determine eligibility; there are no difficulty-based agent lanes.

| Label | Meaning |
| --- | --- |
| `plan:v0.1` | Approved execution scope |
| `execution:single-agent` | Uses the continuous execution loop |
| `ready-for-agent` | Self-contained accepted task; still check blockers |
| `status:ready` | Ready for selection after prerequisite checks |
| `status:in-progress` | The current implementation |
| `status:review` | The same agent is reviewing or waiting for required CI |
| `status:blocked` | A required dependency is incomplete |
| `needs-info` | Specific external information is missing |
| `ready-for-human` | A human action or judgment is required |
| `tracking-only` | Scope/evidence umbrella, not a duplicate implementation |
| `wontfix` | Explicitly will not be implemented |

Remove retired status:assigned and difficulty lane labels from the execution queue. Historical difficulty metadata in local publication snapshots is only an estimate.

The same agent updates status, verifies results, closes completed issues and continues. If one issue is externally blocked, select another unblocked issue. If none are executable, report the exact external blockers; do not manufacture human gold or unavailable Harness evidence.
