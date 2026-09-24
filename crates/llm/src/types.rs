use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProtocol {
    #[serde(rename = "openaiChatCompletions")]
    OpenAiChatCompletions,
    #[serde(rename = "anthropicMessages")]
    AnthropicMessages,
    #[serde(rename = "openaiResponses")]
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMode {
    Bearer,
    XApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub id: String,
    pub protocol: LlmProtocol,
    pub base_url: String,
    pub model: String,
    pub auth_mode: AuthMode,
    pub timeout_ms: u64,
    pub context_window_tokens: Option<u32>,
    pub max_output_tokens: u32,
}

impl ConnectionConfig {
    pub fn input_token_budget(&self) -> Result<u32, LlmError> {
        let context = self.context_window_tokens.ok_or_else(|| {
            LlmError::new(
                LlmErrorKind::ContextLimitUnknown,
                "connection does not define a context window",
            )
        })?;
        if context <= self.max_output_tokens {
            Err(LlmError::new(
                LlmErrorKind::ContextLimitExceeded,
                "max output tokens must be smaller than the context window",
            ))
        } else {
            Ok(context - self.max_output_tokens)
        }
    }
}

#[derive(Clone, Default)]
pub struct LlmCancellation(Arc<AtomicBool>);

impl LlmCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), LlmError> {
        if self.is_cancelled() {
            Err(LlmError::new(
                LlmErrorKind::Cancelled,
                "LLM request was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviderDescriptor {
    pub id: String,
    pub local: bool,
    pub protocol: LlmProtocol,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmRequest {
    pub instructions: Option<String>,
    /// Provider-specific reasoning mode hint. `Some(false)` requests a
    /// non-thinking response where the provider supports that switch.
    #[serde(default)]
    pub thinking: Option<bool>,
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<LlmToolDefinition>,
    #[serde(default)]
    pub tool_choice: LlmToolChoice,
    pub max_output_tokens: Option<u32>,
}

impl LlmRequest {
    pub fn validate(&self) -> Result<(), LlmError> {
        if self.messages.is_empty() {
            return Err(LlmError::new(
                LlmErrorKind::RequestInvalid,
                "LLM request must contain at least one message",
            ));
        }
        if self.max_output_tokens == Some(0) {
            return Err(LlmError::new(
                LlmErrorKind::RequestInvalid,
                "max output tokens must be greater than zero",
            ));
        }
        if self.tool_choice == LlmToolChoice::Required && self.tools.is_empty() {
            return Err(LlmError::new(
                LlmErrorKind::RequestInvalid,
                "required tool choice needs at least one tool",
            ));
        }

        for (message_index, message) in self.messages.iter().enumerate() {
            match message {
                LlmMessage::User(blocks) => {
                    if blocks.is_empty() {
                        return Err(request_invalid(format!(
                            "user message {message_index} must contain at least one block"
                        )));
                    }
                    for block in blocks {
                        match block {
                            LlmInputBlock::Text { text } if text.is_empty() => {
                                return Err(request_invalid(format!(
                                    "user message {message_index} contains empty text"
                                )));
                            }
                            LlmInputBlock::ToolResult {
                                tool_call_id,
                                payload_json,
                                ..
                            } => {
                                if tool_call_id.trim().is_empty() {
                                    return Err(request_invalid(format!(
                                        "user message {message_index} contains a tool result without an id"
                                    )));
                                }
                                serde_json::from_str::<serde_json::Value>(payload_json).map_err(
                                    |error| {
                                        request_invalid(format!(
                                            "user message {message_index} contains invalid tool result JSON: {error}"
                                        ))
                                    },
                                )?;
                            }
                            LlmInputBlock::Text { .. } => {}
                        }
                    }
                }
                LlmMessage::Assistant(blocks) => {
                    if blocks.is_empty() {
                        return Err(request_invalid(format!(
                            "assistant message {message_index} must contain at least one block"
                        )));
                    }
                    for block in blocks {
                        match block {
                            LlmOutputBlock::Text { text } if text.is_empty() => {
                                return Err(request_invalid(format!(
                                    "assistant message {message_index} contains empty text"
                                )));
                            }
                            LlmOutputBlock::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                if id.trim().is_empty() || name.trim().is_empty() {
                                    return Err(request_invalid(format!(
                                        "assistant message {message_index} contains a tool call without an id or name"
                                    )));
                                }
                                let arguments = serde_json::from_str::<serde_json::Value>(
                                    arguments_json,
                                )
                                .map_err(|error| {
                                    request_invalid(format!(
                                        "assistant message {message_index} contains invalid tool arguments JSON: {error}"
                                    ))
                                })?;
                                if !arguments.is_object() {
                                    return Err(request_invalid(format!(
                                        "assistant message {message_index} tool arguments must be a JSON object"
                                    )));
                                }
                            }
                            LlmOutputBlock::Text { .. } => {}
                        }
                    }
                }
            }
        }

        let mut tool_names = HashSet::new();
        for tool in &self.tools {
            if tool.name.trim().is_empty() {
                return Err(request_invalid("tool name must not be empty"));
            }
            if !tool.input_schema.is_object() {
                return Err(request_invalid(format!(
                    "tool {} input schema must be a JSON object",
                    tool.name
                )));
            }
            if !tool_names.insert(tool.name.as_str()) {
                return Err(request_invalid(format!(
                    "tool name is duplicated: {}",
                    tool.name
                )));
            }
        }
        Ok(())
    }
}

fn request_invalid(message: impl Into<String>) -> LlmError {
    LlmError::new(LlmErrorKind::RequestInvalid, message)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", content = "content", rename_all = "camelCase")]
pub enum LlmMessage {
    User(Vec<LlmInputBlock>),
    Assistant(Vec<LlmOutputBlock>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum LlmInputBlock {
    Text {
        text: String,
    },
    ToolResult {
        tool_call_id: String,
        payload_json: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum LlmOutputBlock {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments_json: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LlmToolChoice {
    None,
    #[default]
    Auto,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmOutput {
    pub blocks: Vec<LlmOutputBlock>,
    pub stop_reason: LlmStopReason,
    pub usage: Option<LlmUsage>,
    pub requested_model: String,
    pub resolved_model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmStopReason {
    Complete,
    ToolUse,
    MaxTokens,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub text: bool,
    pub client_tools: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub ready: bool,
    pub models: Option<Vec<String>>,
    pub requested_model: String,
    pub resolved_model: Option<String>,
    pub capabilities: ModelCapabilities,
    pub context_window_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    RequestInvalid,
    ConnectionNotConfigured,
    Authentication,
    RateLimited,
    Timeout,
    CapabilityUnsupported,
    ResponseInvalid,
    ContextLimitUnknown,
    ContextLimitExceeded,
    Cancelled,
    Transport,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LlmError {
    pub kind: LlmErrorKind,
    pub message: String,
    pub retryable: bool,
    pub status: Option<u16>,
    pub retry_after_ms: Option<u64>,
}

impl LlmError {
    pub fn new(kind: LlmErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable: matches!(
                kind,
                LlmErrorKind::RateLimited | LlmErrorKind::Timeout | LlmErrorKind::Transport
            ),
            status: None,
            retry_after_ms: None,
        }
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub(crate) fn with_retry_after(mut self, retry_after_ms: Option<u64>) -> Self {
        self.retry_after_ms = retry_after_ms;
        self
    }
}

pub trait LlmProvider: Send + Sync {
    fn descriptor(&self) -> LlmProviderDescriptor;
    fn complete(
        &self,
        request: &LlmRequest,
        cancellation: &LlmCancellation,
    ) -> Result<LlmOutput, LlmError>;
    fn probe(&self, cancellation: &LlmCancellation) -> Result<ProbeResult, LlmError>;
}
