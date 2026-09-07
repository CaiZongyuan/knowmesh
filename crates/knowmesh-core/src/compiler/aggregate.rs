use std::collections::BTreeMap;

use super::{Candidates, validate};
use crate::error::AppResult;

pub(super) fn append(total: &mut Candidates, mut local: Candidates, ordinal: u32) -> AppResult<()> {
    let mut entities = BTreeMap::new();
    for (index, entity) in local.entities.iter_mut().enumerate() {
        let id = format!("ent_c{ordinal}_{index}");
        entities.insert(std::mem::replace(&mut entity.temp_id, id.clone()), id);
    }
    for (index, claim) in local.claims.iter_mut().enumerate() {
        claim.temp_id = format!("claim_c{ordinal}_{index}");
        claim.subject_ref = entities[&claim.subject_ref].clone();
    }
    for (index, relation) in local.relations.iter_mut().enumerate() {
        relation.temp_id = format!("relation_c{ordinal}_{index}");
        relation.source_ref = entities[&relation.source_ref].clone();
        relation.target_ref = entities[&relation.target_ref].clone();
    }
    total.entities.extend(local.entities);
    total.claims.extend(local.claims);
    total.relations.extend(local.relations);
    total.warnings.extend(local.warnings);
    if total.entities.len() + total.claims.len() + total.relations.len() > 4096
        || total.warnings.len() > 1024
    {
        return Err(validate::rejected("CANDIDATE_LIMIT_EXCEEDED"));
    }
    validate::references(total)
}
