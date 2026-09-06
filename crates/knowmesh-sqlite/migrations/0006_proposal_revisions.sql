CREATE TABLE proposal_revisions (
    proposal_id TEXT NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision > 0),
    snapshot_json TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (proposal_id, revision)
) STRICT;
