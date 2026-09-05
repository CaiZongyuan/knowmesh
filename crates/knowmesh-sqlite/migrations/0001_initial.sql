CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    checksum TEXT NOT NULL
) STRICT;

CREATE TABLE workspace_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    workspace_id TEXT NOT NULL UNIQUE,
    schema_hash TEXT NOT NULL,
    canonical_generation INTEGER NOT NULL DEFAULT 0,
    indexed_generation INTEGER NOT NULL DEFAULT 0,
    active_embedding_profile_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE file_manifest (
    path TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    public_id TEXT,
    byte_size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    format_version INTEGER NOT NULL,
    indexed_at TEXT NOT NULL
) STRICT;

CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    language TEXT,
    storage_mode TEXT NOT NULL CHECK (storage_mode IN ('managed','referenced','snapshot-url')),
    manifest_path TEXT NOT NULL UNIQUE,
    current_revision_id TEXT,
    identifiers_json TEXT NOT NULL DEFAULT '{}',
    authors_json TEXT NOT NULL DEFAULT '[]',
    tags_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    removed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE source_revisions (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    content_sha256 TEXT NOT NULL,
    blob_path TEXT,
    original_uri TEXT,
    mime_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    captured_at TEXT NOT NULL,
    parser_name TEXT,
    parser_version TEXT,
    extraction_status TEXT NOT NULL,
    extraction_quality_json TEXT NOT NULL DEFAULT '{}',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE(source_id, content_sha256)
) STRICT;

CREATE INDEX idx_source_revisions_source ON source_revisions(source_id, captured_at DESC);

CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    schema_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    node_type TEXT NOT NULL,
    canonical_name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    slug TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    lifecycle_status TEXT NOT NULL CHECK (lifecycle_status IN ('active','superseded','retracted')),
    properties_json TEXT NOT NULL DEFAULT '{}',
    tags_json TEXT NOT NULL DEFAULT '[]',
    canonical_path TEXT NOT NULL UNIQUE,
    content_sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_nodes_type_name ON nodes(node_type, normalized_name);
CREATE INDEX idx_nodes_status ON nodes(lifecycle_status);

CREATE TABLE node_aliases (
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    normalized_alias TEXT NOT NULL,
    locale TEXT,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0,1)),
    PRIMARY KEY (node_id, normalized_alias)
) STRICT;

CREATE INDEX idx_node_alias_lookup ON node_aliases(normalized_alias);

CREATE TABLE source_node_links (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('primary','supplement','representation')),
    PRIMARY KEY (source_id, node_id, role)
) STRICT;

CREATE INDEX idx_source_node_links_node ON source_node_links(node_id);

CREATE TABLE claims (
    id TEXT PRIMARY KEY,
    subject_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    statement TEXT NOT NULL,
    normalized_hash TEXT NOT NULL,
    semantic_sha256 TEXT NOT NULL,
    lifecycle_status TEXT NOT NULL CHECK (lifecycle_status IN ('active','superseded','retracted')),
    evidence_status TEXT NOT NULL CHECK (evidence_status IN ('supported','uncertain','conflicting','unreviewed')),
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    qualifiers_json TEXT NOT NULL DEFAULT '{}',
    valid_from TEXT,
    valid_until TEXT,
    canonical_path TEXT NOT NULL,
    canonical_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_claims_subject ON claims(subject_node_id, lifecycle_status);
CREATE INDEX idx_claims_evidence_status ON claims(evidence_status);
CREATE UNIQUE INDEX idx_claims_one_active_duplicate
ON claims(subject_node_id, normalized_hash)
WHERE lifecycle_status = 'active';

CREATE TABLE conflict_groups (
    id TEXT PRIMARY KEY,
    subject_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('open','resolved','dismissed')),
    created_at TEXT NOT NULL,
    resolved_at TEXT
) STRICT;

CREATE TABLE conflict_group_claims (
    conflict_group_id TEXT NOT NULL REFERENCES conflict_groups(id) ON DELETE CASCADE,
    claim_id TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    PRIMARY KEY (conflict_group_id, claim_id)
) STRICT;

CREATE TABLE relations (
    id TEXT PRIMARY KEY,
    source_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    predicate TEXT NOT NULL,
    target_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    directed INTEGER NOT NULL CHECK (directed IN (0,1)),
    lifecycle_status TEXT NOT NULL CHECK (lifecycle_status IN ('active','superseded','retracted')),
    evidence_status TEXT NOT NULL CHECK (evidence_status IN ('supported','uncertain','conflicting','unreviewed')),
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    qualifiers_json TEXT NOT NULL DEFAULT '{}',
    semantic_sha256 TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    canonical_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (source_node_id <> target_node_id OR predicate IN ('related_to','same_as'))
) STRICT;

CREATE INDEX idx_relations_out ON relations(source_node_id, predicate, lifecycle_status);
CREATE INDEX idx_relations_in ON relations(target_node_id, predicate, lifecycle_status);

CREATE TABLE evidence (
    id TEXT PRIMARY KEY,
    source_revision_id TEXT NOT NULL REFERENCES source_revisions(id) ON DELETE RESTRICT,
    stance TEXT NOT NULL CHECK (stance IN ('supports','contradicts','context')),
    quote TEXT NOT NULL,
    quote_sha256 TEXT NOT NULL,
    locator_json TEXT NOT NULL,
    extraction_method TEXT NOT NULL CHECK (extraction_method IN ('parser','model','human')),
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    canonical_path TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_evidence_revision ON evidence(source_revision_id);
CREATE INDEX idx_evidence_dedup ON evidence(source_revision_id, quote_sha256);

CREATE TABLE claim_evidence (
    claim_id TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    evidence_id TEXT NOT NULL REFERENCES evidence(id) ON DELETE RESTRICT,
    PRIMARY KEY (claim_id, evidence_id)
) STRICT;

CREATE TABLE relation_evidence (
    relation_id TEXT NOT NULL REFERENCES relations(id) ON DELETE CASCADE,
    evidence_id TEXT NOT NULL REFERENCES evidence(id) ON DELETE RESTRICT,
    PRIMARY KEY (relation_id, evidence_id)
) STRICT;

CREATE TABLE node_mentions (
    id TEXT PRIMARY KEY,
    source_revision_id TEXT REFERENCES source_revisions(id) ON DELETE CASCADE,
    source_node_id TEXT REFERENCES nodes(id) ON DELETE CASCADE,
    target_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    surface TEXT NOT NULL,
    locator_json TEXT NOT NULL DEFAULT '{}',
    confidence REAL,
    mention_kind TEXT NOT NULL CHECK (mention_kind IN ('source','wiki_link')),
    CHECK ((source_revision_id IS NOT NULL) <> (source_node_id IS NOT NULL))
) STRICT;

CREATE INDEX idx_mentions_target ON node_mentions(target_node_id);

CREATE TABLE syntheses (
    id TEXT PRIMARY KEY,
    schema_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    title TEXT NOT NULL,
    question TEXT,
    status TEXT NOT NULL CHECK (status IN ('draft','reviewed','archived')),
    body_markdown TEXT NOT NULL,
    canonical_path TEXT NOT NULL UNIQUE,
    content_sha256 TEXT NOT NULL,
    generated_run_id TEXT,
    dependency_snapshot_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE synthesis_evidence (
    synthesis_id TEXT NOT NULL REFERENCES syntheses(id) ON DELETE CASCADE,
    evidence_id TEXT NOT NULL REFERENCES evidence(id) ON DELETE RESTRICT,
    citation_order INTEGER NOT NULL,
    PRIMARY KEY (synthesis_id, evidence_id)
) STRICT;

CREATE TABLE synthesis_nodes (
    synthesis_id TEXT NOT NULL REFERENCES syntheses(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY (synthesis_id, node_id)
) STRICT;

CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('source_revision','node','synthesis')),
    owner_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    heading_path_json TEXT NOT NULL DEFAULT '[]',
    text TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    locator_json TEXT NOT NULL DEFAULT '{}',
    language TEXT,
    token_count INTEGER,
    token_count_estimated INTEGER NOT NULL DEFAULT 0 CHECK (token_count_estimated IN (0,1)),
    UNIQUE(owner_kind, owner_id, ordinal, content_sha256)
) STRICT;

CREATE INDEX idx_chunks_owner ON chunks(owner_kind, owner_id, ordinal);

CREATE TABLE search_units (
    rowid INTEGER PRIMARY KEY,
    unit_id TEXT NOT NULL UNIQUE,
    record_type TEXT NOT NULL CHECK (record_type IN ('node','claim','source','synthesis','chunk')),
    record_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    aliases TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '',
    language TEXT,
    lifecycle_status TEXT NOT NULL DEFAULT 'active',
    content_sha256 TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_search_units_record ON search_units(record_type, record_id);

CREATE VIRTUAL TABLE search_fts_word USING fts5(
    title,
    aliases,
    body,
    tags,
    content='search_units',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3 4'
);

CREATE VIRTUAL TABLE search_fts_tri USING fts5(
    title,
    aliases,
    body,
    tags,
    content='search_units',
    content_rowid='rowid',
    tokenize='trigram case_sensitive 0'
);

CREATE TRIGGER search_units_ai AFTER INSERT ON search_units BEGIN
  INSERT INTO search_fts_word(rowid,title,aliases,body,tags)
    VALUES (new.rowid,new.title,new.aliases,new.body,new.tags);
  INSERT INTO search_fts_tri(rowid,title,aliases,body,tags)
    VALUES (new.rowid,new.title,new.aliases,new.body,new.tags);
END;

CREATE TRIGGER search_units_ad AFTER DELETE ON search_units BEGIN
  INSERT INTO search_fts_word(search_fts_word,rowid,title,aliases,body,tags)
    VALUES ('delete',old.rowid,old.title,old.aliases,old.body,old.tags);
  INSERT INTO search_fts_tri(search_fts_tri,rowid,title,aliases,body,tags)
    VALUES ('delete',old.rowid,old.title,old.aliases,old.body,old.tags);
END;

CREATE TRIGGER search_units_au AFTER UPDATE ON search_units BEGIN
  INSERT INTO search_fts_word(search_fts_word,rowid,title,aliases,body,tags)
    VALUES ('delete',old.rowid,old.title,old.aliases,old.body,old.tags);
  INSERT INTO search_fts_word(rowid,title,aliases,body,tags)
    VALUES (new.rowid,new.title,new.aliases,new.body,new.tags);
  INSERT INTO search_fts_tri(search_fts_tri,rowid,title,aliases,body,tags)
    VALUES ('delete',old.rowid,old.title,old.aliases,old.body,old.tags);
  INSERT INTO search_fts_tri(rowid,title,aliases,body,tags)
    VALUES (new.rowid,new.title,new.aliases,new.body,new.tags);
END;

CREATE TABLE embedding_profiles (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL CHECK (dimensions > 0 AND dimensions <= 65536),
    distance_metric TEXT NOT NULL CHECK (distance_metric IN ('cosine','l2')),
    config_hash TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0,1)),
    created_at TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_one_active_embedding
ON embedding_profiles(active) WHERE active = 1;

CREATE TABLE search_vector_state (
    search_unit_rowid INTEGER PRIMARY KEY REFERENCES search_units(rowid) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    content_sha256 TEXT NOT NULL,
    embedded_at TEXT NOT NULL
) STRICT;

-- Created only when vector capability is enabled. The rowid equals search_units.rowid.
-- CREATE VIRTUAL TABLE search_vectors USING vec0(
--     embedding float[${EMBEDDING_DIMENSIONS}]
-- );

CREATE TABLE proposals (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('draft','reviewing','approved','applied','rejected','stale')),
    revision INTEGER NOT NULL DEFAULT 1,
    base_generation INTEGER NOT NULL,
    source_revision_id TEXT REFERENCES source_revisions(id) ON DELETE SET NULL,
    schema_hash TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    compiler_run_id TEXT,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    applied_at TEXT,
    applied_generation INTEGER
) STRICT;

CREATE INDEX idx_proposals_state ON proposals(state, updated_at DESC);

CREATE TABLE proposal_items (
    id TEXT PRIMARY KEY,
    proposal_id TEXT NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    op TEXT NOT NULL,
    target_id TEXT,
    payload_json TEXT NOT NULL,
    before_sha256 TEXT,
    evidence_ids_json TEXT NOT NULL DEFAULT '[]',
    compiler_confidence REAL,
    risk TEXT NOT NULL CHECK (risk IN ('low','medium','high')),
    decision TEXT NOT NULL CHECK (decision IN ('pending','accepted','rejected')),
    decision_reason TEXT,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    UNIQUE(proposal_id, ordinal)
) STRICT;

CREATE TABLE operation_runs (
    id TEXT PRIMARY KEY,
    parent_run_id TEXT REFERENCES operation_runs(id) ON DELETE SET NULL,
    operation TEXT NOT NULL,
    surface TEXT NOT NULL CHECK (surface IN ('cli','http','internal')),
    actor TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued','running','paused','interrupted','succeeded','failed','cancelled','partial')),
    control_action TEXT CHECK (control_action IN ('pause','cancel')),
    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    checkpoint_json TEXT,
    input_json TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    dependency_snapshot_json TEXT NOT NULL DEFAULT '{}',
    config_hash TEXT,
    purpose_sha256 TEXT,
    output_refs_json TEXT NOT NULL DEFAULT '[]',
    output_json TEXT,
    model_json TEXT,
    prompt_id TEXT,
    prompt_sha256 TEXT,
    schema_hash TEXT,
    usage_json TEXT NOT NULL DEFAULT '{}',
    budget_json TEXT NOT NULL DEFAULT '{}',
    retries_json TEXT NOT NULL DEFAULT '{}',
    error_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
) STRICT;

CREATE INDEX idx_runs_operation_time ON operation_runs(operation, started_at DESC);
CREATE INDEX idx_runs_status_time ON operation_runs(status, created_at DESC);

CREATE TABLE idempotency_keys (
    key TEXT NOT NULL,
    operation TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    run_id TEXT REFERENCES operation_runs(id) ON DELETE SET NULL,
    state TEXT NOT NULL CHECK (state IN ('in_progress','completed')),
    response_json TEXT,
    status_code INTEGER,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    PRIMARY KEY (key, operation),
    CHECK (
      (state = 'in_progress' AND response_json IS NULL AND status_code IS NULL)
      OR (state = 'completed' AND response_json IS NOT NULL AND status_code IS NOT NULL)
    )
) STRICT;

CREATE TABLE audit_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    run_id TEXT REFERENCES operation_runs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    object_type TEXT,
    object_id TEXT,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
) STRICT;
