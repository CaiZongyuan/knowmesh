CREATE TABLE proposal_applications (
    proposal_id TEXT PRIMARY KEY REFERENCES proposals(id) ON DELETE CASCADE,
    reviewed_revision INTEGER NOT NULL,
    receipt_json TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    FOREIGN KEY (proposal_id, reviewed_revision)
        REFERENCES proposal_revisions(proposal_id, revision) ON DELETE RESTRICT
) STRICT;
