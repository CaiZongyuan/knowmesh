use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound::{Excluded, Included, Unbounded},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use super::{ClaimComparisonContext, ClaimPair, PairPage, PairSelection, error, hash};
use crate::{domain::ClaimId, error::AppResult};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    query_sha256: String,
    after: ClaimPair,
}

impl ClaimComparisonContext<'_> {
    pub fn select_pairs(&self, input: &PairSelection) -> AppResult<PairPage> {
        if !(1..=32).contains(&input.limit) || input.focus_ids.len() > 10_000 {
            return Err(error(
                "INVALID_CLAIM_SELECTION",
                "Select at most 10000 focus Claims and a page size in 1..=32.",
            ));
        }
        let focus: BTreeSet<_> = input.focus_ids.iter().cloned().collect();
        let mut focus_scopes = BTreeMap::<String, BTreeSet<ClaimId>>::new();
        for id in &focus {
            let scope = self.scope_keys.get(id).ok_or_else(|| {
                error(
                    "CLAIM_COMPARISON_SCOPE_MISMATCH",
                    "Every focus Claim must be present and active.",
                )
            })?;
            focus_scopes
                .entry(scope.clone())
                .or_default()
                .insert(id.clone());
        }
        let query_sha256 = hash(&(&self.context_sha256, &focus))?;
        let after = input
            .cursor
            .as_ref()
            .map(|cursor| {
                if cursor.len() > 4096 {
                    return Err(cursor_error());
                }
                let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| cursor_error())?;
                let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| cursor_error())?;
                if cursor.version != 1
                    || cursor.query_sha256 != query_sha256
                    || cursor.after.left_id >= cursor.after.right_id
                    || (!focus.contains(&cursor.after.left_id)
                        && !focus.contains(&cursor.after.right_id))
                    || self.validate_pair(&cursor.after).is_err()
                {
                    return Err(cursor_error());
                }
                Ok(cursor.after)
            })
            .transpose()?;
        let start = after
            .as_ref()
            .map_or(Unbounded, |pair| Included(&pair.left_id));
        let mut pairs = vec![];
        'claims: for (left, scope) in self.scope_keys.range((start, Unbounded)) {
            let Some(focused) = focus_scopes.get(scope) else {
                continue;
            };
            let candidates = if focus.contains(left) {
                &self.scopes[scope]
            } else {
                focused
            };
            let lower = after
                .as_ref()
                .filter(|pair| &pair.left_id == left)
                .map_or(left, |pair| &pair.right_id);
            for right in candidates.range((Excluded(lower), Unbounded)) {
                let pair = ClaimPair {
                    left_id: left.clone(),
                    right_id: right.clone(),
                };
                if self.normalized_keys[left] == self.normalized_keys[right] {
                    continue;
                }
                pairs.push(pair);
                if pairs.len() > input.limit {
                    break 'claims;
                }
            }
        }
        let next_cursor = if pairs.len() > input.limit {
            pairs.truncate(input.limit);
            let cursor = Cursor {
                version: 1,
                query_sha256,
                after: pairs.last().expect("positive page size").clone(),
            };
            Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(|_| cursor_error())?))
        } else {
            None
        };
        Ok(PairPage { pairs, next_cursor })
    }
}

fn cursor_error() -> crate::error::AppError {
    error(
        "CLAIM_COMPARISON_CURSOR_MISMATCH",
        "The comparison cursor does not match the current context, focus, or pair position.",
    )
}
