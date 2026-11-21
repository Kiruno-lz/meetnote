use crate::codex::{self, CodexModel};
use crate::http::{auth_headers, endpoint_url, header_secret, Http, Reply, MAX_STREAM_BYTES};
use crate::{
    AnthropicMessagesAdapter, AuthMode, ConnectionConfig, LlmCancellation, LlmError, LlmErrorKind,
    LlmInputBlock, LlmMessage, LlmOutput, LlmOutputBlock, LlmProtocol, LlmProvider,
    LlmProviderDescriptor, LlmRequest, LlmToolChoice, LlmToolDefinition, ModelCapabilities,
    OpenAiChatCompletionsAdapter, OpenAiResponsesAdapter, ProbeResult, ProtocolAdapter,
};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

pub struct Credential(String);

impl Credential {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

/// Where a connection's requests go beyond its protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// The vendor's own API at the connection's base URL.
    Standard,
    /// The ChatGPT backend serving Codex (`https://chatgpt.com/backend-api/codex`),
    /// reached with a ChatGPT sign-in's access token as the credential.
    ChatGpt { account_id: String },
}

pub struct ApiProvider {
    config: ConnectionConfig,
    credential: Option<Credential>,
    backend: Backend,
    http: Http,
    adapter: Box<dyn ProtocolAdapter>,
    local: bool,
    /// The configured model's Codex instructions, read once per provider.
    codex_instructions: Mutex<Option<String>>,
}

impl ApiProvider {
    pub fn new(config: ConnectionConfig, credential: Option<Credential>) -> Result<Self, LlmError> {
        Self::with_backend(config, credential, Backend::Standard)
    }

    pub fn with_backend(
        config: ConnectionConfig,
        credential: Option<Credential>,
        backend: Backend,
    ) -> Result<Self, LlmError> {
        validate_config(&config)?;
        if let Backend::ChatGpt { account_id } = &backend {
            if config.protocol != LlmProtocol::OpenAiResponses
                || config.auth_mode != AuthMode::Bearer
                || credential.is_none()
                || account_id.trim().is_empty()
            {
                return Err(LlmError::new(
                    LlmErrorKind::ConnectionNotConfigured,
                    "a ChatGPT connection needs the Responses protocol, a bearer access token and an account id",
                ));
            }
        }
        let local = crate::http::is_local_url(&config.base_url)?;
        let http = Http::for_url(&config.base_url, Duration::from_millis(config.timeout_ms))?;
        let adapter: Box<dyn ProtocolAdapter> = match config.protocol {
            LlmProtocol::OpenAiChatCompletions => Box::<OpenAiChatCompletionsAdapter>::default(),
            LlmProtocol::AnthropicMessages => Box::<AnthropicMessagesAdapter>::default(),
            LlmProtocol::OpenAiResponses => Box::<OpenAiResponsesAdapter>::default(),
        };
        Ok(Self {
            config,
            credential,
            backend,
            http,
            adapter,
            local,
            codex_instructions: Mutex::new(None),
        })
    }

    pub fn connection(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// The models the endpoint lists, in its order.
    pub fn discover_models(&self, cancellation: &LlmCancellation) -> Result<Vec<String>, LlmError> {
        match &self.backend {
            Backend::Standard => crate::vendors::models::list_with(
                &self.http,
                &self.config.base_url,
                self.config.protocol,
                self.auth_headers()?,
                cancellation,
            ),
            Backend::ChatGpt { .. } => Ok(self
                .codex_models(cancellation)?
                .into_iter()
                .map(|model| model.slug)
                .collect()),
        }
    }

    fn auth_headers(&self) -> Result<Vec<(&'static str, String)>, LlmError> {
        auth_headers(
            self.config.protocol,
            self.config.auth_mode,
            self.credential.as_ref(),
        )
    }

    fn codex_models(&self, cancellation: &LlmCancellation) -> Result<Vec<CodexModel>, LlmError> {
        let Backend::ChatGpt { account_id } = &self.backend else {
            return Err(LlmError::new(
                LlmErrorKind::CapabilityUnsupported,
                "Codex models are only listed by the ChatGPT backend",
            ));
        };
        codex::models(
            &self.http,
            &self.config.base_url,
            self.access_token()?,
            account_id,
            cancellation,
        )
    }

    fn codex_instructions(&self, cancellation: &LlmCancellation) -> Result<String, LlmError> {
        let mut cached = self
            .codex_instructions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(instructions) = cached.as_ref() {
            return Ok(instructions.clone());
        }
        let instructions = self
            .codex_models(cancellation)?
            .into_iter()
            .find(|model| model.slug == self.config.model)
            .and_then(|model| model.instructions)
            .ok_or_else(|| {
                LlmError::new(
                    LlmErrorKind::CapabilityUnsupported,
                    format!(
                        "this ChatGPT account does not offer {} to Codex",
                        self.config.model
                    ),
                )
            })?;
        *cached = Some(instructions.clone());
        Ok(instructions)
    }

    fn access_token(&self) -> Result<&str, LlmError> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            LlmError::new(
                LlmErrorKind::Authentication,
                "the ChatGPT connection has no access token",
            )
        })?;
        header_secret(credential)
    }

    /// Posts an encoded request and returns the provider's response object.
    fn send(
        &self,
        body: &Value,
        cancellation: &LlmCancellation,
    ) -> Result<(Value, Reply), LlmError> {
        let url = endpoint_url(&self.config.base_url, self.adapter.endpoint_path())?;
        match &self.backend {
            Backend::Standard => {
                let reply = self
                    .http
                    .post(url)
                    .headers(self.auth_headers()?)
                    .json(body)
                    .send(cancellation)?
                    .success()?;
                let response = serde_json::from_slice(&reply.body).map_err(|error| {
                    LlmError::new(
                        LlmErrorKind::ResponseInvalid,
                        format!("provider returned malformed JSON: {error}"),
                    )
                })?;
                Ok((response, reply))
            }
            Backend::ChatGpt { account_id } => {
                let instructions = self.codex_instructions(cancellation)?;
                let (body, session) =
                    codex::request_body(body.clone(), &instructions, &self.config.model);
                let reply = self
                    .http
                    .post(url)
                    .headers(codex::backend_headers(self.access_token()?, account_id))
                    .header("Accept", "text/event-stream")
                    .header("session-id", session.clone())
                    .header("thread-id", session)
                    .json(&body)
                    .limit(MAX_STREAM_BYTES)
                    .send(cancellation)?
                    .success()?;
                Ok((codex::read_stream(&reply.body)?, reply))
            }
        }
    }
}

impl LlmProvider for ApiProvider {
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
        let (response, reply) = self.send(&body, cancellation)?;
        let mut output = self.adapter.decode(response, &self.config.model)?;
        if let (Some(usage), Some(cost_microusd)) = (output.usage.as_mut(), reply.cost_microusd) {
            usage.cost_microusd = Some(cost_microusd);
        }
        Ok(output)
    }

    fn probe(&self, cancellation: &LlmCancellation) -> Result<ProbeResult, LlmError> {
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
    let parsed = crate::http::parse_url(&config.base_url)?;
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
    config.input_token_budget().or_else(|error| {
        if error.kind == LlmErrorKind::ContextLimitUnknown {
            Ok(0)
        } else {
            Err(error)
        }
    })?;
    Ok(())
}
