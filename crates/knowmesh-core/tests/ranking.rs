use knowmesh_core::{
    application::{
        lexical::{LexicalHit, RecordType},
        search::ranking::{Channel, ChannelInput, RankingConfig, fuse},
    },
    error::ErrorType,
};

fn hit(id: &str, title: &str, rank: u32) -> LexicalHit {
    LexicalHit {
        unit_id: format!("node:{id}"),
        record_type: RecordType::Node,
        record_id: id.into(),
        title: title.into(),
        aliases: vec![],
        rank,
        bm25: None,
    }
}

fn channel(channel: Channel, hits: Vec<LexicalHit>) -> ChannelInput {
    ChannelInput {
        channel,
        hits,
        unavailable_reason: None,
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn fusion_uses_the_theoretical_bound_and_preserves_a_no_boost_baseline() {
    let channels = vec![
        channel(
            Channel::Word,
            vec![hit("a", "Alpha", 1), hit("b", "Other", 2)],
        ),
        channel(Channel::Trigram, vec![hit("a", "Alpha", 1)]),
        channel(Channel::Vector, vec![hit("b", "Other", 1)]),
    ];
    let ranked = fuse("alpha", &RankingConfig::default(), &channels, &[]).unwrap();
    let a = &ranked.hits[0];
    assert_eq!(a.candidate.record_id, "a");
    near(a.explain.raw_rrf, 1.8 / 61.0);
    near(a.explain.normalization_bound, 2.8 / 61.0);
    near(a.explain.normalized_score, 1.8 / 2.8);
    near(a.explain.boosts.exact_name, 0.05);
    near(a.explain.boosts.title_prefix, 0.02);
    near(a.explain.final_score, 1.8 / 2.8 + 0.07);
    assert_eq!(a.explain.channels.len(), 2);

    let config = RankingConfig {
        boosts_enabled: false,
        ..Default::default()
    };
    let baseline = fuse("alpha", &config, &channels, &[]).unwrap();
    let baseline_a = baseline
        .hits
        .iter()
        .find(|item| item.candidate.record_id == "a")
        .unwrap();
    near(baseline_a.explain.final_score, a.explain.normalized_score);
    near(baseline.hits[0].explain.boosts.total, 0.0);
}

#[test]
fn empty_successful_channels_count_in_the_bound_but_unavailable_channels_do_not() {
    let mut channels = vec![
        channel(Channel::Word, vec![hit("a", "Other", 1)]),
        channel(Channel::Trigram, vec![]),
        ChannelInput {
            channel: Channel::Vector,
            hits: vec![],
            unavailable_reason: Some("VECTOR_UNAVAILABLE".into()),
        },
    ];
    let ranked = fuse("query", &RankingConfig::default(), &channels, &[]).unwrap();
    near(ranked.hits[0].explain.normalization_bound, 1.8 / 61.0);
    near(ranked.hits[0].explain.final_score, 1.0 / 1.8);
    assert_eq!(ranked.channels.len(), 3);
    assert_eq!(
        ranked.channels[2].unavailable_reason.as_deref(),
        Some("VECTOR_UNAVAILABLE")
    );
    channels[1].unavailable_reason = Some("TRIGRAM_FAILED".into());
    let single = fuse("query", &RankingConfig::default(), &channels, &[]).unwrap();
    near(single.hits[0].explain.final_score, 1.0);
    assert!(
        fuse("query", &RankingConfig::default(), &[], &[])
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn boosts_are_bounded_and_exact_id_uses_a_separate_sort_tier() {
    let mut alias = hit("a", "Target", 100);
    alias.aliases = vec!["TARGET".into()];
    let channels = vec![channel(Channel::Word, vec![hit("b", "Other", 1), alias])];
    let ranked = fuse("target", &RankingConfig::default(), &channels, &[]).unwrap();
    let alias = ranked
        .hits
        .iter()
        .find(|item| item.candidate.record_id == "a")
        .unwrap();
    near(alias.explain.boosts.exact_alias, 0.04);
    near(alias.explain.boosts.total, 0.08);
    assert_eq!(ranked.hits[0].candidate.record_id, "b");

    let direct = hit("target", "Unrelated title", 1);
    let ranked = fuse("target", &RankingConfig::default(), &channels, &[direct]).unwrap();
    assert_eq!(ranked.hits[0].candidate.record_id, "target");
    assert!(ranked.hits[0].explain.exact_id_tier);
    near(ranked.hits[0].explain.raw_rrf, 0.0);
    near(ranked.hits[0].explain.final_score, 0.0);
}

#[test]
fn channel_order_duplicates_and_candidate_tail_do_not_change_existing_scores() {
    let a = hit("a", "Other", 1);
    let b = hit("b", "Other", 2);
    let first = channel(Channel::Word, vec![a.clone(), a.clone(), b.clone()]);
    let second = channel(Channel::Trigram, vec![a.clone(), b.clone()]);
    let ranked = fuse(
        "query",
        &RankingConfig::default(),
        &[first.clone(), second.clone()],
        &[],
    )
    .unwrap();
    let reversed = fuse("query", &RankingConfig::default(), &[second, first], &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&ranked).unwrap(),
        serde_json::to_value(&reversed).unwrap()
    );
    near(ranked.hits[0].explain.normalized_score, 1.0);
    assert_eq!(ranked.hits.len(), 2);
    let shorter = fuse(
        "query",
        &RankingConfig::default(),
        &[
            channel(Channel::Word, vec![a.clone()]),
            channel(Channel::Trigram, vec![a]),
        ],
        &[],
    )
    .unwrap();
    near(
        shorter.hits[0].explain.final_score,
        ranked.hits[0].explain.final_score,
    );

    let tied = fuse(
        "query",
        &RankingConfig::default(),
        &[channel(
            Channel::Word,
            vec![hit("b", "Other", 1), hit("a", "Other", 1)],
        )],
        &[],
    )
    .unwrap();
    assert_eq!(tied.hits[0].candidate.record_id, "a");
}

#[test]
fn invalid_weights_ranks_and_duplicate_channels_fail_explicitly() {
    for weight in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let config = RankingConfig {
            word_weight: weight,
            ..Default::default()
        };
        assert_eq!(
            fuse("query", &config, &[], &[]).unwrap_err().error_type,
            ErrorType::Validation
        );
    }
    let zero = RankingConfig {
        k: 0,
        ..Default::default()
    };
    assert_eq!(
        fuse("query", &zero, &[], &[]).unwrap_err().code,
        "INVALID_RANKING_CONFIG"
    );
    let input = channel(Channel::Word, vec![hit("a", "Title", 0)]);
    assert_eq!(
        fuse("query", &RankingConfig::default(), &[input], &[])
            .unwrap_err()
            .code,
        "INVALID_CHANNEL_CANDIDATES"
    );
    let input = channel(Channel::Word, vec![]);
    assert_eq!(
        fuse(
            "query",
            &RankingConfig::default(),
            &[input.clone(), input],
            &[]
        )
        .unwrap_err()
        .code,
        "INVALID_CHANNEL_CANDIDATES"
    );
}
