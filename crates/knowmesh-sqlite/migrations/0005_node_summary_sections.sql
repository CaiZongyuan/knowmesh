-- Refresh summaries derived from quoted or nested headings by older scanners.
UPDATE workspace_state SET snapshot_warnings_json = NULL;
