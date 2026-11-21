//! Signing in with a ChatGPT subscription, as `codex login` does: Codex CLI's
//! OAuth client, its fixed loopback callback and its token refresh. The
//! tokens then reach the Codex backend (`Backend::ChatGpt`). Ported from
//! magpie's signin.go, account.go and codex_signin_gone.go.

use super::{jwt_claims, CallbackServer, Pkce};
use crate::codex;
use crate::http::{endpoint_url, Http, VENDOR_TIMEOUT};
use crate::{LlmCancellation, LlmError, LlmErrorKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

/// Codex CLI's own OAuth client.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
/// OpenAI sends Codex's client back to this port only.
pub const CALLBACK_ADDRESS: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1455));
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
/// An access token this close to its expiry is refreshed first.
pub const REFRESH_MARGIN_SECONDS: u64 = 300;

const AUTH_CLAIM: &str = "https://api.openai.com/auth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptEndpoints {
    /// The OAuth server: `https://auth.openai.com`.
    pub auth: String,
    /// The ChatGPT backend: `https://chatgpt.com/backend-api`.
    pub backend: String,
}

impl Default for ChatGptEndpoints {
    fn default() -> Self {
        Self {
            auth: "https://auth.openai.com".into(),
            backend: "https://chatgpt.com/backend-api".into(),
        }
    }
}

impl ChatGptEndpoints {
    /// The base URL of a `Backend::ChatGpt` connection.
    pub fn codex_base_url(&self) -> String {
        format!("{}/codex", self.backend.trim_end_matches('/'))
    }

    fn token_url(&self) -> Result<url::Url, LlmError> {
        endpoint_url(&self.auth, "/oauth/token")
    }
}

/// A ChatGPT sign-in: its tokens and what their claims say.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    pub email: String,
    pub plan: Option<String>,
    /// When the access token expires, in Unix seconds (0 when unknown).
    pub expires_at: u64,
    /// When the paid plan runs out, in Unix seconds.
    pub subscription_until: Option<u64>,
}

impl ChatGptTokens {
    /// Reads the account off the tokens; `account_id` is used when the ID
    /// token does not name one.
    pub fn from_tokens(
        id_token: &str,
        access_token: &str,
        refresh_token: &str,
        account_id: Option<&str>,
    ) -> Result<Self, LlmError> {
        if access_token.is_empty() || refresh_token.is_empty() || id_token.is_empty() {
            return Err(invalid("ChatGPT sent back no token"));
        }
        let id = jwt_claims(id_token).unwrap_or_default();
        let auth = id.get(AUTH_CLAIM).cloned().unwrap_or_default();
        let claim = |value: &Value, key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let email = claim(&id, "email")
            .ok_or_else(|| invalid("ChatGPT did not say which account signed in"))?;
        let account_id = claim(&auth, "chatgpt_account_id")
            .or_else(|| account_id.filter(|id| !id.is_empty()).map(str::to_owned))
            .ok_or_else(|| invalid("ChatGPT did not say which workspace signed in"))?;
        let expires_at = jwt_claims(access_token)
            .and_then(|claims| claims.get("exp").and_then(Value::as_f64))
            .map(|exp| exp.max(0.0) as u64)
            .unwrap_or(0);
        Ok(Self {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            id_token: id_token.into(),
            account_id,
            email,
            plan: claim(&auth, "chatgpt_plan_type"),
            expires_at,
            subscription_until: auth
                .get("chatgpt_subscription_active_until")
                .and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(parse_rfc3339))
                }),
        })
    }

    /// Whether the access token expires within `REFRESH_MARGIN_SECONDS`.
    pub fn needs_refresh(&self, now: u64) -> bool {
        self.expires_at <= now.saturating_add(REFRESH_MARGIN_SECONDS)
    }
}

impl fmt::Debug for ChatGptTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatGptTokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("id_token", &"<redacted>")
            .field("account_id", &self.account_id)
            .field("email", &self.email)
            .field("plan", &self.plan)
            .field("expires_at", &self.expires_at)
            .field("subscription_until", &self.subscription_until)
            .finish()
    }
}

/// The page that signs in, with Codex CLI's parameters.
pub fn authorize_url(endpoints: &ChatGptEndpoints, pkce: &Pkce) -> Result<String, LlmError> {
    let mut url = endpoint_url(&endpoints.auth, "/oauth/authorize")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", &pkce.state)
        .append_pair("originator", codex::ORIGINATOR);
    Ok(url.into())
}

/// Trades the callback's code for tokens, as Codex sends it (form-encoded).
pub fn exchange_code(
    endpoints: &ChatGptEndpoints,
    code: &str,
    verifier: &str,
    cancellation: &LlmCancellation,
) -> Result<ChatGptTokens, LlmError> {
    let url = endpoints.token_url()?;
    let body = Http::for_url(url.as_str(), VENDOR_TIMEOUT)?
        .post(url)
        .headers(codex::client_headers())
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send(cancellation)?
        .json()?;
    let token = |key: &str| body.get(key).and_then(Value::as_str).unwrap_or_default();
    ChatGptTokens::from_tokens(
        token("id_token"),
        token("access_token"),
        token("refresh_token"),
        None,
    )
}

/// Renews the tokens. A refusal OpenAI means for good (the refresh token
/// expired, was reused or revoked) is `Authentication`; any other failure
/// is a hiccup to retry, never `Authentication`.
pub fn refresh(
    endpoints: &ChatGptEndpoints,
    tokens: &ChatGptTokens,
    cancellation: &LlmCancellation,
) -> Result<ChatGptTokens, LlmError> {
    let url = endpoints.token_url()?;
    let reply = Http::for_url(url.as_str(), VENDOR_TIMEOUT)?
        .post(url)
        .headers(codex::client_headers())
        .json(&json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": tokens.refresh_token,
        }))
        .send(cancellation)?;
    if !reply.is_success() {
        let code = reply
            .error_body()
            .map(|body| refresh_error_code(&body).to_ascii_lowercase())
            .unwrap_or_default();
        if refresh_gone(reply.status, &code) {
            return Err(LlmError::new(
                LlmErrorKind::Authentication,
                "the ChatGPT sign-in has expired or was revoked; sign in again",
            )
            .with_status(reply.status));
        }
        let mut error = reply.error();
        if error.kind == LlmErrorKind::Authentication {
            error = LlmError {
                kind: LlmErrorKind::Transport,
                retryable: true,
                ..error
            };
        }
        return Err(error);
    }
    let body = reply.json()?;
    let token = |key: &str, old: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .unwrap_or(old)
            .to_owned()
    };
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| invalid("ChatGPT refreshed the sign-in without an access token"))?;
    ChatGptTokens::from_tokens(
        &token("id_token", &tokens.id_token),
        access_token,
        &token("refresh_token", &tokens.refresh_token),
        Some(&tokens.account_id),
    )
}

/// The code a token endpoint's refusal carries, read as codex-rs reads it:
/// `error` as a string, then `error.code`, then a top-level `code`.
fn refresh_error_code(body: &Value) -> &str {
    body.get("error")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/error/code").and_then(Value::as_str))
        .or_else(|| body.get("code").and_then(Value::as_str))
        .unwrap_or_default()
}

/// Codex's rule for a refusal that is for good. A 401 counts only with the
/// endpoint's own code in it: one without (a proxy's page) says nothing.
fn refresh_gone(status: u16, code: &str) -> bool {
    matches!(
        code,
        "refresh_token_expired" | "refresh_token_reused" | "refresh_token_invalidated"
    ) || status == 400 && code == "invalid_grant"
        || status == 401 && !code.is_empty()
}

/// Reads Codex CLI's `~/.codex/auth.json`: `{"auth_mode":"chatgpt",
/// "tokens":{id_token, access_token, refresh_token, account_id}, ...}`.
pub fn import_codex_auth(json: &str) -> Result<ChatGptTokens, LlmError> {
    let auth: Value = serde_json::from_str(json)
        .map_err(|error| invalid(format!("Codex auth.json is not JSON: {error}")))?;
    if auth
        .get("auth_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode != "chatgpt")
    {
        return Err(invalid("Codex is signed in with an API key, not ChatGPT"));
    }
    let tokens = auth
        .get("tokens")
        .filter(|tokens| tokens.is_object())
        .ok_or_else(|| invalid("Codex auth.json holds no ChatGPT sign-in"))?;
    let token = |key: &str| tokens.get(key).and_then(Value::as_str).unwrap_or_default();
    ChatGptTokens::from_tokens(
        token("id_token"),
        token("access_token"),
        token("refresh_token"),
        Some(token("account_id")),
    )
}

/// One browser sign-in: open `url()`, then `finish` waits for the callback
/// on port 1455 and trades its code for tokens.
pub struct ChatGptSignIn {
    endpoints: ChatGptEndpoints,
    pkce: Pkce,
    server: CallbackServer,
    url: String,
}

impl ChatGptSignIn {
    pub fn start(endpoints: ChatGptEndpoints) -> Result<Self, LlmError> {
        let pkce = Pkce::new()?;
        let url = authorize_url(&endpoints, &pkce)?;
        let server = CallbackServer::bind_reclaiming(CALLBACK_ADDRESS)?;
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
    ) -> Result<ChatGptTokens, LlmError> {
        let code = self.server.wait(&self.pkce.state, cancellation, timeout)?;
        drop(self.server);
        exchange_code(&self.endpoints, &code, &self.pkce.verifier, cancellation)
    }
}

fn invalid(message: impl Into<String>) -> LlmError {
    LlmError::new(LlmErrorKind::ResponseInvalid, message)
}

/// `2026-10-18T09:00:00+00:00` (or `Z`, with optional fractions) as Unix
/// seconds.
fn parse_rfc3339(value: &str) -> Option<u64> {
    let value = value.trim();
    let (date, time) = value.split_once(['T', 't', ' '])?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, offset) = time.split_at(time.find(['Z', 'z', '+', '-'])?);
    let clock = clock.split('.').next()?;
    let mut clock = clock.splitn(3, ':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    let offset_seconds = match offset {
        "Z" | "z" => 0,
        _ => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = offset[1..].split_once(':')?;
            sign * (hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60)
        }
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // days from 1970-01-01 to the civil date (Howard Hinnant's algorithm)
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds;
    u64::try_from(seconds).ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    pub(crate) fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    pub(crate) fn id_token() -> String {
        jwt(json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1",
                "chatgpt_plan_type": "plus",
                "chatgpt_subscription_active_until": "2026-10-18T09:00:00+00:00"
            }
        }))
    }

    #[test]
    fn claims_give_the_account_plan_expiry_and_subscription() {
        let access = jwt(json!({"exp": 1_800_000_000}));
        let tokens = ChatGptTokens::from_tokens(&id_token(), &access, "refresh", None).unwrap();
        assert_eq!(tokens.email, "user@example.com");
        assert_eq!(tokens.account_id, "acct-1");
        assert_eq!(tokens.plan.as_deref(), Some("plus"));
        assert_eq!(tokens.expires_at, 1_800_000_000);
        assert_eq!(tokens.subscription_until, Some(1_792_314_000));
        assert!(!tokens.needs_refresh(1_800_000_000 - 301));
        assert!(tokens.needs_refresh(1_800_000_000 - 300));
        let debug = format!("{tokens:?}");
        assert!(!debug.contains(&access) && !debug.contains("refresh\""));
    }

    #[test]
    fn missing_account_or_tokens_are_rejected() {
        let anonymous = jwt(json!({"sub": "x"}));
        assert!(ChatGptTokens::from_tokens(&anonymous, "a", "r", Some("acct")).is_err());
        assert!(ChatGptTokens::from_tokens(&id_token(), "", "r", None).is_err());
        let no_account = jwt(json!({"email": "user@example.com"}));
        assert_eq!(
            ChatGptTokens::from_tokens(&no_account, "a", "r", Some("acct-2"))
                .unwrap()
                .account_id,
            "acct-2"
        );
    }

    #[test]
    fn authorize_url_carries_codex_cli_parameters() {
        let pkce = Pkce::from_parts("verifier".into(), "state-1".into());
        let url =
            url::Url::parse(&authorize_url(&ChatGptEndpoints::default(), &pkce).unwrap()).unwrap();
        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://auth.openai.com/oauth/authorize")
        );
        let query = url.query_pairs().into_owned().collect::<Vec<_>>();
        let get = |key: &str| {
            query
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("response_type"), Some("code"));
        assert_eq!(get("client_id"), Some(CLIENT_ID));
        assert_eq!(
            get("redirect_uri"),
            Some("http://localhost:1455/auth/callback")
        );
        assert_eq!(get("scope"), Some(SCOPES));
        assert_eq!(get("code_challenge"), Some(pkce.challenge.as_str()));
        assert_eq!(get("code_challenge_method"), Some("S256"));
        assert_eq!(get("id_token_add_organizations"), Some("true"));
        assert_eq!(get("codex_cli_simplified_flow"), Some("true"));
        assert_eq!(get("state"), Some("state-1"));
        assert_eq!(get("originator"), Some("codex_cli_rs"));
        assert_eq!(
            ChatGptEndpoints::default().codex_base_url(),
            "https://chatgpt.com/backend-api/codex"
        );
    }

    #[test]
    fn refusals_for_good_follow_codex_rules() {
        for (status, body, gone) in [
            (400, json!({"error": "invalid_grant"}), true),
            (
                401,
                json!({"error": {"code": "refresh_token_reused"}}),
                true,
            ),
            (403, json!({"code": "refresh_token_expired"}), true),
            (401, json!({"error": "token_expired"}), true),
            (401, json!({"message": "proxy"}), false),
            (400, json!({"error": "invalid_request"}), false),
            (503, json!({}), false),
        ] {
            let code = refresh_error_code(&body).to_ascii_lowercase();
            assert_eq!(refresh_gone(status, &code), gone, "{status} {body}");
        }
    }

    #[test]
    fn codex_auth_json_is_imported() {
        let access = jwt(json!({"exp": 1_900_000_000}));
        let auth = json!({
            "OPENAI_API_KEY": null,
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": id_token(),
                "access_token": access,
                "refresh_token": "refresh",
                "account_id": "acct-1"
            },
            "last_refresh": "2026-11-01T00:00:00Z"
        });
        let tokens = import_codex_auth(&auth.to_string()).unwrap();
        assert_eq!(tokens.account_id, "acct-1");
        assert_eq!(tokens.expires_at, 1_900_000_000);
        assert!(import_codex_auth(r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk"}"#).is_err());
        assert!(import_codex_auth("{").is_err());
    }

    #[test]
    fn rfc3339_times_parse_to_unix_seconds() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2026-10-18T09:00:00+00:00"),
            Some(1_792_314_000)
        );
        assert_eq!(
            parse_rfc3339("2026-10-18T17:00:00.123+08:00"),
            Some(1_792_314_000)
        );
        assert_eq!(parse_rfc3339("2000-02-29T12:00:00Z"), Some(951_825_600));
        assert_eq!(parse_rfc3339("not a time"), None);
    }
}
