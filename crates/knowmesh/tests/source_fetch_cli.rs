use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0_u8; 8192];
                let n = socket.read(&mut request).unwrap_or(0);
                let path = std::str::from_utf8(&request[..n])
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("");
                let response = match path {
                    "/fake-pdf" => "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 9\r\n\r\nnot a pdf".into(),
                    "/unbounded" => format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}", "a".repeat(1024 * 1024 + 1)),
                    "/truncated" => "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\nshort".into(),
                    "/redirect" => "HTTP/1.1 302 Found\r\nLocation: /paper\r\nContent-Length: 0\r\n\r\n".into(),
                    "/loop" => "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n".into(),
                    "/invalid" => "HTTP/1.1 302 Found\r\nLocation: file:///etc/hosts\r\nContent-Length: 0\r\n\r\n".into(),
                    "/large" => "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 200000000\r\n\r\n".into(),
                    "/missing" => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into(),
                    "/slow" => {
                        thread::sleep(Duration::from_millis(1200));
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\nslow".into()
                    },
                    "/slow-body" => {
                        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\n");
                        thread::sleep(Duration::from_millis(1200));
                        "slow".into()
                    },
                    _ => { let body = "<h1>Fetched paper</h1><p>Synthetic evidence.</p>"; format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}", body.len()) },
                };
                let _ = socket.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn request(root: &Path, url: &str, extra: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .args(["source", "add", url])
        .args(extra)
        .output()
        .unwrap()
}

fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn url_import_rejects_private_targets_by_default_and_snapshots_redirected_content_with_explicit_override()
 {
    let server = Server::start();
    let temp = tempfile::tempdir().unwrap();
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(temp.path())
        .assert()
        .success();
    for url in [
        format!("{}/paper", server.url),
        server.url.replace("127.0.0.1", "localhost"),
    ] {
        let error = request(temp.path(), &url, &[]);
        assert!(error.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"]["code"],
            "PRIVATE_NETWORK_BLOCKED"
        );
    }
    assert!(!temp.path().join(".knowmesh/index.sqlite3").exists());
    let url = format!("{}/redirect", server.url);
    let preview = success(request(
        temp.path(),
        &url,
        &["--allow-private-network", "--dry-run"],
    ));
    assert_eq!(preview["data"]["source"]["storage"], "snapshot-url");
    assert_eq!(
        preview["data"]["revision"]["url"],
        format!("{}/paper", server.url)
    );
    assert!(!temp.path().join(".knowmesh/index.sqlite3").exists());
    assert_eq!(
        fs::read_dir(temp.path().join("sources")).unwrap().count(),
        0
    );
    let added = success(request(temp.path(), &url, &["--allow-private-network"]));
    let source_id = added["data"]["source"]["id"].as_str().unwrap();
    let duplicate = success(request(
        temp.path(),
        &url,
        &["--allow-private-network", "--source-id", source_id],
    ));
    assert_eq!(duplicate["data"]["deduplicated"], true);
    assert_eq!(
        duplicate["data"]["revision"]["id"],
        added["data"]["revision"]["id"]
    );
    drop(server);
    let content = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(temp.path())
        .args(["source", "content", source_id, "--raw"])
        .output()
        .unwrap();
    assert!(content.status.success());
    assert!(content.stdout.starts_with(b"<h1>Fetched paper"));
}

#[test]
fn failed_downloads_do_not_create_snapshots_or_indexes_and_timeouts_are_bounded() {
    let server = Server::start();
    let temp = tempfile::tempdir().unwrap();
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(temp.path())
        .assert()
        .success();
    for (path, code) in [
        ("/loop", "FETCH_REDIRECT_LIMIT"),
        ("/invalid", "INVALID_SOURCE_URL"),
        ("/large", "SOURCE_TOO_LARGE"),
        ("/missing", "FETCH_HTTP_STATUS"),
        ("/truncated", "FETCH_FAILED"),
    ] {
        let error = request(
            temp.path(),
            &format!("{}{path}", server.url),
            &["--allow-private-network"],
        );
        assert!(error.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"]["code"],
            code,
            "{path}"
        );
    }
    let workspace = knowmesh_core::canonical::workspace::Workspace::load(temp.path()).unwrap();
    let mut config = serde_json::to_value(workspace.config).unwrap();
    config["sources"]["fetch_timeout_seconds"] = 1.into();
    config["sources"]["max_file_mib"] = 1.into();
    fs::write(
        temp.path().join("knowmesh.yaml"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    for (path, code) in [
        ("/unbounded", "SOURCE_TOO_LARGE"),
        ("/slow", "FETCH_TIMEOUT"),
        ("/slow-body", "FETCH_TIMEOUT"),
    ] {
        let start = Instant::now();
        let error = request(
            temp.path(),
            &format!("{}{path}", server.url),
            &["--allow-private-network"],
        );
        assert!(error.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"]["code"],
            code,
            "{path}"
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    assert!(!temp.path().join(".knowmesh/index.sqlite3").exists());
    assert_eq!(
        fs::read_dir(temp.path().join("sources")).unwrap().count(),
        0
    );
}

#[test]
fn invalid_downloaded_bytes_are_rejected_before_a_database_is_created() {
    let server = Server::start();
    let temp = tempfile::tempdir().unwrap();
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(temp.path())
        .assert()
        .success();
    let error = request(
        temp.path(),
        &format!("{}/fake-pdf", server.url),
        &["--allow-private-network"],
    );
    assert!(error.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"]["code"],
        "SOURCE_MIME_MISMATCH"
    );
    assert!(!temp.path().join(".knowmesh/index.sqlite3").exists());
    assert_eq!(
        fs::read_dir(temp.path().join("sources")).unwrap().count(),
        0
    );
}
