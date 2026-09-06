use std::fs;

use knowmesh_core::{
    canonical::workspace::{InitOptions, Workspace, initialize},
    domain::{SourceRevision, SourceRevisionId, sha256},
    ingest::{
        TextParser,
        cache::{FileStageCache, ModelIdentity, StageKey},
        chunking::{ChunkOptions, CounterDescriptor},
    },
    ports::SourceParser,
};
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Workspace, FileStageCache, StageKey) {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    let cache = FileStageCache::new(&workspace, 1024 * 1024).unwrap();
    let revision = SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.txt".into(),
        mime_type: "text/plain".into(),
        encoding: None,
        sha256: sha256(b"source"),
        byte_size: 6,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    };
    let key = StageKey::parse(
        &revision,
        TextParser::default().descriptor("text/plain").unwrap(),
    );
    (temp, workspace, cache, key)
}

#[test]
fn cache_hits_require_complete_matching_artifacts_and_reads_never_create_directories() {
    let (temp, workspace, cache, key) = setup();
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
    assert!(!workspace.root.join(".knowmesh/cache").exists());
    let value = json!({"result": ["one", "two"]});
    let reference = cache.store(&key, &value).unwrap();
    assert_eq!(
        cache
            .load::<Value>(&key, |_| Ok(()))
            .unwrap()
            .unwrap()
            .value,
        value
    );
    assert_eq!(
        cache
            .read_reference::<Value>(&reference, |_| Ok(()))
            .unwrap()
            .unwrap(),
        value
    );
    let artifact = workspace.root.join(".knowmesh").join(&reference.path);
    fs::write(&artifact, b"corrupt").unwrap();
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
    cache.store(&key, &value).unwrap();
    fs::remove_file(artifact).unwrap();
    assert!(
        cache
            .read_reference::<Value>(&reference, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    let artifact = workspace.root.join(".knowmesh").join(&reference.path);
    fs::create_dir(&artifact).unwrap();
    assert!(
        cache
            .read_reference::<Value>(&reference, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
    assert!(temp.path().join("knowmesh.yaml").is_file());
}

#[test]
fn invalid_or_future_manifests_and_typed_payloads_are_misses() {
    let (_temp, workspace, cache, key) = setup();
    cache.store(&key, &json!({"number": 1})).unwrap();
    let manifest_path = cache.manifest_path(&key).unwrap();
    let original = fs::read(&manifest_path).unwrap();
    let mut manifest: Value = serde_json::from_slice(&original).unwrap();
    manifest["version"] = 999.into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
    fs::write(&manifest_path, &original).unwrap();
    assert!(
        cache
            .load::<Vec<String>>(&key, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    assert!(
        cache
            .load::<Value>(&key, |_| Err(knowmesh_core::error::AppError::new(
                knowmesh_core::error::ErrorType::Validation,
                "INVALID_FIXTURE",
                "Invalid."
            )))
            .unwrap()
            .is_none()
    );
    manifest = serde_json::from_slice(&original).unwrap();
    manifest["artifact"]["path"] = "../knowmesh.yaml".into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
    assert!(workspace.root.join("knowmesh.yaml").is_file());
    fs::remove_file(&manifest_path).unwrap();
    fs::create_dir(&manifest_path).unwrap();
    assert!(cache.load::<Value>(&key, |_| Ok(())).unwrap().is_none());
}

#[test]
fn a_failed_replacement_preserves_the_previous_manifest_and_checkpoint_reference() {
    let (_temp, workspace, cache, key) = setup();
    let first = cache.store(&key, &json!({"original": true})).unwrap();
    let before = fs::read(cache.manifest_path(&key).unwrap()).unwrap();
    let limited = FileStageCache::new(&workspace, 8).unwrap();
    assert_eq!(
        limited
            .store(&key, &json!({"too_large": "x".repeat(128)}))
            .unwrap_err()
            .code,
        "CACHE_ARTIFACT_TOO_LARGE"
    );
    assert_eq!(
        fs::read(cache.manifest_path(&key).unwrap()).unwrap(),
        before
    );
    cache.store(&key, &json!({"replacement": true})).unwrap();
    assert_eq!(
        cache
            .read_reference::<Value>(&first, |_| Ok(()))
            .unwrap()
            .unwrap(),
        json!({"original": true})
    );
    let before = fs::read(cache.manifest_path(&key).unwrap()).unwrap();
    let value = json!({"new": "cannot publish"});
    let hash = sha256(&serde_json::to_vec(&value).unwrap());
    let collision = workspace
        .root
        .join(".knowmesh")
        .join(first.path.replace(&first.sha256, &hash));
    fs::create_dir(&collision).unwrap();
    assert_eq!(
        cache.store(&key, &value).unwrap_err().code,
        "CACHE_WRITE_FAILED"
    );
    assert_eq!(
        fs::read(cache.manifest_path(&key).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        cache
            .load::<Value>(&key, |_| Ok(()))
            .unwrap()
            .unwrap()
            .value,
        json!({"replacement": true})
    );
}

#[test]
fn stage_keys_bind_all_semantic_inputs_and_embedding_reuse_ignores_chunk_positions() {
    let (_temp, _, _, parse) = setup();
    let model = ModelIdentity {
        provider: "fixture".into(),
        model: "model-a".into(),
        config_sha256: sha256(b"profile"),
    };
    let embedding = StageKey::Embedding {
        input_sha256: sha256(b"actual text with title"),
        model: model.clone(),
        dimensions: 3,
        preprocessing_version: "1".into(),
    };
    assert_eq!(
        embedding.fingerprint().unwrap(),
        embedding.clone().fingerprint().unwrap()
    );
    let changed = StageKey::Embedding {
        input_sha256: sha256(b"different title and text"),
        model: model.clone(),
        dimensions: 3,
        preprocessing_version: "1".into(),
    };
    assert_ne!(
        embedding.fingerprint().unwrap(),
        changed.fingerprint().unwrap()
    );
    let mut profile = model.clone();
    profile.model = "model-b".into();
    let changed = StageKey::Embedding {
        input_sha256: sha256(b"actual text with title"),
        model: profile,
        dimensions: 3,
        preprocessing_version: "1".into(),
    };
    assert_ne!(
        embedding.fingerprint().unwrap(),
        changed.fingerprint().unwrap()
    );
    let chunk = StageKey::Chunk {
        parsed_sha256: sha256(b"parsed"),
        chunker_version: "1".into(),
        options: ChunkOptions::default(),
        counter: CounterDescriptor {
            name: "fixture".into(),
            version: "1".into(),
            config_sha256: sha256(b"tokenizer"),
        },
    };
    let extract = StageKey::CandidateExtract {
        revision_id: SourceRevisionId::new(),
        input_sha256: sha256(b"blocks"),
        prompt_sha256: sha256(b"prompt"),
        schema_sha256: sha256(b"schema"),
        purpose_sha256: Some(sha256(b"purpose")),
        model: model.clone(),
        sampling_sha256: sha256(b"sampling"),
    };
    let knowledge = StageKey::Knowledge {
        operation: "entity-resolution".into(),
        candidate_sha256: sha256(b"candidate"),
        query_sha256: sha256(b"query and filters"),
        generation: 2,
        context_sha256: sha256(b"knowledge context"),
        rules_version: "1".into(),
        model: Some(model),
    };
    let keys = [parse, embedding, chunk, extract, knowledge];
    let fingerprints: std::collections::BTreeSet<_> =
        keys.iter().map(|key| key.fingerprint().unwrap()).collect();
    assert_eq!(fingerprints.len(), keys.len());
    let json = serde_json::to_string(&keys).unwrap();
    assert!(!json.contains("api_key"));
    assert!(!json.contains("ordinal"));
    assert!(!json.contains("locator"));
    for (index, field) in [
        (3, "prompt_sha256"),
        (3, "schema_sha256"),
        (3, "purpose_sha256"),
        (4, "context_sha256"),
    ] {
        let mut modified = serde_json::to_value(&keys[index]).unwrap();
        modified[field] = sha256(b"changed dependency").into();
        let changed: StageKey = serde_json::from_value(modified).unwrap();
        assert_ne!(
            keys[index].fingerprint().unwrap(),
            changed.fingerprint().unwrap()
        );
    }
}

#[test]
fn concurrent_writers_publish_complete_manifests_and_keep_every_checkpoint_artifact() {
    let (_temp, _workspace, cache, key) = setup();
    let references = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..8)
            .map(|number| {
                let cache = &cache;
                let key = &key;
                scope.spawn(move || {
                    (
                        number,
                        cache.store(key, &json!({"worker": number})).unwrap(),
                    )
                })
            })
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect::<Vec<_>>()
    });
    for (number, reference) in references {
        assert_eq!(
            cache
                .read_reference::<Value>(&reference, |_| Ok(()))
                .unwrap()
                .unwrap(),
            json!({"worker": number})
        );
    }
    let value = cache
        .load::<Value>(&key, |_| Ok(()))
        .unwrap()
        .unwrap()
        .value;
    assert!(value["worker"].as_u64().unwrap() < 8);
}

#[cfg(unix)]
#[test]
fn cache_writes_reject_symlinked_runtime_paths() {
    let (temp, workspace, cache, key) = setup();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(workspace.root.join(".knowmesh")).unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join(".knowmesh/cache")).unwrap();
    assert!(cache.store(&key, &json!({"value": true})).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
