use llm::vendors::endpoints::Endpoints;
use llm::vendors::models::{list_chatgpt_models, list_models, ListedModel};
use llm::vendors::oauth::chatgpt::{self, ChatGptEndpoints, ChatGptTokens};
use llm::vendors::oauth::openrouter::{self, OpenRouterEndpoints};
use llm::vendors::oauth::CallbackServer;
use llm::vendors::quota::{balance, chatgpt_quota, BalanceSource};
use llm::{
    ApiProvider, AuthMode, Backend, ConnectionConfig, Credential, LlmCancellation, LlmErrorKind,
    LlmInputBlock, LlmMessage, LlmOutputBlock, LlmProtocol, LlmProvider, LlmRequest, LlmStopReason,
    LlmToolChoice, LlmToolDefinition,
};
use serde_json::{json, Value};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::Duration;
use support::{spawn_server, CapturedRequest, ResponseSpec};

mod support;

fn next(captured: &std::sync::mpsc::Receiver<CapturedRequest>) -> CapturedRequest {
    captured.recv_timeout(Duration::from_secs(2)).unwrap()
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> Option<&'a str> {
    request.headers.get(name).map(String::as_str)
}

fn jwt(claims: Value) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

fn id_token() -> String {
    jwt(json!({
        "email": "user@example.com",
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct-1", "chatgpt_plan_type": "pro"}
    }))
}

fn tokens() -> ChatGptTokens {
    ChatGptTokens {
        access_token: "access-1".into(),
        refresh_token: "refresh-1".into(),
        id_token: id_token(),
        account_id: "acct-1".into(),
        email: "user@example.com".into(),
        plan: Some("pro".into()),
        expires_at: 0,
        subscription_until: Some(1_900_000_000),
    }
}

fn chatgpt_endpoints(base: &str) -> ChatGptEndpoints {
    ChatGptEndpoints {
        auth: base.into(),
        backend: format!("{base}/backend-api"),
    }
}

fn assert_codex_client(request: &CapturedRequest) {
    assert_eq!(header(request, "originator"), Some("codex_cli_rs"));
    assert!(header(request, "user-agent")
        .is_some_and(|agent| agent.starts_with("codex_cli_rs/0.169.1 (")));
}

#[test]
fn bearer_model_list_keeps_vendor_order_and_strips_gemini_prefixes() {
    let (base, captured, server) = spawn_server(vec![ResponseSpec::json(json!({
        "object": "list",
        "data": [{"id": "models/gemini-3.5-flash"}, {"id": "gemini-3.1-pro"}, {"id": "gemini-3.5-flash"}]
    }))]);
    let ids = list_models(
        &Endpoints {
            chat: Some(format!("{base}/v1beta/openai")),
            ..Endpoints::default()
        },
        None,
        Some(&Credential::new("key-1")),
        &LlmCancellation::new(),
    )
    .unwrap()
    .into_iter()
    .map(|model| model.id)
    .collect::<Vec<_>>();
    assert_eq!(ids, ["gemini-3.5-flash", "gemini-3.1-pro"]);
    let request = next(&captured);
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/v1beta/openai/models");
    assert_eq!(header(&request, "authorization"), Some("Bearer key-1"));
    server.join().unwrap();
}

#[test]
fn anthropic_model_list_follows_pages_with_its_key_header() {
    let (base, captured, server) = spawn_server(vec![
        ResponseSpec::json(json!({
            "data": [{"id": "claude-opus-5-5"}], "has_more": true, "last_id": "claude-opus-5-5"
        })),
        ResponseSpec::json(json!({"data": [{"id": "claude-haiku-5-5"}], "has_more": false})),
    ]);
    let ids = list_models(
        &Endpoints {
            messages: Some(format!("{base}/v1")),
            ..Endpoints::default()
        },
        None,
        Some(&Credential::new("key-2")),
        &LlmCancellation::new(),
    )
    .unwrap()
    .into_iter()
    .map(|model| model.id)
    .collect::<Vec<_>>();
    assert_eq!(ids, ["claude-opus-5-5", "claude-haiku-5-5"]);
    let first = next(&captured);
    let second = next(&captured);
    assert_eq!(first.path, "/v1/models?limit=1000");
    assert_eq!(
        second.path,
        "/v1/models?after_id=claude-opus-5-5&limit=1000"
    );
    for request in [first, second] {
        assert_eq!(header(&request, "x-api-key"), Some("key-2"));
        assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));
        assert!(!request.headers.contains_key("authorization"));
    }
    server.join().unwrap();
}

fn listed(id: &str, protocols: &[LlmProtocol]) -> ListedModel {
    ListedModel {
        id: id.into(),
        protocols: protocols.to_vec(),
    }
}

#[test]
fn a_vendors_lists_on_each_api_merge_into_one() {
    let (base, captured, server) = spawn_server(vec![
        ResponseSpec::json(json!({"data": [{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}]})),
        ResponseSpec::json(json!({"data": [{"id": "deepseek-reasoner"}, {"id": "deepseek-v4"}]})),
    ]);
    let models = list_models(
        &Endpoints {
            chat: Some(format!("{base}/v1")),
            responses: Some(format!("{base}/v1")),
            messages: Some(format!("{base}/anthropic/v1")),
        },
        None,
        Some(&Credential::new("key-3")),
        &LlmCancellation::new(),
    )
    .unwrap();
    use LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };
    assert_eq!(
        models,
        [
            listed("deepseek-chat", &[Chat, Responses]),
            listed("deepseek-reasoner", &[Chat, Responses, Messages]),
            listed("deepseek-v4", &[Messages]),
        ]
    );
    let openai = next(&captured);
    assert_eq!(openai.path, "/v1/models");
    assert_eq!(header(&openai, "authorization"), Some("Bearer key-3"));
    let anthropic = next(&captured);
    assert_eq!(anthropic.path, "/anthropic/v1/models?limit=1000");
    assert_eq!(header(&anthropic, "x-api-key"), Some("key-3"));
    assert_eq!(header(&anthropic, "anthropic-version"), Some("2023-06-01"));
    server.join().unwrap();
}

#[test]
fn a_list_missing_on_one_api_is_skipped_and_the_gateways_word_wins() {
    let (base, captured, server) = spawn_server(vec![
        ResponseSpec::json(json!({"data": [
            {"id": "claude-x", "native_endpoints": ["/v1/messages"]},
            {"id": "kimi-k2"}
        ]})),
        ResponseSpec::status(404, json!({"error": {"message": "not found"}})),
    ]);
    let models = list_models(
        &Endpoints {
            chat: Some(format!("{base}/v1")),
            responses: None,
            messages: Some(format!("{base}/anthropic/v1")),
        },
        Some(AuthMode::Bearer),
        Some(&Credential::new("key-4")),
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(
        models,
        [
            listed("claude-x", &[LlmProtocol::AnthropicMessages]),
            listed(
                "kimi-k2",
                &[
                    LlmProtocol::OpenAiChatCompletions,
                    LlmProtocol::AnthropicMessages
                ]
            ),
        ]
    );
    next(&captured);
    let anthropic = next(&captured);
    assert_eq!(header(&anthropic, "authorization"), Some("Bearer key-4"));
    assert!(!anthropic.headers.contains_key("x-api-key"));
    server.join().unwrap();
}

#[test]
fn chatgpt_code_exchange_is_form_encoded_like_codex() {
    let access = jwt(json!({"exp": 1_800_000_000}));
    let (base, captured, server) = spawn_server(vec![ResponseSpec::json(json!({
        "id_token": id_token(), "access_token": access, "refresh_token": "refresh-1"
    }))]);
    let tokens = chatgpt::exchange_code(
        &chatgpt_endpoints(&base),
        "code-1",
        "verifier-1",
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(tokens.account_id, "acct-1");
    assert_eq!(tokens.plan.as_deref(), Some("pro"));
    assert_eq!(tokens.expires_at, 1_800_000_000);

    let request = next(&captured);
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/oauth/token")
    );
    assert_eq!(
        header(&request, "content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(header(&request, "accept"), Some("application/json"));
    assert_codex_client(&request);
    let form = url::form_urlencoded::parse(request.raw_body.as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    assert_eq!(
        form,
        [
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), "code-1".into()),
            (
                "redirect_uri".into(),
                "http://localhost:1455/auth/callback".into()
            ),
            ("client_id".into(), chatgpt::CLIENT_ID.into()),
            ("code_verifier".into(), "verifier-1".into()),
        ]
    );
    server.join().unwrap();
}

#[test]
fn chatgpt_refresh_sends_json_and_keeps_unrotated_tokens() {
    let access = jwt(json!({"exp": 1_800_003_600}));
    let (base, captured, server) =
        spawn_server(vec![ResponseSpec::json(json!({"access_token": access}))]);
    let fresh = chatgpt::refresh(
        &chatgpt_endpoints(&base),
        &tokens(),
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(fresh.access_token, access);
    assert_eq!(fresh.refresh_token, "refresh-1");
    assert_eq!(fresh.id_token, tokens().id_token);
    assert_eq!(fresh.expires_at, 1_800_003_600);

    let request = next(&captured);
    assert_eq!(request.path, "/oauth/token");
    assert_eq!(header(&request, "content-type"), Some("application/json"));
    assert_codex_client(&request);
    assert_eq!(
        request.body,
        json!({"client_id": chatgpt::CLIENT_ID, "grant_type": "refresh_token", "refresh_token": "refresh-1"})
    );
    server.join().unwrap();
}

#[test]
fn chatgpt_refresh_tells_a_dead_sign_in_from_a_hiccup() {
    for (status, body, kind) in [
        (
            400,
            json!({"error": "invalid_grant"}),
            LlmErrorKind::Authentication,
        ),
        (
            401,
            json!({"error": {"code": "refresh_token_reused", "message": "reused"}}),
            LlmErrorKind::Authentication,
        ),
        (
            401,
            json!({"message": "proxy says no"}),
            LlmErrorKind::Transport,
        ),
        (503, json!({}), LlmErrorKind::Transport),
    ] {
        let (base, captured, server) = spawn_server(vec![ResponseSpec::status(status, body)]);
        let error = chatgpt::refresh(
            &chatgpt_endpoints(&base),
            &tokens(),
            &LlmCancellation::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind, "HTTP {status}");
        assert_eq!(error.status, Some(status));
        if kind == LlmErrorKind::Authentication {
            assert!(error.message.contains("sign in again"));
        } else {
            assert!(error.retryable);
        }
        next(&captured);
        server.join().unwrap();
    }
}

#[test]
fn openrouter_code_exchange_returns_the_minted_key() {
    let (base, captured, server) =
        spawn_server(vec![ResponseSpec::json(json!({"key": "sk-or-v1-minted"}))]);
    let key = openrouter::exchange_code(
        &OpenRouterEndpoints { site: base },
        "code-2",
        "verifier-2",
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(key, "sk-or-v1-minted");
    let request = next(&captured);
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/api/v1/auth/keys")
    );
    assert_eq!(
        request.body,
        json!({"code": "code-2", "code_verifier": "verifier-2", "code_challenge_method": "S256"})
    );
    server.join().unwrap();
}

fn get(url: &str) -> u16 {
    match ureq::get(url).call() {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn callback_server_ignores_wrong_state_and_returns_the_code() {
    let server = CallbackServer::bind("127.0.0.1:0").unwrap();
    let port = server.port();
    let waiter = thread::spawn(move || {
        server.wait("state-1", &LlmCancellation::new(), Duration::from_secs(5))
    });
    let base = format!("http://127.0.0.1:{port}");
    assert_eq!(
        get(&format!("{base}/auth/callback?code=evil&state=wrong")),
        400
    );
    assert_eq!(get(&format!("{base}/auth/callback?state=state-1")), 400);
    assert_eq!(get(&format!("{base}/favicon.ico")), 404);
    assert_eq!(
        get(&format!("{base}/auth/callback?code=code-3&state=state-1")),
        200
    );
    assert_eq!(waiter.join().unwrap().unwrap(), "code-3");
}

#[test]
fn callback_server_reports_provider_errors_cancellation_and_timeouts() {
    let server = CallbackServer::bind("127.0.0.1:0").unwrap();
    let port = server.port();
    let waiter = thread::spawn(move || {
        server.wait("state-1", &LlmCancellation::new(), Duration::from_secs(5))
    });
    assert_eq!(
        get(&format!(
            "http://127.0.0.1:{port}/callback?error=access_denied&error_description=User+said+no"
        )),
        400
    );
    let error = waiter.join().unwrap().unwrap_err();
    assert_eq!(error.kind, LlmErrorKind::Authentication);
    assert_eq!(error.message, "User said no");

    let server = CallbackServer::bind("127.0.0.1:0").unwrap();
    let cancellation = LlmCancellation::new();
    cancellation.cancel();
    assert_eq!(
        server
            .wait("state-1", &cancellation, Duration::from_secs(5))
            .unwrap_err()
            .kind,
        LlmErrorKind::Cancelled
    );
    assert_eq!(
        server
            .wait(
                "state-1",
                &LlmCancellation::new(),
                Duration::from_millis(150)
            )
            .unwrap_err()
            .kind,
        LlmErrorKind::Timeout
    );
}

#[test]
fn reclaiming_a_busy_port_cancels_the_waiting_sign_in() {
    let address: SocketAddr = {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap()
    };
    let holder = CallbackServer::bind(address).unwrap();
    let waiter = thread::spawn(move || {
        let outcome = holder.wait("state-1", &LlmCancellation::new(), Duration::from_secs(5));
        drop(holder);
        outcome
    });
    let reclaimed = CallbackServer::bind_reclaiming(address).unwrap();
    assert_eq!(reclaimed.port(), address.port());
    assert_eq!(
        waiter.join().unwrap().unwrap_err().kind,
        LlmErrorKind::Cancelled
    );
}

fn codex_models_response() -> ResponseSpec {
    ResponseSpec::json(json!({"models": [
        {"slug": "gpt-6-luna", "priority": 2, "base_instructions": "You are Codex (luna)."},
        {"slug": "gpt-6-sol", "priority": 1, "base_instructions": "You are Codex.",
         "model_messages": {"instructions_template": "You are Codex, sol."}},
        {"slug": "codex-internal", "priority": 0, "visibility": "hide"}
    ]}))
}

fn chatgpt_provider(base: &str) -> ApiProvider {
    ApiProvider::with_backend(
        ConnectionConfig {
            id: "chatgpt".into(),
            protocol: LlmProtocol::OpenAiResponses,
            base_url: format!("{base}/backend-api/codex"),
            model: "gpt-6-sol".into(),
            auth_mode: AuthMode::Bearer,
            timeout_ms: 2_000,
            context_window_tokens: Some(200_000),
            max_output_tokens: 4_096,
        },
        Some(Credential::new("access-1")),
        Backend::ChatGpt {
            account_id: "acct-1".into(),
        },
    )
    .unwrap()
}

fn tool_request() -> LlmRequest {
    LlmRequest {
        instructions: Some("Use only granted documents.".into()),
        thinking: None,
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "Read doc-1".into(),
        }])],
        tools: vec![LlmToolDefinition {
            name: "read_document".into(),
            description: "Read one document".into(),
            input_schema: json!({"type": "object", "properties": {"id": {"type": "string"}}}),
        }],
        tool_choice: LlmToolChoice::Auto,
        max_output_tokens: Some(512),
    }
}

#[test]
fn chatgpt_backend_streams_a_codex_request_round_trip() {
    let (base, captured, server) = spawn_server(vec![
        codex_models_response(),
        ResponseSpec::event_stream(&[
            json!({"type": "response.created", "response": {"status": "in_progress", "output": []}}),
            json!({"type": "response.output_item.done", "item": {
                "type": "function_call", "call_id": "call-1", "name": "read_document",
                "arguments": "{\"id\":\"doc-1\"}", "status": "completed"}}),
            json!({"type": "response.completed", "response": {
                "status": "completed", "model": "gpt-6-sol", "output": [],
                "usage": {"input_tokens": 10, "output_tokens": 4, "total_tokens": 14}}}),
        ]),
        ResponseSpec::event_stream(&[json!({"type": "response.completed", "response": {
            "status": "completed", "model": "gpt-6-sol",
            "output": [{"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": "Done"}]}]}})]),
    ]);
    let provider = chatgpt_provider(&base);
    let first = provider
        .complete(&tool_request(), &LlmCancellation::new())
        .unwrap();
    assert_eq!(first.stop_reason, LlmStopReason::ToolUse);
    assert!(matches!(
        first.blocks.as_slice(),
        [LlmOutputBlock::ToolCall { id, name, .. }] if id == "call-1" && name == "read_document"
    ));
    assert_eq!(first.usage.unwrap().total_tokens, 14);
    let second = provider
        .complete(&tool_request(), &LlmCancellation::new())
        .unwrap();
    assert_eq!(
        second.blocks,
        [LlmOutputBlock::Text {
            text: "Done".into()
        }]
    );

    let models = next(&captured);
    assert_eq!(models.method, "GET");
    assert_eq!(
        models.path,
        "/backend-api/codex/models?client_version=0.169.1"
    );
    assert_eq!(header(&models, "authorization"), Some("Bearer access-1"));
    assert_eq!(header(&models, "chatgpt-account-id"), Some("acct-1"));

    let request = next(&captured);
    assert_eq!(request.path, "/backend-api/codex/responses");
    assert_eq!(header(&request, "authorization"), Some("Bearer access-1"));
    assert_eq!(header(&request, "chatgpt-account-id"), Some("acct-1"));
    assert_eq!(
        header(&request, "openai-beta"),
        Some("responses=experimental")
    );
    assert_eq!(header(&request, "version"), Some("0.169.1"));
    assert_eq!(header(&request, "accept"), Some("text/event-stream"));
    assert_codex_client(&request);
    let body = &request.body;
    assert_eq!(
        header(&request, "session-id"),
        body["prompt_cache_key"].as_str()
    );
    assert_eq!(body["instructions"], "You are Codex, sol.");
    assert_eq!(body["input"][0]["role"], "developer");
    assert_eq!(
        body["input"][0]["content"][0]["text"],
        "Use only granted documents."
    );
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["tools"][0]["strict"], false);
    assert!(body.get("max_output_tokens").is_none());
    // the instructions are read once per provider
    assert_eq!(next(&captured).path, "/backend-api/codex/responses");
    server.join().unwrap();
}

#[test]
fn chatgpt_backend_maps_auth_and_rate_limit_failures() {
    for (status, kind) in [
        (401, LlmErrorKind::Authentication),
        (429, LlmErrorKind::RateLimited),
    ] {
        let (base, captured, server) = spawn_server(vec![
            codex_models_response(),
            ResponseSpec::status(
                status,
                json!({"error": {"type": "usage_limit_reached", "message": "The usage limit has been reached"}}),
            ),
        ]);
        let error = chatgpt_provider(&base)
            .complete(&tool_request(), &LlmCancellation::new())
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.message, "The usage limit has been reached");
        next(&captured);
        next(&captured);
        server.join().unwrap();
    }
}

#[test]
fn chatgpt_models_are_listed_from_the_codex_backend() {
    let (base, captured, server) = spawn_server(vec![codex_models_response()]);
    let ids = list_chatgpt_models(
        &chatgpt_endpoints(&base),
        &tokens(),
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(ids, ["gpt-6-sol", "gpt-6-luna"]);
    assert_eq!(
        next(&captured).path,
        "/backend-api/codex/models?client_version=0.169.1"
    );
    server.join().unwrap();
}

#[test]
fn chatgpt_quota_reads_usage_then_counts_available_resets() {
    let (base, captured, server) = spawn_server(vec![
        ResponseSpec::json(json!({
            "plan_type": "pro",
            "rate_limit": {"primary_window": {
                "used_percent": 12.0, "limit_window_seconds": 18000, "reset_at": 1_800_000_000}},
            "rate_limit_reset_credits": {"available_count": 2}
        })),
        ResponseSpec::json(json!({"available_count": 2, "credits": [
            {"id": "a", "status": "available"}, {"id": "b", "status": "consumed"}
        ]})),
    ]);
    let quota = chatgpt_quota(
        &chatgpt_endpoints(&base),
        &tokens(),
        &LlmCancellation::new(),
    )
    .unwrap();
    assert_eq!(quota.plan.as_deref(), Some("pro"));
    assert_eq!(quota.windows.len(), 1);
    assert_eq!(quota.windows[0].resets_at, Some(1_800_000_000));
    assert_eq!(quota.reset_cards, Some(1));
    assert_eq!(quota.subscription_until, Some(1_900_000_000));

    let usage = next(&captured);
    assert_eq!(usage.path, "/backend-api/wham/usage");
    assert_eq!(header(&usage, "authorization"), Some("Bearer access-1"));
    assert_eq!(header(&usage, "chatgpt-account-id"), Some("acct-1"));
    assert_eq!(header(&usage, "accept"), Some("application/json"));
    assert_codex_client(&usage);
    assert_eq!(
        next(&captured).path,
        "/backend-api/wham/rate-limit-reset-credits"
    );
    server.join().unwrap();
}

#[test]
fn balances_are_read_where_each_vendor_keeps_them() {
    let (base, captured, server) = spawn_server(vec![
        ResponseSpec::json(
            json!({"balance_infos": [{"currency": "CNY", "total_balance": "110.00"}]}),
        ),
        ResponseSpec::json(json!({"code": 0, "data": {"available_balance": 49.58}})),
        ResponseSpec::json(json!({"data": {"total_credits": 20, "total_usage": 3.5}})),
    ]);
    let key = Credential::new("key-3");
    let cancellation = LlmCancellation::new();
    let base_v1 = format!("{base}/v1");
    assert_eq!(
        balance(BalanceSource::DeepSeek, &base_v1, &key, &cancellation).unwrap(),
        "¥110.00"
    );
    assert_eq!(
        balance(BalanceSource::Moonshot, &base_v1, &key, &cancellation).unwrap(),
        "$49.58"
    );
    assert_eq!(
        balance(
            BalanceSource::OpenRouter,
            &format!("{base}/api/v1"),
            &key,
            &cancellation
        )
        .unwrap(),
        "$16.50"
    );
    let paths = (0..3).map(|_| next(&captured).path).collect::<Vec<_>>();
    assert_eq!(
        paths,
        ["/user/balance", "/v1/users/me/balance", "/api/v1/credits"]
    );
    server.join().unwrap();
}
