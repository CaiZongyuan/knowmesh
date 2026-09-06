-- Reconcile legacy case-folded Claim keys before reusing filesystem scan hints.
UPDATE workspace_state SET snapshot_warnings_json = NULL;
