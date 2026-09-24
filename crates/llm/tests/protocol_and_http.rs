use llm::{
    AnthropicMessagesAdapter, AuthMode, ConnectionConfig, Credential, LiteLlmProvider,
    LlmCancellation, LlmErrorKind, LlmInputBlock, LlmMessage, LlmOutputBlock, LlmProtocol,
    LlmProvider, LlmRequest, LlmStopReason, LlmToolChoice, LlmToolDefinition,
    OpenAiChatCompletionsAdapter, OpenAiResponsesAdapter, ProtocolAdapter,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHAT_FIXTURE: &str = include_str!("fixtures/openai_chat.json");
const ANTHROPIC_FIXTURE: &str = include_str!("fixtures/anthropic_messages.json");
const RESPONSES_FIXTURE: &str = include_str!("fixtures/openai_responses.json");

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

struct ResponseSpec {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    delay: Duration,
}

impl ResponseSpec {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }
}

fn spawn_server(
    responses: Vec<ResponseSpec>,
) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            sender.send(request).unwrap();
            if !response.delay.is_zero() {
                thread::sleep(response.delay);
            }
            write_response(&mut stream, response);
        }
    });
    (format!("http://{address}"), receiver, handle)
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending complete headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };

    let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending complete body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = if content_length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
    };

    CapturedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: ResponseSpec) {
    let reason = match response.status {
        200 => "OK",
        302 => "Found",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Test Status",
    };
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason,
        response.body.len()
    );
    for (name, value) in response.headers {
        headers.push_str(&format!("{name}: {value}\r\n"));
    }
    headers.push_str("\r\n");
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
}

fn fixture(protocol: LlmProtocol) -> Value {
    let source = match protocol {
        LlmProtocol::OpenAiChatCompletions => CHAT_FIXTURE,
        LlmProtocol::AnthropicMessages => ANTHROPIC_FIXTURE,
        LlmProtocol::OpenAiResponses => RESPONSES_FIXTURE,
    };
    serde_json::from_str(source).unwrap()
}

fn adapter(protocol: LlmProtocol) -> Box<dyn ProtocolAdapter> {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => Box::new(OpenAiChatCompletionsAdapter),
        LlmProtocol::AnthropicMessages => Box::new(AnthropicMessagesAdapter),
        LlmProtocol::OpenAiResponses => Box::new(OpenAiResponsesAdapter),
    }
}

fn tool() -> LlmToolDefinition {
    LlmToolDefinition {
        name: "read_document".into(),
        description: "Read one document".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"documentId": {"type": "string"}},
            "required": ["documentId"],
            "additionalProperties": false
        }),
    }
}

fn golden_request() -> LlmRequest {
    LlmRequest {
        instructions: Some("Use only granted documents.".into()),
        thinking: None,
        messages: vec![
            LlmMessage::User(vec![LlmInputBlock::Text {
                text: "Read both documents".into(),
            }]),
            LlmMessage::Assistant(vec![
                LlmOutputBlock::Text {
                    text: "I will read both.".into(),
                },
                LlmOutputBlock::ToolCall {
                    id: "call-1".into(),
                    name: "read_document".into(),
                    arguments_json: r#"{"documentId":"doc-1"}"#.into(),
                },
                LlmOutputBlock::ToolCall {
                    id: "call-2".into(),
                    name: "read_document".into(),
                    arguments_json: r#"{"documentId":"doc-2"}"#.into(),
                },
            ]),
            LlmMessage::User(vec![
                LlmInputBlock::Text {
                    text: "Continue with what is available".into(),
                },
                LlmInputBlock::ToolResult {
                    tool_call_id: "call-1".into(),
                    payload_json: r#"{"title":"Example"}"#.into(),
                    is_error: false,
                },
                LlmInputBlock::ToolResult {
                    tool_call_id: "call-2".into(),
                    payload_json: r#"{"error":{"code":"DOCUMENT_NOT_FOUND"}}"#.into(),
                    is_error: true,
                },
            ]),
        ],
        tools: vec![tool()],
        tool_choice: LlmToolChoice::Auto,
        max_output_tokens: Some(256),
    }
}

fn simple_request() -> LlmRequest {
    LlmRequest {
        instructions: Some("Answer briefly.".into()),
        thinking: None,
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "Hello".into(),
        }])],
        tools: Vec::new(),
        tool_choice: LlmToolChoice::None,
        max_output_tokens: Some(64),
    }
}

fn probe_tool() -> LlmToolDefinition {
    LlmToolDefinition {
        name: "meetnote_probe".into(),
        description: "Return the supplied probe value".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false
        }),
    }
}

fn tool_round_request() -> LlmRequest {
    LlmRequest {
        instructions: Some("Call meetnote_probe exactly once with value ready.".into()),
        thinking: None,
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "Read the document and report the result.".into(),
        }])],
        tools: vec![probe_tool()],
        tool_choice: LlmToolChoice::Required,
        max_output_tokens: Some(64),
    }
}

fn corrected_request(first_output: &[LlmOutputBlock]) -> LlmRequest {
    LlmRequest {
        instructions: Some("Use the tool error to correct the answer.".into()),
        thinking: None,
        messages: vec![
            LlmMessage::User(vec![LlmInputBlock::Text {
                text: "Read the document and correct any tool failure.".into(),
            }]),
            LlmMessage::Assistant(first_output.to_vec()),
            LlmMessage::User(vec![LlmInputBlock::ToolResult {
                tool_call_id: "call-probe".into(),
                payload_json: r#"{"error":{"code":"DOCUMENT_NOT_FOUND"}}"#.into(),
                is_error: true,
            }]),
        ],
        tools: vec![probe_tool()],
        tool_choice: LlmToolChoice::None,
        max_output_tokens: Some(64),
    }
}

fn connection(protocol: LlmProtocol, base_url: String, timeout_ms: u64) -> ConnectionConfig {
    ConnectionConfig {
        id: "fixture".into(),
        protocol,
        base_url,
        model: "route-test".into(),
        auth_mode: if protocol == LlmProtocol::AnthropicMessages {
            AuthMode::XApiKey
        } else {
            AuthMode::Bearer
        },
        timeout_ms,
        context_window_tokens: Some(8_192),
        max_output_tokens: 512,
    }
}

fn protocol_path(protocol: LlmProtocol) -> &'static str {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => "/gateway/v1/chat/completions",
        LlmProtocol::AnthropicMessages => "/gateway/v1/messages",
        LlmProtocol::OpenAiResponses => "/gateway/v1/responses",
    }
}

fn assert_text(output: &[LlmOutputBlock], expected: &str) {
    assert_eq!(
        output,
        &[LlmOutputBlock::Text {
            text: expected.into()
        }]
    );
}

#[test]
fn protocol_golden_fixtures_are_canonical_and_equivalent() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let adapter = adapter(protocol);
        let fixture = fixture(protocol);
        let encoded = adapter.encode(&golden_request(), "route-test").unwrap();
        assert_eq!(encoded, fixture["expectedRequest"], "{protocol:?}");

        let text = adapter
            .decode(fixture["text"].clone(), "route-test")
            .unwrap();
        assert_text(&text.blocks, "Done");
        assert_eq!(text.stop_reason, LlmStopReason::Complete);
        assert_eq!(text.requested_model, "route-test");
        assert_eq!(text.resolved_model, "resolved-test");
        assert_eq!(text.usage.unwrap().total_tokens, 14);

        let tools = adapter
            .decode(fixture["multiTool"].clone(), "route-test")
            .unwrap();
        assert_eq!(tools.stop_reason, LlmStopReason::ToolUse);
        assert_eq!(tools.blocks.len(), 2);
        assert!(matches!(
            &tools.blocks[0],
            LlmOutputBlock::ToolCall { id, name, arguments_json }
                if id == "call-a" && name == "read_document"
                    && arguments_json == r#"{"documentId":"doc-a"}"#
        ));
        assert!(matches!(
            &tools.blocks[1],
            LlmOutputBlock::ToolCall { id, name, arguments_json }
                if id == "call-b" && name == "read_document"
                    && arguments_json == r#"{"documentId":"doc-b"}"#
        ));
        assert_eq!(tools.usage.unwrap().total_tokens, 18);

        let truncated = adapter
            .decode(fixture["truncated"].clone(), "route-test")
            .unwrap();
        assert_eq!(truncated.stop_reason, LlmStopReason::MaxTokens);
        assert!(truncated.blocks.is_empty());

        let malformed = adapter.decode(json!({"unexpected": true}), "route-test");
        assert_eq!(malformed.unwrap_err().kind, LlmErrorKind::ResponseInvalid);
    }
}

#[test]
fn gateway_cost_is_optional_and_rejects_invalid_values() {
    for (header, expected) in [
        ("0.001234", Some(1234)),
        ("0", Some(0)),
        ("-1", None),
        ("NaN", None),
        ("inf", None),
        ("1e30", None),
    ] {
        let protocol = LlmProtocol::OpenAiChatCompletions;
        let mut response = ResponseSpec::json(fixture(protocol)["text"].clone());
        response
            .headers
            .push(("x-litellm-response-cost-original".into(), header.into()));
        let (base, captured, server) = spawn_server(vec![response]);
        let provider = LiteLlmProvider::new(
            connection(protocol, base, 2_000),
            Some(Credential::new("fixture-secret")),
        )
        .unwrap();
        let output = provider
            .complete(&golden_request(), &LlmCancellation::new())
            .unwrap();
        assert_eq!(output.usage.unwrap().cost_microusd, expected, "{header}");
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn http_client_preserves_custom_path_port_auth_and_protocol_payload() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let fixture = fixture(protocol);
        let (base, captured, server) =
            spawn_server(vec![ResponseSpec::json(fixture["text"].clone())]);
        let provider = LiteLlmProvider::new(
            connection(protocol, format!("{base}/gateway/v1/"), 2_000),
            Some(Credential::new("fixture-secret")),
        )
        .unwrap();

        let output = provider
            .complete(&golden_request(), &LlmCancellation::new())
            .unwrap();
        assert_text(&output.blocks, "Done");
        let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, protocol_path(protocol));
        assert_eq!(request.body, fixture["expectedRequest"]);
        assert!(request
            .headers
            .get("content-type")
            .is_some_and(|value| value.starts_with("application/json")));
        match protocol {
            LlmProtocol::AnthropicMessages => {
                assert_eq!(
                    request.headers.get("x-api-key").map(String::as_str),
                    Some("fixture-secret")
                );
                assert_eq!(
                    request.headers.get("anthropic-version").map(String::as_str),
                    Some("2023-06-01")
                );
                assert!(!request.headers.contains_key("authorization"));
            }
            _ => {
                assert_eq!(
                    request.headers.get("authorization").map(String::as_str),
                    Some("Bearer fixture-secret")
                );
                assert!(!request.headers.contains_key("x-api-key"));
            }
        }
        server.join().unwrap();
    }
}

#[test]
fn probe_checks_readiness_discovery_text_and_tool_result_round_trip() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let text = fixture(protocol)["text"].clone();
        let tool_call = probe_tool_response(protocol);
        let (base, captured, server) = spawn_server(vec![
            ResponseSpec::json(json!({"status": "healthy"})),
            ResponseSpec::json(json!({
                "data": [{"id": "route-z"}, {"id": "route-test"}, {"id": "route-test"}]
            })),
            ResponseSpec::json(text.clone()),
            ResponseSpec::json(tool_call),
            ResponseSpec::json(text),
        ]);
        let provider = LiteLlmProvider::new(
            connection(protocol, format!("{base}/gateway/v1"), 2_000),
            Some(Credential::new("fixture-secret")),
        )
        .unwrap();

        let probe = provider.probe(&LlmCancellation::new()).unwrap();
        assert!(probe.ready);
        assert_eq!(
            probe.models,
            Some(vec!["route-test".into(), "route-z".into()])
        );
        assert_eq!(probe.requested_model, "route-test");
        assert_eq!(probe.resolved_model.as_deref(), Some("resolved-test"));
        assert!(probe.capabilities.text);
        assert!(probe.capabilities.client_tools);

        let requests = (0..5)
            .map(|_| captured.recv_timeout(Duration::from_secs(1)).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/gateway/health/readiness");
        assert_eq!(requests[1].path, "/gateway/v1/models");
        assert_eq!(requests[2].path, protocol_path(protocol));
        assert_eq!(requests[3].path, protocol_path(protocol));
        assert_eq!(requests[4].path, protocol_path(protocol));
        assert!(request_requires_tool(protocol, &requests[3].body));
        assert!(request_contains_tool_result(protocol, &requests[4].body));
        if protocol == LlmProtocol::OpenAiResponses {
            assert_eq!(requests[4].body["store"], false);
            assert!(!contains_string(&requests[4].body, "output_text"));
            assert!(contains_string(&requests[4].body, "input_text"));
        }
        server.join().unwrap();
    }
}

fn probe_tool_response(protocol: LlmProtocol) -> Value {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => json!({
            "model": "resolved-test",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "probe-call", "type": "function", "function": {
                        "name": "meetnote_probe", "arguments": "{\"value\":\"ready\"}"
                    }
                }]}
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 4}
        }),
        LlmProtocol::AnthropicMessages => json!({
            "model": "resolved-test",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use", "id": "probe-call", "name": "meetnote_probe",
                "input": {"value": "ready"}
            }],
            "usage": {"input_tokens": 8, "output_tokens": 4}
        }),
        LlmProtocol::OpenAiResponses => json!({
            "model": "resolved-test",
            "status": "completed",
            "output": [{
                "type": "function_call", "call_id": "probe-call", "name": "meetnote_probe",
                "arguments": "{\"value\":\"ready\"}", "status": "completed"
            }],
            "usage": {"input_tokens": 8, "output_tokens": 4, "total_tokens": 12}
        }),
    }
}

fn parallel_probe_tool_response(protocol: LlmProtocol) -> Value {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => json!({
            "model": "resolved-test",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "probe-a", "type": "function", "function": {
                        "name": "meetnote_probe", "arguments": "{\"value\":\"ready\"}"
                    }},
                    {"id": "probe-b", "type": "function", "function": {
                        "name": "meetnote_probe", "arguments": "{\"value\":\"ready\"}"
                    }}
                ]}
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 8}
        }),
        LlmProtocol::AnthropicMessages => json!({
            "model": "resolved-test",
            "stop_reason": "tool_use",
            "content": [
                {"type": "tool_use", "id": "probe-a", "name": "meetnote_probe",
                 "input": {"value": "ready"}},
                {"type": "tool_use", "id": "probe-b", "name": "meetnote_probe",
                 "input": {"value": "ready"}}
            ],
            "usage": {"input_tokens": 8, "output_tokens": 8}
        }),
        LlmProtocol::OpenAiResponses => json!({
            "model": "resolved-test",
            "status": "completed",
            "output": [
                {"type": "function_call", "call_id": "probe-a", "name": "meetnote_probe",
                 "arguments": "{\"value\":\"ready\"}", "status": "completed"},
                {"type": "function_call", "call_id": "probe-b", "name": "meetnote_probe",
                 "arguments": "{\"value\":\"ready\"}", "status": "completed"}
            ],
            "usage": {"input_tokens": 8, "output_tokens": 8, "total_tokens": 16}
        }),
    }
}

fn malformed_tool_response(protocol: LlmProtocol) -> Value {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => json!({
            "choices": [{"finish_reason": "tool_calls", "message": {
                "tool_calls": [{"id": "call-bad", "type": "function", "function": {
                    "name": "read_document", "arguments": "{"
                }}]
            }}]
        }),
        LlmProtocol::AnthropicMessages => json!({
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "id": "call-bad",
                "name": "read_document", "input": []}]
        }),
        LlmProtocol::OpenAiResponses => json!({
            "status": "completed",
            "output": [{"type": "function_call", "call_id": "call-bad",
                "name": "read_document", "arguments": "{"}]
        }),
    }
}

fn request_requires_tool(protocol: LlmProtocol, body: &Value) -> bool {
    match protocol {
        LlmProtocol::OpenAiChatCompletions | LlmProtocol::OpenAiResponses => {
            body["tool_choice"] == "required"
        }
        LlmProtocol::AnthropicMessages => body["tool_choice"]["type"] == "any",
    }
}

fn request_contains_tool_result(protocol: LlmProtocol, body: &Value) -> bool {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool" && message["tool_call_id"] == "probe-call"),
        LlmProtocol::AnthropicMessages => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .any(|block| block["type"] == "tool_result" && block["tool_use_id"] == "probe-call"),
        LlmProtocol::OpenAiResponses => {
            body["input"].as_array().unwrap().iter().any(|item| {
                item["type"] == "function_call_output" && item["call_id"] == "probe-call"
            })
        }
    }
}

fn tool_result_count(protocol: LlmProtocol, body: &Value) -> usize {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .count(),
        LlmProtocol::AnthropicMessages => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|block| block["type"] == "tool_result")
            .count(),
        LlmProtocol::OpenAiResponses => body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .count(),
    }
}

fn contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value == needle,
        Value::Array(values) => values.iter().any(|value| contains_string(value, needle)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key == needle || contains_string(value, needle)),
        _ => false,
    }
}

#[test]
fn http_statuses_and_retry_hints_map_to_stable_error_kinds() {
    let cases = [
        (
            401,
            "context length exceeded",
            LlmErrorKind::Authentication,
            false,
        ),
        (
            403,
            "unknown model route",
            LlmErrorKind::Authentication,
            false,
        ),
        (
            404,
            "context length exceeded",
            LlmErrorKind::CapabilityUnsupported,
            false,
        ),
        (
            400,
            "unknown model route",
            LlmErrorKind::ResponseInvalid,
            false,
        ),
        (
            413,
            "arbitrary provider text",
            LlmErrorKind::ContextLimitExceeded,
            false,
        ),
        (
            422,
            "context length exceeded",
            LlmErrorKind::ResponseInvalid,
            false,
        ),
        (
            429,
            "arbitrary provider text",
            LlmErrorKind::RateLimited,
            true,
        ),
        (
            503,
            "arbitrary provider text",
            LlmErrorKind::Transport,
            true,
        ),
    ];

    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        for (status, message, expected, retryable) in cases {
            let mut response = ResponseSpec::status(status, json!({"error": {"message": message}}));
            if status == 429 {
                response.headers.push(("Retry-After".into(), "2".into()));
            }
            let (base, captured, server) = spawn_server(vec![response]);
            let provider = LiteLlmProvider::new(
                connection(protocol, base, 2_000),
                Some(Credential::new("fixture-secret")),
            )
            .unwrap();

            let error = provider
                .complete(&simple_request(), &LlmCancellation::new())
                .unwrap_err();
            assert_eq!(error.kind, expected, "{protocol:?} HTTP {status}");
            assert_eq!(error.retryable, retryable, "{protocol:?} HTTP {status}");
            assert_eq!(error.status, Some(status));
            if status == 429 {
                assert_eq!(error.retry_after_ms, Some(2_000));
            }
            captured.recv_timeout(Duration::from_secs(1)).unwrap();
            server.join().unwrap();
        }
    }
}

#[test]
fn provider_supports_bounded_tool_round_and_error_correction_for_each_protocol() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) = spawn_server(vec![
            ResponseSpec::json(probe_tool_response(protocol)),
            ResponseSpec::json(fixture(protocol)["text"].clone()),
        ]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();

        let first = provider
            .complete(&tool_round_request(), &LlmCancellation::new())
            .unwrap();
        assert_eq!(first.stop_reason, LlmStopReason::ToolUse, "{protocol:?}");
        let second = provider
            .complete(&corrected_request(&first.blocks), &LlmCancellation::new())
            .unwrap();
        assert_text(&second.blocks, "Done");

        let _first_request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
        let correction_request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_tool_error_round(protocol, &correction_request.body);
        server.join().unwrap();
    }
}

fn assert_tool_error_round(protocol: LlmProtocol, body: &Value) {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => {
            assert!(body["messages"].as_array().unwrap().iter().any(|message| {
                message["role"] == "tool"
                    && message["tool_call_id"] == "call-probe"
                    && message["content"] == r#"{"error":{"code":"DOCUMENT_NOT_FOUND"}}"#
            }))
        }
        LlmProtocol::AnthropicMessages => assert!(body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .any(|block| {
                block["type"] == "tool_result"
                    && block["tool_use_id"] == "call-probe"
                    && block["is_error"] == true
            })),
        LlmProtocol::OpenAiResponses => {
            assert!(body["input"].as_array().unwrap().iter().any(|item| {
                item["type"] == "function_call_output"
                    && item["call_id"] == "call-probe"
                    && item["output"] == r#"{"error":{"code":"DOCUMENT_NOT_FOUND"}}"#
            }))
        }
    }
}

#[test]
fn malformed_http_responses_are_rejected_for_each_protocol() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) =
            spawn_server(vec![ResponseSpec::json(json!({"unexpected": true}))]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();
        let error = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid, "{protocol:?}");
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn provider_decodes_multi_tool_calls_through_the_http_boundary() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) = spawn_server(vec![ResponseSpec::json(
            fixture(protocol)["multiTool"].clone(),
        )]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();

        let output = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap();
        assert_eq!(output.stop_reason, LlmStopReason::ToolUse);
        assert_eq!(
            output
                .blocks
                .iter()
                .filter(|block| matches!(block, LlmOutputBlock::ToolCall { .. }))
                .count(),
            2
        );
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn provider_rejects_malformed_tool_calls_through_the_http_boundary() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) =
            spawn_server(vec![ResponseSpec::json(malformed_tool_response(protocol))]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();

        let error = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid, "{protocol:?}");
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn probe_keeps_manual_alias_when_discovery_fails_and_answers_parallel_tools() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let text = fixture(protocol)["text"].clone();
        let (base, captured, server) = spawn_server(vec![
            ResponseSpec::json(json!({"status": "healthy"})),
            ResponseSpec::status(404, json!({"error": {"message": "discovery disabled"}})),
            ResponseSpec::json(text.clone()),
            ResponseSpec::json(parallel_probe_tool_response(protocol)),
            ResponseSpec::json(text),
        ]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();

        let probe = provider.probe(&LlmCancellation::new()).unwrap();
        assert_eq!(probe.models, None);
        assert_eq!(probe.requested_model, "route-test");
        assert_eq!(probe.resolved_model.as_deref(), Some("resolved-test"));
        assert!(probe.capabilities.text);
        assert!(probe.capabilities.client_tools);

        let requests = (0..5)
            .map(|_| captured.recv_timeout(Duration::from_secs(1)).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(requests[1].path, "/v1/models");
        assert_eq!(tool_result_count(protocol, &requests[4].body), 2);
        server.join().unwrap();
    }
}

#[test]
fn incomplete_or_inconsistent_tool_responses_are_never_executable() {
    let cases: Vec<(Box<dyn ProtocolAdapter>, Value)> = vec![
        (
            Box::new(OpenAiChatCompletionsAdapter),
            json!({
                "choices": [{"finish_reason": "length", "message": {
                    "tool_calls": [{"id": "call-1", "function": {
                        "name": "read_document", "arguments": "{}"
                    }}]
                }}]
            }),
        ),
        (
            Box::new(AnthropicMessagesAdapter),
            json!({
                "stop_reason": "max_tokens",
                "content": [{"type": "tool_use", "id": "call-1",
                    "name": "read_document", "input": {}}]
            }),
        ),
        (
            Box::new(OpenAiResponsesAdapter),
            json!({
                "status": "incomplete",
                "incomplete_details": {"reason": "max_output_tokens"},
                "output": [{"type": "function_call", "call_id": "call-1",
                    "name": "read_document", "arguments": "{}", "status": "completed"}]
            }),
        ),
    ];

    for (adapter, response) in cases {
        let error = adapter.decode(response, "route-test").unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid);
    }

    let response = json!({
        "status": "completed",
        "output": [{"type": "function_call", "call_id": "call-1",
            "name": "read_document", "arguments": "{}", "status": "in_progress"}]
    });
    assert_eq!(
        OpenAiResponsesAdapter
            .decode(response, "route-test")
            .unwrap_err()
            .kind,
        LlmErrorKind::ResponseInvalid
    );
}

#[test]
fn connection_limits_are_explicit_and_enforced_before_http() {
    let configured = connection(
        LlmProtocol::OpenAiChatCompletions,
        "http://127.0.0.1:9".into(),
        2_000,
    );
    assert_eq!(configured.input_token_budget().unwrap(), 7_680);

    let mut unknown = configured.clone();
    unknown.context_window_tokens = None;
    assert_eq!(
        unknown.input_token_budget().unwrap_err().kind,
        LlmErrorKind::ContextLimitUnknown
    );
    assert!(LiteLlmProvider::new(unknown, None).is_ok());

    let mut oversized_request = simple_request();
    oversized_request.max_output_tokens = Some(configured.max_output_tokens + 1);
    let provider = LiteLlmProvider::new(configured, None).unwrap();
    assert_eq!(
        provider
            .complete(&oversized_request, &LlmCancellation::new())
            .unwrap_err()
            .kind,
        LlmErrorKind::RequestInvalid
    );
}

#[test]
fn malformed_http_body_is_rejected() {
    let (base, captured, server) = spawn_server(vec![ResponseSpec {
        status: 200,
        headers: Vec::new(),
        body: "{".into(),
        delay: Duration::ZERO,
    }]);
    let provider = LiteLlmProvider::new(
        connection(LlmProtocol::OpenAiChatCompletions, base, 2_000),
        None,
    )
    .unwrap();
    let error = provider
        .complete(&simple_request(), &LlmCancellation::new())
        .unwrap_err();
    assert_eq!(error.kind, LlmErrorKind::ResponseInvalid);
    captured.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();
}

#[test]
fn oversized_http_bodies_are_rejected_before_protocol_decode() {
    let body = format!(r#"{{"padding":"{}"}}"#, "x".repeat(4 * 1024 * 1024));
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) = spawn_server(vec![ResponseSpec {
            status: 200,
            headers: Vec::new(),
            body: body.clone(),
            delay: Duration::ZERO,
        }]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();
        let error = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid, "{protocol:?}");
        assert!(error.message.contains("maximum supported size"));
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn request_timeout_is_classified_without_accepting_late_output() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) = spawn_server(vec![ResponseSpec {
            status: 200,
            headers: Vec::new(),
            body: fixture(protocol)["text"].to_string(),
            delay: Duration::from_millis(150),
        }]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 20), None).unwrap();
        let error = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Timeout, "{protocol:?}");
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn cancellation_interrupts_waiting_without_accepting_late_output() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let (base, captured, server) = spawn_server(vec![ResponseSpec {
            status: 200,
            headers: Vec::new(),
            body: fixture(protocol)["text"].to_string(),
            delay: Duration::from_millis(250),
        }]);
        let provider = LiteLlmProvider::new(connection(protocol, base, 2_000), None).unwrap();
        let cancellation = LlmCancellation::new();
        let cancel_from_thread = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            cancel_from_thread.cancel();
        });
        let started = Instant::now();
        let error = provider
            .complete(&simple_request(), &cancellation)
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Cancelled, "{protocol:?}");
        assert!(started.elapsed() < Duration::from_millis(150));
        captured.recv_timeout(Duration::from_secs(1)).unwrap();
        canceller.join().unwrap();
        server.join().unwrap();
    }
}

#[test]
fn redirects_are_rejected_without_forwarding_credentials() {
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url = format!("http://{}", target.local_addr().unwrap());
        let mut redirect = ResponseSpec::status(307, json!({"redirect": true}));
        redirect
            .headers
            .push(("Location".into(), format!("{target_url}/stolen")));
        let (base, captured, server) = spawn_server(vec![redirect]);
        let provider = LiteLlmProvider::new(
            connection(protocol, base, 2_000),
            Some(Credential::new("must-not-leak")),
        )
        .unwrap();

        let error = provider
            .complete(&simple_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid, "{protocol:?}");
        assert_eq!(error.status, Some(307));
        let source_request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
        let (header, value) = match protocol {
            LlmProtocol::AnthropicMessages => ("x-api-key", "must-not-leak"),
            LlmProtocol::OpenAiChatCompletions | LlmProtocol::OpenAiResponses => {
                ("authorization", "Bearer must-not-leak")
            }
        };
        assert_eq!(
            source_request.headers.get(header).map(String::as_str),
            Some(value)
        );
        thread::sleep(Duration::from_millis(30));
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        server.join().unwrap();
    }
}

#[test]
fn unsafe_header_values_and_remote_plaintext_connections_are_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = LiteLlmProvider::new(
        connection(
            LlmProtocol::OpenAiChatCompletions,
            format!("http://{}", listener.local_addr().unwrap()),
            2_000,
        ),
        Some(Credential::new("secret\r\nX-Injected: yes")),
    )
    .unwrap();
    let error = provider
        .complete(&simple_request(), &LlmCancellation::new())
        .unwrap_err();
    assert_eq!(error.kind, LlmErrorKind::Authentication);
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));

    let remote = LiteLlmProvider::new(
        connection(
            LlmProtocol::OpenAiResponses,
            "http://example.com:4321/proxy".into(),
            2_000,
        ),
        None,
    );
    assert!(matches!(
        remote,
        Err(error) if error.kind == LlmErrorKind::ConnectionNotConfigured
    ));
    assert!(LiteLlmProvider::new(
        connection(
            LlmProtocol::OpenAiResponses,
            "https://example.com:4321/proxy".into(),
            2_000,
        ),
        None,
    )
    .is_ok());
}

#[test]
fn canonical_validation_rejects_bad_json_before_http() {
    let mut request = simple_request();
    request
        .messages
        .push(LlmMessage::Assistant(vec![LlmOutputBlock::ToolCall {
            id: "call-1".into(),
            name: "read_document".into(),
            arguments_json: "{".into(),
        }]));
    request.tools.push(tool());

    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let error = adapter(protocol)
            .encode(&request, "route-test")
            .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::RequestInvalid);
    }
}
