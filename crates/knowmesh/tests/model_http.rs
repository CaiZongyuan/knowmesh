use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use knowmesh::model_provider::{OpenAiCompatible, TransportOptions};
use knowmesh_core::{
    canonical::workspace::CompilerSettings,
    model::{Message, MessageRole, ModelRequest, ResponseFormat, StopReason},
    ports::ModelProvider,
};
use serde_json::{Value, json};

fn request() -> ModelRequest {
    ModelRequest {
        messages: vec![Message {
            role: MessageRole::System,
            content: "Return JSON.".into(),
        }],
        output_schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
        schema_name: "result".into(),
        max_output_tokens: 64,
        temperature: Some(0.0),
        timeout_ms: 1000,
    }
}

fn start_server(
    status: u16,
    headers: &str,
    body: String,
    delay: Duration,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(vec![]));
    let copy = captured.clone();
    let headers = headers.to_owned();
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(&mut socket);
        let mut lines = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 { return; }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().unwrap();
            }
            lines.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).unwrap();
        drop(reader);
        copy.lock()
            .unwrap()
            .extend([lines, String::from_utf8(payload).unwrap()]);
        thread::sleep(delay);
        let response = format!(
            "HTTP/1.1 {status} Result\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes());
    });
    (url, captured, handle)
}

fn provider(url: &str, mode: ResponseFormat, limit: u64) -> OpenAiCompatible {
    let mut settings = CompilerSettings::default();
    settings.model = "fixture-model".into();
    settings.base_url = url.into();
    settings.api_key = "${FIXTURE_MODEL_KEY}".into();
    settings.response_format = mode;
    let env = std::collections::BTreeMap::from([(
        "FIXTURE_MODEL_KEY".into(),
        "fixture-sensitive-key".into(),
    )]);
    OpenAiCompatible::new(
        settings.resolve(&env).unwrap(),
        TransportOptions {
            max_response_bytes: limit,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn adapter_uses_configured_model_auth_and_json_modes_and_decodes_usage() {
    for mode in [
        ResponseFormat::JsonObject,
        ResponseFormat::JsonSchema,
        ResponseFormat::SchemaPrompt,
    ] {
        let body = json!({"choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}}).to_string();
        let (url, captured, server) = start_server(200, "", body, Duration::ZERO);
        let provider = provider(&url, mode, 4096);
        assert!(!format!("{provider:?}").contains("fixture-sensitive-key"));
        let response = provider.complete(&request()).unwrap();
        assert_eq!(response.stop_reason, StopReason::Complete);
        assert_eq!(response.usage.unwrap().input_tokens, 8);
        server.join().unwrap();
        let captured = captured.lock().unwrap();
        assert!(captured[0].starts_with("POST /v1/chat/completions "));
        assert!(
            captured[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-sensitive-key")
        );
        let body: Value = serde_json::from_str(&captured[1]).unwrap();
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["max_tokens"], 64);
        match mode {
            ResponseFormat::JsonObject => {
                assert_eq!(body["response_format"]["type"], "json_object")
            }
            ResponseFormat::JsonSchema => {
                assert_eq!(body["response_format"]["json_schema"]["strict"], true)
            }
            ResponseFormat::SchemaPrompt => assert!(body.get("response_format").is_none()),
        }
    }
}

#[test]
fn http_errors_keep_safe_codes_retry_metadata_and_never_echo_remote_error_bodies() {
    for (status, code, retryable) in [
        (401, "MODEL_AUTH_FAILED", false),
        (429, "MODEL_RATE_LIMIT", true),
        (503, "MODEL_UNAVAILABLE", true),
        (302, "MODEL_REQUEST_REJECTED", false),
    ] {
        let (url, _, server) = start_server(
            status,
            "Retry-After: 2\r\nLocation: http://127.0.0.1:1/redirect\r\n",
            "fixture-sensitive-key private prompt".into(),
            Duration::ZERO,
        );
        let error = provider(&url, ResponseFormat::JsonObject, 4096)
            .complete(&request())
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("fixture-sensitive-key")
        );
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private prompt")
        );
        if status == 429 {
            assert_eq!(error.details.as_ref().unwrap()["retry_after_ms"], 2000);
        }
        server.join().unwrap();
    }
}

#[test]
fn oversized_timed_out_and_nonterminal_responses_are_not_successful_json() {
    let (url, _, server) = start_server(200, "", "x".repeat(4096), Duration::ZERO);
    assert_eq!(
        provider(&url, ResponseFormat::JsonObject, 100)
            .complete(&request())
            .unwrap_err()
            .code,
        "MODEL_RESPONSE_TOO_LARGE"
    );
    server.join().unwrap();
    let (url, _, server) = start_server(200, "", "{}".into(), Duration::from_millis(80));
    let mut input = request();
    input.timeout_ms = 10;
    assert_eq!(
        provider(&url, ResponseFormat::JsonObject, 4096)
            .complete(&input)
            .unwrap_err()
            .code,
        "MODEL_TIMEOUT"
    );
    server.join().unwrap();
    for (finish, reason) in [
        ("length", StopReason::Length),
        ("tool_calls", StopReason::ToolCall),
        ("content_filter", StopReason::ContentFilter),
    ] {
        let body =
            json!({"choices":[{"message":{"content":null},"finish_reason":finish}]}).to_string();
        let (url, _, server) = start_server(200, "", body, Duration::ZERO);
        assert_eq!(
            provider(&url, ResponseFormat::JsonObject, 4096)
                .complete(&request())
                .unwrap()
                .stop_reason,
            reason
        );
        server.join().unwrap();
    }
}
