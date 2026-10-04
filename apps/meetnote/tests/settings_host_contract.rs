//! L2 contract for the Settings capability through every retained Host entry.
//!
//! The same scenario runs through the one-shot `agent host call` adapter and the
//! persistent `agent host serve` adapter. Both must reach one Settings port
//! rule set: field grants, projection, redaction, target/credential binding and
//! revision conflicts. Removed legacy Settings CLIs must stay unavailable.

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

const LEAK_MARKERS: [&str; 3] = [
    "SENTINEL",
    "ROTATED_DIRECT_SECRET",
    "NEW_SECRET_SHOULD_NOT_ECHO",
];
const WRITE_POLICY: &str = "assistant.writePolicy";
const CONNECTIONS: &str = "assistant.connections";
const TARGETS: &str = "assistant.connectionTargets";
const CREDENTIALS: &str = "assistant.credentials";

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "meetnote-settings-host-{label}-{}-{}",
        std::process::id(),
        FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("data/documents")).unwrap();
    fs::create_dir_all(root.join("data/settings")).unwrap();
    let path = root.join("data/settings/settings.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&sentinel_settings()).unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    root
}

fn sentinel_settings() -> settings::AppSettings {
    let mut settings = settings::AppSettings::default();
    settings.asr_backend_id = Some("whisper-rs".into());
    settings.asr_model = Some(contracts::ModelRef {
        backend_id: "whisper-rs".into(),
        model_id: "SENTINEL_ASR_MODEL".into(),
        revision: None,
        artifact_path: "models/SENTINEL_ASR_PATH".into(),
        artifacts: Vec::new(),
    });
    settings.diarization_backend_id = Some("diarization-native".into());
    settings.diarization_model = Some(contracts::ModelRef {
        backend_id: "diarization-native".into(),
        model_id: "SENTINEL_DIARIZATION_MODEL".into(),
        revision: None,
        artifact_path: "models/SENTINEL_DIARIZATION_PATH".into(),
        artifacts: Vec::new(),
    });
    settings.language = Some("SENTINEL_LANGUAGE".into());
    settings.speaker_library_path = "data/speakers/SENTINEL_LIBRARY.json".into();
    let connection = |id: &str, api_key: Option<&str>, api_key_file: Option<&str>| {
        settings::AssistantConnectionProfile {
            id: id.into(),
            name: format!("Connection {id}"),
            protocol: settings::LlmProtocol::OpenAiChatCompletions,
            base_url: "http://localhost:1234".into(),
            model: "fixture".into(),
            auth_mode: settings::AssistantAuthMode::Bearer,
            api_key: api_key.map(str::to_owned),
            api_key_file: api_key_file.map(str::to_owned),
            context_window_tokens: Some(8_192),
            timeout_ms: 1_000,
            max_output_tokens: 128,
        }
    };
    settings.assistant.active_connection_id = Some("direct".into());
    settings.assistant.connections = vec![
        connection("direct", Some("SENTINEL_DIRECT_KEY"), None),
        connection("file", None, Some("credentials/SENTINEL_KEY_FILE")),
    ];
    settings
}

fn persisted(root: &Path) -> settings::AppSettings {
    serde_json::from_slice(&fs::read(root.join("data/settings/settings.json")).unwrap()).unwrap()
}

fn connection<'a>(
    settings: &'a settings::AppSettings,
    id: &str,
) -> &'a settings::AssistantConnectionProfile {
    settings
        .assistant
        .connections
        .iter()
        .find(|connection| connection.id == id)
        .unwrap()
}

/// Unrelated configuration must survive every Settings capability write.
fn assert_unrelated_configuration_unchanged(settings: &settings::AppSettings) {
    let initial = sentinel_settings();
    assert_eq!(settings.asr_backend_id, initial.asr_backend_id);
    assert_eq!(settings.asr_model, initial.asr_model);
    assert_eq!(
        settings.diarization_backend_id,
        initial.diarization_backend_id
    );
    assert_eq!(settings.diarization_model, initial.diarization_model);
    assert_eq!(settings.language, initial.language);
    assert_eq!(settings.speaker_library_path, initial.speaker_library_path);
}

trait HostEntry {
    /// Send one wire request and return the parsed response plus raw stdout.
    fn call(&mut self, request: Value) -> (Value, String);
}

struct OneShotEntry {
    root: PathBuf,
    trusted: Vec<String>,
}

impl HostEntry for OneShotEntry {
    fn call(&mut self, request: Value) -> (Value, String) {
        let mut args = vec![
            "agent".to_string(),
            "host".into(),
            "call".into(),
            "--json-stdin".into(),
        ];
        for field in &self.trusted {
            args.extend(["--field".to_string(), field.clone()]);
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_meetnote"))
            .args(&args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        serde_json::to_writer(child.stdin.take().unwrap(), &request).unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let response: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
            panic!(
                "Host call returned invalid JSON: {error}; stdout={stdout}; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
        let expected_exit = if response["ok"] == true { 0 } else { 2 };
        assert_eq!(output.status.code(), Some(expected_exit), "{response}");
        (response, stdout)
    }
}

struct ServeEntry {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ServeEntry {
    fn start(root: &Path, trusted: &[String]) -> Self {
        let mut args = vec!["agent".to_string(), "host".into(), "serve".into()];
        for field in trusted {
            args.extend(["--field".to_string(), field.clone()]);
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_meetnote"))
            .args(&args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
}

impl HostEntry for ServeEntry {
    fn call(&mut self, request: Value) -> (Value, String) {
        writeln!(self.stdin, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("Host serve returned invalid JSON: {error}; {line}"));
        (response, line)
    }
}

impl Drop for ServeEntry {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    HostCall,
    HostServe,
}

fn open_entry(kind: EntryKind, root: &Path, trusted: &[&str]) -> Box<dyn HostEntry> {
    let trusted = trusted
        .iter()
        .map(|field| field.to_string())
        .collect::<Vec<_>>();
    match kind {
        EntryKind::HostCall => Box::new(OneShotEntry {
            root: root.to_path_buf(),
            trusted,
        }),
        EntryKind::HostServe => Box::new(ServeEntry::start(root, &trusted)),
    }
}

/// Records every observable outcome so the two entries can be compared.
struct Scenario {
    index: usize,
    outcomes: Vec<(String, bool, Option<String>)>,
}

impl Scenario {
    fn call(
        &mut self,
        entry: &mut dyn HostEntry,
        step: &str,
        fields: &[&str],
        operation: &str,
        arguments: Value,
    ) -> Value {
        self.index += 1;
        let request = serde_json::json!({
            "schemaVersion": 1,
            "requestId": format!("settings-contract-{}", self.index),
            "callId": format!("settings-contract-call-{}", self.index),
            "capabilityId": "settings.app",
            "capabilityVersion": 1,
            "operation": operation,
            "authorization": {"settingsFields": fields},
            "arguments": arguments
        });
        let (response, raw) = entry.call(request.clone());
        assert_eq!(response["callId"], request["callId"], "{step}: {response}");
        for marker in LEAK_MARKERS {
            assert!(!raw.contains(marker), "{step} leaked {marker}: {raw}");
        }
        let code = response["error"]["code"].as_str().map(str::to_owned);
        self.outcomes
            .push((step.to_string(), response["ok"] == true, code));
        response
    }
}

fn assert_error(response: &Value, code: &str, field: Option<&str>) {
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], code, "{response}");
    assert_eq!(
        response["error"]["context"]["service"], "settings",
        "{response}"
    );
    if let Some(field) = field {
        assert_eq!(response["error"]["context"]["field"], field, "{response}");
    }
}

fn revision(response: &Value) -> String {
    assert_eq!(response["ok"], true, "{response}");
    response["result"]["revision"].as_str().unwrap().to_owned()
}

fn run_settings_contract(
    kind: EntryKind,
) -> (Vec<(String, bool, Option<String>)>, settings::AppSettings) {
    let root = fixture_root(match kind {
        EntryKind::HostCall => "call",
        EntryKind::HostServe => "serve",
    });
    let mut scenario = Scenario {
        index: 0,
        outcomes: Vec::new(),
    };

    // Write policy only: read -> patch -> read back, stale revision, and no
    // credential or removed whole-document update path.
    let mut entry = open_entry(kind, &root, &[WRITE_POLICY]);
    let read = scenario.call(
        &mut *entry,
        "policy-read",
        &[WRITE_POLICY],
        "read",
        serde_json::json!({"section": "writePolicy"}),
    );
    let initial_revision = revision(&read);
    let patched = scenario.call(
        &mut *entry,
        "policy-patch",
        &[WRITE_POLICY],
        "patch",
        serde_json::json!({"expectedRevision": initial_revision, "patch": {"maxToolRounds": 7}}),
    );
    let policy_revision = revision(&patched);
    assert_ne!(policy_revision, initial_revision);
    assert_eq!(
        patched["result"]["settings"]["assistant"]["maxToolRounds"],
        7
    );
    assert!(patched["result"]["settings"]["assistant"]
        .get("connections")
        .is_none());
    let readback = scenario.call(
        &mut *entry,
        "policy-readback",
        &[WRITE_POLICY],
        "read",
        serde_json::json!({"section": "writePolicy"}),
    );
    assert_eq!(revision(&readback), policy_revision);
    assert_eq!(readback["result"]["value"]["maxToolRounds"], 7);
    let stale = scenario.call(
        &mut *entry,
        "policy-stale",
        &[WRITE_POLICY],
        "patch",
        serde_json::json!({"expectedRevision": initial_revision, "patch": {"maxToolRounds": 9}}),
    );
    assert_error(&stale, "SETTINGS_REVISION_CONFLICT", None);
    assert_eq!(stale["error"]["retryable"], true);
    let credential = scenario.call(
        &mut *entry,
        "policy-credential-denied",
        &[WRITE_POLICY],
        "patch",
        serde_json::json!({
            "expectedRevision": policy_revision,
            "patch": {"connection": {"id": "direct", "credential": {"kind": "clear"}}}
        }),
    );
    assert_error(&credential, "SETTINGS_FIELD_NOT_GRANTED", Some(CREDENTIALS));
    let removed_update = scenario.call(
        &mut *entry,
        "policy-update-removed",
        &[WRITE_POLICY],
        "update",
        serde_json::json!({
            "expectedRevision": policy_revision,
            "settings": serde_json::to_value(settings::AppSettings::default()).unwrap()
        }),
    );
    assert_error(&removed_update, "SETTINGS_OPERATION_UNKNOWN", None);
    let partial_snapshot = scenario.call(
        &mut *entry,
        "policy-snapshot-denied",
        &[WRITE_POLICY],
        "snapshot",
        serde_json::json!({}),
    );
    assert_error(&partial_snapshot, "SETTINGS_FIELD_NOT_GRANTED", None);
    drop(entry);
    let state = persisted(&root);
    assert_unrelated_configuration_unchanged(&state);
    assert_eq!(state.assistant.max_tool_rounds, 7);
    assert_eq!(
        connection(&state, "direct").api_key.as_deref(),
        Some("SENTINEL_DIRECT_KEY")
    );
    assert_eq!(
        connection(&state, "file").api_key_file.as_deref(),
        Some("credentials/SENTINEL_KEY_FILE")
    );

    // Target authority owns endpoint and budget fields but cannot forward an
    // existing credential to a changed endpoint.
    let mut entry = open_entry(kind, &root, &[TARGETS]);
    let targets = scenario.call(
        &mut *entry,
        "targets-read",
        &[TARGETS],
        "read",
        serde_json::json!({"section": "connectionTargets"}),
    );
    let targets_revision = revision(&targets);
    let moved = scenario.call(
        &mut *entry,
        "targets-move-without-credential",
        &[TARGETS],
        "patch",
        serde_json::json!({
            "expectedRevision": targets_revision,
            "patch": {"connection": {"id": "direct", "baseUrl": "http://localhost:9999"}}
        }),
    );
    assert_error(&moved, "SETTINGS_CREDENTIAL_TARGET_CONFLICT", None);
    let moved_with_secret = scenario.call(
        &mut *entry,
        "targets-move-with-ungranted-credential",
        &[TARGETS],
        "patch",
        serde_json::json!({
            "expectedRevision": targets_revision,
            "patch": {"connection": {
                "id": "direct",
                "baseUrl": "http://localhost:9999",
                "credential": {"kind": "direct", "value": "NEW_SECRET_SHOULD_NOT_ECHO"}
            }}
        }),
    );
    assert_error(
        &moved_with_secret,
        "SETTINGS_FIELD_NOT_GRANTED",
        Some(CREDENTIALS),
    );
    let budget = scenario.call(
        &mut *entry,
        "targets-budget",
        &[TARGETS],
        "patch",
        serde_json::json!({
            "expectedRevision": targets_revision,
            "patch": {"connection": {"id": "direct", "timeoutMs": 5000}}
        }),
    );
    revision(&budget);
    assert_eq!(
        budget["result"]["settings"]["assistant"]["connections"][0]["timeoutMs"],
        5000
    );
    drop(entry);
    let state = persisted(&root);
    assert_unrelated_configuration_unchanged(&state);
    assert_eq!(
        connection(&state, "direct").base_url,
        "http://localhost:1234"
    );
    assert_eq!(connection(&state, "direct").timeout_ms, 5_000);
    assert_eq!(
        connection(&state, "direct").api_key.as_deref(),
        Some("SENTINEL_DIRECT_KEY")
    );

    // Connection authority owns display metadata only.
    let mut entry = open_entry(kind, &root, &[CONNECTIONS]);
    let connections = scenario.call(
        &mut *entry,
        "connections-read",
        &[CONNECTIONS],
        "read",
        serde_json::json!({"section": "connections"}),
    );
    let connections_revision = revision(&connections);
    let budget_denied = scenario.call(
        &mut *entry,
        "connections-budget-denied",
        &[CONNECTIONS],
        "patch",
        serde_json::json!({
            "expectedRevision": connections_revision,
            "patch": {"connection": {"id": "direct", "maxOutputTokens": 256}}
        }),
    );
    assert_error(&budget_denied, "SETTINGS_FIELD_NOT_GRANTED", Some(TARGETS));
    let renamed = scenario.call(
        &mut *entry,
        "connections-rename",
        &[CONNECTIONS],
        "patch",
        serde_json::json!({
            "expectedRevision": connections_revision,
            "patch": {"connection": {"id": "direct", "name": "Renamed"}}
        }),
    );
    revision(&renamed);
    drop(entry);

    // All Assistant sections: typed snapshot contract, projection-only writes
    // and explicit credential decisions.
    let all = [CONNECTIONS, TARGETS, CREDENTIALS, WRITE_POLICY];
    let mut entry = open_entry(kind, &root, &all);
    let not_object = scenario.call(
        &mut *entry,
        "all-snapshot-not-object",
        &all,
        "snapshot",
        serde_json::json!([]),
    );
    assert_error(&not_object, "SETTINGS_ARGUMENTS_INVALID", None);
    let extra = scenario.call(
        &mut *entry,
        "all-snapshot-extra",
        &all,
        "snapshot",
        serde_json::json!({"unexpected": true}),
    );
    assert_error(&extra, "SETTINGS_ARGUMENTS_INVALID", None);
    let snapshot = scenario.call(
        &mut *entry,
        "all-snapshot",
        &all,
        "snapshot",
        serde_json::json!({}),
    );
    let snapshot_revision = revision(&snapshot);
    for section in [
        "connections",
        "connectionTargets",
        "credentials",
        "writePolicy",
    ] {
        assert_eq!(
            snapshot["result"][section]["revision"],
            snapshot_revision.as_str(),
            "{snapshot}"
        );
        assert!(
            !snapshot["result"][section]["value"].is_null(),
            "{snapshot}"
        );
    }
    let policy = scenario.call(
        &mut *entry,
        "all-policy-patch",
        &all,
        "patch",
        serde_json::json!({"expectedRevision": snapshot_revision, "patch": {"maxToolRounds": 8}}),
    );
    let policy_revision = revision(&policy);
    assert_eq!(
        policy["result"]["settings"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["assistant"],
        "{policy}"
    );
    let rotated = scenario.call(
        &mut *entry,
        "all-move-with-explicit-credential",
        &all,
        "patch",
        serde_json::json!({
            "expectedRevision": policy_revision,
            "patch": {"connection": {
                "id": "direct",
                "baseUrl": "http://localhost:4321",
                "credential": {"kind": "direct", "value": "ROTATED_DIRECT_SECRET"}
            }}
        }),
    );
    let rotated_revision = revision(&rotated);
    let cleared = scenario.call(
        &mut *entry,
        "all-clear-file-credential",
        &all,
        "patch",
        serde_json::json!({
            "expectedRevision": rotated_revision,
            "patch": {"connection": {"id": "file", "credential": {"kind": "clear"}}}
        }),
    );
    revision(&cleared);
    let credentials = scenario.call(
        &mut *entry,
        "all-credentials-read",
        &all,
        "read",
        serde_json::json!({"section": "credentials"}),
    );
    let statuses = credentials["result"]["value"].as_array().unwrap();
    assert_eq!(statuses[0]["hasApiKey"], true, "{credentials}");
    assert_eq!(statuses[1]["hasApiKeyFile"], false, "{credentials}");
    drop(entry);

    let state = persisted(&root);
    assert_unrelated_configuration_unchanged(&state);
    assert_eq!(state.assistant.max_tool_rounds, 8);
    let direct = connection(&state, "direct");
    assert_eq!(direct.name, "Renamed");
    assert_eq!(direct.timeout_ms, 5_000);
    assert_eq!(direct.base_url, "http://localhost:4321");
    assert_eq!(direct.api_key.as_deref(), Some("ROTATED_DIRECT_SECRET"));
    assert_eq!(connection(&state, "file").api_key_file, None);
    fs::remove_dir_all(root).unwrap();
    (scenario.outcomes, state)
}

#[test]
fn settings_contract_is_identical_through_every_retained_host_entry() {
    let (one_shot, one_shot_state) = run_settings_contract(EntryKind::HostCall);
    let (serve, serve_state) = run_settings_contract(EntryKind::HostServe);
    assert_eq!(one_shot, serve);
    assert_eq!(one_shot_state, serve_state);
}

#[test]
fn settings_discovery_declares_only_the_callable_typed_operations() {
    let root = fixture_root("discover");
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .args([
            "agent",
            "host",
            "discover",
            "--field",
            CONNECTIONS,
            "--field",
            TARGETS,
            "--field",
            CREDENTIALS,
            "--field",
            WRITE_POLICY,
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for marker in LEAK_MARKERS {
        assert!(!stdout.contains(marker), "discovery leaked {marker}");
    }
    let discovery: Value = serde_json::from_str(&stdout).unwrap();
    let settings = discovery["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capability| capability["id"] == "settings.app")
        .unwrap();
    let operations = settings["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(operations, ["snapshot", "read", "patch"]);
    let snapshot = &settings["operations"][0];
    assert_eq!(
        snapshot["outputSchema"]["required"],
        serde_json::json!([
            "revision",
            "connections",
            "connectionTargets",
            "credentials",
            "writePolicy"
        ])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removed_settings_cli_entries_cannot_bypass_the_host() {
    let root = fixture_root("legacy");
    let before = fs::read(root.join("data/settings/settings.json")).unwrap();
    let update = serde_json::json!({
        "schemaVersion": 1,
        "requestId": "legacy-update",
        "expectedRevision": "any",
        "settings": serde_json::to_value(settings::AppSettings::default()).unwrap()
    });
    let call = serde_json::json!({
        "schemaVersion": 1,
        "requestId": "legacy-call",
        "arguments": {}
    });
    for (arguments, input) in [
        (
            vec!["agent", "settings", "describe", "--field", "assistant"],
            None,
        ),
        (
            vec![
                "agent",
                "settings",
                "update",
                "--json-stdin",
                "--field",
                "assistant",
            ],
            Some(&update),
        ),
        (
            vec![
                "agent",
                "call",
                "settings",
                "snapshot",
                "--json-stdin",
                "--field",
                "assistant",
            ],
            Some(&call),
        ),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_meetnote"));
        command
            .args(&arguments)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            // A removed entry may exit before reading stdin; a closed pipe is fine.
            let _ = serde_json::to_writer(child.stdin.take().unwrap(), input);
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "{arguments:?} must be a usage error"
        );
        assert!(
            output.stdout.is_empty(),
            "{arguments:?} wrote a capability response"
        );
    }
    assert_eq!(
        fs::read(root.join("data/settings/settings.json")).unwrap(),
        before
    );
    fs::remove_dir_all(root).unwrap();
}
