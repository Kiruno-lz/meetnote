//! OpenRouter's OAuth PKCE, which ends in an ordinary API key.
//!
//! Per https://openrouter.ai/docs/guides/overview/auth/oauth (read
//! 2026-11-20): the browser opens
//! `https://openrouter.ai/auth?callback_url=..&code_challenge=..&code_challenge_method=S256`,
//! "localhost callbacks are supported on any port", an optional `state` is
//! "returned unchanged as a `state` query parameter on the callback URL",
//! and `POST https://openrouter.ai/api/v1/auth/keys` with
//! `{code, code_verifier, code_challenge_method}` answers `{key}`. Codes are
//! single-use and expire after 10 minutes. So the callback listens on an
//! ephemeral loopback port and the state is checked like any other sign-in.

use super::{CallbackServer, Pkce};
use crate::http::{endpoint_url, Http, VENDOR_TIMEOUT};
use crate::{LlmCancellation, LlmError, LlmErrorKind};
use serde_json::{json, Value};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRouterEndpoints {
    /// The site: `https://openrouter.ai`.
    pub site: String,
}

impl Default for OpenRouterEndpoints {
    fn default() -> Self {
        Self {
            site: "https://openrouter.ai".into(),
        }
    }
}

/// The loopback URL the browser is sent back to on `port`.
pub fn callback_url(port: u16) -> String {
    format!("http://localhost:{port}/callback")
}

pub fn authorize_url(
    endpoints: &OpenRouterEndpoints,
    callback_url: &str,
    pkce: &Pkce,
) -> Result<String, LlmError> {
    let mut url = endpoint_url(&endpoints.site, "/auth")?;
    url.query_pairs_mut()
        .append_pair("callback_url", callback_url)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &pkce.state);
    Ok(url.into())
}

/// Trades the callback's code for an OpenRouter API key.
pub fn exchange_code(
    endpoints: &OpenRouterEndpoints,
    code: &str,
    verifier: &str,
    cancellation: &LlmCancellation,
) -> Result<String, LlmError> {
    let url = endpoint_url(&endpoints.site, "/api/v1/auth/keys")?;
    let body = Http::for_url(url.as_str(), VENDOR_TIMEOUT)?
        .post(url)
        .header("Accept", "application/json")
        .json(&json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send(cancellation)?
        .json()?;
    body.get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| LlmError::new(LlmErrorKind::ResponseInvalid, "OpenRouter sent back no key"))
}

/// One browser sign-in: open `url()`, then `finish` waits for the callback
/// and returns the new API key.
pub struct OpenRouterSignIn {
    endpoints: OpenRouterEndpoints,
    pkce: Pkce,
    server: CallbackServer,
    url: String,
}

impl OpenRouterSignIn {
    pub fn start(endpoints: OpenRouterEndpoints) -> Result<Self, LlmError> {
        let pkce = Pkce::new()?;
        let server =
            CallbackServer::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))?;
        let url = authorize_url(&endpoints, &callback_url(server.port()), &pkce)?;
        Ok(Self {
            endpoints,
            pkce,
            server,
            url,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn finish(
        self,
        cancellation: &LlmCancellation,
        timeout: Duration,
    ) -> Result<String, LlmError> {
        let code = self.server.wait(&self.pkce.state, cancellation, timeout)?;
        drop(self.server);
        exchange_code(&self.endpoints, &code, &self.pkce.verifier, cancellation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_sends_the_challenge_state_and_loopback_callback() {
        let pkce = Pkce::from_parts("verifier".into(), "state-1".into());
        let url = url::Url::parse(
            &authorize_url(&OpenRouterEndpoints::default(), &callback_url(51423), &pkce).unwrap(),
        )
        .unwrap();
        assert_eq!(url.path(), "/auth");
        assert_eq!(url.host_str(), Some("openrouter.ai"));
        let query = url.query_pairs().into_owned().collect::<Vec<_>>();
        assert_eq!(
            query,
            vec![
                (
                    "callback_url".into(),
                    "http://localhost:51423/callback".into()
                ),
                ("code_challenge".into(), pkce.challenge.clone()),
                ("code_challenge_method".into(), "S256".into()),
                ("state".into(), "state-1".into()),
            ]
        );
    }
}
