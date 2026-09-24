use crate::{
    LlmError, LlmErrorKind, LlmInputBlock, LlmMessage, LlmOutput, LlmOutputBlock, LlmProtocol,
    LlmRequest, LlmStopReason, LlmToolChoice, LlmUsage,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub trait ProtocolAdapter: Send + Sync {
    fn protocol(&self) -> LlmProtocol;
    fn endpoint_path(&self) -> &'static str;
    fn encode(&self, request: &LlmRequest, model: &str) -> Result<Value, LlmError>;
    fn decode(&self, response: Value, requested_model: &str) -> Result<LlmOutput, LlmError>;
}

#[derive(Debug, Default)]
pub struct OpenAiChatCompletionsAdapter;

impl ProtocolAdapter for OpenAiChatCompletionsAdapter {
    fn protocol(&self) -> LlmProtocol {
        LlmProtocol::OpenAiChatCompletions
    }

    fn endpoint_path(&self) -> &'static str {
        "/v1/chat/completions"
    }

    fn encode(&self, request: &LlmRequest, model: &str) -> Result<Value, LlmError> {
        request.validate()?;
        let mut messages = Vec::new();
        if let Some(instructions) = &request.instructions {
            messages.push(json!({"role": "system", "content": instructions}));
        }
        for message in &request.messages {
            match message {
                LlmMessage::User(blocks) => {
                    let mut tool_results = Vec::new();
                    let mut text_messages = Vec::new();
                    for block in blocks {
                        match block {
                            LlmInputBlock::Text { text } => {
                                text_messages.push(json!({"role": "user", "content": text}));
                            }
                            LlmInputBlock::ToolResult {
                                tool_call_id,
                                payload_json,
                                ..
                            } => {
                                require_request_json(payload_json, "tool result")?;
                                tool_results.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_call_id,
                                    "content": payload_json,
                                }));
                            }
                        }
                    }
                    messages.extend(tool_results);
                    messages.extend(text_messages);
                }
                LlmMessage::Assistant(blocks) => {
                    let mut text = String::new();
                    let mut tool_calls = Vec::new();
                    for block in blocks {
                        match block {
                            LlmOutputBlock::Text { text: block_text } => {
                                text.push_str(block_text);
                            }
                            LlmOutputBlock::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                require_request_json_object(arguments_json, "tool arguments")?;
                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {"name": name, "arguments": arguments_json},
                                }));
                            }
                        }
                    }
                    let mut wire = Map::new();
                    wire.insert("role".into(), Value::String("assistant".into()));
                    wire.insert(
                        "content".into(),
                        if text.is_empty() {
                            Value::Null
                        } else {
                            Value::String(text)
                        },
                    );
                    if !tool_calls.is_empty() {
                        wire.insert("tool_calls".into(), Value::Array(tool_calls));
                    }
                    messages.push(Value::Object(wire));
                }
            }
        }

        let mut body = json!({"model": model, "messages": messages, "stream": false});
        add_openai_tools(&mut body, request);
        add_thinking_hint(&mut body, request.thinking, false);
        if let Some(max_tokens) = request.max_output_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        Ok(body)
    }

    fn decode(&self, response: Value, requested_model: &str) -> Result<LlmOutput, LlmError> {
        let choice = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .ok_or_else(|| invalid("chat completion response has no choice"))?;
        let message = choice
            .get("message")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("chat completion choice has no message"))?;
        let mut blocks = Vec::new();
        if let Some(text) = message.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(LlmOutputBlock::Text { text: text.into() });
            }
        }
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let id = string_at(call, "id", "chat tool call id")?;
                let function = call
                    .get("function")
                    .ok_or_else(|| invalid("chat tool call has no function"))?;
                let name = string_at(function, "name", "chat tool name")?;
                let arguments_json = string_at(function, "arguments", "chat tool arguments")?;
                require_json_object(&arguments_json, "chat tool arguments")?;
                blocks.push(LlmOutputBlock::ToolCall {
                    id,
                    name,
                    arguments_json,
                });
            }
        }
        let finish = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("chat completion choice has no finish reason"))?;
        let stop_reason = match finish {
            "tool_calls" | "function_call" => LlmStopReason::ToolUse,
            "length" => LlmStopReason::MaxTokens,
            "stop" => LlmStopReason::Complete,
            "content_filter" => LlmStopReason::Incomplete,
            other => {
                return Err(invalid(format!(
                    "chat completion has unsupported finish reason: {other}"
                )))
            }
        };
        if blocks.is_empty()
            && matches!(
                stop_reason,
                LlmStopReason::Complete | LlmStopReason::ToolUse
            )
        {
            return Err(invalid(
                "chat completion response has no text or tool calls",
            ));
        }
        validate_decoded_blocks(&blocks, stop_reason, "chat completion")?;
        Ok(output(
            response.get("model"),
            requested_model,
            blocks,
            stop_reason,
            parse_usage(&response, "prompt_tokens", "completion_tokens"),
        ))
    }
}

#[derive(Debug, Default)]
pub struct AnthropicMessagesAdapter;

impl ProtocolAdapter for AnthropicMessagesAdapter {
    fn protocol(&self) -> LlmProtocol {
        LlmProtocol::AnthropicMessages
    }

    fn endpoint_path(&self) -> &'static str {
        "/v1/messages"
    }

    fn encode(&self, request: &LlmRequest, model: &str) -> Result<Value, LlmError> {
        request.validate()?;
        let max_tokens = request.max_output_tokens.ok_or_else(|| {
            LlmError::new(
                LlmErrorKind::RequestInvalid,
                "Anthropic Messages requires max output tokens",
            )
        })?;
        let mut messages = Vec::new();
        for message in &request.messages {
            let (role, blocks) = match message {
                LlmMessage::User(blocks) => {
                    let mut tool_results = Vec::new();
                    let mut text = Vec::new();
                    for block in blocks {
                        match block {
                            LlmInputBlock::Text { text: value } => {
                                text.push(json!({"type": "text", "text": value}));
                            }
                            LlmInputBlock::ToolResult {
                                tool_call_id,
                                payload_json,
                                is_error,
                            } => {
                                require_request_json(payload_json, "tool result")?;
                                tool_results.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_call_id,
                                    "content": payload_json,
                                    "is_error": is_error,
                                }));
                            }
                        }
                    }
                    tool_results.extend(text);
                    ("user", tool_results)
                }
                LlmMessage::Assistant(blocks) => {
                    let mut wire = Vec::new();
                    for block in blocks {
                        match block {
                            LlmOutputBlock::Text { text } => {
                                wire.push(json!({"type": "text", "text": text}));
                            }
                            LlmOutputBlock::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                wire.push(json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": require_request_json_object(arguments_json, "tool arguments")?,
                                }));
                            }
                        }
                    }
                    ("assistant", wire)
                }
            };
            messages.push(json!({"role": role, "content": blocks}));
        }
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": max_tokens,
            "stream": false,
        });
        if let Some(instructions) = &request.instructions {
            body["system"] = Value::String(instructions.clone());
        }
        add_thinking_hint(&mut body, request.thinking, false);
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
            body["tool_choice"] = match request.tool_choice {
                LlmToolChoice::None => json!({"type": "none"}),
                LlmToolChoice::Auto => json!({"type": "auto"}),
                LlmToolChoice::Required => json!({"type": "any"}),
            };
        }
        Ok(body)
    }

    fn decode(&self, response: Value, requested_model: &str) -> Result<LlmOutput, LlmError> {
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Anthropic response has no content array"))?;
        let mut blocks = Vec::new();
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = string_at(block, "text", "Anthropic text")?;
                    if !text.is_empty() {
                        blocks.push(LlmOutputBlock::Text { text });
                    }
                }
                Some("tool_use") => {
                    let input = block
                        .get("input")
                        .and_then(Value::as_object)
                        .ok_or_else(|| invalid("Anthropic tool input must be an object"))?;
                    blocks.push(LlmOutputBlock::ToolCall {
                        id: string_at(block, "id", "Anthropic tool id")?,
                        name: string_at(block, "name", "Anthropic tool name")?,
                        arguments_json: serde_json::to_string(input)
                            .map_err(|error| invalid(error.to_string()))?,
                    });
                }
                Some(_) => {}
                None => return Err(invalid("Anthropic content block has no type")),
            }
        }
        let stop = match response.get("stop_reason").and_then(Value::as_str) {
            Some("tool_use") => LlmStopReason::ToolUse,
            Some("max_tokens") => LlmStopReason::MaxTokens,
            Some("end_turn" | "stop_sequence") => LlmStopReason::Complete,
            Some("pause_turn" | "refusal") => LlmStopReason::Incomplete,
            Some(other) => {
                return Err(invalid(format!(
                    "Anthropic response has unsupported stop reason: {other}"
                )))
            }
            None => return Err(invalid("Anthropic response has no stop reason")),
        };
        if blocks.is_empty() && matches!(stop, LlmStopReason::Complete | LlmStopReason::ToolUse) {
            return Err(invalid("Anthropic response has no supported content"));
        }
        validate_decoded_blocks(&blocks, stop, "Anthropic response")?;
        Ok(output(
            response.get("model"),
            requested_model,
            blocks,
            stop,
            parse_usage(&response, "input_tokens", "output_tokens"),
        ))
    }
}

#[derive(Debug, Default)]
pub struct OpenAiResponsesAdapter;

impl ProtocolAdapter for OpenAiResponsesAdapter {
    fn protocol(&self) -> LlmProtocol {
        LlmProtocol::OpenAiResponses
    }

    fn endpoint_path(&self) -> &'static str {
        "/v1/responses"
    }

    fn encode(&self, request: &LlmRequest, model: &str) -> Result<Value, LlmError> {
        request.validate()?;
        let mut input = Vec::new();
        for message in &request.messages {
            match message {
                LlmMessage::User(blocks) => {
                    let mut text_blocks = Vec::new();
                    for block in blocks {
                        match block {
                            LlmInputBlock::Text { text } => {
                                text_blocks.push(json!({"type": "input_text", "text": text}));
                            }
                            LlmInputBlock::ToolResult {
                                tool_call_id,
                                payload_json,
                                ..
                            } => {
                                require_request_json(payload_json, "tool result")?;
                                input.push(json!({
                                    "type": "function_call_output",
                                    "call_id": tool_call_id,
                                    "output": payload_json,
                                }));
                            }
                        }
                    }
                    if !text_blocks.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "user",
                            "content": text_blocks,
                        }));
                    }
                }
                LlmMessage::Assistant(blocks) => {
                    let mut text_blocks = Vec::new();
                    for block in blocks {
                        match block {
                            LlmOutputBlock::Text { text } => {
                                text_blocks.push(json!({"type": "input_text", "text": text}));
                            }
                            LlmOutputBlock::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                push_responses_message(&mut input, "assistant", &mut text_blocks);
                                require_request_json_object(arguments_json, "tool arguments")?;
                                input.push(json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": arguments_json,
                                }));
                            }
                        }
                    }
                    push_responses_message(&mut input, "assistant", &mut text_blocks);
                }
            }
        }
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": model,
            "input": input,
            "store": false,
            "stream": false,
        });
        if let Some(instructions) = &request.instructions {
            body["instructions"] = Value::String(instructions.clone());
        }
        add_thinking_hint(&mut body, request.thinking, true);
        if let Some(max_tokens) = request.max_output_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
            body["tool_choice"] = Value::String(
                match request.tool_choice {
                    LlmToolChoice::None => "none",
                    LlmToolChoice::Auto => "auto",
                    LlmToolChoice::Required => "required",
                }
                .into(),
            );
        }
        Ok(body)
    }

    fn decode(&self, response: Value, requested_model: &str) -> Result<LlmOutput, LlmError> {
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Responses API response has no status"))?;
        if status == "failed" {
            let message = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Responses API returned failed status");
            return Err(invalid(message));
        }
        if matches!(status, "queued" | "in_progress") {
            return Err(invalid(format!(
                "Responses API returned non-terminal status: {status}"
            )));
        }
        if !matches!(status, "completed" | "incomplete" | "cancelled") {
            return Err(invalid(format!(
                "Responses API returned unsupported status: {status}"
            )));
        }
        let items = response
            .get("output")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Responses API response has no output array"))?;
        let mut blocks = Vec::new();
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    let content = item
                        .get("content")
                        .and_then(Value::as_array)
                        .ok_or_else(|| invalid("Responses message has no content array"))?;
                    for part in content {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            let text = string_at(part, "text", "Responses output text")?;
                            if !text.is_empty() {
                                blocks.push(LlmOutputBlock::Text { text });
                            }
                        }
                    }
                }
                Some("function_call") => {
                    if item
                        .get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| status != "completed")
                    {
                        return Err(invalid(
                            "Responses function call is not in completed status",
                        ));
                    }
                    let arguments_json = string_at(item, "arguments", "Responses arguments")?;
                    require_json_object(&arguments_json, "Responses arguments")?;
                    blocks.push(LlmOutputBlock::ToolCall {
                        id: string_at(item, "call_id", "Responses call id")?,
                        name: string_at(item, "name", "Responses function name")?,
                        arguments_json,
                    });
                }
                Some(_) => {}
                None => return Err(invalid("Responses output item has no type")),
            }
        }
        let stop = match status {
            "completed"
                if blocks
                    .iter()
                    .any(|block| matches!(block, LlmOutputBlock::ToolCall { .. })) =>
            {
                LlmStopReason::ToolUse
            }
            "completed" => LlmStopReason::Complete,
            "incomplete"
                if response
                    .pointer("/incomplete_details/reason")
                    .and_then(Value::as_str)
                    == Some("max_output_tokens") =>
            {
                LlmStopReason::MaxTokens
            }
            "incomplete" | "cancelled" => LlmStopReason::Incomplete,
            _ => unreachable!("terminal response status was validated above"),
        };
        if blocks.is_empty() && stop == LlmStopReason::Complete {
            return Err(invalid("Responses API response has no supported output"));
        }
        validate_decoded_blocks(&blocks, stop, "Responses API response")?;
        Ok(output(
            response.get("model"),
            requested_model,
            blocks,
            stop,
            parse_usage(&response, "input_tokens", "output_tokens"),
        ))
    }
}

fn push_responses_message(input: &mut Vec<Value>, role: &str, content: &mut Vec<Value>) {
    if !content.is_empty() {
        input.push(json!({
            "type": "message",
            "role": role,
            "content": std::mem::take(content),
        }));
    }
}

fn add_openai_tools(body: &mut Value, request: &LlmRequest) {
    if request.tools.is_empty() {
        return;
    }
    body["tools"] = Value::Array(
        request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect(),
    );
    body["tool_choice"] = Value::String(
        match request.tool_choice {
            LlmToolChoice::None => "none",
            LlmToolChoice::Auto => "auto",
            LlmToolChoice::Required => "required",
        }
        .into(),
    );
}

fn add_thinking_hint(body: &mut Value, thinking: Option<bool>, responses: bool) {
    match (thinking, responses) {
        (Some(false), false) => body["thinking"] = json!({"type": "disabled"}),
        (Some(false), true) => body["reasoning"] = json!({"effort": "none"}),
        (Some(true), false) => body["thinking"] = json!({"type": "enabled"}),
        (Some(true), true) => body["reasoning"] = json!({"effort": "high"}),
        (None, _) => {}
    }
}

fn validate_decoded_blocks(
    blocks: &[LlmOutputBlock],
    stop_reason: LlmStopReason,
    label: &str,
) -> Result<(), LlmError> {
    let mut tool_call_ids = HashSet::new();
    let mut has_tool_call = false;
    for block in blocks {
        if let LlmOutputBlock::ToolCall {
            id,
            name,
            arguments_json,
        } = block
        {
            has_tool_call = true;
            if id.trim().is_empty() || name.trim().is_empty() {
                return Err(invalid(format!(
                    "{label} contains a tool call without an id or name"
                )));
            }
            require_json_object(arguments_json, &format!("{label} tool arguments"))?;
            if !tool_call_ids.insert(id.as_str()) {
                return Err(invalid(format!(
                    "{label} contains duplicate tool call id: {id}"
                )));
            }
        }
    }

    match (stop_reason, has_tool_call) {
        (LlmStopReason::ToolUse, false) => Err(invalid(format!(
            "{label} stopped for tool use without a call"
        ))),
        (LlmStopReason::ToolUse, true) => Ok(()),
        (_, true) => Err(invalid(format!(
            "{label} returned a tool call without a completed tool-use stop"
        ))),
        (_, false) => Ok(()),
    }
}

fn output(
    model: Option<&Value>,
    requested_model: &str,
    blocks: Vec<LlmOutputBlock>,
    stop_reason: LlmStopReason,
    usage: Option<LlmUsage>,
) -> LlmOutput {
    LlmOutput {
        blocks,
        stop_reason,
        usage,
        requested_model: requested_model.into(),
        resolved_model: model
            .and_then(Value::as_str)
            .unwrap_or(requested_model)
            .into(),
    }
}

fn parse_usage(response: &Value, input_key: &str, output_key: &str) -> Option<LlmUsage> {
    let usage = response.get("usage")?;
    let input_tokens = usage.get(input_key)?.as_u64()?;
    let output_tokens = usage.get(output_key)?.as_u64()?;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cost_microusd: None,
    })
}

fn string_at(value: &Value, key: &str, label: &str) -> Result<String, LlmError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("{label} is missing or not a string")))
}

fn require_json(value: &str, label: &str) -> Result<Value, LlmError> {
    serde_json::from_str(value)
        .map_err(|error| invalid(format!("{label} is invalid JSON: {error}")))
}

fn require_json_object(value: &str, label: &str) -> Result<Value, LlmError> {
    let value = require_json(value, label)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(invalid(format!("{label} must be a JSON object")))
    }
}

fn require_request_json(value: &str, label: &str) -> Result<Value, LlmError> {
    serde_json::from_str(value).map_err(|error| {
        LlmError::new(
            LlmErrorKind::RequestInvalid,
            format!("{label} is invalid JSON: {error}"),
        )
    })
}

fn require_request_json_object(value: &str, label: &str) -> Result<Value, LlmError> {
    let value = require_request_json(value, label)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(LlmError::new(
            LlmErrorKind::RequestInvalid,
            format!("{label} must be a JSON object"),
        ))
    }
}

fn invalid(message: impl Into<String>) -> LlmError {
    LlmError::new(LlmErrorKind::ResponseInvalid, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> crate::LlmToolDefinition {
        crate::LlmToolDefinition {
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

    fn tool_round_request() -> LlmRequest {
        LlmRequest {
            instructions: Some("Use only granted documents.".into()),
            thinking: None,
            messages: vec![
                LlmMessage::User(vec![LlmInputBlock::Text {
                    text: "Read the document".into(),
                }]),
                LlmMessage::Assistant(vec![LlmOutputBlock::ToolCall {
                    id: "call-1".into(),
                    name: "read_document".into(),
                    arguments_json: r#"{"documentId":"doc-1"}"#.into(),
                }]),
                LlmMessage::User(vec![
                    LlmInputBlock::Text {
                        text: "Summarize it".into(),
                    },
                    LlmInputBlock::ToolResult {
                        tool_call_id: "call-1".into(),
                        payload_json: r#"{"title":"Example"}"#.into(),
                        is_error: false,
                    },
                ]),
            ],
            tools: vec![tool()],
            tool_choice: LlmToolChoice::Auto,
            max_output_tokens: Some(256),
        }
    }

    #[test]
    fn chat_completion_golden_tool_round_trip() {
        let adapter = OpenAiChatCompletionsAdapter;
        let payload = adapter.encode(&tool_round_request(), "route-a").unwrap();

        assert_eq!(adapter.endpoint_path(), "/v1/chat/completions");
        assert_eq!(payload["stream"], false);
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][2]["tool_calls"][0]["id"], "call-1");
        assert_eq!(payload["messages"][3]["tool_call_id"], "call-1");
        assert_eq!(payload["messages"][4]["role"], "user");
        assert_eq!(payload["tools"][0]["function"]["name"], "read_document");

        let output = adapter
            .decode(
                json!({
                    "model": "resolved-a",
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call-2",
                                "type": "function",
                                "function": {
                                    "name": "read_document",
                                    "arguments": "{\"documentId\":\"doc-2\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}
                }),
                "route-a",
            )
            .unwrap();
        assert_eq!(output.resolved_model, "resolved-a");
        assert_eq!(output.stop_reason, LlmStopReason::ToolUse);
        assert!(matches!(
            &output.blocks[0],
            LlmOutputBlock::ToolCall { id, name, .. }
                if id == "call-2" && name == "read_document"
        ));
    }

    #[test]
    fn anthropic_golden_puts_tool_results_before_user_text() {
        let adapter = AnthropicMessagesAdapter;
        let payload = adapter.encode(&tool_round_request(), "route-b").unwrap();

        assert_eq!(adapter.endpoint_path(), "/v1/messages");
        assert_eq!(payload["system"], "Use only granted documents.");
        assert_eq!(payload["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(payload["messages"][2]["content"][1]["type"], "text");
        assert_eq!(payload["tools"][0]["input_schema"]["type"], "object");

        let output = adapter
            .decode(
                json!({
                    "model": "resolved-b",
                    "stop_reason": "tool_use",
                    "content": [{
                        "type": "tool_use",
                        "id": "call-2",
                        "name": "read_document",
                        "input": {"documentId": "doc-2"}
                    }],
                    "usage": {"input_tokens": 10, "output_tokens": 5}
                }),
                "route-b",
            )
            .unwrap();
        assert_eq!(output.stop_reason, LlmStopReason::ToolUse);
        assert_eq!(output.usage.unwrap().total_tokens, 15);
    }

    #[test]
    fn responses_golden_is_stateless_and_replays_supported_items() {
        let adapter = OpenAiResponsesAdapter;
        let payload = adapter.encode(&tool_round_request(), "route-c").unwrap();

        assert_eq!(adapter.endpoint_path(), "/v1/responses");
        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], false);
        assert!(payload.get("previous_response_id").is_none());
        assert!(payload.get("conversation").is_none());
        assert_eq!(payload["input"][1]["type"], "function_call");
        assert_eq!(payload["input"][2]["type"], "function_call_output");
        assert_eq!(payload["input"][3]["content"][0]["type"], "input_text");

        let output = adapter
            .decode(
                json!({
                    "model": "resolved-c",
                    "status": "completed",
                    "output": [
                        {"type": "message", "role": "assistant", "content": [
                            {"type": "output_text", "text": "Done"}
                        ]},
                        {"type": "function_call", "call_id": "call-2", "name": "read_document",
                         "arguments": "{\"documentId\":\"doc-2\"}"}
                    ],
                    "usage": {"input_tokens": 14, "output_tokens": 6, "total_tokens": 20}
                }),
                "route-c",
            )
            .unwrap();
        assert_eq!(output.resolved_model, "resolved-c");
        assert_eq!(output.stop_reason, LlmStopReason::ToolUse);
        assert_eq!(output.blocks.len(), 2);
    }

    #[test]
    fn every_adapter_rejects_incomplete_tool_arguments() {
        let chat = OpenAiChatCompletionsAdapter.decode(
            json!({
                "choices": [{"finish_reason": "tool_calls", "message": {
                    "tool_calls": [{"id": "call-1", "function": {
                        "name": "read_document", "arguments": "{"
                    }}]
                }}]
            }),
            "route",
        );
        let responses = OpenAiResponsesAdapter.decode(
            json!({
                "status": "completed",
                "output": [{"type": "function_call", "call_id": "call-1",
                    "name": "read_document", "arguments": "{"}]
            }),
            "route",
        );

        assert_eq!(chat.unwrap_err().kind, LlmErrorKind::ResponseInvalid);
        assert_eq!(responses.unwrap_err().kind, LlmErrorKind::ResponseInvalid);
    }
}
