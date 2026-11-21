//! Browser sign-in building blocks: PKCE and a loopback callback server.

pub mod chatgpt;
pub mod openrouter;

use crate::{LlmCancellation, LlmError, LlmErrorKind};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const POLL: Duration = Duration::from_millis(100);
const MAX_REQUEST_HEAD: usize = 16 * 1024;

/// An authorization request's PKCE pair (RFC 7636, S256) and its CSRF state.
#[derive(Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    /// A 48-byte verifier and a 24-byte state, base64url without padding.
    pub fn new() -> Result<Self, LlmError> {
        Ok(Self::from_parts(random_token(48)?, random_token(24)?))
    }

    pub fn from_parts(verifier: String, state: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
            state,
        }
    }
}

impl fmt::Debug for Pkce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .field("state", &self.state)
            .finish()
    }
}

fn random_token(bytes: usize) -> Result<String, LlmError> {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer).map_err(|error| {
        LlmError::new(
            LlmErrorKind::Transport,
            format!("the system random source failed: {error}"),
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// A loopback listener the browser is sent back to with the authorization
/// code, at `/auth/callback` or `/callback`; `/cancel` ends the wait.
pub struct CallbackServer {
    listener: TcpListener,
}

impl CallbackServer {
    pub fn bind(address: impl ToSocketAddrs) -> Result<Self, LlmError> {
        let listener = TcpListener::bind(address).map_err(bind_error)?;
        listener.set_nonblocking(true).map_err(bind_error)?;
        Ok(Self { listener })
    }

    /// Binds a fixed port another sign-in may hold: the holder (a waiting
    /// `codex login` or an earlier sign-in) is asked to stop at `/cancel`,
    /// then the bind is retried 10 times, 200 ms apart.
    pub fn bind_reclaiming(address: SocketAddr) -> Result<Self, LlmError> {
        let mut last = None;
        for attempt in 0..10 {
            match Self::bind(address) {
                Ok(server) => return Ok(server),
                Err(error) => last = Some(error),
            }
            if attempt == 0 {
                let _ = ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .get(&format!("http://{address}/cancel"))
                    .call();
            }
            thread::sleep(Duration::from_millis(200));
        }
        Err(last.expect("at least one bind was attempted"))
    }

    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or_default()
    }

    /// Waits for the browser's callback and returns its authorization code.
    /// A callback whose `state` differs from `expected_state` is answered
    /// and ignored, so a stray request cannot end the sign-in.
    pub fn wait(
        &self,
        expected_state: &str,
        cancellation: &LlmCancellation,
        timeout: Duration,
    ) -> Result<String, LlmError> {
        let deadline = Instant::now() + timeout;
        loop {
            cancellation.check()?;
            if Instant::now() >= deadline {
                return Err(LlmError::new(
                    LlmErrorKind::Timeout,
                    "the sign-in timed out; start it again",
                ));
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(outcome) = answer(stream, expected_state) {
                        return outcome;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(LlmError::new(
                        LlmErrorKind::Transport,
                        format!("the sign-in callback failed: {error}"),
                    ))
                }
            }
        }
    }
}

fn bind_error(error: std::io::Error) -> LlmError {
    LlmError::new(
        LlmErrorKind::Transport,
        format!("the sign-in callback port could not be opened: {error}"),
    )
}

/// Answers one request; `Some` ends the wait.
fn answer(mut stream: TcpStream, expected_state: &str) -> Option<Result<String, LlmError>> {
    let target = read_target(&mut stream)?;
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    match url.path() {
        "/auth/callback" | "/callback" => {}
        "/cancel" => {
            respond(&mut stream, 204, "");
            return Some(Err(LlmError::new(
                LlmErrorKind::Cancelled,
                "the sign-in was cancelled by another sign-in",
            )));
        }
        _ => {
            respond(&mut stream, 404, "");
            return None;
        }
    }
    if let Some(error) = query("error") {
        let message = query("error_description").unwrap_or(error);
        respond(&mut stream, 400, &page("Sign-in failed", &message));
        return Some(Err(LlmError::new(LlmErrorKind::Authentication, message)));
    }
    match (query("state"), query("code")) {
        (Some(state), Some(code)) if state == expected_state && !code.is_empty() => {
            respond(
                &mut stream,
                200,
                &page("Signed in — you can close this window", ""),
            );
            Some(Ok(code))
        }
        _ => {
            respond(
                &mut stream,
                400,
                &page(
                    "This link is not from MeetNote's sign-in",
                    "Start the sign-in again from MeetNote.",
                ),
            );
            None
        }
    }
}

/// The request target of a request head, read within a short timeout.
fn read_target(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut head = Vec::new();
    let mut chunk = [0_u8; 2048];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || head.len() + read > MAX_REQUEST_HEAD {
            return None;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let line = String::from_utf8_lossy(&head);
    let mut parts = line.lines().next()?.split_whitespace();
    (parts.next()? == "GET").then_some(())?;
    parts
        .next()
        .filter(|target| target.starts_with('/'))
        .map(str::to_owned)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;text-align:center;margin-top:20vh\">\
         <h1>{title}</h1><p>{}</p></body></html>",
        escape(message),
        title = escape(title)
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The payload of a JWT, decoded without checking its signature: the tokens
/// are the user's own and only their claims are read.
pub(crate) fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let payload = parts.nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_appendix_b() {
        let pkce = Pkce::from_parts(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            "state".into(),
        );
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(!format!("{pkce:?}").contains("dBjftJeZ"));
    }

    #[test]
    fn fresh_pkce_has_the_specified_lengths_and_differs() {
        let first = Pkce::new().unwrap();
        let second = Pkce::new().unwrap();
        assert_eq!(first.verifier.len(), 64);
        assert_eq!(first.state.len(), 32);
        assert_eq!(first.challenge.len(), 43);
        assert_ne!(first.verifier, second.verifier);
    }

    #[test]
    fn jwt_claims_decode_the_payload() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"a@b.c","exp":10}"#);
        let claims = jwt_claims(&format!("e30.{payload}.sig")).unwrap();
        assert_eq!(claims["email"], "a@b.c");
        assert!(jwt_claims("not-a-jwt").is_none());
    }

    #[test]
    fn pages_escape_provider_text() {
        assert!(page("Sign-in failed", "<script>").contains("&lt;script&gt;"));
    }
}
