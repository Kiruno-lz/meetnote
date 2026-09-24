//! L2 contract for `meetnote agent host probe`.
//!
//! A local HTTP stub stands in for the provider so the Host probe path is
//! exercised end to end: settings → shared credential resolution →
//! `ConnectionConfig` → `AgentHost::provider_probe` → protocol adapter → HTTP.

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);
const SECRET: &str = "PROBE_SECRET_SHOULD_NOT_ECHO";

struct CapturedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Value,
}

/// Serve `responses` in order, one connection each, and report every request.
fn spawn_stub(responses: Vec<(u16, Value)>) -> (String, Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let request = read_request(&mut stream);
            let _ = sender.send(request);
            let body = body.to_string();
            let reason = if status == 200 { "OK" } else { "Error" };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (format!("http://{address}"), receiver)
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let mut authorization = None;
    let mut content_length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "authorization" => authorization = Some(value.trim().to_owned()),
                "content-length" => content_length = value.trim().parse().unwrap(),
                _ => {}
            }
        }
    }
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&bytes[header_end..header_end + content_length])
        .unwrap_or(Value::Null);
    CapturedRequest {
        method,
        path,
        authorization,
        body,
    }
}

fn fixture(base_url: &str, api_key: Option<&str>, api_key_file: Option<&str>) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "meetnote-host-probe-{}-{}",
        std::process::id(),
        FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("data/documents")).unwrap();
    fs::create_dir_all(root.join("data/settings")).unwrap();
    let mut settings = settings::AppSettings::default();
    settings.assistant.active_connection_id = Some("probe".into());
    settings
        .assistant
        .connections
        .push(settings::AssistantConnectionProfile {
            id: "probe".into(),
            name: "Probe".into(),
            protocol: settings::LlmProtocol::OpenAiChatCompletions,
            base_url: base_url.into(),
            model: "probe-requested".into(),
            auth_mode: settings::AssistantAuthMode::Bearer,
            api_key: api_key.map(str::to_owned),
            api_key_file: api_key_file.map(str::to_owned),
            context_window_tokens: Some(8_192),
            timeout_ms: 5_000,
            max_output_tokens: 128,
        });
    let path = root.join("data/settings/settings.json");
    fs::write(&path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    root
}

fn probe(root: &Path) -> (Option<i32>, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .args(["agent", "host", "probe"])
        .current_dir(root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let response = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "probe returned invalid JSON: {error}; stdout={stdout}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        !stdout.contains(SECRET),
        "probe output leaked the credential"
    );
    (output.status.code(), response, stdout)
}

#[test]
fn host_probe_decodes_a_real_success_response_from_the_configured_connection() {
    let (base_url, requests) = spawn_stub(vec![(
        200,
        serde_json::json!({
            "model": "probe-resolved",
            "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "ready"}}],
            "usage": {"prompt_tokens": 9, "completion_tokens": 1, "total_tokens": 10}
        }),
    )]);
    let root = fixture(&base_url, Some(SECRET), None);
    let (code, response, _) = probe(&root);
    assert_eq!(code, Some(0), "{response}");
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["providerId"], "probe");
    assert_eq!(response["requestedModel"], "probe-requested");
    assert_eq!(response["resolvedModel"], "probe-resolved");
    assert_eq!(response["text"], "ready");

    let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.authorization.as_deref(),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert_eq!(request.body["model"], "probe-requested");
    assert!(request.body.get("tools").is_none(), "{}", request.body);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_probe_reports_a_valid_non_200_provider_response_with_a_fixed_code() {
    let (base_url, requests) = spawn_stub(vec![(
        401,
        serde_json::json!({"error": {"message": "invalid api key", "type": "invalid_request_error"}}),
    )]);
    let root = fixture(&base_url, Some(SECRET), None);
    let (code, response, _) = probe(&root);
    assert_eq!(code, Some(2), "{response}");
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["providerId"], "probe");
    assert_eq!(response["error"]["code"], "PROVIDER_AUTHENTICATION_FAILED");
    assert_eq!(response["error"]["retryable"], false);
    assert_eq!(response["error"]["context"]["status"], 401);
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(5)).unwrap().path,
        "/v1/chat/completions"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_probe_rejects_an_unreadable_api_key_file_before_contacting_the_provider() {
    let (base_url, requests) = spawn_stub(vec![(
        200,
        serde_json::json!({
            "model": "must-not-be-called",
            "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "ready"}}]
        }),
    )]);
    let root = fixture(&base_url, None, Some("credentials/missing-probe-key"));
    let (code, response, _) = probe(&root);
    assert_eq!(code, Some(2), "{response}");
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(
        response["error"]["code"], "ASSISTANT_API_KEY_FILE_READ_FAILED",
        "{response}"
    );
    assert_eq!(response["error"]["retryable"], false);
    assert!(
        requests.recv_timeout(Duration::from_millis(500)).is_err(),
        "probe contacted the provider without the configured credential"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_probe_reads_the_api_key_file_that_settings_validates() {
    let (base_url, requests) = spawn_stub(vec![(
        200,
        serde_json::json!({
            "model": "probe-resolved",
            "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "ready"}}]
        }),
    )]);
    let root = fixture(&base_url, None, Some("credentials/probe-key"));
    let validated = root.join("data/settings/credentials/probe-key");
    fs::create_dir_all(validated.parent().unwrap()).unwrap();
    fs::write(&validated, format!("{SECRET}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&validated, fs::Permissions::from_mode(0o600)).unwrap();
    }
    // The same reference resolved against the working directory is a file
    // Settings never permission-checks; it must not be read.
    fs::create_dir_all(root.join("credentials")).unwrap();
    fs::write(root.join("credentials/probe-key"), "DECOY_UNCHECKED_KEY").unwrap();

    let (code, response, stdout) = probe(&root);
    assert_eq!(code, Some(0), "{response}");
    assert_eq!(response["ok"], true, "{response}");
    assert!(!stdout.contains("DECOY_UNCHECKED_KEY"));
    let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        request.authorization.as_deref(),
        Some(format!("Bearer {SECRET}").as_str()),
        "probe must send the key Settings validated"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_probe_refuses_a_symlinked_credentials_directory_before_contacting_the_provider() {
    use std::os::unix::fs::PermissionsExt;
    let (base_url, requests) = spawn_stub(vec![(
        200,
        serde_json::json!({
            "model": "must-not-be-called",
            "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "ready"}}]
        }),
    )]);
    let root = fixture(&base_url, None, Some("credentials/probe-key"));
    // An owner-only key outside the settings directory, reachable only by
    // following a symlinked `credentials` directory.
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("probe-key"), "OUTSIDE_SYMLINK_KEY").unwrap();
    fs::set_permissions(outside.join("probe-key"), fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("data/settings/credentials")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .args(["agent", "host", "probe"])
        .current_dir(&root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_ne!(output.status.code(), Some(0), "stdout={stdout}");
    assert!(!stdout.contains("OUTSIDE_SYMLINK_KEY"));
    let leaked = requests.recv_timeout(Duration::from_millis(500)).ok();
    assert!(
        leaked.is_none(),
        "probe followed the symlinked credentials directory and sent {:?}",
        leaked.and_then(|request| request.authorization)
    );
    fs::remove_dir_all(root).unwrap();
}
