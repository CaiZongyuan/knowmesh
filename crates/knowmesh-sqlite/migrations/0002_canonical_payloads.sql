ALTER TABLE workspace_state ADD COLUMN snapshot_sha256 TEXT NOT NULL DEFAULT '';
ALTER TABLE sources ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE nodes ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE claims ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE relations ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE evidence ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE syntheses ADD COLUMN canonical_json TEXT NOT NULL DEFAULT '{}';
