//! Blocking HTTP shared by the protocol provider and the vendor helpers:
//! loopback-aware HTTPS policy, header-safe secrets, cancellation, a bounded
//! body read and one status-to-error mapping.

use crate::{AuthMode, Credential, LlmCancellation, LlmError, LlmErrorKind, LlmProtocol};
use serde_json::Value;
use std::error::Error as _;
use std::io::Read;
use std::net::IpAddr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use url::Url;

pub(crate) const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// An event stream repeats its output as deltas, so it gets more room.
pub(crate) const MAX_STREAM_BYTES: u64 = 32 * 1024 * 1024;
/// Vendor calls outside a configured connection (listing, sign-in, quota).
pub(crate) const VENDOR_TIMEOUT: Duration = Duration::from_secs(30);

type Headers = Vec<(&'static str, String)>;

#[derive(Clone)]
pub(crate) struct Http {
    agent: ureq::Agent,
}

impl Http {
    /// An agent for `base_url`: HTTPS only, unless the host is loopback.
    pub(crate) fn for_url(base_url: &str, timeout: Duration) -> Result<Self, LlmError> {
        let local = is_local_url(base_url)?;
        let scheme = parse_url(base_url)?.scheme().to_owned();
        if scheme != "https" && !(scheme == "http" && local) {
            return Err(not_configured(
                "base URL must use HTTPS unless it targets localhost",
            ));
        }
        Ok(Self {
            agent: ureq::AgentBuilder::new()
                .timeout(timeout)
                .timeout_connect(timeout)
                .redirects(0)
                .https_only(!local)
                .build(),
        })
    }

    pub(crate) fn get(&self, url: Url) -> Call {
        self.call("GET", url)
    }

    pub(crate) fn post(&self, url: Url) -> Call {
        self.call("POST", url)
    }

    fn call(&self, method: &'static str, url: Url) -> Call {
        Call {
            agent: self.agent.clone(),
            method,
            url,
            headers: Vec::new(),
            body: None,
            limit: MAX_JSON_BYTES,
        }
    }
}

pub(crate) struct Call {
    agent: ureq::Agent,
    method: &'static str,
    url: Url,
    headers: Headers,
    body: Option<(&'static str, String)>,
    limit: u64,
}

impl Call {
    pub(crate) fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub(crate) fn headers(mut self, headers: Headers) -> Self {
        self.headers.extend(headers);
        self
    }

    pub(crate) fn json(mut self, body: &Value) -> Self {
        self.body = Some(("application/json", body.to_string()));
        self
    }

    pub(crate) fn form(mut self, pairs: &[(&str, &str)]) -> Self {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        self.body = Some(("application/x-www-form-urlencoded", body));
        self
    }

    pub(crate) fn limit(mut self, bytes: u64) -> Self {
        self.limit = bytes;
        self
    }

    pub(crate) fn send(self, cancellation: &LlmCancellation) -> Result<Reply, LlmError> {
        let Call {
            agent,
            method,
            url,
            headers,
            body,
            limit,
        } = self;
        run_cancellable(cancellation, move || {
            let mut request = agent.request(method, url.as_str());
            for (name, value) in &headers {
                request = request.set(name, value);
            }
            let result = match body {
                Some((content_type, body)) => {
                    request.set("Content-Type", content_type).send_string(&body)
                }
                None => request.call(),
            };
            read_reply(result, limit)
        })
    }
}

/// A response of any non-redirect status, its body read up to the limit.
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    truncated: bool,
    retry_after_ms: Option<u64>,
    pub(crate) cost_microusd: Option<u64>,
}

impl Reply {
    pub(crate) fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body of a successful reply; any other status is its error.
    pub(crate) fn success(self) -> Result<Self, LlmError> {
        if !self.is_success() {
            return Err(self.error());
        }
        if self.truncated {
            return Err(LlmError::new(
                LlmErrorKind::ResponseInvalid,
                "provider response exceeds the maximum supported size",
            ));
        }
        Ok(self)
    }

    pub(crate) fn json(self) -> Result<Value, LlmError> {
        let reply = self.success()?;
        serde_json::from_slice(&reply.body).map_err(|error| {
            LlmError::new(
                LlmErrorKind::ResponseInvalid,
                format!("provider returned malformed JSON: {error}"),
            )
        })
    }

    /// The body as JSON when it is some, for reading an error's details.
    pub(crate) fn error_body(&self) -> Option<Value> {
        (!self.truncated)
            .then(|| serde_json::from_slice(&self.body).ok())
            .flatten()
    }

    pub(crate) fn error(&self) -> LlmError {
        let status = self.status;
        let message = self
            .error_body()
            .as_ref()
            .and_then(error_message)
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
        LlmError::new(kind, message)
            .with_status(status)
            .with_retry_after(self.retry_after_ms)
    }
}

/// What an error body says, in the shapes APIs and OAuth servers use.
pub(crate) fn error_message(body: &Value) -> Option<String> {
    [
        "/error/message",
        "/error_description",
        "/detail",
        "/message",
        "/error",
    ]
    .iter()
    .find_map(|pointer| body.pointer(pointer).and_then(Value::as_str))
    .filter(|message| !message.trim().is_empty())
    .map(str::to_owned)
}

fn read_reply(result: Result<ureq::Response, ureq::Error>, limit: u64) -> Result<Reply, LlmError> {
    let response = match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(error)) => {
            let kind = if transport_timed_out(&error) {
                LlmErrorKind::Timeout
            } else {
                LlmErrorKind::Transport
            };
            return Err(LlmError::new(kind, error.to_string()));
        }
    };
    let status = response.status();
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
    let cost_microusd = response
        .header("x-litellm-response-cost-original")
        .or_else(|| response.header("x-litellm-response-cost"))
        .and_then(parse_cost_microusd);
    let mut body = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|error| LlmError::new(LlmErrorKind::Transport, error.to_string()))?;
    let truncated = body.len() as u64 > limit;
    body.truncate(limit as usize);
    Ok(Reply {
        status,
        body,
        truncated,
        retry_after_ms,
        cost_microusd,
    })
}

pub(crate) fn run_cancellable<T: Send + 'static>(
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

/// The authentication headers a standard endpoint takes.
pub(crate) fn auth_headers(
    protocol: LlmProtocol,
    auth_mode: AuthMode,
    credential: Option<&Credential>,
) -> Result<Headers, LlmError> {
    let mut headers = Vec::new();
    if let Some(credential) = credential {
        let secret = header_secret(credential)?;
        headers.push(match auth_mode {
            AuthMode::Bearer => ("Authorization", format!("Bearer {secret}")),
            AuthMode::XApiKey => ("x-api-key", secret.to_owned()),
        });
    }
    if protocol == LlmProtocol::AnthropicMessages {
        headers.push(("anthropic-version", "2023-06-01".into()));
    }
    Ok(headers)
}

/// A secret that can travel in a header: non-empty visible ASCII.
pub(crate) fn header_secret(credential: &Credential) -> Result<&str, LlmError> {
    let secret = credential.expose();
    if secret.is_empty() || !secret.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(LlmError::new(
            LlmErrorKind::Authentication,
            "connection credential is empty or cannot be represented as an HTTP header",
        ));
    }
    Ok(secret)
}

/// `base_url` with `path` appended; the base already carries its version.
pub(crate) fn endpoint_url(base_url: &str, path: &str) -> Result<Url, LlmError> {
    let mut url = parse_url(base_url)?;
    let joined = format!("{}{path}", url.path().trim_end_matches('/'));
    url.set_path(&joined);
    Ok(url)
}

pub(crate) fn parse_url(value: &str) -> Result<Url, LlmError> {
    Url::parse(value).map_err(|error| not_configured(format!("invalid base URL: {error}")))
}

pub(crate) fn is_local_url(value: &str) -> Result<bool, LlmError> {
    let url = parse_url(value)?;
    let host = url
        .host_str()
        .ok_or_else(|| not_configured("base URL has no host"))?;
    Ok(is_loopback_host(host))
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn not_configured(message: impl Into<String>) -> LlmError {
    LlmError::new(LlmErrorKind::ConnectionNotConfigured, message)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_paths_append_to_the_versioned_base() {
        for (base, path, expected) in [
            (
                "https://api.openai.com/v1",
                "/responses",
                "https://api.openai.com/v1/responses",
            ),
            (
                "https://open.bigmodel.cn/api/paas/v4/",
                "/chat/completions",
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            ),
            (
                "http://127.0.0.1:4000/v1",
                "/models",
                "http://127.0.0.1:4000/v1/models",
            ),
        ] {
            assert_eq!(endpoint_url(base, path).unwrap().as_str(), expected);
        }
    }

    #[test]
    fn error_messages_are_read_from_api_and_oauth_shapes() {
        for (body, expected) in [
            (json!({"error": {"message": "api"}}), Some("api")),
            (
                json!({"error": "invalid_grant", "error_description": "oauth"}),
                Some("oauth"),
            ),
            (json!({"detail": "litellm"}), Some("litellm")),
            (json!({"error": "bare"}), Some("bare")),
            (json!({"error": {"code": "x"}}), None),
        ] {
            assert_eq!(error_message(&body).as_deref(), expected, "{body}");
        }
    }

    #[test]
    fn plaintext_is_only_allowed_on_loopback() {
        assert!(Http::for_url("http://localhost:11434/v1", VENDOR_TIMEOUT).is_ok());
        assert!(Http::for_url("http://[::1]:1234/v1", VENDOR_TIMEOUT).is_ok());
        assert!(Http::for_url("https://api.x.ai/v1", VENDOR_TIMEOUT).is_ok());
        assert_eq!(
            Http::for_url("http://api.x.ai/v1", VENDOR_TIMEOUT)
                .err()
                .unwrap()
                .kind,
            LlmErrorKind::ConnectionNotConfigured
        );
    }
}
