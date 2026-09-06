use knowmesh_core::{
    canonical::snapshot::ConflictGroupProjection, domain::ConflictGroup, error::AppResult,
};
use rusqlite::Transaction;

use super::{database_error, payload_error};

pub(super) fn read(tx: &Transaction<'_>) -> AppResult<Vec<ConflictGroupProjection>> {
    let mut statement = tx.prepare(
        "SELECT g.id,g.subject_node_id,g.reason,g.status,g.created_at,g.resolved_at,
        (SELECT json_group_array(claim_id) FROM (SELECT claim_id FROM conflict_group_claims WHERE conflict_group_id=g.id ORDER BY claim_id))
        FROM conflict_groups g ORDER BY g.id",
    ).map_err(database_error)?;
    let mut groups = vec![];
    for row in statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(database_error)?
    {
        let (id, subject, reason, status, created, resolved, members) =
            row.map_err(database_error)?;
        let group = ConflictGroup {
            id: id.parse()?,
            claim_ids: serde_json::from_str(&members).map_err(|_| payload_error())?,
            reason,
            status: serde_json::from_value(serde_json::Value::String(status))
                .map_err(|_| payload_error())?,
            created_at: created.parse()?,
            resolved_at: resolved.map(|time| time.parse()).transpose()?,
        };
        group.validate()?;
        groups.push(ConflictGroupProjection {
            group,
            subject_node_id: subject.parse()?,
        });
    }
    Ok(groups)
}
