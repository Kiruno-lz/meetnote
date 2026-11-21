//! ProviderService against loopback stubs: token refresh and rotation,
//! model lists, balances, revision conflicts and a browser sign-in.

use application::{
    model_choice, model_ids, CredentialChange, CredentialSummary, ProviderEndpoints,
    ProviderService,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use llm::vendors::oauth::chatgpt::ChatGptEndpoints;
use llm::vendors::oauth::openrouter::OpenRouterEndpoints;
use llm::{
    Backend, LlmCancellation, LlmInputBlock, LlmMessage, LlmProvider, LlmRequest, LlmToolChoice,
};
use serde_json::{json, Value};
use settings::{
    AppSettings, AssistantAuthMode, ChatGptAccount, CredentialStore, CustomEndpoint, EndpointUrls,
    JsonSettingsRepository, LlmProtocol, ModelChoice, ProviderCredential, ProviderSettings,
    SettingsRepository, SettingsService,
};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct Captured {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

/// Answers each connection with the next scripted `(status, body)`.
fn stub(responses: Vec<(u16, Value)>) -> (String, Receiver<Captured>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            sender.send(read_request(&mut stream)).unwrap();
            let body = body.to_string();
            write!(
                stream,
                "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (base, receiver, handle)
}

fn read_request(stream: &mut TcpStream) -> Captured {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap().split_whitespace();
    let (method, path) = (request.next().unwrap(), request.next().unwrap());
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    while bytes.len() - end < length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
    }
    Captured {
        method: method.into(),
        path: path.into(),
        headers,
        body: String::from_utf8(bytes[end..end + length].to_vec()).unwrap(),
    }
}

/// A settings directory removed when dropped.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "meetnote-providers-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn settings_path(&self) -> PathBuf {
        self.0.join("settings.json")
    }

    fn credentials(&self) -> CredentialStore {
        CredentialStore::beside(self.settings_path())
    }

    fn save(&self, providers: Vec<ProviderSettings>) {
        let mut settings = AppSettings::default();
        settings.assistant.providers = providers;
        JsonSettingsRepository::open(self.settings_path())
            .save(&settings)
            .unwrap();
    }

    fn service(&self, endpoints: ProviderEndpoints) -> ProviderService {
        let settings =
            SettingsService::open(Arc::new(JsonSettingsRepository::open(self.settings_path())))
                .unwrap();
        ProviderService::new(settings, self.credentials()).with_endpoints(endpoints)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn jwt(claims: Value) -> String {
    format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

fn id_token() -> String {
    jwt(json!({
        "email": "user@example.com",
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct-1", "chatgpt_plan_type": "plus"}
    }))
}

fn chatgpt_endpoints(base: &str) -> ChatGptEndpoints {
    ChatGptEndpoints {
        auth: base.into(),
        backend: format!("{base}/backend-api"),
    }
}

fn custom(id: &str, chat: String) -> ProviderSettings {
    ProviderSettings {
        endpoints: EndpointUrls {
            chat: Some(chat),
            ..EndpointUrls::default()
        },
        custom: Some(CustomEndpoint {
            name: "Gateway".into(),
            auth_mode: AssistantAuthMode::Bearer,
        }),
        ..ProviderSettings::new(id)
    }
}

fn choice(provider: &str, model: &str, protocol: Option<LlmProtocol>) -> ModelChoice {
    ModelChoice {
        provider: provider.into(),
        model: model.into(),
        protocol,
    }
}

#[test]
fn an_expiring_chatgpt_sign_in_is_refreshed_and_stored_before_use() {
    let fresh_access = jwt(json!({"exp": 4_000_000_000_u64}));
    let (base, captured, server) = stub(vec![(
        200,
        json!({"access_token": fresh_access, "refresh_token": "refresh-2", "id_token": id_token()}),
    )]);
    let fixture = Fixture::new();
    fixture.save(vec![ProviderSettings::new("openai")]);
    fixture
        .credentials()
        .set(
            "openai",
            Some(ProviderCredential::ChatGpt(ChatGptAccount {
                access_token: "access-old".into(),
                refresh_token: "refresh-old".into(),
                id_token: id_token(),
                account_id: "acct-1".into(),
                email: "user@example.com".into(),
                plan: Some("plus".into()),
                expires_at: 1,
                subscription_until: None,
            })),
        )
        .unwrap();
    let service = fixture.service(ProviderEndpoints {
        chatgpt: chatgpt_endpoints(&base),
        ..ProviderEndpoints::default()
    });

    let client = service
        .client(&choice(
            "openai",
            "gpt-5",
            Some(LlmProtocol::OpenAiChatCompletions),
        ))
        .unwrap();

    let refresh = captured.recv().unwrap();
    assert_eq!(
        (refresh.method.as_str(), refresh.path.as_str()),
        ("POST", "/oauth/token")
    );
    let body: Value = serde_json::from_str(&refresh.body).unwrap();
    assert_eq!(body["grant_type"], "refresh_token");
    assert_eq!(body["refresh_token"], "refresh-old");
    assert_eq!(
        client.backend(),
        &Backend::ChatGpt {
            account_id: "acct-1".into()
        }
    );
    assert_eq!(
        client.connection().base_url,
        format!("{base}/backend-api/codex")
    );
    assert_eq!(client.connection().model, "gpt-5");
    assert_eq!(
        client.connection().protocol,
        llm::LlmProtocol::OpenAiResponses
    );
    match fixture.credentials().get("openai").unwrap() {
        Some(ProviderCredential::ChatGpt(account)) => {
            assert_eq!(account.access_token, fresh_access);
            assert_eq!(account.refresh_token, "refresh-2");
            assert_eq!(account.expires_at, 4_000_000_000);
        }
        other => panic!("unexpected credential {other:?}"),
    }
    server.join().unwrap();

    // The stored tokens are now fresh: no second refresh is attempted.
    service.client(&choice("openai", "gpt-5", None)).unwrap();
}

#[test]
fn a_refused_refresh_keeps_the_old_tokens_and_never_quotes_them() {
    let (base, _captured, server) = stub(vec![(400, json!({"error": "invalid_grant"}))]);
    let fixture = Fixture::new();
    fixture.save(vec![ProviderSettings::new("openai")]);
    let stored = ProviderCredential::ChatGpt(ChatGptAccount {
        access_token: "access-old".into(),
        refresh_token: "refresh-old".into(),
        id_token: id_token(),
        account_id: "acct-1".into(),
        email: "user@example.com".into(),
        plan: None,
        expires_at: 1,
        subscription_until: None,
    });
    fixture
        .credentials()
        .set("openai", Some(stored.clone()))
        .unwrap();
    let service = fixture.service(ProviderEndpoints {
        chatgpt: chatgpt_endpoints(&base),
        ..ProviderEndpoints::default()
    });

    let Err(error) = service.client(&choice("openai", "gpt-5", None)) else {
        panic!("a refused refresh must fail");
    };
    assert_eq!(error.code, "PROVIDER_AUTHENTICATION_FAILED");
    assert!(!error.message.contains("refresh-old") && !error.message.contains("access-old"));
    assert_eq!(fixture.credentials().get("openai").unwrap(), Some(stored));
    server.join().unwrap();
}

#[test]
fn models_and_balances_are_read_with_the_stored_key() {
    let (base, captured, server) = stub(vec![
        (200, json!({"data": [{"id": "model-a"}, {"id": "model-b"}]})),
        (
            200,
            json!({"balance_infos": [{"currency": "USD", "total_balance": "12.50"}]}),
        ),
    ]);
    let fixture = Fixture::new();
    fixture.save(vec![
        custom("custom-gw", format!("{base}/v1")),
        ProviderSettings {
            endpoints: EndpointUrls {
                chat: Some(format!("{base}/v1")),
                ..EndpointUrls::default()
            },
            ..ProviderSettings::new("deepseek")
        },
    ]);
    let service = fixture.service(ProviderEndpoints::default());
    let (revision, _) = service.views().unwrap();
    service
        .save(
            &revision,
            custom("custom-gw", format!("{base}/v1")),
            CredentialChange::ApiKey("sk-gateway-key".into()),
        )
        .unwrap();
    fixture
        .credentials()
        .set(
            "deepseek",
            Some(ProviderCredential::ApiKey {
                key: "sk-deepseek-key".into(),
            }),
        )
        .unwrap();
    let cancellation = LlmCancellation::new();

    assert_eq!(
        model_ids(&service.models("custom-gw", &cancellation).unwrap()),
        ["model-a", "model-b"]
    );
    let listed = captured.recv().unwrap();
    assert_eq!(listed.path, "/v1/models");
    assert_eq!(listed.headers["authorization"], "Bearer sk-gateway-key");

    let quota = service.quota("deepseek", &cancellation).unwrap().unwrap();
    assert_eq!(quota.balance.as_deref(), Some("$12.50"));
    let balance = captured.recv().unwrap();
    assert_eq!(balance.path, "/user/balance");
    assert_eq!(balance.headers["authorization"], "Bearer sk-deepseek-key");
    assert_eq!(service.quota("custom-gw", &cancellation).unwrap(), None);
    server.join().unwrap();
}

#[test]
fn views_list_every_vendor_then_custom_providers_without_secrets() {
    let fixture = Fixture::new();
    fixture.save(vec![
        ProviderSettings {
            enabled: false,
            ..ProviderSettings::new("deepseek")
        },
        custom("custom-gw", "http://127.0.0.1:9/v1".into()),
    ]);
    fixture
        .credentials()
        .set(
            "deepseek",
            Some(ProviderCredential::ApiKey {
                key: "sk-deepseek-1234".into(),
            }),
        )
        .unwrap();
    let service = fixture.service(ProviderEndpoints::default());

    let (_, views) = service.views().unwrap();

    let catalog = llm::vendors::catalog::catalog();
    assert_eq!(views.len(), catalog.len() + 1);
    let deepseek = views.iter().find(|view| view.id == "deepseek").unwrap();
    assert!(deepseek.configured && !deepseek.enabled);
    assert_eq!(deepseek.region, Some("Global"));
    assert_eq!(
        deepseek.endpoints.chat.as_deref(),
        Some("https://api.deepseek.com/v1")
    );
    assert_eq!(
        deepseek.endpoints.messages.as_deref(),
        Some("https://api.deepseek.com/anthropic/v1")
    );
    assert_eq!(
        deepseek.credential,
        CredentialSummary::ApiKey {
            last4: "1234".into()
        }
    );
    let openai = views.iter().find(|view| view.id == "openai").unwrap();
    assert!(!openai.configured && !openai.enabled);
    assert_eq!(
        openai.endpoints.protocols(),
        [
            llm::LlmProtocol::OpenAiChatCompletions,
            llm::LlmProtocol::OpenAiResponses
        ]
    );
    let gateway = views.last().unwrap();
    assert_eq!(
        (gateway.id.as_str(), gateway.name.as_str()),
        ("custom-gw", "Gateway")
    );
    assert!(gateway.vendor.is_none() && gateway.configured && gateway.region.is_none());
    assert_eq!(
        gateway.endpoints.chat.as_deref(),
        Some("http://127.0.0.1:9/v1")
    );
    assert!(!format!("{views:?}").contains("sk-deepseek-1234"));
}

#[test]
fn a_stale_revision_writes_neither_settings_nor_credentials() {
    let fixture = Fixture::new();
    fixture.save(vec![ProviderSettings::new("deepseek")]);
    let service = fixture.service(ProviderEndpoints::default());
    let (revision, _) = service.views().unwrap();
    let other = fixture.service(ProviderEndpoints::default());
    other
        .set_model(&revision, Some(choice("deepseek", "deepseek-chat", None)))
        .unwrap();

    let error = service
        .save(
            &revision,
            ProviderSettings::new("deepseek"),
            CredentialChange::ApiKey("sk-new".into()),
        )
        .unwrap_err();

    assert!(error.is_settings_conflict() && error.retryable);
    assert_eq!(fixture.credentials().get("deepseek").unwrap(), None);
    assert!(service
        .save(
            &revision,
            ProviderSettings::new("nonsense"),
            CredentialChange::Keep
        )
        .is_err());
}

#[test]
fn removing_a_provider_drops_its_credential_and_the_model_choice() {
    let fixture = Fixture::new();
    fixture.save(vec![
        ProviderSettings::new("deepseek"),
        ProviderSettings::new("xai"),
    ]);
    let service = fixture.service(ProviderEndpoints::default());
    let (revision, _) = service.views().unwrap();
    let saved = service
        .save(
            &revision,
            ProviderSettings::new("deepseek"),
            CredentialChange::ApiKey("sk-deepseek".into()),
        )
        .unwrap();
    let saved = service
        .set_model(
            &SettingsService::revision_of(&saved),
            Some(choice(
                "deepseek",
                "deepseek-chat",
                Some(LlmProtocol::AnthropicMessages),
            )),
        )
        .unwrap();

    let saved = service
        .remove(&SettingsService::revision_of(&saved), "deepseek")
        .unwrap();

    assert_eq!(saved.assistant.model, None);
    assert_eq!(
        saved
            .assistant
            .providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        ["xai"]
    );
    assert_eq!(fixture.credentials().get("deepseek").unwrap(), None);
    let Err(error) = service.client(&choice("deepseek", "deepseek-chat", None)) else {
        panic!("a removed provider has no client");
    };
    assert_eq!(error.code, "CONNECTION_NOT_CONFIGURED");
}

/// The callback the browser would make after signing in, read from the
/// authorize URL's `callback_url` and `state`.
fn complete_in_browser(url: &str, code: &str) -> JoinHandle<()> {
    let url = url::Url::parse(url).unwrap();
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap()
    };
    let callback = url::Url::parse(&query("callback_url")).unwrap();
    let target = format!("{}?code={code}&state={}", callback.path(), query("state"));
    let port = callback.port().unwrap();
    thread::spawn(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    })
}

#[test]
fn an_openrouter_sign_in_stores_the_minted_key_and_enables_the_provider() {
    let (base, captured, server) = stub(vec![(200, json!({"key": "sk-or-minted-key"}))]);
    let fixture = Fixture::new();
    fixture.save(Vec::new());
    let service = fixture.service(ProviderEndpoints {
        openrouter: OpenRouterEndpoints { site: base.clone() },
        ..ProviderEndpoints::default()
    });

    let pending = service.begin_sign_in("openrouter").unwrap();
    assert!(pending.url.starts_with(&format!("{base}/auth?")));
    let browser = complete_in_browser(&pending.url, "code-1");
    let summary = pending.finish(&LlmCancellation::new()).unwrap();
    browser.join().unwrap();

    assert_eq!(
        summary,
        CredentialSummary::ApiKey {
            last4: "-key".into()
        }
    );
    let exchange = captured.recv().unwrap();
    assert_eq!(exchange.path, "/api/v1/auth/keys");
    assert_eq!(
        serde_json::from_str::<Value>(&exchange.body).unwrap()["code"],
        "code-1"
    );
    assert_eq!(
        fixture.credentials().get("openrouter").unwrap(),
        Some(ProviderCredential::ApiKey {
            key: "sk-or-minted-key".into()
        })
    );
    let saved = JsonSettingsRepository::open(fixture.settings_path())
        .load()
        .unwrap();
    assert!(saved.assistant.provider("openrouter").unwrap().enabled);
    server.join().unwrap();
}

#[test]
fn a_cancelled_sign_in_is_told_apart_from_a_failure() {
    let fixture = Fixture::new();
    fixture.save(Vec::new());
    let service = fixture.service(ProviderEndpoints {
        openrouter: OpenRouterEndpoints {
            site: "http://127.0.0.1:9".into(),
        },
        ..ProviderEndpoints::default()
    });
    let pending = service.begin_sign_in("openrouter").unwrap();
    let cancellation = LlmCancellation::new();
    cancellation.cancel();

    let error = pending.finish(&cancellation).unwrap_err();

    assert!(error.is_cancelled(), "{error}");
    assert_eq!(fixture.credentials().get("openrouter").unwrap(), None);
    assert!(service.begin_sign_in("deepseek").is_err());
}

#[test]
fn a_codex_login_is_imported_without_touching_it() {
    let fixture = Fixture::new();
    fixture.save(Vec::new());
    let codex = fixture.0.join("codex");
    std::fs::create_dir_all(&codex).unwrap();
    let auth = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": id_token(),
            "access_token": jwt(json!({"exp": 4_000_000_000_u64})),
            "refresh_token": "refresh-codex",
            "account_id": "acct-1"
        }
    })
    .to_string();
    std::fs::write(codex.join("auth.json"), &auth).unwrap();
    let service = fixture.service(ProviderEndpoints {
        codex_home: Some(codex.clone()),
        ..ProviderEndpoints::default()
    });

    let summary = service.import_codex_login().unwrap();

    assert_eq!(
        summary,
        CredentialSummary::Account {
            email: "user@example.com".into(),
            plan: Some("plus".into()),
            subscription_until: None,
        }
    );
    assert_eq!(
        std::fs::read_to_string(codex.join("auth.json")).unwrap(),
        auth
    );
    assert!(matches!(
        fixture.credentials().get("openai").unwrap(),
        Some(ProviderCredential::ChatGpt(account)) if account.refresh_token == "refresh-codex"
    ));
    let (_, views) = service.views().unwrap();
    let openai = views.iter().find(|view| view.id == "openai").unwrap();
    assert!(openai.configured && openai.enabled);
    assert_eq!(
        openai.endpoints.responses.as_deref(),
        Some("https://chatgpt.com/backend-api/codex")
    );
    assert_eq!(openai.endpoints.protocols().len(), 1);
}

fn hello() -> LlmRequest {
    LlmRequest {
        instructions: None,
        thinking: None,
        messages: vec![LlmMessage::User(vec![LlmInputBlock::Text {
            text: "Hello".into(),
        }])],
        tools: Vec::new(),
        tool_choice: LlmToolChoice::None,
        max_output_tokens: Some(16),
    }
}

/// A reply of `protocol` saying "Done".
fn done(protocol: llm::LlmProtocol) -> Value {
    match protocol {
        llm::LlmProtocol::OpenAiChatCompletions => json!({
            "model": "m", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "Done"}}]
        }),
        llm::LlmProtocol::OpenAiResponses => json!({
            "model": "m", "status": "completed",
            "output": [{"type": "message", "role": "assistant", "status": "completed",
                        "content": [{"type": "output_text", "text": "Done", "annotations": []}]}]
        }),
        llm::LlmProtocol::AnthropicMessages => json!({
            "model": "m", "stop_reason": "end_turn", "content": [{"type": "text", "text": "Done"}]
        }),
    }
}

#[test]
fn each_model_of_magpies_gateway_is_asked_on_its_native_api() {
    use llm::LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };
    let (base, captured, server) = stub(vec![
        (
            200,
            json!({"data": [
                {"id": "claude-opus-5-5", "native_endpoints": ["/v1/chat/completions", "/v1/messages"]},
                {"id": "gpt-5.1", "native_endpoints": ["/v1/chat/completions", "/v1/responses"]},
                {"id": "kimi-k2", "native_endpoints": ["/v1/chat/completions"]}
            ]}),
        ),
        (200, done(Messages)),
        (200, done(Responses)),
        (200, done(Chat)),
    ]);
    let gateway = format!("{base}/v1");
    let fixture = Fixture::new();
    fixture.save(vec![ProviderSettings {
        endpoints: EndpointUrls {
            chat: Some(gateway.clone()),
            responses: Some(gateway.clone()),
            messages: Some(gateway),
        },
        ..ProviderSettings::new("magpie")
    }]);
    let service = fixture.service(ProviderEndpoints::default());
    let cancellation = LlmCancellation::new();

    let live = service.models("magpie", &cancellation).unwrap();
    assert_eq!(captured.recv().unwrap().path, "/v1/models");
    assert_eq!(live[0].protocols, [Chat, Messages]);
    let (_, views) = service.views().unwrap();
    let view = views.iter().find(|view| view.id == "magpie").unwrap();

    for (model, protocol, path) in [
        (
            "claude-opus-5-5",
            LlmProtocol::AnthropicMessages,
            "/v1/messages",
        ),
        ("gpt-5.1", LlmProtocol::OpenAiResponses, "/v1/responses"),
        (
            "kimi-k2",
            LlmProtocol::OpenAiChatCompletions,
            "/v1/chat/completions",
        ),
    ] {
        let picked = model_choice(view, &live, model);
        assert_eq!(picked, choice("magpie", model, Some(protocol)));
        let output = service
            .client(&picked)
            .unwrap()
            .complete(&hello(), &cancellation)
            .unwrap();
        assert!(!output.blocks.is_empty(), "{model}");
        let request = captured.recv().unwrap();
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("POST", path)
        );
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap()["model"],
            model
        );
    }
    server.join().unwrap();
}

#[test]
fn a_stored_api_no_longer_served_falls_back_to_the_native_rule() {
    let fixture = Fixture::new();
    fixture.save(vec![ProviderSettings {
        region: Some("China".into()),
        ..ProviderSettings::new("moonshot")
    }]);
    let service = fixture.service(ProviderEndpoints::default());
    let connection = |model: &str, protocol| {
        service
            .client(&choice("moonshot", model, protocol))
            .unwrap()
            .connection()
            .clone()
    };

    let kept = connection("kimi-k2", Some(LlmProtocol::AnthropicMessages));
    assert_eq!(kept.protocol, llm::LlmProtocol::AnthropicMessages);
    assert_eq!(kept.base_url, "https://api.moonshot.cn/anthropic/v1");
    assert_eq!(kept.auth_mode, llm::AuthMode::XApiKey);
    let gone = connection("kimi-k2", Some(LlmProtocol::OpenAiResponses));
    assert_eq!(gone.protocol, llm::LlmProtocol::OpenAiChatCompletions);
    assert_eq!(gone.base_url, "https://api.moonshot.cn/v1");
    assert_eq!(gone.auth_mode, llm::AuthMode::Bearer);
    let claude = connection("claude-sonnet-5-5", None);
    assert_eq!(claude.protocol, llm::LlmProtocol::AnthropicMessages);
}

#[test]
fn a_region_must_be_the_vendors_and_overrides_apply_per_served_api() {
    let fixture = Fixture::new();
    fixture.save(Vec::new());
    let service = fixture.service(ProviderEndpoints::default());
    let (revision, _) = service.views().unwrap();
    let error = service
        .save(
            &revision,
            ProviderSettings {
                region: Some("Mars".into()),
                ..ProviderSettings::new("moonshot")
            },
            CredentialChange::Keep,
        )
        .unwrap_err();
    assert!(error.message.contains("Mars"), "{error}");

    service
        .save(
            &revision,
            ProviderSettings {
                region: Some("China".into()),
                endpoints: EndpointUrls {
                    chat: Some("https://proxy.example.com/v1".into()),
                    // Kimi serves no Responses API: its override is ignored.
                    responses: Some("https://proxy.example.com/v1".into()),
                    messages: None,
                },
                ..ProviderSettings::new("moonshot")
            },
            CredentialChange::Keep,
        )
        .unwrap();
    let (_, views) = service.views().unwrap();
    let kimi = views.iter().find(|view| view.id == "moonshot").unwrap();
    assert_eq!(kimi.region, Some("China"));
    assert_eq!(
        kimi.endpoints.iter().collect::<Vec<_>>(),
        [
            (
                llm::LlmProtocol::OpenAiChatCompletions,
                "https://proxy.example.com/v1"
            ),
            (
                llm::LlmProtocol::AnthropicMessages,
                "https://api.moonshot.cn/anthropic/v1"
            ),
        ]
    );
}
