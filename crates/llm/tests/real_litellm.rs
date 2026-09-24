use llm::{
    AuthMode, ConnectionConfig, Credential, LiteLlmProvider, LlmCancellation, LlmInputBlock,
    LlmMessage, LlmOutputBlock, LlmProtocol, LlmProvider, LlmRequest, LlmToolChoice,
    LlmToolDefinition,
};
use serde_json::json;
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROXY_SECRET: &str = "sk-meetnote-proxy-canary";
const UPSTREAM_SECRET: &str = "sk-meetnote-upstream-canary";
const PATH_CANARY: &str = "/Users/private/meetnote-canary";

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct TempGuard(PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "fixture proxy only: requires the local LiteLLM executable and local fake upstream"]
fn litellm_fixture_proxy_routes_three_protocols_and_preserves_fallback_tools() {
    require_explicit_gate(
        "MEETNOTE_RUN_LITELLM_FIXTURE",
        "set MEETNOTE_RUN_LITELLM_FIXTURE=1 to run the local LiteLLM fixture",
    );
    let litellm = std::env::var("MEETNOTE_LITELLM_BIN")
        .unwrap_or_else(|_| "/opt/miniconda3/envs/default/bin/litellm".into());
    assert!(
        Path::new(&litellm).is_file(),
        "LiteLLM executable is missing"
    );

    let root = std::env::temp_dir().join(format!(
        "meetnote-real-litellm-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let _temp = TempGuard(root.clone());
    let upstream_port = free_port();
    let proxy_port = free_port();
    let upstream_log = root.join("upstream.jsonl");
    let upstream_script = root.join("upstream.py");
    fs::write(&upstream_script, UPSTREAM_SERVER).unwrap();
    let upstream_stdout = File::create(root.join("upstream.stdout.log")).unwrap();
    let upstream_stderr = File::create(root.join("upstream.stderr.log")).unwrap();
    let upstream = Command::new("python3")
        .args([
            upstream_script.to_str().unwrap(),
            &upstream_port.to_string(),
            upstream_log.to_str().unwrap(),
        ])
        .stdout(Stdio::from(upstream_stdout))
        .stderr(Stdio::from(upstream_stderr))
        .spawn()
        .unwrap();
    let _upstream = ChildGuard { child: upstream };

    let config = root.join("litellm.yaml");
    fs::write(
        &config,
        format!(
            r#"model_list:
  - model_name: primary-route
    litellm_params:
      model: openai/fixture-primary
      api_base: http://127.0.0.1:{upstream_port}/v1
      api_key: {UPSTREAM_SECRET}
  - model_name: fallback-entry
    litellm_params:
      model: openai/fixture-fail
      api_base: http://127.0.0.1:{upstream_port}/v1
      api_key: {UPSTREAM_SECRET}
  - model_name: fallback-target
    litellm_params:
      model: openai/fixture-fallback
      api_base: http://127.0.0.1:{upstream_port}/v1
      api_key: {UPSTREAM_SECRET}

router_settings:
  num_retries: 0
  fallbacks:
    - fallback-entry: [fallback-target]

general_settings:
  master_key: {PROXY_SECRET}
  disable_spend_logs: true

litellm_settings:
  telemetry: false
"#
        ),
    )
    .unwrap();
    let proxy_stdout_path = root.join("litellm.stdout.log");
    let proxy_stderr_path = root.join("litellm.stderr.log");
    let proxy = Command::new(&litellm)
        .args([
            "--config",
            config.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &proxy_port.to_string(),
        ])
        .env("SERVER_ROOT_PATH", "/gateway")
        .env("LITELLM_TELEMETRY", "False")
        .stdout(Stdio::from(File::create(&proxy_stdout_path).unwrap()))
        .stderr(Stdio::from(File::create(&proxy_stderr_path).unwrap()))
        .spawn()
        .unwrap();
    let _proxy = ChildGuard { child: proxy };

    let base_url = format!("http://127.0.0.1:{proxy_port}/gateway");
    let cancellation = LlmCancellation::new();
    wait_until_ready(&base_url, &cancellation);
    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let provider = provider(protocol, &base_url, "primary-route");
        let text = provider.complete(&text_request(), &cancellation).unwrap();
        assert_eq!(text.requested_model, "primary-route");
        assert!(!text.resolved_model.is_empty());
        assert!(matches!(
            text.blocks.as_slice(),
            [LlmOutputBlock::Text { text }] if text == "fixture text"
        ));
        let tool = provider.complete(&tool_request(), &cancellation).unwrap();
        assert!(matches!(
            tool.blocks.as_slice(),
            [LlmOutputBlock::ToolCall { name, arguments_json, .. }]
                if name == "meetnote_probe" && arguments_json.contains("ready")
        ));
    }

    let fallback = provider(
        LlmProtocol::OpenAiChatCompletions,
        &base_url,
        "fallback-entry",
    )
    .complete(&tool_request(), &cancellation)
    .unwrap();
    assert!(matches!(
        fallback.blocks.as_slice(),
        [LlmOutputBlock::ToolCall { name, .. }] if name == "meetnote_probe"
    ));

    thread::sleep(Duration::from_millis(200));
    let upstream = fs::read_to_string(&upstream_log).unwrap();
    assert!(upstream.contains("fixture-primary"));
    assert!(upstream.contains("fixture-fail"));
    assert!(upstream.contains("fixture-fallback"));
    assert!(upstream.contains(UPSTREAM_SECRET));
    assert!(!upstream.contains(PROXY_SECRET));
    assert!(!upstream.contains(PATH_CANARY));
    let proxy_logs = format!(
        "{}\n{}",
        fs::read_to_string(proxy_stdout_path).unwrap(),
        fs::read_to_string(proxy_stderr_path).unwrap()
    );
    assert!(!proxy_logs.contains(PROXY_SECRET));
    assert!(!proxy_logs.contains(UPSTREAM_SECRET));
    assert!(!proxy_logs.contains(PATH_CANARY));
}

#[test]
#[ignore = "real upstream only: requires explicit external endpoint, model, and credential"]
fn real_upstream_three_protocols_text_and_tools() {
    require_explicit_gate(
        "MEETNOTE_RUN_REAL_LLM",
        "set MEETNOTE_RUN_REAL_LLM=1 to run real upstream calls",
    );
    let base_url = std::env::var("MEETNOTE_REAL_LLM_BASE_URL")
        .expect("set MEETNOTE_REAL_LLM_BASE_URL to run the real-upstream test");
    let model = std::env::var("MEETNOTE_REAL_LLM_MODEL")
        .expect("set MEETNOTE_REAL_LLM_MODEL to run the real-upstream test");
    let credential = std::env::var("MEETNOTE_REAL_LLM_API_KEY")
        .expect("set MEETNOTE_REAL_LLM_API_KEY to run the real-upstream test");
    let cancellation = LlmCancellation::new();

    for protocol in [
        LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::AnthropicMessages,
        LlmProtocol::OpenAiResponses,
    ] {
        let protocol_base_url = protocol_base_url(protocol, &base_url);
        let provider = configured_provider(
            "real-upstream",
            &protocol_base_url,
            &model,
            protocol,
            auth_mode(protocol),
            &credential,
        );
        let text = provider.complete(&text_request(), &cancellation).unwrap();
        assert!(
            text.blocks
                .iter()
                .any(|block| matches!(block, LlmOutputBlock::Text { text } if !text.is_empty())),
            "{protocol:?} real upstream returned no text"
        );
        // DeepSeek reasoning mode rejects forced tool choice; the real upstream
        // path uses auto and verifies that the requested tool is still emitted.
        let tool = provider
            .complete(
                &tool_request_with_choice(LlmToolChoice::Auto),
                &cancellation,
            )
            .unwrap();
        assert!(
            tool.blocks.iter().any(|block| matches!(
                block,
                LlmOutputBlock::ToolCall { name, .. } if name == "meetnote_probe"
            )),
            "{protocol:?} real upstream returned no probe tool call"
        );
    }
}

fn require_explicit_gate(variable: &str, message: &str) {
    assert_eq!(
        std::env::var(variable).as_deref(),
        Ok("1"),
        "{message}; ignored tests never provide real-model evidence by default"
    );
}

fn auth_mode(protocol: LlmProtocol) -> AuthMode {
    let variable = match protocol {
        LlmProtocol::OpenAiChatCompletions => "MEETNOTE_REAL_LLM_OPENAI_AUTH_MODE",
        LlmProtocol::AnthropicMessages => "MEETNOTE_REAL_LLM_ANTHROPIC_AUTH_MODE",
        LlmProtocol::OpenAiResponses => "MEETNOTE_REAL_LLM_RESPONSES_AUTH_MODE",
    };
    std::env::var(variable)
        .or_else(|_| std::env::var("MEETNOTE_REAL_LLM_AUTH_MODE"))
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "bearer" => AuthMode::Bearer,
            "x-api-key" | "xapikey" => AuthMode::XApiKey,
            other => {
                panic!("{variable} must be bearer or x-api-key, got unsupported value {other:?}")
            }
        })
        .unwrap_or(match protocol {
            LlmProtocol::AnthropicMessages => AuthMode::XApiKey,
            LlmProtocol::OpenAiChatCompletions | LlmProtocol::OpenAiResponses => AuthMode::Bearer,
        })
}

fn protocol_base_url(protocol: LlmProtocol, fallback: &str) -> String {
    let variable = match protocol {
        LlmProtocol::OpenAiChatCompletions => "MEETNOTE_REAL_LLM_OPENAI_BASE_URL",
        LlmProtocol::AnthropicMessages => "MEETNOTE_REAL_LLM_ANTHROPIC_BASE_URL",
        LlmProtocol::OpenAiResponses => "MEETNOTE_REAL_LLM_RESPONSES_BASE_URL",
    };
    std::env::var(variable).unwrap_or_else(|_| fallback.to_owned())
}

fn provider(protocol: LlmProtocol, base_url: &str, model: &str) -> LiteLlmProvider {
    configured_provider(
        "real-litellm",
        base_url,
        model,
        protocol,
        AuthMode::Bearer,
        PROXY_SECRET,
    )
}

fn configured_provider(
    id: &str,
    base_url: &str,
    model: &str,
    protocol: LlmProtocol,
    auth_mode: AuthMode,
    credential: &str,
) -> LiteLlmProvider {
    LiteLlmProvider::new(
        ConnectionConfig {
            id: id.into(),
            protocol,
            base_url: base_url.into(),
            model: model.into(),
            auth_mode,
            timeout_ms: 10_000,
            context_window_tokens: Some(8_192),
            max_output_tokens: 256,
        },
        Some(Credential::new(credential)),
    )
    .unwrap()
}

fn text_request() -> LlmRequest {
    LlmRequest {
        instructions: Some("Return fixture text. Never use local paths.".into()),
        thinking: Some(false),
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "hello".into(),
        }])],
        tools: Vec::new(),
        tool_choice: LlmToolChoice::None,
        max_output_tokens: Some(64),
    }
}

fn tool_request() -> LlmRequest {
    tool_request_with_choice(LlmToolChoice::Required)
}

fn tool_request_with_choice(tool_choice: LlmToolChoice) -> LlmRequest {
    LlmRequest {
        instructions: Some("Call meetnote_probe with value ready.".into()),
        thinking: Some(false),
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "use tool".into(),
        }])],
        tools: vec![LlmToolDefinition {
            name: "meetnote_probe".into(),
            description: "probe".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false
            }),
        }],
        tool_choice,
        max_output_tokens: Some(64),
    }
}

fn wait_until_ready(base_url: &str, cancellation: &LlmCancellation) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if provider(
            LlmProtocol::OpenAiChatCompletions,
            base_url,
            "primary-route",
        )
        .readiness(cancellation)
        .is_ok()
        {
            return;
        }
        assert!(Instant::now() < deadline, "LiteLLM did not become ready");
        thread::sleep(Duration::from_millis(200));
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

const UPSTREAM_SERVER: &str = r#"
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port = int(sys.argv[1])
log_path = sys.argv[2]

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get('content-length', '0'))
        body = json.loads(self.rfile.read(length) or b'{}')
        with open(log_path, 'a', encoding='utf-8') as log:
            log.write(json.dumps({
                'path': self.path,
                'authorization': self.headers.get('authorization'),
                'body': body,
            }, sort_keys=True) + '\n')
        if body.get('model') == 'fixture-fail':
            self.respond(500, {'error': {'message': 'intentional primary failure'}})
            return
        tools = body.get('tools') or []
        name = None
        if tools:
            first = tools[0]
            name = (first.get('function') or {}).get('name') or first.get('name')
        if self.path.endswith('/chat/completions'):
            message = {'role': 'assistant', 'content': 'fixture text'}
            finish = 'stop'
            if name:
                message = {
                    'role': 'assistant',
                    'content': None,
                    'tool_calls': [{
                        'id': 'call-probe',
                        'type': 'function',
                        'function': {'name': name, 'arguments': '{"value":"ready"}'},
                    }],
                }
                finish = 'tool_calls'
            self.respond(200, {
                'id': 'chatcmpl-fixture',
                'object': 'chat.completion',
                'created': 1,
                'model': 'fixture-resolved',
                'choices': [{'index': 0, 'message': message, 'finish_reason': finish}],
                'usage': {'prompt_tokens': 7, 'completion_tokens': 3, 'total_tokens': 10},
            })
            return
        if self.path.endswith('/responses'):
            output = [{
                'id': 'msg-fixture',
                'type': 'message',
                'status': 'completed',
                'role': 'assistant',
                'content': [{'type': 'output_text', 'text': 'fixture text', 'annotations': []}],
            }]
            if name:
                output = [{
                    'id': 'fc-fixture',
                    'type': 'function_call',
                    'status': 'completed',
                    'call_id': 'call-probe',
                    'name': name,
                    'arguments': '{"value":"ready"}',
                }]
            self.respond(200, {
                'id': 'resp-fixture',
                'object': 'response',
                'created_at': 1,
                'status': 'completed',
                'model': 'fixture-resolved',
                'output': output,
                'usage': {'input_tokens': 7, 'output_tokens': 3, 'total_tokens': 10},
            })
            return
        self.respond(404, {'error': {'message': 'unknown fixture path'}})

    def respond(self, status, payload):
        body = json.dumps(payload).encode('utf-8')
        self.send_response(status)
        self.send_header('content-type', 'application/json')
        self.send_header('content-length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

ThreadingHTTPServer(('127.0.0.1', port), Handler).serve_forever()
"#;
