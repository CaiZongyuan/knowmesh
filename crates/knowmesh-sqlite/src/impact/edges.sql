WITH matched_evidence AS (
    SELECT e.id, e.source_revision_id
    FROM evidence e JOIN source_revisions r ON r.id=e.source_revision_id
    WHERE r.source_id=?1 AND (?2 IS NULL OR r.id=?2)
), matched_assertions AS (
    SELECT 'claim' AS kind, ce.claim_id AS id, e.id AS dependency_id
    FROM claim_evidence ce JOIN matched_evidence e ON e.id=ce.evidence_id
    UNION ALL
    SELECT 'relation', re.relation_id, e.id
    FROM relation_evidence re JOIN matched_evidence e ON e.id=re.evidence_id
), edges AS (
    SELECT 'evidence' AS kind, id, source_revision_id AS dependency_id, 'source_revision' AS reason
    FROM matched_evidence
    UNION ALL
    SELECT kind, id, dependency_id, 'evidence_reference' FROM matched_assertions
    UNION ALL
    SELECT 'synthesis', se.synthesis_id, e.id, 'evidence_reference'
    FROM synthesis_evidence se JOIN matched_evidence e ON e.id=se.evidence_id
    UNION ALL
    SELECT 'synthesis', s.id, a.id, 'assertion_dependency'
    FROM syntheses s, json_each(s.dependency_snapshot_json, '$.assertions') d
    JOIN matched_assertions a ON a.id=json_extract(d.value, '$.id') AND a.kind=json_extract(d.value, '$.kind')
    UNION ALL
    SELECT 'synthesis', s.id, json_extract(h.value, '$.revision_id'), 'source_head'
    FROM syntheses s, json_each(s.dependency_snapshot_json, '$.source_heads') h
    WHERE json_extract(h.value, '$.source_id')=?1 AND (?2 IS NULL OR json_extract(h.value, '$.revision_id')=?2)
)
