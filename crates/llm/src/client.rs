use crate::{
    AnthropicMessagesAdapter, AuthMode, ConnectionConfig, LlmCancellation, LlmError, LlmErrorKind,
    LlmInputBlock, LlmMessage, LlmOutput, LlmOutputBlock, LlmProtocol, LlmProvider,
    LlmProviderDescriptor, LlmRequest, LlmToolChoice, LlmToolDefinition, ModelCapabilities,
    OpenAiChatCompletionsAdapter, OpenAiResponsesAdapter, ProbeResult, ProtocolAdapter,
};
use serde_json::{json, Value};
use std::error::Error as _;
use std::io::Read;
use std::net::IpAddr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use url::Url;

const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

struct HttpJsonResponse {
    body: Value,
    cost_microusd: Option<u64>,
}

pub struct Credential(String);

impl Credential {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

pub struct LiteLlmProvider {
    config: ConnectionConfig,
    credential: Option<Credential>,
    agent: ureq::Agent,
    adapter: Box<dyn ProtocolAdapter>,
    local: bool,
}

impl LiteLlmProvider {
    pub fn new(config: ConnectionConfig, credential: Option<Credential>) -> Result<Self, LlmError> {
        validate_config(&config)?;
        let local = is_local_url(&config.base_url)?;
        let timeout = Duration::from_millis(config.timeout_ms);
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .timeout_connect(timeout)
            .redirects(0)
            .https_only(!local)
            .build();
        let adapter: Box<dyn ProtocolAdapter> = match config.protocol {
            LlmProtocol::OpenAiChatCompletions => Box::<OpenAiChatCompletionsAdapter>::default(),
            LlmProtocol::AnthropicMessages => Box::<AnthropicMessagesAdapter>::default(),
            LlmProtocol::OpenAiResponses => Box::<OpenAiResponsesAdapter>::default(),
        };
        Ok(Self {
            config,
            credential,
            agent,
            adapter,
            local,
        })
    }

    pub fn connection(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn readiness(&self, cancellation: &LlmCancellation) -> Result<(), LlmError> {
        cancellation.check()?;
        self.get_json("/health/readiness", cancellation).map(|_| ())
    }

    pub fn discover_models(&self, cancellation: &LlmCancellation) -> Result<Vec<String>, LlmError> {
        cancellation.check()?;
        let response = self.get_json("/v1/models", cancellation)?;
        let models = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                LlmError::new(
                    LlmErrorKind::ResponseInvalid,
                    "model discovery response has no data array",
                )
            })?;
        let mut ids = models
            .iter()
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn post_json(
        &self,
        endpoint: &str,
        body: &Value,
        cancellation: &LlmCancellation,
    ) -> Result<HttpJsonResponse, LlmError> {
        cancellation.check()?;
        let url = endpoint_url(&self.config.base_url, endpoint)?;
        let agent = self.agent.clone();
        let headers = self.authentication_headers()?;
        let body = serde_json::to_string(body)
            .map_err(|error| LlmError::new(LlmErrorKind::RequestInvalid, error.to_string()))?;
        run_cancellable(cancellation, move || {
            let mut request = agent
                .post(url.as_str())
                .set("Content-Type", "application/json");
            for (name, value) in headers {
                request = request.set(name, &value);
            }
            parse_http_result(request.send_string(&body))
        })
    }

    fn get_json(&self, endpoint: &str, cancellation: &LlmCancellation) -> Result<Value, LlmError> {
        cancellation.check()?;
        let url = endpoint_url(&self.config.base_url, endpoint)?;
        let agent = self.agent.clone();
        let headers = self.authentication_headers()?;
        run_cancellable(cancellation, move || {
            let mut request = agent.get(url.as_str());
            for (name, value) in headers {
                request = request.set(name, &value);
            }
            parse_http_result(request.call()).map(|response| response.body)
        })
    }

    fn authentication_headers(&self) -> Result<Vec<(&'static str, String)>, LlmError> {
        let mut headers = Vec::new();
        if let Some(credential) = &self.credential {
            if credential.expose().is_empty()
                || !credential
                    .expose()
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic())
            {
                return Err(LlmError::new(
                    LlmErrorKind::Authentication,
                    "connection credential is empty or cannot be represented as an HTTP header",
                ));
            }
            headers.push(match self.config.auth_mode {
                AuthMode::Bearer => ("Authorization", format!("Bearer {}", credential.expose())),
                AuthMode::XApiKey => ("x-api-key", credential.expose().to_owned()),
            });
        }
        if self.config.protocol == LlmProtocol::AnthropicMessages {
            headers.push(("anthropic-version", "2023-06-01".into()));
        }
        Ok(headers)
    }
}

impl LlmProvider for LiteLlmProvider {
    fn descriptor(&self) -> LlmProviderDescriptor {
        LlmProviderDescriptor {
            id: self.config.id.clone(),
            local: self.local,
            protocol: self.config.protocol,
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancellation: &LlmCancellation,
    ) -> Result<LlmOutput, LlmError> {
        cancellation.check()?;
        let mut request = request.clone();
        request.max_output_tokens = request
            .max_output_tokens
            .or(Some(self.config.max_output_tokens));
        if request
            .max_output_tokens
            .is_some_and(|tokens| tokens > self.config.max_output_tokens)
        {
            return Err(LlmError::new(
                LlmErrorKind::RequestInvalid,
                "request max output tokens exceed the connection limit",
            ));
        }
        let body = self.adapter.encode(&request, &self.config.model)?;
        let response = self.post_json(self.adapter.endpoint_path(), &body, cancellation)?;
        let mut output = self.adapter.decode(response.body, &self.config.model)?;
        if let (Some(usage), Some(cost_microusd)) = (output.usage.as_mut(), response.cost_microusd)
        {
            usage.cost_microusd = Some(cost_microusd);
        }
        Ok(output)
    }

    fn probe(&self, cancellation: &LlmCancellation) -> Result<ProbeResult, LlmError> {
        self.readiness(cancellation)?;
        let models = self.discover_models(cancellation).ok();

        let text = self.complete(
            &LlmRequest {
                instructions: Some("Return only the word ready.".into()),
                thinking: None,
                messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
                    text: "Connection readiness probe".into(),
                }])],
                tools: Vec::new(),
                tool_choice: LlmToolChoice::None,
                max_output_tokens: Some(16.min(self.config.max_output_tokens)),
            },
            cancellation,
        )?;
        let text_capability = text
            .blocks
            .iter()
            .any(|block| matches!(block, LlmOutputBlock::Text { .. }));

        let tool = LlmToolDefinition {
            name: "meetnote_probe".into(),
            description: "Return the supplied probe value".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false
            }),
        };
        let tool_prompt = LlmMessage::User(vec![LlmInputBlock::Text {
            text: "Run the client tool capability probe".into(),
        }]);
        let tool_output = self.complete(
            &LlmRequest {
                instructions: Some("Call meetnote_probe exactly once with value ready.".into()),
                thinking: None,
                messages: vec![tool_prompt.clone()],
                tools: vec![tool.clone()],
                tool_choice: LlmToolChoice::Required,
                max_output_tokens: Some(64.min(self.config.max_output_tokens)),
            },
            cancellation,
        );
        let client_tools = match tool_output {
            Ok(output) => {
                if output.stop_reason != crate::LlmStopReason::ToolUse {
                    return Ok(ProbeResult {
                        ready: true,
                        models,
                        requested_model: self.config.model.clone(),
                        resolved_model: Some(text.resolved_model),
                        capabilities: ModelCapabilities {
                            text: text_capability,
                            client_tools: false,
                        },
                        context_window_tokens: self.config.context_window_tokens,
                    });
                }

                let tool_calls = output
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        LlmOutputBlock::ToolCall {
                            id,
                            name,
                            arguments_json,
                        } => Some((id, name, arguments_json)),
                        LlmOutputBlock::Text { .. } => None,
                    })
                    .collect::<Vec<_>>();
                let all_calls_match = !tool_calls.is_empty()
                    && tool_calls.iter().all(|(_, name, arguments_json)| {
                        name.as_str() == "meetnote_probe"
                            && serde_json::from_str::<Value>(arguments_json)
                                .ok()
                                .and_then(|arguments| {
                                    arguments
                                        .get("value")
                                        .and_then(Value::as_str)
                                        .map(|value| value == "ready")
                                })
                                .unwrap_or(false)
                    });
                if !all_calls_match {
                    false
                } else {
                    let tool_results = tool_calls
                        .into_iter()
                        .map(|(id, _, _)| LlmInputBlock::ToolResult {
                            tool_call_id: id.clone(),
                            payload_json: r#"{"value":"ready"}"#.into(),
                            is_error: false,
                        })
                        .collect();
                    let final_output = self.complete(
                        &LlmRequest {
                            instructions: Some(
                                "Acknowledge a successful tool result with text.".into(),
                            ),
                            thinking: None,
                            messages: vec![
                                tool_prompt,
                                LlmMessage::Assistant(output.blocks),
                                LlmMessage::User(tool_results),
                            ],
                            tools: vec![tool],
                            tool_choice: LlmToolChoice::None,
                            max_output_tokens: Some(32.min(self.config.max_output_tokens)),
                        },
                        cancellation,
                    )?;
                    final_output.stop_reason == crate::LlmStopReason::Complete
                        && final_output
                            .blocks
                            .iter()
                            .any(|block| matches!(block, LlmOutputBlock::Text { .. }))
                }
            }
            Err(error) if error.kind == LlmErrorKind::CapabilityUnsupported => false,
            Err(error) => return Err(error),
        };

        Ok(ProbeResult {
            ready: true,
            models,
            requested_model: self.config.model.clone(),
            resolved_model: Some(text.resolved_model),
            capabilities: ModelCapabilities {
                text: text_capability,
                client_tools,
            },
            context_window_tokens: self.config.context_window_tokens,
        })
    }
}

fn validate_config(config: &ConnectionConfig) -> Result<(), LlmError> {
    if config.id.trim().is_empty()
        || config.base_url.trim().is_empty()
        || config.model.trim().is_empty()
        || config.timeout_ms == 0
        || config.max_output_tokens == 0
    {
        return Err(LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            "LLM connection is incomplete",
        ));
    }
    let parsed = Url::parse(&config.base_url).map_err(|error| {
        LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            format!("invalid LLM base URL: {error}"),
        )
    })?;
    if parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            "LLM base URL cannot contain credentials, query, or fragment",
        ));
    }
    let local = is_local_url(&config.base_url)?;
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        return Err(LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            "LLM base URL must use HTTPS unless it targets localhost",
        ));
    }
    config.input_token_budget().or_else(|error| {
        if error.kind == LlmErrorKind::ContextLimitUnknown {
            Ok(0)
        } else {
            Err(error)
        }
    })?;
    Ok(())
}

fn is_local_url(base_url: &str) -> Result<bool, LlmError> {
    let url = Url::parse(base_url).map_err(|error| {
        LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            format!("invalid LLM base URL: {error}"),
        )
    })?;
    let host = url.host_str().ok_or_else(|| {
        LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            "LLM base URL has no host",
        )
    })?;
    Ok(is_loopback_host(host))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn endpoint_url(base_url: &str, endpoint: &str) -> Result<Url, LlmError> {
    let mut url = Url::parse(base_url).map_err(|error| {
        LlmError::new(
            LlmErrorKind::ConnectionNotConfigured,
            format!("invalid LLM base URL: {error}"),
        )
    })?;
    let mut base_path = url.path().trim_end_matches('/');
    let endpoint = if base_path.ends_with("/v1") && endpoint.starts_with("/v1/") {
        endpoint.strip_prefix("/v1").unwrap_or(endpoint)
    } else {
        if base_path.ends_with("/v1") && !endpoint.starts_with("/v1/") {
            base_path = base_path.trim_end_matches("/v1");
        }
        endpoint
    };
    url.set_path(&format!("{base_path}{endpoint}"));
    Ok(url)
}

fn run_cancellable<T: Send + 'static>(
    cancellation: &LlmCancellation,
    operation: impl FnOnce() -> Result<T, LlmError> + Send + 'static,
) -> Result<T, LlmError> {
    cancellation.check()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("meetnote-llm-http".into())
        .spawn(move || {
            let _ = sender.send(operation());
        })
        .map_err(|error| {
            LlmError::new(
                LlmErrorKind::Transport,
                format!("failed to start LLM HTTP request: {error}"),
            )
        })?;

    loop {
        cancellation.check()?;
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(LlmError::new(
                    LlmErrorKind::Transport,
                    "LLM HTTP request ended without a result",
                ))
            }
        }
    }
}

fn parse_http_result(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<HttpJsonResponse, LlmError> {
    match result {
        Ok(response) if (300..400).contains(&response.status()) => {
            let status = response.status();
            Err(LlmError::new(
                LlmErrorKind::ResponseInvalid,
                "provider redirect was rejected to prevent credential forwarding",
            )
            .with_status(status))
        }
        Ok(response) => {
            let cost_microusd = response
                .header("x-litellm-response-cost-original")
                .or_else(|| response.header("x-litellm-response-cost"))
                .and_then(parse_cost_microusd);
            parse_response_body(response).map(|body| HttpJsonResponse {
                body,
                cost_microusd,
            })
        }
        Err(ureq::Error::Status(status, response)) => {
            if (300..400).contains(&status) {
                return Err(LlmError::new(
                    LlmErrorKind::ResponseInvalid,
                    "provider redirect was rejected to prevent credential forwarding",
                )
                .with_status(status));
            }
            let retry_after_ms = response
                .header("retry-after")
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1_000));
            let provider_message = parse_response_body(response)
                .ok()
                .and_then(|body| {
                    body.pointer("/error/message")
                        .or_else(|| body.get("detail"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("provider returned HTTP {status}"));
            let kind = match status {
                401 | 403 => LlmErrorKind::Authentication,
                408 | 504 => LlmErrorKind::Timeout,
                429 => LlmErrorKind::RateLimited,
                413 => LlmErrorKind::ContextLimitExceeded,
                404 => LlmErrorKind::CapabilityUnsupported,
                500..=599 => LlmErrorKind::Transport,
                _ => LlmErrorKind::ResponseInvalid,
            };
            Err(LlmError::new(kind, provider_message)
                .with_status(status)
                .with_retry_after(retry_after_ms))
        }
        Err(ureq::Error::Transport(error)) => {
            let timed_out = transport_timed_out(&error);
            let kind = if timed_out {
                LlmErrorKind::Timeout
            } else {
                LlmErrorKind::Transport
            };
            Err(LlmError::new(kind, error.to_string()))
        }
    }
}

fn parse_cost_microusd(value: &str) -> Option<u64> {
    let dollars = value.parse::<f64>().ok()?;
    if !dollars.is_finite() || dollars < 0.0 {
        return None;
    }
    let microusd = dollars * 1_000_000.0;
    (microusd <= u64::MAX as f64).then(|| microusd.round() as u64)
}

fn transport_timed_out(error: &ureq::Transport) -> bool {
    let mut source = error.source();
    while let Some(current) = source {
        if current
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| {
                matches!(
                    io_error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
            })
        {
            return true;
        }
        source = current.source();
    }
    false
}

fn parse_response_body(response: ureq::Response) -> Result<Value, LlmError> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| LlmError::new(LlmErrorKind::Transport, error.to_string()))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(LlmError::new(
            LlmErrorKind::ResponseInvalid,
            "provider response exceeds the maximum supported size",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        LlmError::new(
            LlmErrorKind::ResponseInvalid,
            format!("provider returned malformed JSON: {error}"),
        )
    })
}
