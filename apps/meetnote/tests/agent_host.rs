use domain::{DocumentId, DocumentState, TranscriptSegment};
use persistence::DocumentPackage;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "meetnote-agent-host-{}-{}",
        std::process::id(),
        FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn create_fixture(root: &Path) {
    fs::create_dir_all(root.join("data/documents")).unwrap();
    write_fixture_wav(root);
    for id in ["document-1", "document-2"] {
        fs::create_dir_all(root.join(format!("data/documents/{id}"))).unwrap();
        let mut state = DocumentState::new(DocumentId::new(id), "Fixture", "fixture.wav");
        state.results.transcript = vec![
            TranscriptSegment {
                id: "segment-1".into(),
                start_ms: 0,
                end_ms: 1_000,
                text: "fixture fact one".into(),
            },
            TranscriptSegment {
                id: "segment-2".into(),
                start_ms: 1_000,
                end_ms: 2_000,
                text: "fixture fact two".into(),
            },
        ];
        DocumentPackage::new(
            root.join(format!("data/documents/{id}")),
            state,
            "# Fixture\n\nA deliberately perturbed fact\n",
        )
        .unwrap()
        .save()
        .unwrap();
    }
}

fn write_fixture_wav(root: &Path) {
    let sample_rate = 16_000u32;
    let sample_count = sample_rate * 2;
    let data_size = sample_count * 2;
    let mut wav = Vec::with_capacity(44 + data_size as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.resize(44 + data_size as usize, 0);
    fs::write(root.join("fixture.wav"), wav).unwrap();
}

fn create_settings(root: &Path, write_mode: contracts::AgentWriteMode) {
    let path = root.join("data/settings");
    fs::create_dir_all(&path).unwrap();
    let mut settings = settings::AppSettings::default();
    settings.assistant.write_mode = write_mode;
    fs::write(
        path.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
}

fn build_plugin(workspace: &Path) -> PathBuf {
    static PLUGIN: OnceLock<PathBuf> = OnceLock::new();
    PLUGIN
        .get_or_init(|| {
            let status = Command::new("moon")
                .args(["build", "--target", "native"])
                .current_dir(workspace.join("plugins/assistant-test-plugin"))
                .status()
                .expect("MoonBit compiler must be available for the Host acceptance test");
            assert!(status.success(), "MoonBit test plugin did not compile");
            workspace.join("plugins/assistant-test-plugin/_build/native/debug/build/main/main.exe")
        })
        .clone()
}

fn plugin_call(
    binary: &Path,
    operation: &str,
    response: Option<&Value>,
    history: Option<&Value>,
) -> Value {
    let mut command = Command::new(binary);
    command.env("MEETNOTE_PLUGIN_OPERATION", operation);
    if let Some(response) = response {
        command.env(
            "MEETNOTE_HOST_RESPONSE",
            serde_json::to_string(response).unwrap(),
        );
    }
    if let Some(history) = history {
        command.env(
            "MEETNOTE_HOST_HISTORY",
            serde_json::to_string(history).unwrap(),
        );
    }
    let output = command.output().expect("compiled MoonBit plugin must run");
    assert!(
        output.status.success(),
        "plugin stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "MoonBit plugin returned invalid JSON: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(envelope["pluginId"], "meetnote.assistant-test");
    envelope["call"].clone()
}

fn feature_host_binary() -> PathBuf {
    static HOST: OnceLock<PathBuf> = OnceLock::new();
    HOST.get_or_init(|| {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target_dir = workspace.join("target/agent-host-fake");
        let status = Command::new("cargo")
            .args([
                "build",
                "-p",
                "meetnote",
                "--bin",
                "meetnote",
                "--features",
                "host-test-fake-audio",
            ])
            .env("CARGO_TARGET_DIR", &target_dir)
            .current_dir(&workspace)
            .status()
            .expect("Cargo must build the explicit Host fake-audio test binary");
        assert!(
            status.success(),
            "Host fake-audio test binary did not compile"
        );
        target_dir.join("debug/meetnote")
    })
    .clone()
}

fn host_call(root: &Path, index: usize, call: Value, trusted: &[&str]) -> Value {
    let mut args = vec!["agent", "host", "call", "--json-stdin"];
    args.extend_from_slice(trusted);
    let mut child = Command::new(feature_host_binary())
        .args(args)
        .current_dir(root)
        .env("MEETNOTE_AGENT_HOST_FAKE_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let call = wire_call(index, call);
    serde_json::to_writer(child.stdin.take().unwrap(), &call).unwrap();
    let output = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "Host returned invalid JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(response["callId"], call["callId"]);
    response
}

fn schema_contains_required(schema: &Value, required: &[&str]) -> bool {
    match schema {
        Value::Object(object) => {
            let found = object
                .get("required")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                        == required.iter().copied().collect()
                })
                .unwrap_or(false);
            found
                || object
                    .values()
                    .any(|value| schema_contains_required(value, required))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| schema_contains_required(value, required)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

/// The Host discovery envelope schema must describe exactly the wire fields a
/// caller sends and the Host returns.
fn assert_envelope_schema_matches(schema: &Value, value: &Value) {
    let properties = schema["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("envelope schema declares no properties: {schema}"));
    assert_eq!(schema["additionalProperties"], false, "{schema}");
    let object = value.as_object().unwrap();
    for required in schema["required"].as_array().unwrap() {
        let required = required.as_str().unwrap();
        assert!(
            object.contains_key(required),
            "missing required {required}: {value}"
        );
    }
    for key in object.keys() {
        assert!(
            properties.contains_key(key),
            "undeclared envelope field {key}: {schema}"
        );
    }
}

#[test]
fn normal_agent_binary_does_not_switch_to_fake_audio_from_environment() {
    let root = fixture_root();
    create_fixture(&root);
    let mut child = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .args([
            "agent",
            "host",
            "call",
            "--json-stdin",
            "--project",
            "document-1",
        ])
        .current_dir(&root)
        .env("MEETNOTE_AGENT_HOST_FAKE_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = wire_call(
        50,
        serde_json::json!({
            "capabilityId": "audio.processing",
            "capabilityVersion": 1,
            "operation": "probe",
            "authorization": {"projectIds": ["document-1"]},
            "arguments": {
                "projectId": "document-1",
                "asrModel": {
                    "backendId": "fake-asr",
                    "modelId": "fixture",
                    "revision": null,
                    "artifactPath": "",
                    "artifacts": []
                },
                "diarizationModel": {
                    "backendId": "fake-diarization",
                    "modelId": "fixture",
                    "revision": null,
                    "artifactPath": "",
                    "artifacts": []
                }
            }
        }),
    );
    serde_json::to_writer(child.stdin.take().unwrap(), &request).unwrap();
    let output = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["ok"], false,
        "normal binary used fake audio: {response}"
    );
    assert_eq!(response["error"]["code"], "AUDIO_BACKEND_UNAVAILABLE");
    fs::remove_dir_all(root).unwrap();
}

struct HostSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_index: usize,
}

impl HostSession {
    fn start(root: &Path, trusted: &[&str]) -> Self {
        let mut args = vec!["agent", "host", "serve"];
        args.extend_from_slice(trusted);
        let mut child = Command::new(feature_host_binary())
            .args(args)
            .current_dir(root)
            .env("MEETNOTE_AGENT_HOST_FAKE_AUDIO", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
            next_index: 1_000,
        }
    }

    fn call(&mut self, call: Value) -> Value {
        let call = wire_call(self.next_index, call);
        self.next_index += 1;
        writeln!(self.stdin, "{}", serde_json::to_string(&call).unwrap()).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["callId"], call["callId"]);
        response
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wire_call(index: usize, mut call: Value) -> Value {
    call["schemaVersion"] = Value::from(1);
    call["requestId"] = Value::from(format!("codex-host-task-{index}"));
    call["callId"] = Value::from(format!("plugin-call-{index}"));
    call
}

#[test]
fn response_driven_plugin_corrects_perturbed_fixture_and_reads_back() {
    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let trusted = ["--document", "document-1"];

    let bad = plugin_call(&plugin, "workflow", None, None);
    let error = host_call(&root, 1, bad, &trusted);
    assert_eq!(error["error"]["code"], "TOOL_ARGUMENTS_INVALID");

    let read_before_call = plugin_call(&plugin, "workflow", Some(&error), None);
    let read_before = host_call(&root, 2, read_before_call, &trusted);
    assert_eq!(read_before["ok"], true);
    assert_eq!(read_before["result"]["payload"]["metadata"]["revision"], 0);
    assert_eq!(
        read_before["result"]["payload"]["metadata"]["title"],
        "Fixture"
    );
    let actual_hash = read_before["result"]["payload"]["metadata"]["markdownSha256"].clone();

    let update_call = plugin_call(&plugin, "workflow", Some(&read_before), None);
    assert_eq!(
        update_call["arguments"]["sourceMarkdownSha256"],
        actual_hash
    );
    assert_eq!(update_call["arguments"]["expectedRevision"], 0);
    let mutation = host_call(&root, 3, update_call, &trusted);
    assert_eq!(mutation["ok"], true);
    assert_eq!(mutation["result"]["type"], "mutation");
    let undo_token = mutation["result"]["payload"]["undoToken"].clone();

    let read_after_call = plugin_call(&plugin, "workflow", Some(&mutation), None);
    let read_after = host_call(&root, 4, read_after_call, &trusted);
    assert_eq!(
        read_after["result"]["payload"]["metadata"]["title"],
        "Codex Host task"
    );

    let undo_call = plugin_call(&plugin, "workflow", Some(&read_after), Some(&mutation));
    assert_eq!(undo_call["arguments"]["undoToken"], undo_token);
    let undone = host_call(&root, 5, undo_call, &trusted);
    assert_eq!(undone["ok"], true);

    let final_call = plugin_call(&plugin, "workflow", Some(&undone), None);
    let final_read = host_call(&root, 6, final_call, &trusted);
    assert_eq!(
        final_read["result"]["payload"]["metadata"]["title"],
        "Fixture"
    );
    assert_eq!(final_read["result"]["payload"]["metadata"]["revision"], 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_keeps_trusted_grants_and_exposes_exact_descriptors() {
    let root = fixture_root();
    create_fixture(&root);
    let discovery = Command::new(feature_host_binary())
        .args([
            "agent",
            "host",
            "discover",
            "--document",
            "document-1",
            "--project",
            "document-1",
            "--field",
            "assistant.writePolicy",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(discovery.status.success());
    let discovery: Value = serde_json::from_slice(&discovery.stdout).unwrap();
    let request_schema = &discovery["envelope"]["request"];
    let response_schema = &discovery["envelope"]["response"];
    let mut required = request_schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("discovery must describe the call envelope: {discovery}"))
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    required.sort_unstable();
    assert_eq!(
        required,
        [
            "arguments",
            "callId",
            "capabilityId",
            "capabilityVersion",
            "operation",
            "requestId",
            "schemaVersion"
        ]
    );
    let sent = wire_call(
        9,
        serde_json::json!({
            "capabilityId": "workspace.documents",
            "capabilityVersion": 1,
            "operation": "read_document",
            "authorization": {"documentIds": ["document-1"]},
            "arguments": {"documentId": "document-1", "projection": "metadata"}
        }),
    );
    assert_envelope_schema_matches(request_schema, &sent);
    let returned = host_call(&root, 9, sent.clone(), &["--document", "document-1"]);
    assert_eq!(returned["ok"], true, "{returned}");
    assert_envelope_schema_matches(response_schema, &returned);
    let capabilities = discovery["capabilities"].as_array().unwrap();
    let audio = capabilities
        .iter()
        .find(|value| value["id"] == "audio.processing")
        .unwrap();
    let cancel = audio["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == "cancel")
        .unwrap();
    assert_eq!(
        cancel["inputSchema"]["required"],
        serde_json::json!(["projectId", "asrJobId", "diarizationJobId"])
    );
    assert!(
        audio["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["name"] == "result")
    );
    let workspace_capability = capabilities
        .iter()
        .find(|value| value["id"] == "workspace.documents")
        .unwrap();
    for control in ["confirm_mutation", "reject_mutation", "undo_mutation"] {
        let operation = workspace_capability["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["name"] == control)
            .unwrap();
        assert_ne!(
            operation["inputSchema"],
            serde_json::json!({"type": "object"})
        );
    }
    for operation in workspace_capability["operations"].as_array().unwrap() {
        assert_ne!(
            operation["outputSchema"],
            serde_json::json!({"type": "object"})
        );
        let schema = operation["outputSchema"].to_string();
        assert!(schema.contains("\"mutation\""));
        assert!(!schema.contains("\"Mutation\""));
    }
    let audio_input = audio["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == "start")
        .unwrap()["inputSchema"]
        .clone();
    assert!(schema_contains_required(
        &audio_input,
        &["backendId", "modelId", "artifactPath"]
    ));
    assert!(audio_input.to_string().contains("\"revision\""));
    assert!(audio_input.to_string().contains("\"artifacts\""));
    for operation in audio["operations"].as_array().unwrap() {
        assert_ne!(
            operation["outputSchema"],
            serde_json::json!({"type": "object"})
        );
    }
    let settings_capability = capabilities
        .iter()
        .find(|value| value["id"] == "settings.app")
        .unwrap();
    for operation in settings_capability["operations"].as_array().unwrap() {
        assert_ne!(
            operation["outputSchema"],
            serde_json::json!({"type": "object"})
        );
    }

    let broaden = host_call(
        &root,
        10,
        serde_json::json!({
            "capabilityId": "workspace.documents",
            "capabilityVersion": 1,
            "operation": "read_document",
            "authorization": {"allDocuments": true},
            "arguments": {"documentId": "document-2", "projection": "metadata"}
        }),
        &["--document", "document-1"],
    );
    assert_eq!(broaden["error"]["code"], "DOCUMENT_ACCESS_DENIED");
    assert_envelope_schema_matches(response_schema, &broaden);

    let settings = host_call(
        &root,
        11,
        serde_json::json!({
            "capabilityId": "settings.app",
            "capabilityVersion": 1,
            "operation": "read",
            "authorization": {"settingsFields": ["assistant.credentials"]},
            "arguments": {"section": "credentials"}
        }),
        &["--field", "assistant.writePolicy"],
    );
    assert_eq!(settings["error"]["code"], "SETTINGS_FIELD_NOT_GRANTED");

    let audio = host_call(
        &root,
        12,
        serde_json::json!({
            "capabilityId": "audio.processing",
            "capabilityVersion": 1,
            "operation": "probe",
            "authorization": {"projectIds": ["document-2"]},
            "arguments": {
                "projectId": "document-2",
                "asrModel": {"backendId": "whisper-rs", "modelId": "missing", "revision": null, "artifactPath": ""},
                "diarizationModel": {"backendId": "diarization-native", "modelId": "missing", "revision": null, "artifactPath": ""}
            }
        }),
        &["--project", "document-1"],
    );
    assert_eq!(
        audio["error"]["code"], "AUDIO_ACCESS_DENIED",
        "audio response: {audio}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_plugin_negative_calls_have_stable_errors_and_no_side_effects() {
    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let trusted = ["--document", "document-1"];

    for (index, operation, code, host_operation) in [
        (
            500,
            "unknown-field",
            "TOOL_ARGUMENTS_INVALID",
            "read_document",
        ),
        (
            501,
            "readonly",
            "DOCUMENT_FIELD_READ_ONLY",
            "update_document",
        ),
        (
            502,
            "version-error",
            "CAPABILITY_VERSION_UNSUPPORTED",
            "read_document",
        ),
    ] {
        let response = host_call(
            &root,
            index,
            plugin_call(&plugin, operation, None, None),
            &trusted,
        );
        assert_eq!(response["ok"], false, "negative response: {response}");
        assert_eq!(response["error"]["code"], code);
        assert_eq!(response["error"]["context"]["service"], "workspace");
        assert_eq!(response["error"]["context"]["operation"], host_operation);
    }

    let settings = host_call(
        &root,
        503,
        plugin_call(&plugin, "settings-credentials", None, None),
        &["--field", "assistant.writePolicy"],
    );
    assert_eq!(settings["error"]["code"], "SETTINGS_FIELD_NOT_GRANTED");
    assert_eq!(settings["error"]["context"]["service"], "settings");
    assert_eq!(settings["error"]["context"]["operation"], "read");

    let audio = host_call(
        &root,
        504,
        plugin_call(&plugin, "audio-invalid-job", None, None),
        &["--project", "document-1"],
    );
    assert_eq!(audio["error"]["code"], "AUDIO_ACCESS_DENIED");
    assert_eq!(audio["error"]["context"]["resourceType"], "project");
    assert_eq!(audio["error"]["context"]["resourceId"], "document-1");

    let bad = host_call(
        &root,
        505,
        plugin_call(&plugin, "workflow", None, None),
        &trusted,
    );
    let read = host_call(
        &root,
        506,
        plugin_call(&plugin, "workflow", Some(&bad), None),
        &trusted,
    );
    let mutation = host_call(
        &root,
        507,
        plugin_call(&plugin, "workflow", Some(&read), None),
        &trusted,
    );
    assert_ok_mutation(&mutation);
    let stale_read = host_call(
        &root,
        508,
        plugin_call(&plugin, "stale-revision", None, None),
        &trusted,
    );
    let stale = host_call(
        &root,
        509,
        plugin_call(&plugin, "stale-revision", Some(&stale_read), None),
        &trusted,
    );
    assert_eq!(stale["error"]["code"], "DOCUMENT_REVISION_CONFLICT");
    assert_eq!(stale["error"]["context"]["expectedRevision"], "0");
    assert_eq!(stale["error"]["context"]["actualRevision"], "1");
    let final_read = host_call(
        &root,
        510,
        serde_json::json!({
            "capabilityId": "workspace.documents",
            "capabilityVersion": 1,
            "operation": "read_document",
            "authorization": {"documentIds": ["document-1"]},
            "arguments": {"documentId": "document-1", "projection": "metadata"}
        }),
        &trusted,
    );
    assert_eq!(
        final_read["result"]["payload"]["metadata"]["title"],
        "Codex Host task"
    );
    assert_eq!(final_read["result"]["payload"]["metadata"]["revision"], 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_plugin_reaches_every_host_operation_shape() {
    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let created = host_call(
        &root,
        100,
        plugin_call(&plugin, "create", None, None),
        &["--all-documents"],
    );
    assert_ok_mutation(&created);
    let listed = host_call(
        &root,
        101,
        plugin_call(&plugin, "list", None, None),
        &["--all-documents"],
    );
    assert!(listed["ok"].as_bool().unwrap(), "list: {listed}");
    assert_eq!(listed["result"]["type"], "documents");
    assert!(
        listed["result"]["payload"]["documents"]
            .as_array()
            .unwrap()
            .len()
            >= 2
    );

    let semantic_read = host_call(
        &root,
        102,
        plugin_call(&plugin, "semantic", None, None),
        &["--document", "document-1"],
    );
    assert!(semantic_read["ok"].as_bool().unwrap());
    let semantic = host_call(
        &root,
        103,
        plugin_call(&plugin, "semantic", Some(&semantic_read), None),
        &["--document", "document-1"],
    );
    assert_ok_mutation(&semantic);

    let move_read = host_call(
        &root,
        104,
        plugin_call(&plugin, "move", None, None),
        &["--document", "document-1"],
    );
    let moved = host_call(
        &root,
        105,
        plugin_call(&plugin, "move", Some(&move_read), None),
        &["--all-documents"],
    );
    assert_ok_mutation(&moved);

    let probe = host_call(
        &root,
        106,
        plugin_call(&plugin, "audio-probe", None, None),
        &["--project", "document-1"],
    );
    assert!(probe["ok"].as_bool().unwrap(), "audio probe: {probe}");
    let started = host_call(
        &root,
        107,
        plugin_call(&plugin, "audio-start", None, None),
        &["--project", "document-1"],
    );
    assert!(started["ok"].as_bool().unwrap(), "audio start: {started}");
    assert!(started["result"]["asrJobId"].is_number());
    assert!(started["result"]["diarizationJobId"].is_number());

    for (index, operation, field, section) in [
        (
            108,
            "settings-connections",
            "assistant.connections",
            "connections",
        ),
        (
            109,
            "settings-targets",
            "assistant.connectionTargets",
            "connectionTargets",
        ),
        (
            110,
            "settings-credentials",
            "assistant.credentials",
            "credentials",
        ),
        (
            111,
            "settings-policy",
            "assistant.writePolicy",
            "writePolicy",
        ),
    ] {
        let response = host_call(
            &root,
            index,
            plugin_call(&plugin, operation, None, None),
            &["--field", field],
        );
        assert!(response["ok"].as_bool().unwrap(), "{section}: {response}");
        assert!(response["result"]["value"].is_object() || response["result"]["value"].is_array());
    }

    let settings_read = plugin_call(&plugin, "settings-patch", None, None);
    let read = host_call(
        &root,
        120,
        settings_read,
        &["--field", "assistant.writePolicy"],
    );
    assert_eq!(read["ok"], true);
    let settings_patch = plugin_call(&plugin, "settings-patch", Some(&read), None);
    let patched = host_call(
        &root,
        121,
        settings_patch,
        &["--field", "assistant.writePolicy"],
    );
    assert!(
        patched["ok"].as_bool().unwrap(),
        "settings patch: {patched}"
    );
    assert_ne!(patched["result"]["revision"], read["result"]["revision"]);
    let settings_readback = host_call(
        &root,
        122,
        plugin_call(&plugin, "settings-patch", Some(&patched), None),
        &["--field", "assistant.writePolicy"],
    );
    assert!(settings_readback["ok"].as_bool().unwrap());
    assert_eq!(settings_readback["result"]["value"]["maxToolRounds"], 9);
    let settings_snapshot = host_call(
        &root,
        123,
        plugin_call(&plugin, "settings-snapshot", None, None),
        &[
            "--field",
            "assistant.connections",
            "--field",
            "assistant.connectionTargets",
            "--field",
            "assistant.credentials",
            "--field",
            "assistant.writePolicy",
        ],
    );
    assert!(
        settings_snapshot["ok"].as_bool().unwrap(),
        "settings snapshot: {settings_snapshot}"
    );
    let snapshot = &settings_snapshot["result"];
    let revision = snapshot["revision"].as_str().unwrap();
    for section in [
        "connections",
        "connectionTargets",
        "credentials",
        "writePolicy",
    ] {
        assert_eq!(
            snapshot[section]["revision"].as_str(),
            Some(revision),
            "{section}: {snapshot}"
        );
    }
    assert_eq!(snapshot["writePolicy"]["value"]["maxToolRounds"], 9);
    fs::remove_dir_all(root).unwrap();
}

fn assert_ok_mutation(response: &Value) {
    assert!(
        response["ok"].as_bool().unwrap(),
        "mutation response: {response}"
    );
    assert_eq!(response["result"]["type"], "mutation");
    assert!(response["result"]["payload"]["undoToken"].is_string());
    assert!(
        !response["result"]["payload"]["affectedDocuments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn real_plugin_exercises_confirm_each_through_host() {
    let root = fixture_root();
    create_fixture(&root);
    create_settings(&root, contracts::AgentWriteMode::ConfirmEach);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let trusted = ["--document", "document-1"];
    let read_call = plugin_call(&plugin, "guarded-update", None, None);
    let read = host_call(&root, 200, read_call, &trusted);
    assert_eq!(read["ok"], true);
    let update_call = plugin_call(&plugin, "guarded-update", Some(&read), None);
    let pending = host_call(&root, 201, update_call, &trusted);
    assert_eq!(pending["ok"], true);
    assert_eq!(pending["result"]["type"], "pending");

    let narrowed = host_call(
        &root,
        202,
        serde_json::json!({
            "capabilityId": "workspace.documents",
            "capabilityVersion": 1,
            "operation": "confirm_mutation",
            "authorization": {"documentIds": ["document-2"]},
            "arguments": {"confirmationId": pending["result"]["payload"]["confirmationId"]}
        }),
        &trusted,
    );
    assert_eq!(narrowed["error"]["code"], "DOCUMENT_ACCESS_DENIED");
    let confirm_call = plugin_call(&plugin, "guarded-update", Some(&pending), None);
    let committed = host_call(&root, 203, confirm_call, &trusted);
    assert_eq!(committed["ok"], true);
    assert_eq!(committed["result"]["type"], "mutation");
    assert!(committed["result"]["payload"]["undoToken"].is_string());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_plugin_confirms_and_rejects_live_pending_mutations() {
    for (index, operation) in [(300, "confirm"), (400, "reject")] {
        let root = fixture_root();
        create_fixture(&root);
        create_settings(&root, contracts::AgentWriteMode::ConfirmEach);
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plugin = build_plugin(&workspace);
        let trusted = ["--document", "document-1"];

        let read = host_call(
            &root,
            index,
            plugin_call(&plugin, operation, None, None),
            &trusted,
        );
        assert!(read["ok"].as_bool().unwrap());
        let pending = host_call(
            &root,
            index + 1,
            plugin_call(&plugin, operation, Some(&read), None),
            &trusted,
        );
        assert!(pending["ok"].as_bool().unwrap(), "pending: {pending}");
        assert_eq!(pending["result"]["type"], "pending");
        let control = host_call(
            &root,
            index + 2,
            plugin_call(&plugin, operation, Some(&pending), None),
            &trusted,
        );
        if operation == "confirm" {
            assert_ok_mutation(&control);
            let readback = host_call(
                &root,
                index + 3,
                serde_json::json!({
                    "capabilityId": "workspace.documents",
                    "capabilityVersion": 1,
                    "operation": "read_document",
                    "authorization": {"documentIds": ["document-1"]},
                    "arguments": {"documentId": "document-1", "projection": "metadata"}
                }),
                &trusted,
            );
            assert_eq!(
                readback["result"]["payload"]["metadata"]["title"],
                "Plugin guarded task"
            );
        } else {
            assert_eq!(control["ok"], false);
            assert_eq!(control["error"]["code"], "WRITE_POLICY_REJECTED");
            let readback = host_call(
                &root,
                index + 3,
                serde_json::json!({
                    "capabilityId": "workspace.documents",
                    "capabilityVersion": 1,
                    "operation": "read_document",
                    "authorization": {"documentIds": ["document-1"]},
                    "arguments": {"documentId": "document-1", "projection": "metadata"}
                }),
                &trusted,
            );
            assert_eq!(
                readback["result"]["payload"]["metadata"]["title"],
                "Fixture"
            );
            assert_eq!(readback["result"]["payload"]["metadata"]["revision"], 0);
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn control_operations_reject_unknown_arguments_before_consuming_state() {
    for (index, operation, control_operation) in [
        (600, "confirm", "confirm_mutation"),
        (700, "reject", "reject_mutation"),
    ] {
        let root = fixture_root();
        create_fixture(&root);
        create_settings(&root, contracts::AgentWriteMode::ConfirmEach);
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plugin = build_plugin(&workspace);
        let trusted = ["--document", "document-1"];

        let read = host_call(
            &root,
            index,
            plugin_call(&plugin, operation, None, None),
            &trusted,
        );
        let pending = host_call(
            &root,
            index + 1,
            plugin_call(&plugin, operation, Some(&read), None),
            &trusted,
        );
        assert_eq!(pending["result"]["type"], "pending");
        let mut invalid_control = plugin_call(&plugin, operation, Some(&pending), None);
        invalid_control["arguments"]["unexpected"] = Value::from("must be rejected");
        let invalid = host_call(&root, index + 2, invalid_control, &trusted);
        assert_eq!(invalid["ok"], false, "invalid control: {invalid}");
        assert_eq!(invalid["error"]["code"], "TOOL_ARGUMENTS_INVALID");
        assert_eq!(invalid["error"]["context"]["operation"], control_operation);

        let valid = host_call(
            &root,
            index + 3,
            plugin_call(&plugin, operation, Some(&pending), None),
            &trusted,
        );
        if operation == "confirm" {
            assert_ok_mutation(&valid);
        } else {
            assert_eq!(valid["error"]["code"], "WRITE_POLICY_REJECTED");
        }
        fs::remove_dir_all(root).unwrap();
    }

    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let trusted = ["--document", "document-1"];
    let bad = host_call(
        &root,
        800,
        plugin_call(&plugin, "workflow", None, None),
        &trusted,
    );
    let read = host_call(
        &root,
        801,
        plugin_call(&plugin, "workflow", Some(&bad), None),
        &trusted,
    );
    let mutation = host_call(
        &root,
        802,
        plugin_call(&plugin, "workflow", Some(&read), None),
        &trusted,
    );
    let read_after = host_call(
        &root,
        803,
        plugin_call(&plugin, "workflow", Some(&mutation), None),
        &trusted,
    );
    let mut invalid_undo = plugin_call(&plugin, "workflow", Some(&read_after), Some(&mutation));
    invalid_undo["arguments"]["unexpected"] = Value::from("must be rejected");
    let invalid = host_call(&root, 804, invalid_undo, &trusted);
    assert_eq!(invalid["ok"], false, "invalid undo: {invalid}");
    assert_eq!(invalid["error"]["code"], "TOOL_ARGUMENTS_INVALID");
    assert_eq!(invalid["error"]["context"]["operation"], "undo_mutation");

    let undone = host_call(
        &root,
        805,
        plugin_call(&plugin, "workflow", Some(&read_after), Some(&mutation)),
        &trusted,
    );
    assert!(undone["ok"].as_bool().unwrap(), "undo: {undone}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_plugin_audio_handles_survive_a_persistent_host() {
    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let mut host = HostSession::start(&root, &["--project", "document-1"]);

    let started_call = plugin_call(&plugin, "audio-success", None, None);
    let started = host.call(started_call);
    assert!(started["ok"].as_bool().unwrap(), "start: {started}");
    let mut query = Value::Null;
    let mut previous = started.clone();
    for _ in 0..20 {
        let call = plugin_call(&plugin, "audio-success", Some(&previous), Some(&started));
        query = host.call(call);
        assert!(query["ok"].as_bool().unwrap(), "query: {query}");
        if query["result"]["state"] == "Completed" {
            break;
        }
        previous = query.clone();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        query["result"]["state"], "Completed",
        "audio did not finish: {query}"
    );
    let result = host.call(plugin_call(
        &plugin,
        "audio-success",
        Some(&query),
        Some(&started),
    ));
    assert!(result["ok"].as_bool().unwrap(), "result: {result}");
    assert_eq!(result["result"]["ready"], true);
    assert!(result["result"]["document"].is_object());

    let cancelled_started = host.call(plugin_call(&plugin, "audio-cancel-flow", None, None));
    assert!(cancelled_started["ok"].as_bool().unwrap());
    let cancelled = host.call(plugin_call(
        &plugin,
        "audio-cancel-flow",
        Some(&cancelled_started),
        Some(&cancelled_started),
    ));
    assert!(cancelled["ok"].as_bool().unwrap(), "cancel: {cancelled}");
    assert_eq!(cancelled["result"]["cancelled"], true);
    let cancelled_query = host.call(plugin_call(
        &plugin,
        "audio-cancel-flow",
        Some(&cancelled),
        Some(&cancelled_started),
    ));
    assert!(
        cancelled_query["ok"].as_bool().unwrap(),
        "cancel query: {cancelled_query}"
    );
    assert_eq!(cancelled_query["result"]["state"], "Cancelled");
    drop(host);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_replays_the_same_mutation_receipt_idempotently() {
    let root = fixture_root();
    create_fixture(&root);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let plugin = build_plugin(&workspace);
    let trusted = ["--document", "document-1"];
    let bad = plugin_call(&plugin, "workflow", None, None);
    let error = host_call(&root, 220, bad, &trusted);
    let read = host_call(
        &root,
        221,
        plugin_call(&plugin, "workflow", Some(&error), None),
        &trusted,
    );
    let update = plugin_call(&plugin, "workflow", Some(&read), None);
    let first = host_call(&root, 222, update.clone(), &trusted);
    let replay = host_call(&root, 222, update, &trusted);
    assert_eq!(first["ok"], true);
    assert_eq!(replay["ok"], true);
    assert_eq!(
        first["result"]["payload"]["undoToken"],
        replay["result"]["payload"]["undoToken"]
    );
    assert_eq!(
        replay["result"]["payload"]["affectedDocuments"][0]["revision"],
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_serve_reuses_one_host_for_multiple_capability_calls() {
    let root = fixture_root();
    create_fixture(&root);
    let read = |index: usize| {
        wire_call(
            index,
            serde_json::json!({
                "capabilityId": "workspace.documents",
                "capabilityVersion": 1,
                "operation": "read_document",
                "authorization": {"documentIds": ["document-1"]},
                "arguments": {"documentId": "document-1", "projection": "metadata"}
            }),
        )
    };
    let mut child = Command::new(feature_host_binary())
        .args(["agent", "host", "serve", "--document", "document-1"])
        .current_dir(&root)
        .env("MEETNOTE_AGENT_HOST_FAKE_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", serde_json::to_string(&read(300)).unwrap()).unwrap();
        writeln!(stdin, "{}", serde_json::to_string(&read(301)).unwrap()).unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["callId"], "plugin-call-300");
    assert_eq!(responses[1]["callId"], "plugin-call-301");
    assert!(responses.iter().all(|response| response["ok"] == true));
    fs::remove_dir_all(root).unwrap();
}
