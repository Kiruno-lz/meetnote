//! The ChatGPT backend that serves Codex (`chatgpt.com/backend-api/codex`),
//! spoken the way Codex CLI speaks it: the backend gates its newest models on
//! a Codex client, requires Codex's own base instructions, streams only, and
//! keeps nothing (`store: false`). Ported from magpie's codex_request.go and
//! codex_models.go.

use crate::http::{endpoint_url, Http, MAX_STREAM_BYTES};
use crate::{LlmCancellation, LlmError, LlmErrorKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The Codex CLI version requests claim to be and the model list is asked
/// for: the backend leaves out models newer than the asking client.
pub(crate) const CODEX_VERSION: &str = "0.169.1";
pub(crate) const ORIGINATOR: &str = "codex_cli_rs";

/// `codex_cli_rs/0.169.1 (Mac OS; arm64) Apple_Terminal/455`, as Codex CLI
/// names itself. The OS version Codex reads from the system is left out.
pub(crate) fn user_agent() -> String {
    let (os, terminal) = match std::env::consts::OS {
        "macos" => ("Mac OS", "Apple_Terminal/455"),
        "windows" => ("Windows", "WindowsTerminal"),
        _ => ("Linux", "xterm-256color"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("{ORIGINATOR}/{CODEX_VERSION} ({os}; {arch}) {terminal}")
}

/// Every request Codex makes carries its originator and User-Agent.
pub(crate) fn client_headers() -> Vec<(&'static str, String)> {
    vec![
        ("originator", ORIGINATOR.into()),
        ("User-Agent", user_agent()),
    ]
}

/// The headers of a backend request on behalf of `account_id`.
pub(crate) fn backend_headers(access_token: &str, account_id: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Authorization", format!("Bearer {access_token}")),
        ("chatgpt-account-id", account_id.to_owned()),
        ("OpenAI-Beta", "responses=experimental".into()),
        ("version", CODEX_VERSION.into()),
    ];
    headers.extend(client_headers());
    headers
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexModel {
    pub(crate) slug: String,
    pub(crate) instructions: Option<String>,
}

/// The account's Codex models in the backend's priority order, hidden ones
/// left out, each with the instructions Codex sends it with.
pub(crate) fn models(
    http: &Http,
    base_url: &str,
    access_token: &str,
    account_id: &str,
    cancellation: &LlmCancellation,
) -> Result<Vec<CodexModel>, LlmError> {
    let mut url = endpoint_url(base_url, "/models")?;
    url.query_pairs_mut()
        .append_pair("client_version", CODEX_VERSION);
    let body = http
        .get(url)
        .headers(backend_headers(access_token, account_id))
        .header("Accept", "application/json")
        .limit(MAX_STREAM_BYTES)
        .send(cancellation)?
        .json()?;
    let models = parse_models(&body);
    if models.is_empty() {
        return Err(LlmError::new(
            LlmErrorKind::ResponseInvalid,
            "ChatGPT listed no Codex models for this account",
        ));
    }
    Ok(models)
}

fn parse_models(body: &Value) -> Vec<CodexModel> {
    let mut entries = body
        .get("models")
        .and_then(Value::as_array)
        .map(|models| models.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    entries.sort_by_key(|entry| entry.get("priority").and_then(Value::as_i64).unwrap_or(0));
    entries
        .into_iter()
        .filter(|entry| entry.get("visibility").and_then(Value::as_str) != Some("hide"))
        .filter_map(|entry| {
            let slug = entry.get("slug").and_then(Value::as_str)?;
            let instructions = entry
                .pointer("/model_messages/instructions_template")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    entry
                        .get("base_instructions")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                })
                .map(str::to_owned);
            (!slug.is_empty()).then(|| CodexModel {
                slug: slug.to_owned(),
                instructions,
            })
        })
        .collect()
}

/// A Responses body rewritten for the backend (magpie `codexBody`): our
/// instructions become a leading developer message and Codex's own take
/// their place; sampling and length limits go, the backend refuses them.
/// Returns the body and its conversation key, sent as the session headers.
pub(crate) fn request_body(
    mut body: Value,
    codex_instructions: &str,
    model: &str,
) -> (Value, String) {
    let Some(object) = body.as_object_mut() else {
        return (body, String::new());
    };
    for key in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "previous_response_id",
        "user",
    ] {
        object.remove(key);
    }
    let own = object
        .remove("instructions")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|text| !text.trim().is_empty());
    let mut input = match object.remove("input") {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let key = conversation_key(own.as_deref().unwrap_or_default(), &input);
    if let Some(own) = own {
        input.insert(0, developer_message(&own));
    }
    if instructions_after_input(model) {
        input.insert(0, developer_message(codex_instructions));
    } else {
        object.insert("instructions".into(), json!(codex_instructions));
    }
    object.insert("input".into(), Value::Array(input));
    object.insert("store".into(), json!(false));
    object.insert("stream".into(), json!(true));
    object.insert("tool_choice".into(), json!("auto"));
    // Codex marks every function tool non-strict; strict schemas the
    // backend cannot compile turn the whole request away.
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if tool.get("type").and_then(Value::as_str) == Some("function") {
                tool["strict"] = json!(false);
            }
        }
    }
    object.entry("parallel_tool_calls").or_insert(json!(true));
    object
        .entry("text")
        .or_insert(json!({"verbosity": "medium"}));
    let effort = object
        .get("reasoning")
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str);
    if effort == Some("none") {
        object.remove("include");
    } else {
        if let Some(reasoning) = object.get_mut("reasoning").and_then(Value::as_object_mut) {
            reasoning.entry("summary").or_insert(json!("auto"));
        }
        object.insert("include".into(), json!(["reasoning.encrypted_content"]));
    }
    object.insert("prompt_cache_key".into(), json!(key));
    (body, key)
}

fn developer_message(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "developer",
        "content": [{"type": "input_text", "text": text}],
    })
}

/// GPT-6.2 and later append top-level instructions after the input, out of
/// the cached prefix, so Codex sends them first in the input instead.
fn instructions_after_input(model: &str) -> bool {
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let Some(version) = model.strip_prefix("gpt-") else {
        return false;
    };
    let version = version.split('-').next().unwrap_or_default();
    let (major, minor) = version.split_once('.').unwrap_or((version, "0"));
    match (major.parse::<u32>(), minor.parse::<u32>()) {
        (Ok(major), Ok(minor)) => major > 6 || major == 6 && minor >= 2,
        _ => false,
    }
}

/// A stable id for the conversation: its instructions and input up to the
/// first user message, shaped as a UUID. The backend finds a conversation's
/// cached prompt by it.
fn conversation_key(instructions: &str, input: &[Value]) -> String {
    let mut hash = Sha256::new();
    hash.update(instructions.as_bytes());
    for item in input {
        hash.update(item.to_string().as_bytes());
        if item.get("role").and_then(Value::as_str) == Some("user") {
            break;
        }
    }
    let hex = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The final Responses object of an event stream. The backend may finish
/// with an empty `output` and send each item in `response.output_item.done`
/// instead, so those are collected and stand in for it.
pub(crate) fn read_stream(body: &[u8]) -> Result<Value, LlmError> {
    let text = String::from_utf8_lossy(body);
    let mut items = Vec::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    items.push(item.clone());
                }
            }
            Some("response.completed" | "response.incomplete" | "response.done") => {
                let mut response = event.get("response").cloned().ok_or_else(|| {
                    LlmError::new(
                        LlmErrorKind::ResponseInvalid,
                        "ChatGPT stream ended without its response",
                    )
                })?;
                let empty = response
                    .get("output")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty);
                if empty {
                    response["output"] = Value::Array(items);
                }
                return Ok(response);
            }
            Some("response.failed" | "error") => return Err(stream_error(&event)),
            _ => {}
        }
    }
    Err(LlmError::new(
        LlmErrorKind::ResponseInvalid,
        "ChatGPT stream ended before the response completed",
    ))
}

fn stream_error(event: &Value) -> LlmError {
    let error = event
        .pointer("/response/error")
        .or_else(|| event.get("error"))
        .unwrap_or(event);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .unwrap_or("ChatGPT failed the response");
    let code = ["code", "type"]
        .iter()
        .find_map(|key| error.get(*key).and_then(Value::as_str))
        .unwrap_or_default();
    let kind = match code {
        "rate_limit_exceeded" | "usage_limit_reached" | "usage_not_included" => {
            LlmErrorKind::RateLimited
        }
        "context_length_exceeded" => LlmErrorKind::ContextLimitExceeded,
        _ => LlmErrorKind::ResponseInvalid,
    };
    LlmError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_codex_cli() {
        let agent = user_agent();
        assert!(agent.starts_with("codex_cli_rs/0.169.1 ("), "{agent}");
        assert!(!agent.contains("aarch64"));
    }

    #[test]
    fn models_follow_priority_skip_hidden_and_prefer_the_template() {
        let models = parse_models(&json!({"models": [
            {"slug": "gpt-6-luna", "priority": 3, "base_instructions": "luna base"},
            {"slug": "gpt-6-sol", "priority": 1, "base_instructions": "sol base",
             "model_messages": {"instructions_template": "sol template"}},
            {"slug": "codex-auto-review", "priority": 2, "visibility": "hide"},
            {"slug": "gpt-6-astra", "priority": 2}
        ]}));
        assert_eq!(
            models,
            vec![
                CodexModel {
                    slug: "gpt-6-sol".into(),
                    instructions: Some("sol template".into())
                },
                CodexModel {
                    slug: "gpt-6-astra".into(),
                    instructions: None
                },
                CodexModel {
                    slug: "gpt-6-luna".into(),
                    instructions: Some("luna base".into())
                },
            ]
        );
    }

    fn responses_body() -> Value {
        json!({
            "model": "gpt-6-sol",
            "instructions": "Use only granted documents.",
            "input": [{"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "hi"}]}],
            "tools": [{"type": "function", "name": "read", "parameters": {"type": "object"}}],
            "tool_choice": "required",
            "max_output_tokens": 512,
            "store": false,
            "stream": false
        })
    }

    #[test]
    fn body_moves_our_instructions_into_a_developer_message() {
        let (body, key) = request_body(responses_body(), "You are Codex.", "gpt-6-sol");
        assert_eq!(body["instructions"], "You are Codex.");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "Use only granted documents."
        );
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["tools"][0]["strict"], false);
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["text"]["verbosity"], "medium");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert!(body.get("max_output_tokens").is_none());
        assert_eq!(body["prompt_cache_key"], key);
        assert_eq!(key.len(), 36);
        assert_eq!(key, request_body(responses_body(), "other", "gpt-6-sol").1);
    }

    #[test]
    fn newer_models_take_codex_instructions_first_in_the_input() {
        let (body, _) = request_body(responses_body(), "You are Codex.", "gpt-6.2-sol");
        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"][0]["content"][0]["text"], "You are Codex.");
        assert_eq!(
            body["input"][1]["content"][0]["text"],
            "Use only granted documents."
        );
        for (model, after) in [
            ("gpt-6.2-sol", true),
            ("openai/gpt-7", true),
            ("gpt-6.1-sol", false),
            ("gpt-6-sol", false),
            ("o3", false),
        ] {
            assert_eq!(instructions_after_input(model), after, "{model}");
        }
    }

    #[test]
    fn no_reasoning_means_no_encrypted_reasoning() {
        let mut body = responses_body();
        body["reasoning"] = json!({"effort": "none"});
        let (body, _) = request_body(body, "You are Codex.", "gpt-6-sol");
        assert!(body.get("include").is_none());
        let mut high = responses_body();
        high["reasoning"] = json!({"effort": "high"});
        let (high, _) = request_body(high, "You are Codex.", "gpt-6-sol");
        assert_eq!(high["reasoning"]["summary"], "auto");
    }

    #[test]
    fn stream_completion_collects_items_when_output_is_empty() {
        let stream = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"status\":\"in_progress\",\"output\":[]}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"O\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"OK\"}]}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"gpt-6-sol\",\"output\":[]}}\n\n",
        );
        let response = read_stream(stream.as_bytes()).unwrap();
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"][0]["content"][0]["text"], "OK");
    }

    #[test]
    fn stream_failures_and_truncation_are_errors() {
        let failed = read_stream(
            b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"slow down\"}}}\n\n",
        )
        .unwrap_err();
        assert_eq!(failed.kind, LlmErrorKind::RateLimited);
        assert_eq!(failed.message, "slow down");
        let error = read_stream(
            b"data: {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"bad\"}}\n\n",
        )
        .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::ResponseInvalid);
        assert_eq!(error.message, "bad");
        let cut = read_stream(b"data: {\"type\":\"response.created\"}\n\n").unwrap_err();
        assert_eq!(cut.kind, LlmErrorKind::ResponseInvalid);
    }
}
