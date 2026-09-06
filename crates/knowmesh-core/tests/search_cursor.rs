use knowmesh_core::{
    application::{
        lexical::{LexicalHit, RecordType},
        search::{
            pagination::{PageContext, PageInput, paginate},
            ranking::{Channel, ChannelInput, RankingConfig, RankingResult, fuse},
        },
    },
    domain::{WorkspaceId, sha256},
};

fn context() -> PageContext {
    PageContext {
        workspace_id: WorkspaceId::new(),
        query_sha256: sha256(b"query and filters"),
        generation: 3,
        snapshot_sha256: sha256(b"canonical snapshot"),
        ranking: RankingConfig::default(),
        candidate_limit: 100,
    }
}

fn results(config: &RankingConfig, vector_available: bool) -> RankingResult {
    let hits = (0..5)
        .map(|index| LexicalHit {
            unit_id: format!("node:fixture-{index}"),
            record_type: RecordType::Node,
            record_id: format!("fixture-{index}"),
            title: "Other".into(),
            aliases: vec![],
            preview: String::new(),
            rank: index + 1,
            bm25: None,
        })
        .collect();
    fuse(
        "query",
        config,
        &[
            ChannelInput {
                channel: Channel::Word,
                hits,
                unavailable_reason: None,
            },
            ChannelInput {
                channel: Channel::Vector,
                hits: vec![],
                unavailable_reason: (!vector_available).then(|| "VECTOR_UNAVAILABLE".into()),
            },
        ],
        &[],
    )
    .unwrap()
}

#[test]
fn pages_follow_the_same_ranked_snapshot_with_variable_page_sizes_and_no_duplicates() {
    let context = context();
    let ranked = results(&context.ranking, false);
    let mut input = PageInput {
        limit: 2,
        cursor: None,
    };
    let mut ids = Vec::new();
    loop {
        let page = paginate(&ranked, &context, &input).unwrap();
        ids.extend(page.hits.iter().map(|hit| hit.candidate.unit_id.clone()));
        let Some(cursor) = page.next_cursor else {
            break;
        };
        input.cursor = Some(cursor);
        input.limit = 1;
    }
    assert_eq!(
        ids,
        ranked
            .hits
            .iter()
            .map(|hit| hit.candidate.unit_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(ids.len(), 5);
}

#[test]
fn another_workspace_query_or_filter_cannot_reuse_a_cursor() {
    let original = context();
    let ranked = results(&original.ranking, false);
    let cursor = paginate(
        &ranked,
        &original,
        &PageInput {
            limit: 1,
            cursor: None,
        },
    )
    .unwrap()
    .next_cursor;
    let input = PageInput { limit: 1, cursor };
    let mut changed = original.clone();
    changed.query_sha256 = sha256(b"changed filter");
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_QUERY_MISMATCH"
    );
    changed = original.clone();
    changed.workspace_id = WorkspaceId::new();
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_QUERY_MISMATCH"
    );
}

#[test]
fn generation_ranking_channels_and_candidate_changes_expire_a_cursor() {
    let original = context();
    let ranked = results(&original.ranking, false);
    let cursor = paginate(
        &ranked,
        &original,
        &PageInput {
            limit: 1,
            cursor: None,
        },
    )
    .unwrap()
    .next_cursor;
    let input = PageInput { limit: 1, cursor };
    let mut changed = original.clone();
    changed.generation += 1;
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
    changed = original.clone();
    changed.snapshot_sha256 = sha256(b"replacement snapshot");
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
    changed = original.clone();
    changed.ranking.boosts_enabled = false;
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
    changed = original.clone();
    changed.candidate_limit = 200;
    assert_eq!(
        paginate(&ranked, &changed, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
    let degraded = results(&original.ranking, true);
    assert_eq!(
        paginate(&degraded, &original, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
    let mut truncated = results(&original.ranking, false);
    truncated.hits.pop();
    assert_eq!(
        paginate(&truncated, &original, &input).unwrap_err().code,
        "CURSOR_STALE"
    );
}

#[test]
fn cursors_and_page_limits_are_bounded_and_invalid_positions_fail_explicitly() {
    let context = context();
    let ranked = results(&context.ranking, false);
    for limit in [0, 101] {
        assert_eq!(
            paginate(
                &ranked,
                &context,
                &PageInput {
                    limit,
                    cursor: None
                }
            )
            .unwrap_err()
            .code,
            "INVALID_PAGE_LIMIT"
        );
    }
    for cursor in ["not a cursor".into(), "a".repeat(4097)] {
        assert_eq!(
            paginate(
                &ranked,
                &context,
                &PageInput {
                    limit: 1,
                    cursor: Some(cursor)
                }
            )
            .unwrap_err()
            .code,
            "INVALID_CURSOR"
        );
    }
    let cursor = paginate(
        &ranked,
        &context,
        &PageInput {
            limit: 1,
            cursor: None,
        },
    )
    .unwrap()
    .next_cursor
    .unwrap();
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut decoded: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cursor).unwrap()).unwrap();
    assert!(decoded.get("offset").is_none());
    decoded["position"]["unit_id"] = "absent".into();
    let cursor = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&decoded).unwrap());
    assert_eq!(
        paginate(
            &ranked,
            &context,
            &PageInput {
                limit: 1,
                cursor: Some(cursor)
            }
        )
        .unwrap_err()
        .code,
        "INVALID_CURSOR"
    );
}
