WITH search_owners AS (
    SELECT unit_id,record_type AS kind,record_id AS id FROM search_units WHERE record_type<>'chunk'
    UNION ALL
    SELECT u.unit_id,CASE c.owner_kind WHEN 'source_revision' THEN 'source' ELSE c.owner_kind END,
        CASE c.owner_kind WHEN 'source_revision' THEN r.source_id ELSE c.owner_id END
    FROM search_units u JOIN chunks c ON u.record_type='chunk' AND c.id=u.record_id
    LEFT JOIN source_revisions r ON c.owner_kind='source_revision' AND r.id=c.owner_id
), assertion_links AS (
    SELECT 'claim' AS kind,c.id AS assertion_id,c.subject_node_id AS node_id,r.source_id
    FROM claims c LEFT JOIN claim_evidence ce ON ce.claim_id=c.id
    LEFT JOIN evidence e ON e.id=ce.evidence_id LEFT JOIN source_revisions r ON r.id=e.source_revision_id
    UNION
    SELECT 'relation',rel.id,rel.source_node_id,r.source_id
    FROM relations rel LEFT JOIN relation_evidence re ON re.relation_id=rel.id
    LEFT JOIN evidence e ON e.id=re.evidence_id LEFT JOIN source_revisions r ON r.id=e.source_revision_id
    UNION
    SELECT 'relation',rel.id,rel.target_node_id,r.source_id
    FROM relations rel LEFT JOIN relation_evidence re ON re.relation_id=rel.id
    LEFT JOIN evidence e ON e.id=re.evidence_id LEFT JOIN source_revisions r ON r.id=e.source_revision_id
), node_sources AS (
    SELECT node_id,source_id FROM source_node_links
    UNION
    SELECT node_id,source_id FROM assertion_links WHERE source_id IS NOT NULL
), synthesis_links AS (
    SELECT s.id AS synthesis_id,link.node_id,link.source_id FROM syntheses s
    JOIN json_each(s.dependency_snapshot_json,'$.assertions') AS a
    JOIN assertion_links link ON link.kind=json_extract(a.value,'$.kind') AND link.assertion_id=json_extract(a.value,'$.id')
), search_nodes AS (
    SELECT unit_id,id AS node_id FROM search_owners WHERE kind='node'
    UNION
    SELECT o.unit_id,c.subject_node_id FROM search_owners o JOIN claims c ON o.kind='claim' AND c.id=o.id
    UNION
    SELECT o.unit_id,n.node_id FROM search_owners o JOIN node_sources n ON o.kind='source' AND n.source_id=o.id
    UNION
    SELECT o.unit_id,n.node_id FROM search_owners o JOIN synthesis_nodes n ON o.kind='synthesis' AND n.synthesis_id=o.id
    UNION
    SELECT o.unit_id,n.node_id FROM search_owners o JOIN synthesis_links n ON o.kind='synthesis' AND n.synthesis_id=o.id
), search_sources AS (
    SELECT unit_id,id AS source_id FROM search_owners WHERE kind='source'
    UNION
    SELECT o.unit_id,n.source_id FROM search_owners o JOIN node_sources n ON o.kind='node' AND n.node_id=o.id
    UNION
    SELECT o.unit_id,r.source_id FROM search_owners o JOIN claim_evidence ce ON o.kind='claim' AND ce.claim_id=o.id
    JOIN evidence e ON e.id=ce.evidence_id JOIN source_revisions r ON r.id=e.source_revision_id
    UNION
    SELECT o.unit_id,r.source_id FROM search_owners o JOIN synthesis_evidence se ON o.kind='synthesis' AND se.synthesis_id=o.id
    JOIN evidence e ON e.id=se.evidence_id JOIN source_revisions r ON r.id=e.source_revision_id
    UNION
    SELECT o.unit_id,json_extract(head.value,'$.source_id') FROM search_owners o
    JOIN syntheses s ON o.kind='synthesis' AND s.id=o.id
    JOIN json_each(s.dependency_snapshot_json,'$.source_heads') AS head
    UNION
    SELECT o.unit_id,n.source_id FROM search_owners o JOIN synthesis_links n ON o.kind='synthesis' AND n.synthesis_id=o.id
    WHERE n.source_id IS NOT NULL
), search_tags AS (
    SELECT o.unit_id,t.value AS tag FROM search_owners o JOIN sources s ON o.kind='source' AND s.id=o.id
    JOIN json_each(s.tags_json) AS t
    UNION
    SELECT sn.unit_id,t.value FROM search_nodes sn JOIN nodes n ON n.id=sn.node_id
    JOIN json_each(n.tags_json) AS t
)
