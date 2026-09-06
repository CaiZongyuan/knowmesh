use std::collections::{BTreeMap, BTreeSet};

use knowmesh_core::{
    application::{
        impact::{ImpactObject, ImpactRow},
        lexical::{LexicalQuery, RecordType},
        search::{KnowledgeDependencies, SearchData},
    },
    domain::NodeId,
    error::{AppError, AppResult, ErrorType},
    ports::SearchStore,
};
use rusqlite::Transaction;

use crate::{SqliteStore, database_error, impact::context, lexical, reconcile::json_text};

impl SearchStore for SqliteStore {
    fn search_data(&self, query: &LexicalQuery) -> AppResult<SearchData> {
        query.validate()?;
        let deadline = lexical::deadline::Deadline::new(&self.connection, query.timeout_ms)?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let result = read_data(&tx, query);
        deadline.check()?;
        result
    }
}

fn read_data(tx: &Transaction<'_>, query: &LexicalQuery) -> AppResult<SearchData> {
    let workspace: String = tx
        .query_row(
            "SELECT workspace_id FROM workspace_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let lexical = lexical::read_candidates(tx, query)?;
    let exact_candidates = lexical::exact_matches(tx, query)?;
    let candidates: BTreeMap<_, _> = lexical
        .channels
        .iter()
        .flat_map(|part| &part.hits)
        .chain(&exact_candidates)
        .map(|hit| {
            (
                hit.unit_id.clone(),
                (hit.record_type, hit.record_id.clone()),
            )
        })
        .collect();
    let mut objects = BTreeSet::new();
    let mut nodes = BTreeMap::<NodeId, Vec<ImpactObject>>::new();
    for (kind, id) in candidates.values() {
        match kind {
            RecordType::Node => {
                nodes.insert(id.parse()?, vec![]);
            }
            RecordType::Claim => {
                objects.insert(ImpactObject::Claim(id.parse()?));
            }
            RecordType::Synthesis => {
                objects.insert(ImpactObject::Synthesis(id.parse()?));
            }
            RecordType::Source | RecordType::Chunk => {}
        }
    }
    node_assertions(tx, &mut nodes, &mut objects)?;
    let mut rows: Vec<_> = objects
        .into_iter()
        .map(|object| ImpactRow {
            object,
            dependency_ids: vec![],
            reasons: vec![],
            evidence_ids: vec![],
            snapshot: None,
        })
        .collect();
    let context = context::load(tx, &mut rows)?;
    let rows: BTreeMap<_, _> = rows
        .into_iter()
        .map(|row| (row.object.clone(), row))
        .collect();
    let mut dependencies = BTreeMap::new();
    for (unit_id, (kind, id)) in candidates {
        let dependency = match kind {
            RecordType::Node => {
                let assertions = nodes.get(&id.parse()?).ok_or_else(invalid)?;
                let mut evidence = BTreeSet::new();
                for assertion in assertions {
                    evidence.extend(
                        rows.get(assertion)
                            .ok_or_else(invalid)?
                            .evidence_ids
                            .iter()
                            .cloned(),
                    );
                }
                KnowledgeDependencies::Assertion(evidence.into_iter().collect())
            }
            RecordType::Claim => KnowledgeDependencies::Assertion(
                rows.get(&ImpactObject::Claim(id.parse()?))
                    .ok_or_else(invalid)?
                    .evidence_ids
                    .clone(),
            ),
            RecordType::Synthesis => {
                let row = rows
                    .get(&ImpactObject::Synthesis(id.parse()?))
                    .ok_or_else(invalid)?;
                KnowledgeDependencies::Synthesis {
                    evidence_ids: row.evidence_ids.clone(),
                    snapshot: row.snapshot.clone(),
                }
            }
            RecordType::Source | RecordType::Chunk => continue,
        };
        dependencies.insert(unit_id, dependency);
    }
    Ok(SearchData {
        workspace_id: workspace.parse()?,
        lexical,
        exact_candidates,
        dependencies,
        context,
    })
}

fn node_assertions(
    tx: &Transaction<'_>,
    nodes: &mut BTreeMap<NodeId, Vec<ImpactObject>>,
    objects: &mut BTreeSet<ImpactObject>,
) -> AppResult<()> {
    if nodes.is_empty() {
        return Ok(());
    }
    let ids = json_text(&nodes.keys().collect::<Vec<_>>())?;
    let count: usize = tx
        .query_row(
            "SELECT count(*) FROM nodes WHERE id IN (SELECT value FROM json_each(?1))",
            [&ids],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if count != nodes.len() {
        return Err(invalid());
    }
    let mut statement = tx.prepare("SELECT id,subject_node_id FROM claims WHERE lifecycle_status='active' AND subject_node_id IN (SELECT value FROM json_each(?1)) ORDER BY id").map_err(database_error)?;
    for row in statement
        .query_map([&ids], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?
    {
        let (id, subject) = row.map_err(database_error)?;
        let object = ImpactObject::Claim(id.parse()?);
        nodes
            .get_mut(&subject.parse()?)
            .ok_or_else(invalid)?
            .push(object.clone());
        objects.insert(object);
    }
    let mut statement = tx.prepare("SELECT id,source_node_id,target_node_id FROM relations WHERE lifecycle_status='active' AND (source_node_id IN (SELECT value FROM json_each(?1)) OR target_node_id IN (SELECT value FROM json_each(?1))) ORDER BY id").map_err(database_error)?;
    for row in statement
        .query_map([&ids], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
    {
        let (id, source, target) = row.map_err(database_error)?;
        let object = ImpactObject::Relation(id.parse()?);
        for id in [source, target] {
            if let Some(assertions) = nodes.get_mut(&id.parse()?) {
                assertions.push(object.clone());
            }
        }
        objects.insert(object);
    }
    Ok(())
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "Search references missing knowledge projections.",
    )
}
