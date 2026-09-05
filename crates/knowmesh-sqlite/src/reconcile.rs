use std::path::Path;

use knowmesh_core::{
    canonical::snapshot::{CanonicalSnapshot, SourceProjection},
    domain::{StorageMode, Timestamp, normalize_name, sha256},
    error::{AppError, AppResult, ErrorType},
    ports::{ProjectionStore, ReconcileReport},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{SqliteStore, database_error};

impl ProjectionStore for SqliteStore {
    fn reconcile(&mut self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport> {
        snapshot.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let (workspace_id, previous_hash, generation): (String, String, u64) = tx.query_row("SELECT workspace_id,snapshot_sha256,indexed_generation FROM workspace_state WHERE singleton=1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(database_error)?;
        if workspace_id != snapshot.workspace_id.as_str() {
            return Err(AppError::new(
                ErrorType::Configuration,
                "WORKSPACE_ID_MISMATCH",
                "The projection snapshot belongs to another workspace.",
            ));
        }
        if previous_hash == snapshot.content_sha256 {
            for file in &snapshot.files {
                tx.execute(
                    "UPDATE file_manifest SET mtime_ns=?1,byte_size=?2 WHERE path=?3 AND sha256=?4",
                    params![
                        file.mtime_ns,
                        file.byte_size,
                        path_text(&file.path)?,
                        file.sha256
                    ],
                )
                .map_err(database_error)?;
            }
            tx.execute(
                "UPDATE workspace_state SET snapshot_warnings_json=?1 WHERE singleton=1",
                [json_text(&snapshot.warnings)?],
            )
            .map_err(database_error)?;
            tx.commit().map_err(database_error)?;
            return Ok(report(snapshot, generation, false));
        }
        for source in &snapshot.sources {
            let previous: Option<String> = tx
                .query_row(
                    "SELECT canonical_json FROM sources WHERE id=?1",
                    [source.manifest.id.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if let Some(previous) = previous.filter(|text| text != "{}") {
                let previous: SourceProjection =
                    serde_json::from_str(&previous).map_err(|_| payload_error())?;
                source.manifest.validate_update(&previous.manifest)?;
            }
        }
        tx.execute_batch("DELETE FROM claim_evidence; DELETE FROM relation_evidence; DELETE FROM synthesis_evidence; DELETE FROM synthesis_nodes; DELETE FROM source_node_links; DELETE FROM node_aliases; DELETE FROM node_mentions;").map_err(database_error)?;
        // Vacate unique keys inside this transaction so swaps preserve object rows and runtime links.
        // NUL cannot occur in a canonical filesystem path or a SHA-256 digest.
        tx.execute_batch(
            "UPDATE sources SET manifest_path=char(0)||id;
            UPDATE nodes SET canonical_path=char(0)||id;
            UPDATE syntheses SET canonical_path=char(0)||id;
            UPDATE claims SET normalized_hash=char(0)||id;",
        )
        .map_err(database_error)?;
        for source in &snapshot.sources {
            let value = &source.manifest;
            tx.execute(
                "INSERT INTO sources(id,slug,kind,title,language,storage_mode,manifest_path,current_revision_id,identifiers_json,authors_json,tags_json,status,removed_at,created_at,updated_at,canonical_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(id) DO UPDATE SET slug=excluded.slug,kind=excluded.kind,title=excluded.title,language=excluded.language,storage_mode=excluded.storage_mode,manifest_path=excluded.manifest_path,
                 status=CASE WHEN sources.current_revision_id IS NOT excluded.current_revision_id THEN excluded.status ELSE sources.status END,current_revision_id=excluded.current_revision_id,
                 identifiers_json=excluded.identifiers_json,authors_json=excluded.authors_json,tags_json=excluded.tags_json,removed_at=excluded.removed_at,created_at=excluded.created_at,updated_at=excluded.updated_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), value.slug, value.kind, value.title, value.language, enum_text(&value.storage)?, path_text(&source.manifest_path)?, value.current_revision_id.as_str(), json_text(&value.identifiers)?, json_text(&value.authors)?, json_text(&value.tags)?, if value.storage == StorageMode::Referenced { "registered" } else { "snapshotted" }, value.removed_at.map(|time| time.to_string()), value.created_at.to_string(), value.updated_at.to_string(), json_text(source)?],
            ).map_err(database_error)?;
            for revision in &value.revisions {
                let blob = if value.storage == StorageMode::Referenced {
                    None
                } else {
                    Some(
                        path_text(
                            &source
                                .manifest_path
                                .parent()
                                .ok_or_else(payload_error)?
                                .join(&revision.path),
                        )?
                        .to_owned(),
                    )
                };
                let uri = if value.storage == StorageMode::Referenced {
                    Some(revision.path.clone())
                } else {
                    revision.url.clone()
                };
                tx.execute("INSERT INTO source_revisions(id,source_id,content_sha256,blob_path,original_uri,mime_type,byte_size,captured_at,extraction_status)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending') ON CONFLICT(id) DO UPDATE SET source_id=excluded.source_id,content_sha256=excluded.content_sha256,blob_path=excluded.blob_path,original_uri=excluded.original_uri,mime_type=excluded.mime_type,byte_size=excluded.byte_size,captured_at=excluded.captured_at",
                    params![revision.id.as_str(), value.id.as_str(), revision.sha256, blob, uri, revision.mime_type, revision.byte_size, revision.captured_at.to_string()]).map_err(database_error)?;
            }
        }
        for node in &snapshot.nodes {
            let value = &node.metadata;
            let (schema_id, schema_version) = schema_reference(&value.schema)?;
            let slug = node
                .canonical_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("node")
                .split("--")
                .next()
                .unwrap_or("node");
            tx.execute("INSERT INTO nodes(id,schema_id,schema_version,node_type,canonical_name,normalized_name,slug,summary,lifecycle_status,properties_json,tags_json,canonical_path,content_sha256,created_at,updated_at,canonical_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                ON CONFLICT(id) DO UPDATE SET schema_id=excluded.schema_id,schema_version=excluded.schema_version,node_type=excluded.node_type,canonical_name=excluded.canonical_name,normalized_name=excluded.normalized_name,slug=excluded.slug,summary=excluded.summary,lifecycle_status=excluded.lifecycle_status,properties_json=excluded.properties_json,tags_json=excluded.tags_json,canonical_path=excluded.canonical_path,content_sha256=excluded.content_sha256,created_at=excluded.created_at,updated_at=excluded.updated_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), schema_id, schema_version, value.node_type, value.name, normalize_name(&value.name), slug, node.summary, enum_text(&value.lifecycle_status)?, json_text(&value.properties)?, json_text(&value.tags)?, path_text(&node.canonical_path)?, node.content_sha256, value.created_at.to_string(), value.updated_at.to_string(), json_text(node)?]).map_err(database_error)?;
            for (ordinal, alias) in std::iter::once(&value.name)
                .chain(&value.aliases)
                .enumerate()
            {
                tx.execute("INSERT OR IGNORE INTO node_aliases(node_id,alias,normalized_alias,is_primary) VALUES(?1,?2,?3,?4)", params![value.id.as_str(), alias, normalize_name(alias), ordinal == 0]).map_err(database_error)?;
            }
        }
        for source in &snapshot.sources {
            for node in &source.manifest.represented_nodes {
                tx.execute("INSERT INTO source_node_links(source_id,node_id,role) VALUES(?1,?2,'representation')", params![source.manifest.id.as_str(), node.as_str()]).map_err(database_error)?;
            }
        }
        for item in &snapshot.claims {
            let value = &item.claim.assertion;
            tx.execute("INSERT INTO claims(id,subject_node_id,statement,normalized_hash,semantic_sha256,lifecycle_status,evidence_status,confidence,qualifiers_json,canonical_path,canonical_order,created_at,updated_at,canonical_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                ON CONFLICT(id) DO UPDATE SET subject_node_id=excluded.subject_node_id,statement=excluded.statement,normalized_hash=excluded.normalized_hash,semantic_sha256=excluded.semantic_sha256,lifecycle_status=excluded.lifecycle_status,evidence_status=excluded.evidence_status,confidence=excluded.confidence,qualifiers_json=excluded.qualifiers_json,canonical_path=excluded.canonical_path,canonical_order=excluded.canonical_order,created_at=excluded.created_at,updated_at=excluded.updated_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), item.claim.subject_node_id.as_str(), value.statement, item.normalized_hash, item.semantic_sha256, enum_text(&value.lifecycle_status)?, enum_text(&value.evidence_status)?, value.confidence, json_text(&value.qualifiers)?, path_text(&item.canonical_path)?, item.canonical_order as u64, item.created_at.to_string(), item.updated_at.to_string(), json_text(item)?]).map_err(database_error)?;
        }
        for item in &snapshot.relations {
            let value = &item.relation.assertion;
            tx.execute("INSERT INTO relations(id,source_node_id,predicate,target_node_id,directed,lifecycle_status,evidence_status,confidence,qualifiers_json,semantic_sha256,canonical_path,canonical_order,created_at,updated_at,canonical_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                ON CONFLICT(id) DO UPDATE SET source_node_id=excluded.source_node_id,predicate=excluded.predicate,target_node_id=excluded.target_node_id,directed=excluded.directed,lifecycle_status=excluded.lifecycle_status,evidence_status=excluded.evidence_status,confidence=excluded.confidence,qualifiers_json=excluded.qualifiers_json,semantic_sha256=excluded.semantic_sha256,canonical_path=excluded.canonical_path,canonical_order=excluded.canonical_order,created_at=excluded.created_at,updated_at=excluded.updated_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), item.relation.source_node_id.as_str(), value.predicate, value.target_node_id.as_str(), value.directed, enum_text(&value.lifecycle_status)?, enum_text(&value.evidence_status)?, value.confidence, json_text(&value.qualifiers)?, item.semantic_sha256, path_text(&item.canonical_path)?, item.canonical_order as u64, item.created_at.to_string(), item.updated_at.to_string(), json_text(item)?]).map_err(database_error)?;
        }
        for item in &snapshot.evidence {
            let value = &item.evidence;
            tx.execute("INSERT INTO evidence(id,source_revision_id,stance,quote,quote_sha256,locator_json,extraction_method,confidence,canonical_path,created_at,canonical_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                ON CONFLICT(id) DO UPDATE SET source_revision_id=excluded.source_revision_id,stance=excluded.stance,quote=excluded.quote,quote_sha256=excluded.quote_sha256,locator_json=excluded.locator_json,extraction_method=excluded.extraction_method,confidence=excluded.confidence,canonical_path=excluded.canonical_path,created_at=excluded.created_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), value.source_revision_id.as_str(), enum_text(&value.stance)?, value.quote, value.quote_sha256, json_text(&value.locator)?, enum_text(&value.extraction_method)?, value.confidence, path_text(&item.canonical_path)?, item.created_at.to_string(), json_text(item)?]).map_err(database_error)?;
        }
        for item in &snapshot.claims {
            for evidence in &item.claim.assertion.evidence {
                tx.execute(
                    "INSERT INTO claim_evidence(claim_id,evidence_id) VALUES(?1,?2)",
                    params![item.claim.assertion.id.as_str(), evidence.id.as_str()],
                )
                .map_err(database_error)?;
            }
        }
        for item in &snapshot.relations {
            for evidence in &item.relation.assertion.evidence {
                tx.execute(
                    "INSERT INTO relation_evidence(relation_id,evidence_id) VALUES(?1,?2)",
                    params![item.relation.assertion.id.as_str(), evidence.id.as_str()],
                )
                .map_err(database_error)?;
            }
        }
        for item in &snapshot.syntheses {
            let value = &item.metadata;
            let (schema_id, schema_version) = schema_reference(&value.schema)?;
            let dependency = value
                .dependency_snapshot
                .as_ref()
                .map(json_text)
                .transpose()?
                .unwrap_or_else(|| "{}".into());
            tx.execute("INSERT INTO syntheses(id,schema_id,schema_version,title,question,status,body_markdown,canonical_path,content_sha256,generated_run_id,dependency_snapshot_json,created_at,updated_at,canonical_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                ON CONFLICT(id) DO UPDATE SET schema_id=excluded.schema_id,schema_version=excluded.schema_version,title=excluded.title,question=excluded.question,status=excluded.status,body_markdown=excluded.body_markdown,canonical_path=excluded.canonical_path,content_sha256=excluded.content_sha256,generated_run_id=excluded.generated_run_id,dependency_snapshot_json=excluded.dependency_snapshot_json,created_at=excluded.created_at,updated_at=excluded.updated_at,canonical_json=excluded.canonical_json",
                params![value.id.as_str(), schema_id, schema_version, value.title, value.question, enum_text(&value.status)?, item.body_markdown, path_text(&item.canonical_path)?, item.content_sha256, value.generated_by.as_ref().map(|g| g.run_id.as_str()), dependency, value.created_at.to_string(), value.updated_at.to_string(), json_text(item)?]).map_err(database_error)?;
            for (ordinal, evidence) in value.evidence_ids.iter().enumerate() {
                tx.execute("INSERT INTO synthesis_evidence(synthesis_id,evidence_id,citation_order) VALUES(?1,?2,?3)", params![value.id.as_str(), evidence.as_str(), ordinal as u64]).map_err(database_error)?;
            }
            for node in &value.related_nodes {
                tx.execute(
                    "INSERT INTO synthesis_nodes(synthesis_id,node_id) VALUES(?1,?2)",
                    params![value.id.as_str(), node.as_str()],
                )
                .map_err(database_error)?;
            }
        }
        for mention in &snapshot.mentions {
            tx.execute("INSERT INTO node_mentions(id,source_node_id,target_node_id,surface,locator_json,confidence,mention_kind) VALUES(?1,?2,?3,?4,?5,1.0,'wiki_link')", params![mention.id, mention.source_node_id.as_str(), mention.target_node_id.as_str(), mention.surface, json_text(&json!({"byte_start":mention.byte_start,"byte_end":mention.byte_end}))?]).map_err(database_error)?;
        }
        delete_missing(
            &tx,
            "claims",
            snapshot
                .claims
                .iter()
                .map(|v| v.claim.assertion.id.as_str()),
        )?;
        delete_missing(
            &tx,
            "relations",
            snapshot
                .relations
                .iter()
                .map(|v| v.relation.assertion.id.as_str()),
        )?;
        delete_missing(
            &tx,
            "syntheses",
            snapshot.syntheses.iter().map(|v| v.metadata.id.as_str()),
        )?;
        delete_missing(
            &tx,
            "evidence",
            snapshot.evidence.iter().map(|v| v.evidence.id.as_str()),
        )?;
        delete_missing(
            &tx,
            "nodes",
            snapshot.nodes.iter().map(|v| v.metadata.id.as_str()),
        )?;
        delete_missing(
            &tx,
            "source_revisions",
            snapshot
                .sources
                .iter()
                .flat_map(|s| s.manifest.revisions.iter().map(|r| r.id.as_str())),
        )?;
        delete_missing(
            &tx,
            "sources",
            snapshot.sources.iter().map(|v| v.manifest.id.as_str()),
        )?;
        reconcile_search(&tx, snapshot)?;
        tx.execute("DELETE FROM file_manifest", [])
            .map_err(database_error)?;
        let now = Timestamp::now().to_string();
        for file in &snapshot.files {
            tx.execute("INSERT INTO file_manifest(path,kind,public_id,byte_size,mtime_ns,sha256,format_version,indexed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![path_text(&file.path)?, file.kind, file.public_id, file.byte_size, file.mtime_ns, file.sha256, file.format_version, now]).map_err(database_error)?;
        }
        let generation = generation.checked_add(1).ok_or_else(payload_error)?;
        tx.execute("UPDATE workspace_state SET schema_hash=?1,snapshot_sha256=?2,canonical_generation=?3,indexed_generation=?3,updated_at=?4,snapshot_warnings_json=?5 WHERE singleton=1", params![snapshot.schema_hash, snapshot.content_sha256, generation, now, json_text(&snapshot.warnings)?]).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(report(snapshot, generation, true))
    }
}

impl SqliteStore {
    pub fn logical_snapshot(&self) -> AppResult<Value> {
        let mut output = serde_json::Map::new();
        for table in [
            "sources",
            "nodes",
            "claims",
            "relations",
            "evidence",
            "syntheses",
        ] {
            let mut statement = self
                .connection
                .prepare(&format!("SELECT canonical_json FROM {table} ORDER BY id"))
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(database_error)?;
            let mut values = Vec::new();
            for row in rows {
                values.push(
                    serde_json::from_str::<Value>(&row.map_err(database_error)?)
                        .map_err(|_| payload_error())?,
                );
            }
            output.insert(table.into(), Value::Array(values));
        }
        Ok(Value::Object(output))
    }
}

fn reconcile_search(connection: &Connection, snapshot: &CanonicalSnapshot) -> AppResult<()> {
    let mut ids = Vec::new();
    for node in &snapshot.nodes {
        let id = format!("node:{}", node.metadata.id);
        upsert_search(
            connection,
            &id,
            "node",
            node.metadata.id.as_str(),
            &node.metadata.name,
            &node.metadata.aliases.join("\n"),
            &node.summary,
            &node.metadata.tags.join(" "),
            None,
            &enum_text(&node.metadata.lifecycle_status)?,
            node.metadata.updated_at,
        )?;
        ids.push(id);
    }
    for claim in &snapshot.claims {
        let id = format!("claim:{}", claim.claim.assertion.id);
        upsert_search(
            connection,
            &id,
            "claim",
            claim.claim.assertion.id.as_str(),
            "",
            "",
            &claim.claim.assertion.statement,
            "",
            None,
            &enum_text(&claim.claim.assertion.lifecycle_status)?,
            claim.updated_at,
        )?;
        ids.push(id);
    }
    for source in &snapshot.sources {
        let id = format!("source:{}", source.manifest.id);
        upsert_search(
            connection,
            &id,
            "source",
            source.manifest.id.as_str(),
            &source.manifest.title,
            "",
            &source
                .manifest
                .authors
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            &source.manifest.tags.join(" "),
            source.manifest.language.as_deref(),
            if source.manifest.removed_at.is_some() {
                "removed"
            } else {
                "active"
            },
            source.manifest.updated_at,
        )?;
        ids.push(id);
    }
    for synthesis in &snapshot.syntheses {
        let id = format!("synthesis:{}", synthesis.metadata.id);
        upsert_search(
            connection,
            &id,
            "synthesis",
            synthesis.metadata.id.as_str(),
            &synthesis.metadata.title,
            "",
            &synthesis.body_markdown,
            "",
            None,
            if synthesis.metadata.status == knowmesh_core::domain::SynthesisStatus::Archived {
                "archived"
            } else {
                "active"
            },
            synthesis.metadata.updated_at,
        )?;
        ids.push(id);
    }
    connection.execute("DELETE FROM search_units WHERE record_type <> 'chunk' AND unit_id NOT IN (SELECT value FROM json_each(?1))", [json_text(&ids)?]).map_err(database_error)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn upsert_search(
    connection: &Connection,
    unit_id: &str,
    kind: &str,
    record_id: &str,
    title: &str,
    aliases: &str,
    body: &str,
    tags: &str,
    language: Option<&str>,
    status: &str,
    updated_at: Timestamp,
) -> AppResult<()> {
    let hash = sha256(json_text(&(title, aliases, body, tags, language, status))?.as_bytes());
    connection.execute("INSERT INTO search_units(unit_id,record_type,record_id,title,aliases,body,tags,language,lifecycle_status,content_sha256,updated_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(unit_id) DO UPDATE SET record_type=excluded.record_type,record_id=excluded.record_id,title=excluded.title,aliases=excluded.aliases,body=excluded.body,tags=excluded.tags,language=excluded.language,lifecycle_status=excluded.lifecycle_status,content_sha256=excluded.content_sha256,updated_at=excluded.updated_at WHERE search_units.content_sha256 <> excluded.content_sha256",
        params![unit_id, kind, record_id, title, aliases, body, tags, language, status, hash, updated_at.to_string()]).map_err(database_error)?;
    Ok(())
}

fn delete_missing<'a>(
    connection: &Connection,
    table: &str,
    ids: impl Iterator<Item = &'a str>,
) -> AppResult<()> {
    let values: Vec<_> = ids.collect();
    connection
        .execute(
            &format!("DELETE FROM {table} WHERE id NOT IN (SELECT value FROM json_each(?1))"),
            [json_text(&values)?],
        )
        .map_err(database_error)?;
    Ok(())
}

fn report(snapshot: &CanonicalSnapshot, generation: u64, changed: bool) -> ReconcileReport {
    ReconcileReport {
        generation,
        changed,
        source_count: snapshot.sources.len(),
        node_count: snapshot.nodes.len(),
        claim_count: snapshot.claims.len(),
        relation_count: snapshot.relations.len(),
        evidence_count: snapshot.evidence.len(),
        synthesis_count: snapshot.syntheses.len(),
    }
}
fn schema_reference(value: &str) -> AppResult<(&str, u32)> {
    let (id, version) = value.split_once('@').ok_or_else(payload_error)?;
    Ok((id, version.parse().map_err(|_| payload_error())?))
}
pub(crate) fn json_text(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| payload_error())
}
fn enum_text(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_value(value)
        .map_err(|_| payload_error())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(payload_error)
}
fn path_text(path: &Path) -> AppResult<&str> {
    path.to_str().ok_or_else(payload_error)
}
fn payload_error() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "A canonical projection payload is invalid or unsupported.",
    )
}
