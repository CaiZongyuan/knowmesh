use knowmesh_core::{
    application::lexical::{LexicalChannel, LexicalQuery, QuerySyntax, RecordType},
    domain::{WorkspaceId, sha256},
    ports::LexicalSearchStore,
};
use knowmesh_sqlite::SqliteStore;
use rusqlite::{Connection, params};

fn fixture() -> (tempfile::TempDir, SqliteStore, Connection) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    let store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&WorkspaceId::new(), &sha256(b"schema"))
        .unwrap();
    let db = Connection::open(path).unwrap();
    for (id, kind, title, aliases, body, status) in [
        (
            "a",
            "node",
            "scGPT",
            "细胞模型\nCell model",
            "用于单细胞扰动预测 TP53",
            "active",
        ),
        (
            "b",
            "source",
            "细胞图谱",
            "",
            "Gene expression profiling",
            "active",
        ),
        ("c", "claim", "", "", "细胞仅出现在正文", "active"),
        (
            "d",
            "node",
            "alpha OR beta",
            "quoted \"name\"",
            "title:literal NEAR(foo) 100%",
            "active",
        ),
        ("e", "node", "alpha beta", "", "decoy", "active"),
        (
            "f",
            "node",
            "scGPT",
            "细胞模型",
            "用于单细胞扰动预测 TP53",
            "archived",
        ),
        ("g", "node", "%_", "", "literal wildcard name", "active"),
    ] {
        db.execute("INSERT INTO search_units(unit_id,record_type,record_id,title,aliases,body,lifecycle_status,content_sha256,updated_at) VALUES (?1,?2,?1,?3,?4,?5,?6,?7,'2026-09-06T00:00:00Z')",
            params![id, kind, title, aliases, body, status, sha256(body.as_bytes())]).unwrap();
    }
    (temp, store, db)
}

fn query(text: &str) -> LexicalQuery {
    LexicalQuery {
        query: text.into(),
        ..Default::default()
    }
}

#[test]
fn mixed_language_queries_recall_words_substrings_and_short_titles_or_aliases() {
    let (_temp, store, _db) = fixture();
    for (text, channel, expected) in [
        ("TP53", LexicalChannel::Word, vec!["a"]),
        ("扰动预测", LexicalChannel::Trigram, vec!["a"]),
        ("细胞", LexicalChannel::ShortText, vec!["a", "b"]),
        ("图", LexicalChannel::ShortText, vec!["b"]),
        ("scGPT 扰动预测", LexicalChannel::Trigram, vec!["a"]),
        ("scGPT 细胞", LexicalChannel::Trigram, vec!["a"]),
    ] {
        let result = store
            .search_lexical(&query(text))
            .unwrap_or_else(|error| panic!("{text}: {error:?}"));
        let hits = &result
            .channels
            .iter()
            .find(|part| part.channel == channel)
            .unwrap()
            .hits;
        let mut actual: Vec<_> = hits.iter().map(|hit| hit.unit_id.as_str()).collect();
        actual.sort();
        assert_eq!(actual, expected, "{text}");
        assert_eq!(result.generation, 0);
        assert!(
            hits.iter()
                .enumerate()
                .all(|(index, hit)| hit.rank as usize == index + 1)
        );
    }
}

#[test]
fn fts_operators_quotes_and_like_wildcards_are_literal_by_default() {
    let (_temp, store, _db) = fixture();
    for text in [
        "alpha OR beta",
        "quoted \"name\"",
        "title:literal",
        "NEAR(foo)",
    ] {
        let result = store
            .search_lexical(&query(text))
            .unwrap_or_else(|error| panic!("{text}: {error:?}"));
        let words = &result
            .channels
            .iter()
            .find(|part| part.channel == LexicalChannel::Word)
            .unwrap()
            .hits;
        assert_eq!(
            words
                .iter()
                .map(|hit| hit.unit_id.as_str())
                .collect::<Vec<_>>(),
            ["d"],
            "{text}"
        );
    }
    let result = store.search_lexical(&query("%_")).unwrap();
    let short = &result
        .channels
        .iter()
        .find(|part| part.channel == LexicalChannel::ShortText)
        .unwrap()
        .hits;
    assert_eq!(
        short
            .iter()
            .map(|hit| hit.unit_id.as_str())
            .collect::<Vec<_>>(),
        ["g"]
    );
    for text in ["\"", "*", "()", "' OR 1=1 --", "\" OR *"] {
        store.search_lexical(&query(text)).unwrap();
    }
}

#[test]
fn filters_precede_channel_limits_and_triggers_remove_obsolete_matches() {
    let (_temp, store, db) = fixture();
    let mut input = query("scGPT");
    input.statuses = vec!["archived".into()];
    input.record_types = vec![RecordType::Node];
    input.candidate_limit = 1;
    let result = store.search_lexical(&input).unwrap();
    for part in result.channels {
        assert_eq!(part.hits.len(), 1);
        assert_eq!(part.hits[0].unit_id, "f");
    }
    input.record_types = vec![RecordType::Source];
    assert!(
        store
            .search_lexical(&input)
            .unwrap()
            .channels
            .iter()
            .all(|part| part.hits.is_empty())
    );
    db.execute(
        "UPDATE search_units SET title='replacement',aliases='',body='changed' WHERE unit_id='a'",
        [],
    )
    .unwrap();
    assert!(
        store
            .search_lexical(&query("scGPT"))
            .unwrap()
            .channels
            .iter()
            .all(|part| part.hits.is_empty())
    );
    assert!(
        store
            .search_lexical(&query("replacement"))
            .unwrap()
            .channels
            .iter()
            .all(|part| part.hits.len() == 1)
    );
    db.execute("DELETE FROM search_units WHERE unit_id='a'", [])
        .unwrap();
    assert!(
        store
            .search_lexical(&query("replacement"))
            .unwrap()
            .channels
            .iter()
            .all(|part| part.hits.is_empty())
    );
}

#[test]
fn advanced_fts_is_opt_in_and_invalid_queries_leave_the_connection_usable() {
    let (_temp, store, _db) = fixture();
    let mut input = query("scGPT OR profiling");
    input.query_syntax = QuerySyntax::Advanced;
    let result = store.search_lexical(&input).unwrap();
    assert!(result.channels.iter().any(|part| part.hits.len() == 2));
    input.query = "title:(".into();
    assert_eq!(
        store.search_lexical(&input).unwrap_err().code,
        "INVALID_SEARCH_SYNTAX"
    );
    assert!(
        !store.search_lexical(&query("scGPT")).unwrap().channels[0]
            .hits
            .is_empty()
    );
    for text in [
        "",
        " \n\t",
        "bad\0query",
        &"a".repeat(4097),
        &"a ".repeat(65),
    ] {
        assert_eq!(
            store.search_lexical(&query(text)).unwrap_err().code,
            "INVALID_SEARCH_QUERY"
        );
    }
    let mut input = query("scGPT");
    input.candidate_limit = 501;
    assert_eq!(
        store.search_lexical(&input).unwrap_err().code,
        "INVALID_CANDIDATE_LIMIT"
    );
    input.candidate_limit = 100;
    input.timeout_ms = 0;
    assert_eq!(
        store.search_lexical(&input).unwrap_err().code,
        "INVALID_SEARCH_TIMEOUT"
    );
}
