use application::MAX_CAPABILITY_RESULT_BYTES;
use domain::{DocumentId, DocumentState, TranscriptSegment};
use persistence::DocumentPackage;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fixture_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "meetnote-agent-cli-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn create_fixture(root: &Path) {
    let mut state = DocumentState::new(DocumentId::new("document-1"), "Fixture", "fixture.wav");
    state.results.transcript.push(TranscriptSegment {
        id: "segment-1".into(),
        start_ms: 0,
        end_ms: 1000,
        text: "A stable fact".into(),
    });
    DocumentPackage::new(
        root.join("data/documents/document-1"),
        state,
        "# Fixture\n\nA stable fact\n",
    )
    .unwrap()
    .save()
    .unwrap();
}

fn create_paged_fixture(root: &Path) {
    let mut state = DocumentState::new(
        DocumentId::new("document-1"),
        "Paged fixture",
        "fixture.wav",
    );
    for index in 0..3 {
        state.results.transcript.push(TranscriptSegment {
            id: format!("segment-{}", index + 1),
            start_ms: index * 1000,
            end_ms: (index + 1) * 1000,
            text: format!("Fact {}", index + 1),
        });
    }
    DocumentPackage::new(
        root.join("data/documents/document-1"),
        state,
        "# Paged fixture\n",
    )
    .unwrap()
    .save()
    .unwrap();
}

fn agent_command(
    root: &PathBuf,
    arguments: &[&str],
    input: Option<&Value>,
) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_meetnote");
    let mut command = Command::new(binary);
    command
        .args(arguments)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        serde_json::to_writer(child.stdin.take().unwrap(), input).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn call_request(request_id: &str, _scene: &str, _skill: &str, arguments: Value) -> Value {
    serde_json::json!({
        "schemaVersion": 1,
        "requestId": request_id,
        "toolCallId": format!("call-{request_id}"),
        "scope": {"documentIds": ["document-1"]},
        "arguments": arguments
    })
}

fn response_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON stdout: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn source_markdown_hash(root: &PathBuf, document_id: &str) -> String {
    let request = call_request(
        "read-for-source-hash",
        "ask",
        "ask",
        serde_json::json!({"documentId": document_id, "projection": "metadata"}),
    );
    let mut request = request;
    request["scope"]["documentIds"] = serde_json::json!([document_id]);
    let output = agent_command(
        root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            document_id,
        ],
        Some(&request),
    );
    assert_eq!(output.status.code(), Some(0));
    response_json(&output)["result"]["payload"]["metadata"]["markdownSha256"]
        .as_str()
        .expect("read projection must expose the Markdown hash")
        .to_string()
}

fn with_source_hash(root: &PathBuf, mut request: Value) -> Value {
    let document_id = request["arguments"]["documentId"]
        .as_str()
        .or_else(|| request["arguments"]["parentDocumentId"].as_str())
        .expect("update request must identify a document")
        .to_string();
    request["arguments"]["sourceMarkdownSha256"] =
        Value::String(source_markdown_hash(root, &document_id));
    request
}

#[test]
fn external_agent_moves_reorders_corrects_revisions_and_rejects_invalid_trees() {
    let root = fixture_root("tree-operations");
    let parent = DocumentState::new(DocumentId::new("parent"), "Parent", "a.wav");
    let first = parent.semantic_child(DocumentId::new("first"), "First", 0, 2000, Vec::new());
    let mut second =
        parent.semantic_child(DocumentId::new("second"), "Second", 0, 2000, Vec::new());
    second.hierarchy.sibling_order = 1;
    let leaf = first.semantic_child(DocumentId::new("leaf"), "Leaf", 0, 1000, Vec::new());
    let other = DocumentState::new(DocumentId::new("other"), "Other source", "b.wav");
    for state in [parent, first, second, leaf, other] {
        DocumentPackage::new(
            root.join("data/documents").join(state.id().as_str()),
            state,
            "# Tree fixture\n",
        )
        .unwrap()
        .save()
        .unwrap();
    }
    let read = |id: &str| {
        let result = tree_call(
            &root,
            "read_document",
            "read-tree",
            "ask",
            serde_json::json!({"documentId": id, "projection": "metadata"}),
        );
        assert_eq!(result["ok"], true);
        result["result"]["payload"]["metadata"].clone()
    };
    let move_args = |id: &str, revision: u64, parent: &str| {
        serde_json::json!({
            "documentId": id, "expectedRevision": revision, "newParentDocumentId": parent,
            "placement": "first", "targetSiblingDocumentId": null
        })
    };
    let stale = tree_call(
        &root,
        "move_document",
        "stale-tree",
        "move",
        move_args("second", 99, "parent"),
    );
    assert_eq!(stale["error"]["code"], "DOCUMENT_REVISION_CONFLICT");
    assert_eq!(read("second")["revision"], 0);
    let reordered = tree_call(
        &root,
        "move_document",
        "reorder-tree",
        "move",
        move_args("second", 0, "parent"),
    );
    assert_eq!(reordered["ok"], true, "{reordered}");
    assert_eq!(read("second")["siblingOrder"], 0);
    assert_eq!(read("first")["siblingOrder"], 1);
    undo_tree_call(&root, &reordered, "undo-reorder");
    assert_eq!(read("second")["siblingOrder"], 1);
    let moved = tree_call(
        &root,
        "move_document",
        "reparent-tree",
        "move",
        move_args(
            "second",
            read("second")["revision"].as_u64().unwrap(),
            "first",
        ),
    );
    assert_eq!(moved["ok"], true, "{moved}");
    assert_eq!(read("second")["parentDocumentId"], "first");
    assert_eq!(read("second")["depth"], 2);
    for (id, new_parent, code) in [
        ("first", "second", "TREE_CYCLE"),
        ("leaf", "second", "TREE_MAX_DEPTH_EXCEEDED"),
        ("second", "other", "TREE_CROSS_AUDIO_ASSET"),
    ] {
        let before = read(id);
        let failed = tree_call(
            &root,
            "move_document",
            code,
            "move",
            move_args(id, before["revision"].as_u64().unwrap(), new_parent),
        );
        assert_eq!(failed["error"]["code"], code, "{failed}");
        assert_eq!(read(id), before);
    }
    undo_tree_call(&root, &moved, "undo-reparent");
    assert_eq!(read("second")["parentDocumentId"], "parent");
    assert_eq!(read("second")["depth"], 1);
    fs::remove_dir_all(root).unwrap();
}

fn tree_call(root: &PathBuf, tool: &str, request_id: &str, scene: &str, arguments: Value) -> Value {
    let mut request = call_request(request_id, scene, scene, arguments);
    request["scope"] = serde_json::json!({"allDocuments": true});
    let output = agent_command(
        root,
        &["agent", "call", tool, "--json-stdin", "--all-documents"],
        Some(&request),
    );
    let response = response_json(&output);
    assert_eq!(
        output.status.code(),
        Some(if response["ok"] == true { 0 } else { 2 })
    );
    response
}

fn undo_tree_call(root: &PathBuf, response: &Value, request_id: &str) {
    let output = agent_command(
        root,
        &[
            "agent",
            "undo",
            response["result"]["payload"]["undoToken"].as_str().unwrap(),
            request_id,
            "--all-documents",
        ],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn external_agent_executes_synthetic_gold_including_nested_pass_and_undo() {
    let corpus =
        include_str!("../../../crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl");
    let root = fixture_root("synthetic-corpus");
    let mut count = 0;
    for line in corpus.lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let case_root = root.join(fixture["scenarioId"].as_str().unwrap());
        let state: DocumentState =
            serde_json::from_value(fixture["record"]["state"].clone()).unwrap();
        let id = state.id().as_str().to_owned();
        DocumentPackage::new(
            case_root.join("data/documents").join(&id),
            state,
            fixture["record"]["markdown"].as_str().unwrap(),
        )
        .unwrap()
        .save()
        .unwrap();
        let mut read = call_request(
            "read-gold",
            "ask",
            "ask",
            serde_json::json!({"documentId": id, "projection": "complete"}),
        );
        read["scope"]["documentIds"] = serde_json::json!([id]);
        let output = agent_command(
            &case_root,
            &[
                "agent",
                "call",
                "read_document",
                "--json-stdin",
                "--document",
                &id,
            ],
            Some(&read),
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let mode = fixture["gold"]["allowed_mode"].as_str().unwrap();
        if mode != "none" {
            let children = fixture["gold"]["children"].as_array().unwrap().iter().map(|child| serde_json::json!({
                "startSegmentId": child["start_segment_id"], "endSegmentId": child["end_segment_id"],
                "evidenceSegmentIds": child["evidence_segment_ids"], "title": child["title"][0],
                "summary": child["summary"][0], "tags": child["tags"][0]
            })).collect::<Vec<_>>();
            let mut request = with_source_hash(
                &case_root,
                call_request(
                    "gold-first-pass",
                    mode,
                    if mode == "partition" {
                        "topic-partition"
                    } else {
                        "topic-extract"
                    },
                    serde_json::json!({
                        "parentDocumentId": id, "expectedParentRevision": fixture["gold"]["expected_parent_revision"], "mode": mode, "children": children
                    }),
                ),
            );
            request["scope"]["documentIds"] = serde_json::json!([id]);
            let output = agent_command(
                &case_root,
                &[
                    "agent",
                    "call",
                    "create_semantic_children",
                    "--json-stdin",
                    "--document",
                    &id,
                ],
                Some(&request),
            );
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}: {}",
                fixture["scenarioId"],
                String::from_utf8_lossy(&output.stdout)
            );
            let response = response_json(&output);
            assert_eq!(
                response["result"]["payload"]["affectedDocuments"]
                    .as_array()
                    .unwrap()
                    .len(),
                children.len() + 1
            );
            let first_children = read_created_children(&case_root, &response);
            let source = &fixture["record"]["state"]["results"]["transcript"];
            verify_gold_children(&first_children, &fixture["gold"]["children"], source, 1);
            let expected_gaps = fixture["gold"]["uncovered_ranges"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|range| {
                    gold_segment_slice(source, range)
                        .iter()
                        .map(|segment| segment["id"].clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let reported_gaps = response["result"]["payload"]["uncoveredSegmentIds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert_eq!(reported_gaps, expected_gaps);
            if let Some(second) = fixture["gold"].get("second_pass") {
                let parent =
                    &first_children[second["parent_child_index"].as_u64().unwrap() as usize];
                let parent_id = parent["metadata"]["documentId"].as_str().unwrap();
                let second_children = second["children"].as_array().unwrap().iter().map(|child| serde_json::json!({
                    "startSegmentId": child["start_segment_id"], "endSegmentId": child["end_segment_id"],
                    "evidenceSegmentIds": child["evidence_segment_ids"], "title": child["title"][0],
                    "summary": child["summary"][0], "tags": child["tags"][0]
                })).collect::<Vec<_>>();
                let mut nested = with_source_hash(
                    &case_root,
                    call_request(
                        "gold-second-pass",
                        "partition",
                        "topic-partition",
                        serde_json::json!({
                            "parentDocumentId": parent_id, "expectedParentRevision": parent["metadata"]["revision"],
                            "mode": second["mode"], "children": second_children
                        }),
                    ),
                );
                nested["scope"]["documentIds"] = serde_json::json!([parent_id]);
                let output = agent_command(
                    &case_root,
                    &[
                        "agent",
                        "call",
                        "create_semantic_children",
                        "--json-stdin",
                        "--document",
                        parent_id,
                    ],
                    Some(&nested),
                );
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{}",
                    String::from_utf8_lossy(&output.stdout)
                );
                let nested_response = response_json(&output);
                verify_gold_children(
                    &read_created_children(&case_root, &nested_response),
                    &second["children"],
                    source,
                    2,
                );
                let undo = agent_command(
                    &case_root,
                    &[
                        "agent",
                        "undo",
                        nested_response["result"]["payload"]["undoToken"]
                            .as_str()
                            .unwrap(),
                        "undo-nested",
                        "--document",
                        parent_id,
                    ],
                    None,
                );
                assert_eq!(
                    undo.status.code(),
                    Some(0),
                    "{}",
                    String::from_utf8_lossy(&undo.stdout)
                );
            }
            let undo = agent_command(
                &case_root,
                &[
                    "agent",
                    "undo",
                    response["result"]["payload"]["undoToken"].as_str().unwrap(),
                    "undo-first",
                    "--document",
                    &id,
                ],
                None,
            );
            assert_eq!(
                undo.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&undo.stdout)
            );
        }
        count += 1;
    }
    assert_eq!(count, 48);
    fs::remove_dir_all(root).unwrap();
}

fn read_created_children(root: &PathBuf, response: &Value) -> Vec<Value> {
    let mut children = Vec::new();
    for revision in response["result"]["payload"]["affectedDocuments"]
        .as_array()
        .unwrap()
    {
        if !revision["previousRevision"].is_null() {
            continue;
        }
        let id = revision["documentId"].as_str().unwrap();
        let mut request = call_request(
            "read-child",
            "ask",
            "ask",
            serde_json::json!({"documentId": id, "projection": "complete"}),
        );
        request["scope"]["documentIds"] = serde_json::json!([id]);
        let output = agent_command(
            root,
            &[
                "agent",
                "call",
                "read_document",
                "--json-stdin",
                "--document",
                id,
            ],
            Some(&request),
        );
        assert_eq!(output.status.code(), Some(0));
        children.push(response_json(&output)["result"]["payload"].clone());
    }
    children.sort_by_key(|child| child["metadata"]["siblingOrder"].as_u64().unwrap());
    children
}

fn gold_segment_slice<'a>(source: &'a Value, range: &Value) -> &'a [Value] {
    let segments = source.as_array().unwrap();
    let start = segments
        .iter()
        .position(|segment| segment["id"] == range["start_segment_id"])
        .unwrap();
    let end = segments
        .iter()
        .position(|segment| segment["id"] == range["end_segment_id"])
        .unwrap();
    &segments[start..=end]
}

fn verify_gold_children(children: &[Value], gold: &Value, source: &Value, depth: u64) {
    assert_eq!(children.len(), gold.as_array().unwrap().len());
    for (child, expected) in children.iter().zip(gold.as_array().unwrap()) {
        assert_eq!(child["metadata"]["depth"], depth);
        assert_eq!(child["metadata"]["title"], expected["title"][0]);
        let transcript = child["transcript"].as_array().unwrap();
        let expected_transcript = gold_segment_slice(source, expected).iter().map(|segment| serde_json::json!({
            "id": segment["id"], "startMs": segment["start_ms"], "endMs": segment["end_ms"], "text": segment["text"]
        })).collect::<Vec<_>>();
        assert_eq!(*transcript, expected_transcript);
        assert_eq!(
            transcript.first().unwrap()["id"],
            expected["start_segment_id"]
        );
        assert_eq!(transcript.last().unwrap()["id"], expected["end_segment_id"]);
        for evidence in expected["evidence_segment_ids"].as_array().unwrap() {
            assert!(transcript.iter().any(|segment| segment["id"] == *evidence));
            assert!(child["markdown"]
                .as_str()
                .unwrap()
                .contains(evidence.as_str().unwrap()));
        }
    }
}

#[test]
fn external_agent_describes_and_reads_through_json_cli() {
    let root = fixture_root("read");
    create_fixture(&root);
    let binary = env!("CARGO_BIN_EXE_meetnote");

    let describe = Command::new(binary)
        .args(["agent", "describe", "--json", "--document", "document-1"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(describe.status.success());
    assert!(describe.stderr.is_empty());
    let description: Value = serde_json::from_slice(&describe.stdout).unwrap();
    assert_eq!(description["schemaVersion"], 1);
    assert!(description["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "read_document"));

    let mut child = Command::new(binary)
        .args([
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"schemaVersion":1,"requestId":"read-1","toolCallId":"call-1","scope":{"documentIds":["document-1"]},"arguments":{"documentId":"document-1","projection":"complete"}}"#,
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["capabilityId"], "workspace.documents");
    assert_eq!(response["capabilityVersion"], 1);
    assert_eq!(response["requestId"], "read-1");
    assert_eq!(
        response["result"]["payload"]["metadata"]["documentId"],
        "document-1"
    );
    assert_eq!(
        response["result"]["payload"]["transcript"][0]["id"],
        "segment-1"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_reads_transcript_pages_with_a_stable_snapshot() {
    let root = fixture_root("paged-read");
    create_paged_fixture(&root);
    let first = call_request(
        "page-1",
        "ask",
        "ask",
        serde_json::json!({"documentId":"document-1","projection":"transcript","page":{"offset":0,"limit":2}}),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&first),
    );
    assert_eq!(output.status.code(), Some(0));
    let response = response_json(&output);
    assert_eq!(
        response["result"]["payload"]["transcript"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let snapshot = response["result"]["payload"]["page"]["snapshot"].clone();
    assert_eq!(response["result"]["payload"]["page"]["nextOffset"], 2);

    let second = call_request(
        "page-2",
        "ask",
        "ask",
        serde_json::json!({"documentId":"document-1","projection":"transcript","page":{"offset":2,"limit":2,"snapshot":snapshot}}),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&second),
    );
    assert_eq!(output.status.code(), Some(0));
    let response = response_json(&output);
    assert_eq!(
        response["result"]["payload"]["transcript"][0]["id"],
        "segment-3"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_rejects_an_oversized_projection_with_a_structured_limit_error() {
    let root = fixture_root("projection-limit");
    create_fixture(&root);
    let package_path = root.join("data/documents/document-1");
    let package = DocumentPackage::load(&package_path).unwrap();
    DocumentPackage::new(
        package_path,
        package.state,
        "x".repeat(MAX_CAPABILITY_RESULT_BYTES + 1),
    )
    .unwrap()
    .save()
    .unwrap();

    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&call_request(
            "projection-limit",
            "ask",
            "ask",
            serde_json::json!({
                "documentId": "document-1",
                "projection": "markdown"
            }),
        )),
    );
    assert_eq!(output.status.code(), Some(2));
    let response = response_json(&output);
    assert_eq!(response["error"]["code"], "CONTEXT_LIMIT_EXCEEDED");
    assert_eq!(response["error"]["documentId"], "document-1");
    assert_eq!(response["error"]["retryable"], true);
    assert_eq!(
        response["error"]["context"]["maxBytes"],
        MAX_CAPABILITY_RESULT_BYTES
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_one_shot_discovery_is_removed_in_favor_of_host_discovery() {
    let root = fixture_root("legacy-discovery");
    fs::create_dir_all(&root).unwrap();
    let output = agent_command(
        &root,
        &["agent", "discover", "--json", "--document", "document-1"],
        None,
    );
    assert_eq!(output.status.code(), Some(3));
    assert!(
        output.stdout.is_empty(),
        "legacy discovery wrote a response"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_discovers_audio_capability_through_the_same_adapter() {
    let root = fixture_root("audio-capability");
    fs::create_dir_all(&root).unwrap();
    let output = agent_command(
        &root,
        &[
            "agent",
            "host",
            "discover",
            "--document",
            "document-1",
            "--project",
            "document-1",
        ],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = response_json(&output);
    assert_eq!(response["host"]["id"], "agent.host");
    let audio = response["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capability| capability["id"] == "audio.processing")
        .expect("audio capability must be discoverable");
    assert_eq!(audio["longRunning"], true);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_host_rejects_cross_document_audio_requests_without_side_effects() {
    let root = fixture_root("audio-cross-document");
    create_fixture(&root);
    let second = DocumentState::new(DocumentId::new("document-2"), "Other", "fixture.wav");
    DocumentPackage::new(root.join("data/documents/document-2"), second, "# Other\n")
        .unwrap()
        .save()
        .unwrap();
    let host_call = |operation: &str, arguments: Value| {
        agent_command(
            &root,
            &[
                "agent",
                "host",
                "call",
                "--json-stdin",
                "--project",
                "document-1",
            ],
            Some(&serde_json::json!({
                "schemaVersion": 1,
                "requestId": format!("audio-cross-document-{operation}"),
                "callId": format!("call-audio-cross-document-{operation}"),
                "capabilityId": "audio.processing",
                "capabilityVersion": 1,
                "operation": operation,
                "authorization": {"projectIds": ["document-1"]},
                "arguments": arguments
            })),
        )
    };
    let model = |backend_id: &str| {
        serde_json::json!({
            "backendId": backend_id,
            "modelId": "fixture",
            "revision": null,
            "artifactPath": "",
            "artifacts": []
        })
    };
    for (operation, arguments) in [
        (
            "start",
            serde_json::json!({
                "projectId": "document-2",
                "asrModel": model("fake-asr"),
                "diarizationModel": model("fake-diarization")
            }),
        ),
        (
            "query",
            serde_json::json!({"projectId": "document-2", "jobId": 41}),
        ),
        (
            "cancel",
            serde_json::json!({
                "projectId": "document-2",
                "asrJobId": 41,
                "diarizationJobId": 42
            }),
        ),
    ] {
        let output = host_call(operation, arguments);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(
            response_json(&output)["error"]["code"],
            "AUDIO_ACCESS_DENIED"
        );
    }
    assert!(!root.join("data/audio-jobs").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_discovers_and_calls_all_services_with_fixed_schema_and_auth_errors() {
    let root = fixture_root("three-services");
    create_fixture(&root);

    let discovery = agent_command(
        &root,
        &[
            "agent",
            "host",
            "discover",
            "--document",
            "document-1",
            "--project",
            "document-1",
        ],
        None,
    );
    assert_eq!(discovery.status.code(), Some(0));
    let discovery = response_json(&discovery);
    assert_eq!(discovery["host"]["id"], "agent.host");
    let capabilities = discovery["capabilities"].as_array().unwrap();
    for id in ["workspace.documents", "settings.app", "audio.processing"] {
        assert!(capabilities.iter().any(|capability| capability["id"] == id));
    }

    let request = call_request(
        "workspace-explicit-read",
        "ask",
        "ask",
        serde_json::json!({"documentId":"document-1","projection":"metadata"}),
    );
    let workspace_read = agent_command(
        &root,
        &[
            "agent",
            "call",
            "workspace",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    assert_eq!(workspace_read.status.code(), Some(0));
    assert_eq!(response_json(&workspace_read)["ok"], true);

    let mut invalid_schema = request.clone();
    invalid_schema["schemaVersion"] = serde_json::json!(99);
    let rejected_schema = agent_command(
        &root,
        &[
            "agent",
            "call",
            "workspace",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&invalid_schema),
    );
    assert_eq!(rejected_schema.status.code(), Some(2));
    assert_eq!(
        response_json(&rejected_schema)["error"]["code"],
        "TOOL_ARGUMENTS_INVALID"
    );

    let denied_audio = agent_command(
        &root,
        &[
            "agent",
            "host",
            "call",
            "--json-stdin",
            "--project",
            "document-1",
        ],
        Some(&serde_json::json!({
            "schemaVersion": 1,
            "requestId": "audio-denied",
            "callId": "audio-denied-call",
            "capabilityId": "audio.processing",
            "capabilityVersion": 1,
            "operation": "probe",
            "authorization": {"projectIds": ["document-1"]},
            "arguments": {"projectId": "document-2"}
        })),
    );
    assert_eq!(denied_audio.status.code(), Some(2));
    assert_eq!(
        response_json(&denied_audio)["error"]["code"],
        "AUDIO_ACCESS_DENIED"
    );

    let invalid_audio_schema = agent_command(
        &root,
        &[
            "agent",
            "host",
            "call",
            "--json-stdin",
            "--project",
            "document-1",
        ],
        Some(&serde_json::json!({
            "schemaVersion": 99,
            "requestId": "audio-schema",
            "callId": "audio-schema-call",
            "capabilityId": "audio.processing",
            "capabilityVersion": 1,
            "operation": "probe",
            "authorization": {"projectIds": ["document-1"]},
            "arguments": {"projectId": "document-1"}
        })),
    );
    assert_eq!(invalid_audio_schema.status.code(), Some(2));
    assert_eq!(
        response_json(&invalid_audio_schema)["error"]["code"],
        "CAPABILITY_SCHEMA_UNSUPPORTED"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_returns_exit_two_for_structured_tool_error() {
    let root = fixture_root("error");
    create_fixture(&root);
    let binary = env!("CARGO_BIN_EXE_meetnote");
    let mut child = Command::new(binary)
        .args([
            "agent",
            "call",
            "missing_tool",
            "--json-stdin",
            "--document",
            "document-1",
        ])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"schemaVersion":1,"requestId":"bad-1","toolCallId":"call-bad","scope":{},"arguments":{}}"#,
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "TOOL_UNKNOWN");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_requires_a_source_markdown_hash_for_updates() {
    let root = fixture_root("source-hash-required");
    create_fixture(&root);
    let request = call_request(
        "missing-source-hash",
        "organize",
        "organize",
        serde_json::json!({
            "documentId": "document-1",
            "expectedRevision": 0,
            "patch": {"title": "Must read first"}
        }),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        response_json(&output)["error"]["code"],
        "TOOL_ARGUMENTS_INVALID"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_rejects_semantic_write_after_external_markdown_edit() {
    let root = fixture_root("semantic-source-hash");
    create_paged_fixture(&root);
    let source_hash = source_markdown_hash(&root, "document-1");
    fs::write(
        root.join("data/documents/document-1/document.md"),
        "# Changed outside capability\n",
    )
    .unwrap();
    let mut request = call_request(
        "semantic-source-conflict",
        "partition",
        "topic-partition",
        serde_json::json!({
            "parentDocumentId": "document-1",
            "expectedParentRevision": 0,
            "sourceMarkdownSha256": source_hash,
            "mode": "partition",
            "children": [{
                "startSegmentId": "segment-1",
                "endSegmentId": "segment-3",
                "title": "All facts",
                "tags": [],
                "summary": "Facts",
                "evidenceSegmentIds": ["segment-1"]
            }]
        }),
    );
    request["scope"]["documentIds"] = serde_json::json!(["document-1"]);
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "create_semantic_children",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        response_json(&output)["error"]["code"],
        "DOCUMENT_REVISION_CONFLICT"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_updates_replays_reads_back_corrects_an_error_and_undoes() {
    let root = fixture_root("write");
    create_fixture(&root);
    let update = with_source_hash(
        &root,
        call_request(
            "write-1",
            "organize",
            "organize",
            serde_json::json!({
                "documentId": "document-1",
                "expectedRevision": 0,
                "patch": {
                    "title": "Updated through CLI",
                    "summary": "A stable fact",
                    "markdown": "# Updated through CLI\n\nA stable fact\n"
                }
            }),
        ),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&update),
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let response = response_json(&output);
    assert_eq!(response["ok"], true);
    let undo_token = response["result"]["payload"]["undoToken"]
        .as_str()
        .unwrap()
        .to_string();

    let read = call_request(
        "read-after-write",
        "ask",
        "ask",
        serde_json::json!({"documentId": "document-1", "projection": "complete"}),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&read),
    );
    let response = response_json(&output);
    assert_eq!(response["result"]["payload"]["metadata"]["revision"], 1);
    assert_eq!(
        response["result"]["payload"]["metadata"]["title"],
        "Updated through CLI"
    );
    assert_eq!(
        response["result"]["payload"]["markdown"],
        "# Updated through CLI\n\nA stable fact\n"
    );

    let replay = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&update),
    );
    assert_eq!(replay.status.code(), Some(0));
    assert_eq!(
        response_json(&replay)["result"]["payload"]["undoToken"],
        undo_token
    );

    let read_only = with_source_hash(
        &root,
        call_request(
            "readonly-1",
            "organize",
            "organize",
            serde_json::json!({
                "documentId": "document-1",
                "expectedRevision": 1,
                "patch": {"processing": {"lastError": "forged"}}
            }),
        ),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&read_only),
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        response_json(&output)["error"]["code"],
        "DOCUMENT_FIELD_READ_ONLY"
    );

    let corrected = with_source_hash(
        &root,
        call_request(
            "corrected-1",
            "organize",
            "organize",
            serde_json::json!({
                "documentId": "document-1",
                "expectedRevision": 1,
                "patch": {"tags": ["corrected"]}
            }),
        ),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&corrected),
    );
    assert_eq!(output.status.code(), Some(0));

    let stale_undo = agent_command(
        &root,
        &[
            "agent",
            "undo",
            &undo_token,
            "undo-stale",
            "--document",
            "document-1",
        ],
        None,
    );
    assert_eq!(stale_undo.status.code(), Some(2));
    assert_eq!(
        response_json(&stale_undo)["error"]["code"],
        "DOCUMENT_REVISION_CONFLICT"
    );

    let corrected_response = response_json(&output);
    let corrected_undo = corrected_response["result"]["payload"]["undoToken"]
        .as_str()
        .unwrap();
    let output = agent_command(
        &root,
        &[
            "agent",
            "undo",
            corrected_undo,
            "undo-correction",
            "--document",
            "document-1",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let output = agent_command(
        &root,
        &[
            "agent",
            "undo",
            &undo_token,
            "undo-original",
            "--document",
            "document-1",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));

    let read = call_request(
        "read-after-undo",
        "ask",
        "ask",
        serde_json::json!({"documentId": "document-1", "projection": "complete"}),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "read_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&read),
    );
    let response = response_json(&output);
    assert_eq!(response["result"]["payload"]["metadata"]["revision"], 4);
    assert_eq!(
        response["result"]["payload"]["metadata"]["title"],
        "Fixture"
    );
    assert_eq!(
        response["result"]["payload"]["markdown"],
        "# Fixture\n\nA stable fact\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_agent_confirm_each_has_accept_and_reject_lifecycle() {
    let root = fixture_root("confirm");
    create_fixture(&root);
    let mut settings = settings::AppSettings::default();
    settings.assistant.write_mode = contracts::AgentWriteMode::ConfirmEach;
    fs::create_dir_all(root.join("data/settings")).unwrap();
    fs::write(
        root.join("data/settings/settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();

    let request = with_source_hash(
        &root,
        call_request(
            "pending-reject",
            "organize",
            "organize",
            serde_json::json!({
                "documentId": "document-1",
                "expectedRevision": 0,
                "patch": {"title": "Rejected title"}
            }),
        ),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    assert_eq!(output.status.code(), Some(0));
    let pending = response_json(&output);
    assert_eq!(pending["result"]["type"], "pending");
    let confirmation_id = pending["result"]["payload"]["confirmationId"]
        .as_str()
        .unwrap();
    let replay = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    assert_eq!(replay.status.code(), Some(0));
    assert_eq!(
        response_json(&replay)["result"]["payload"]["confirmationId"],
        confirmation_id
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "reject",
            confirmation_id,
            "reject-1",
            "--document",
            "document-1",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        response_json(&output)["error"]["code"],
        "WRITE_POLICY_REJECTED"
    );

    let request = with_source_hash(
        &root,
        call_request(
            "pending-accept",
            "organize",
            "organize",
            serde_json::json!({
                "documentId": "document-1",
                "expectedRevision": 0,
                "patch": {"title": "Accepted title"}
            }),
        ),
    );
    let output = agent_command(
        &root,
        &[
            "agent",
            "call",
            "update_document",
            "--json-stdin",
            "--document",
            "document-1",
        ],
        Some(&request),
    );
    let pending = response_json(&output);
    let confirmation_id = pending["result"]["payload"]["confirmationId"]
        .as_str()
        .unwrap();
    let output = agent_command(
        &root,
        &[
            "agent",
            "confirm",
            confirmation_id,
            "confirm-1",
            "--document",
            "document-1",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(response_json(&output)["result"]["type"], "mutation");

    let stored = DocumentPackage::load(root.join("data/documents/document-1")).unwrap();
    assert_eq!(stored.state.title(), "Accepted title");
    assert_eq!(stored.state.revision(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn agent_settings_failure_is_process_error_with_exit_three_and_no_json_stdout() {
    let root = fixture_root("settings-error");
    fs::create_dir_all(root.join("data/settings")).unwrap();
    fs::write(root.join("data/settings/settings.json"), b"{not-json").unwrap();
    let output = agent_command(
        &root,
        &["agent", "describe", "--json", "--all-documents"],
        None,
    );
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("agent adapter failed"));
    fs::remove_dir_all(root).unwrap();
}
