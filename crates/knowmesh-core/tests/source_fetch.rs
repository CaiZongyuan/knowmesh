use std::{
    net::IpAddr,
    sync::atomic::{AtomicUsize, Ordering},
};

use knowmesh_core::{
    application::source_fetch::{self, FetchRequest},
    canonical::{
        source::{ImportInput, ImportedContent},
        workspace::{InitOptions, Workspace, initialize},
    },
    error::AppResult,
    ports::SourceFetcher,
};

struct FakeFetcher {
    calls: AtomicUsize,
}

impl SourceFetcher for FakeFetcher {
    fn fetch(&self, request: &FetchRequest) -> AppResult<ImportedContent> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(request.max_bytes, 100 * 1024 * 1024);
        assert_eq!(request.url, "https://example.com/paper");
        assert!(!request.allow_private_network);
        Ok(ImportedContent {
            bytes: b"Fetched text".to_vec(),
            mime_type: "text/plain".into(),
            final_url: request.url.clone(),
        })
    }
}

#[test]
fn fetch_planning_validates_workspace_policy_before_network_and_leaves_canonical_files_untouched() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let mut workspace = Workspace::load(temp.path()).unwrap();
    let fetcher = FakeFetcher {
        calls: AtomicUsize::new(0),
    };
    let input = ImportInput {
        path: "https://example.com/paper#section".into(),
        source_id: None,
        storage: None,
        encoding: None,
        title: None,
        kind: "paper".into(),
        tags: vec![],
        dry_run: true,
    };
    let imported = source_fetch::fetch(&workspace, &input, false, &fetcher)
        .unwrap()
        .unwrap();
    assert_eq!(imported.bytes, b"Fetched text");
    assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
    assert!(!workspace.index_path().unwrap().exists());
    assert_eq!(
        std::fs::read_dir(temp.path().join("sources"))
            .unwrap()
            .count(),
        0
    );
    workspace.config.sources.allow_remote_urls = false;
    assert_eq!(
        source_fetch::fetch(&workspace, &input, false, &fetcher)
            .err()
            .unwrap()
            .code,
        "REMOTE_URL_DISABLED"
    );
    assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn public_address_policy_blocks_local_metadata_and_transition_ranges_including_mapped_ipv4() {
    for address in [
        "127.0.0.1",
        "0.0.0.0",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.169.254",
        "100.100.100.200",
        "198.18.0.1",
        "192.0.2.1",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
        "64:ff9b::a00:1",
        "2002:a00:1::1",
    ] {
        let ip: IpAddr = address.parse().unwrap();
        assert_eq!(
            source_fetch::validate_address(ip, false).unwrap_err().code,
            "PRIVATE_NETWORK_BLOCKED",
            "{address}"
        );
        source_fetch::validate_address(ip, true).unwrap();
    }
    for address in [
        "8.8.8.8",
        "1.1.1.1",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
    ] {
        source_fetch::validate_address(address.parse().unwrap(), false).unwrap();
    }
    for url in [
        "http://127.1/",
        "http://2130706433/",
        "http://[::ffff:127.0.0.1]/",
    ] {
        assert_eq!(
            source_fetch::validate_url(url, false).unwrap_err().code,
            "PRIVATE_NETWORK_BLOCKED"
        );
    }
    for url in [
        "file:///etc/hosts",
        "ftp://example.com/file",
        "https://user:secret@example.com/",
    ] {
        assert_eq!(
            source_fetch::validate_url(url, true).unwrap_err().code,
            "INVALID_SOURCE_URL"
        );
    }
}
