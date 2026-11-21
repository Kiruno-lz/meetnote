//! The providers the built-in Assistant can run on: catalog vendors and
//! custom endpoints, their credentials, sign-ins, model lists and quota.
//! Every function blocks; the desktop runs them off the UI thread.

use crate::desktop_assistant::AssistantUiError;
use llm::vendors::catalog::{self, Region, SignIn, Vendor};
use llm::vendors::endpoints::{default_auth, native, Endpoints, Prefer, PROTOCOLS};
use llm::vendors::family::{best_in_family, newest_in_family};
use llm::vendors::models::ListedModel;
use llm::vendors::oauth::chatgpt::{
    self, import_codex_auth, ChatGptEndpoints, ChatGptSignIn, ChatGptTokens,
};
use llm::vendors::oauth::openrouter::{OpenRouterEndpoints, OpenRouterSignIn};
use llm::vendors::quota::{self, Quota};
use llm::{
    ApiProvider, AuthMode, Backend, ConnectionConfig, Credential, LlmCancellation, LlmError,
    LlmErrorKind, LlmProtocol, LlmProvider, ProbeResult,
};
use settings::{
    AppSettings, AssistantAuthMode, ChatGptAccount, CredentialStore, ModelChoice, ModelListMode,
    ModelSelection, ProviderCredential, ProviderSettings, SettingsError, SettingsService,
};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a browser sign-in waits for its callback.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The most vendor-listed models the picker offers.
const MAX_VENDOR_MODELS: usize = 24;

/// Where sign-ins and the Codex login are found; tests point them at stubs.
#[derive(Debug, Clone, Default)]
pub struct ProviderEndpoints {
    pub chatgpt: ChatGptEndpoints,
    pub openrouter: OpenRouterEndpoints,
    /// Codex CLI's home; `$CODEX_HOME`, then `~/.codex`, when `None`.
    pub codex_home: Option<PathBuf>,
}

#[derive(Clone)]
pub struct ProviderService {
    settings: SettingsService,
    credentials: CredentialStore,
    endpoints: ProviderEndpoints,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialChange {
    Keep,
    ApiKey(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSummary {
    None,
    ApiKey {
        /// The key's last four characters; empty for a key too short to
        /// show any of it.
        last4: String,
    },
    Account {
        email: String,
        plan: Option<String>,
        subscription_until: Option<u64>,
    },
}

impl CredentialSummary {
    pub fn of(credential: Option<&ProviderCredential>) -> Self {
        match credential {
            None => Self::None,
            Some(ProviderCredential::ApiKey { key }) => {
                let chars = key.chars().collect::<Vec<_>>();
                let last4 = if chars.len() >= 8 {
                    chars[chars.len() - 4..].iter().collect()
                } else {
                    String::new()
                };
                Self::ApiKey { last4 }
            }
            Some(ProviderCredential::ChatGpt(account)) => Self::Account {
                email: account.email.clone(),
                plan: account.plan.clone(),
                subscription_until: account.subscription_until,
            },
        }
    }
}

/// One row of the Providers page.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderView {
    pub id: String,
    /// `None` for a custom endpoint.
    pub vendor: Option<&'static Vendor>,
    pub name: String,
    /// Whether settings hold an entry for it.
    pub configured: bool,
    pub enabled: bool,
    /// The region requests go to: the saved one, else the vendor's first;
    /// `None` for a custom provider.
    pub region: Option<&'static str>,
    /// Where each API is asked: the region's, with the saved overrides; a
    /// custom provider's own; the ChatGPT backend's Responses once signed
    /// in with ChatGPT.
    pub endpoints: Endpoints,
    /// Where the native rule picks Responses.
    pub prefer: Prefer,
    pub credential: CredentialSummary,
    /// The saved entry, or the defaults an unconfigured vendor would get.
    pub settings: ProviderSettings,
}

/// How a provider is reached.
struct Route {
    endpoints: Endpoints,
    prefer: Prefer,
    /// A custom provider's header for its key; each API's own when `None`.
    auth: Option<AuthMode>,
    backend: Backend,
}

impl ProviderService {
    pub fn new(settings: SettingsService, credentials: CredentialStore) -> Self {
        Self {
            settings,
            credentials,
            endpoints: ProviderEndpoints::default(),
        }
    }

    pub fn with_endpoints(mut self, endpoints: ProviderEndpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    pub fn settings(&self) -> &SettingsService {
        &self.settings
    }

    /// Every catalog vendor, configured or not, then the custom providers,
    /// with the settings revision they were read at.
    pub fn views(&self) -> Result<(String, Vec<ProviderView>), AssistantUiError> {
        let settings = self.settings.snapshot()?;
        let saved = &settings.assistant.providers;
        let entries = catalog::catalog()
            .iter()
            .map(|vendor| {
                saved
                    .iter()
                    .find(|provider| provider.id == vendor.id)
                    .map_or_else(
                        || (ProviderSettings::new(vendor.id), false),
                        |p| (p.clone(), true),
                    )
            })
            .chain(
                saved
                    .iter()
                    .filter(|provider| provider.is_custom())
                    .map(|provider| (provider.clone(), true)),
            );
        let views = entries
            .map(|(provider, configured)| {
                let credential = self.credentials.get(&provider.id)?;
                let route = self.route(&provider, credential.as_ref())?;
                let vendor = catalog::vendor(&provider.id);
                let region = vendor
                    .map(|vendor| region_of(vendor, &provider))
                    .transpose()?
                    .map(|region| region.label);
                Ok(ProviderView {
                    id: provider.id.clone(),
                    vendor,
                    name: match (&provider.custom, vendor) {
                        (Some(custom), _) => custom.name.clone(),
                        (None, Some(vendor)) => vendor.name.into(),
                        (None, None) => provider.id.clone(),
                    },
                    configured,
                    enabled: configured && provider.enabled,
                    region,
                    endpoints: route.endpoints,
                    prefer: route.prefer,
                    credential: CredentialSummary::of(credential.as_ref()),
                    settings: provider,
                })
            })
            .collect::<Result<_, AssistantUiError>>()?;
        Ok((SettingsService::revision_of(&settings), views))
    }

    /// Saves one provider's settings, then applies `credential` to it.
    pub fn save(
        &self,
        expected_revision: &str,
        provider: ProviderSettings,
        credential: CredentialChange,
    ) -> Result<AppSettings, AssistantUiError> {
        if let (false, None) = (provider.is_custom(), catalog::vendor(&provider.id)) {
            return Err(invalid(format!("unknown vendor: {}", provider.id)));
        }
        if let (Some(vendor), Some(label)) = (catalog::vendor(&provider.id), &provider.region) {
            if vendor.region(label).is_none() {
                return Err(invalid(format!("{} has no region {label}", vendor.name)));
            }
        }
        if matches!(&credential, CredentialChange::ApiKey(key) if key.trim().is_empty()) {
            return Err(invalid("the API key is empty"));
        }
        let id = provider.id.clone();
        let saved = self.settings.edit(expected_revision, |settings| {
            let providers = &mut settings.assistant.providers;
            match providers.iter_mut().find(|saved| saved.id == provider.id) {
                Some(saved) => *saved = provider,
                None => providers.push(provider),
            }
            Ok(())
        })?;
        match credential {
            CredentialChange::Keep => {}
            CredentialChange::ApiKey(key) => self.credentials.set(
                &id,
                Some(ProviderCredential::ApiKey {
                    key: key.trim().to_owned(),
                }),
            )?,
            CredentialChange::Clear => self.credentials.set(&id, None)?,
        }
        Ok(saved)
    }

    /// Removes a provider, its credential and a model choice naming it.
    pub fn remove(
        &self,
        expected_revision: &str,
        id: &str,
    ) -> Result<AppSettings, AssistantUiError> {
        let saved = self.settings.edit(expected_revision, |settings| {
            let assistant = &mut settings.assistant;
            assistant.providers.retain(|provider| provider.id != id);
            if assistant
                .model
                .as_ref()
                .is_some_and(|choice| choice.provider == id)
            {
                assistant.model = None;
            }
            Ok(())
        })?;
        self.credentials.set(id, None)?;
        Ok(saved)
    }

    pub fn set_model(
        &self,
        expected_revision: &str,
        choice: Option<ModelChoice>,
    ) -> Result<AppSettings, AssistantUiError> {
        Ok(self.settings.edit(expected_revision, |settings| {
            settings.assistant.model = choice;
            Ok(())
        })?)
    }

    /// Opens the loopback callback of the vendor's browser sign-in.
    pub fn begin_sign_in(&self, id: &str) -> Result<PendingSignIn, AssistantUiError> {
        let flow = match catalog::vendor(id).and_then(|vendor| vendor.sign_in) {
            Some(SignIn::ChatGpt) => {
                Flow::ChatGpt(ChatGptSignIn::start(self.endpoints.chatgpt.clone())?)
            }
            Some(SignIn::OpenRouter) => {
                Flow::OpenRouter(OpenRouterSignIn::start(self.endpoints.openrouter.clone())?)
            }
            None => return Err(invalid(format!("{id} has no browser sign-in"))),
        };
        let url = match &flow {
            Flow::ChatGpt(sign_in) => sign_in.url().to_owned(),
            Flow::OpenRouter(sign_in) => sign_in.url().to_owned(),
        };
        Ok(PendingSignIn {
            url,
            id: id.to_owned(),
            flow,
            service: self.clone(),
        })
    }

    /// Signs in to ChatGPT with Codex CLI's login, which is only read.
    pub fn import_codex_login(&self) -> Result<CredentialSummary, AssistantUiError> {
        let home = self
            .endpoints
            .codex_home
            .clone()
            .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
            .ok_or_else(|| invalid("the Codex home directory is unknown"))?;
        let path = home.join("auth.json");
        let json = std::fs::read_to_string(&path).map_err(|error| AssistantUiError {
            code: "CODEX_LOGIN_UNAVAILABLE".into(),
            message: format!("cannot read {}: {error}", path.display()),
            retryable: false,
        })?;
        let tokens = import_codex_auth(&json)?;
        self.store_sign_in("openai", ProviderCredential::ChatGpt(account(tokens)))
    }

    /// Forgets the provider's credential and disables it.
    pub fn sign_out(
        &self,
        expected_revision: &str,
        id: &str,
    ) -> Result<AppSettings, AssistantUiError> {
        let saved = self.settings.edit(expected_revision, |settings| {
            if let Some(provider) = settings
                .assistant
                .providers
                .iter_mut()
                .find(|provider| provider.id == id)
            {
                provider.enabled = false;
            }
            Ok(())
        })?;
        self.credentials.set(id, None)?;
        Ok(saved)
    }

    /// The models the provider lists now on each of its APIs, merged in
    /// its order.
    pub fn models(
        &self,
        id: &str,
        cancellation: &LlmCancellation,
    ) -> Result<Vec<ListedModel>, AssistantUiError> {
        let provider = self.saved(id)?;
        let credential = self.fresh_credential(id, cancellation)?;
        let result = match &credential {
            Some(ProviderCredential::ChatGpt(account)) => {
                llm::vendors::models::list_chatgpt_models(
                    &self.endpoints.chatgpt,
                    &tokens(account),
                    cancellation,
                )
                .map(|ids| {
                    ids.into_iter()
                        .map(|id| ListedModel {
                            id,
                            protocols: vec![LlmProtocol::OpenAiResponses],
                        })
                        .collect()
                })
            }
            _ => {
                let route = self.route(&provider, credential.as_ref())?;
                llm::vendors::models::list_models(
                    &route.endpoints,
                    route.auth,
                    api_key(credential.as_ref()).as_ref(),
                    cancellation,
                )
            }
        };
        result.map_err(|error| scrub(error.into(), credential.as_ref()))
    }

    /// A ChatGPT plan's usage, or the prepaid balance of a vendor that tells
    /// it; `None` for the rest.
    pub fn quota(
        &self,
        id: &str,
        cancellation: &LlmCancellation,
    ) -> Result<Option<Quota>, AssistantUiError> {
        let provider = self.saved(id)?;
        let credential = self.fresh_credential(id, cancellation)?;
        let result = match (&credential, catalog::vendor(id).and_then(|v| v.balance)) {
            (Some(ProviderCredential::ChatGpt(account)), _) => {
                quota::chatgpt_quota(&self.endpoints.chatgpt, &tokens(account), cancellation)
                    .map(Some)
            }
            (Some(ProviderCredential::ApiKey { key }), Some(source)) => {
                let route = self.route(&provider, credential.as_ref())?;
                let base_url = route
                    .endpoints
                    .chat
                    .ok_or_else(|| not_configured(format!("{id} serves no chat API")))?;
                quota::balance(
                    source,
                    &base_url,
                    &Credential::new(key.clone()),
                    cancellation,
                )
                .map(|balance| {
                    Some(Quota {
                        balance: Some(balance),
                        ..Quota::default()
                    })
                })
            }
            _ => Ok(None),
        };
        result.map_err(|error| scrub(error.into(), credential.as_ref()))
    }

    /// Checks that `choice` answers text and calls client tools, on the
    /// API [`Self::client`] would ask it on.
    pub fn test(
        &self,
        choice: &ModelChoice,
        cancellation: &LlmCancellation,
    ) -> Result<ProbeResult, AssistantUiError> {
        let provider = self.connect(choice, cancellation)?;
        provider
            .probe(cancellation)
            .map_err(|error| self.redact(&choice.provider, error.into()))
    }

    /// The client the Assistant runs `choice` with: on its resolved API
    /// while the provider still serves it, else on the native one. An
    /// expiring ChatGPT sign-in is refreshed first, and the rotated tokens
    /// are stored before they are used.
    pub fn client(&self, choice: &ModelChoice) -> Result<ApiProvider, AssistantUiError> {
        if !self.saved(&choice.provider)?.enabled {
            return Err(not_configured(format!(
                "the provider {} is disabled",
                choice.provider
            )));
        }
        self.connect(choice, &LlmCancellation::new())
    }

    /// `error` with every secret of `id`'s stored credential masked.
    pub fn redact(&self, id: &str, error: AssistantUiError) -> AssistantUiError {
        match self.credentials.get(id) {
            Ok(credential) => scrub(error, credential.as_ref()),
            Err(_) => error,
        }
    }

    fn connect(
        &self,
        choice: &ModelChoice,
        cancellation: &LlmCancellation,
    ) -> Result<ApiProvider, AssistantUiError> {
        let provider = self.saved(&choice.provider)?;
        let credential = self.fresh_credential(&provider.id, cancellation)?;
        let route = self.route(&provider, credential.as_ref())?;
        let (protocol, base_url) = choice
            .protocol
            .map(llm_protocol)
            .filter(|protocol| route.endpoints.get(*protocol).is_some())
            .or_else(|| model_protocol(&route.endpoints, route.prefer, &[], &choice.model))
            .and_then(|protocol| Some((protocol, route.endpoints.get(protocol)?.to_owned())))
            .ok_or_else(|| not_configured(format!("{} serves no API", provider.id)))?;
        let secret = match &credential {
            Some(ProviderCredential::ApiKey { key }) => Some(Credential::new(key.clone())),
            Some(ProviderCredential::ChatGpt(account)) => {
                Some(Credential::new(account.access_token.clone()))
            }
            None => None,
        };
        ApiProvider::with_backend(
            ConnectionConfig {
                id: provider.id.clone(),
                protocol,
                base_url,
                model: choice.model.clone(),
                auth_mode: route.auth.unwrap_or(default_auth(protocol)),
                timeout_ms: provider.timeout_ms,
                context_window_tokens: provider.context_window_tokens,
                max_output_tokens: provider.max_output_tokens,
            },
            secret,
            route.backend,
        )
        .map_err(|error| scrub(error.into(), credential.as_ref()))
    }

    fn saved(&self, id: &str) -> Result<ProviderSettings, AssistantUiError> {
        self.settings
            .snapshot()?
            .assistant
            .provider(id)
            .cloned()
            .ok_or_else(|| not_configured(format!("the provider {id} is not set up")))
    }

    fn route(
        &self,
        provider: &ProviderSettings,
        credential: Option<&ProviderCredential>,
    ) -> Result<Route, AssistantUiError> {
        if let Some(ProviderCredential::ChatGpt(account)) = credential {
            return Ok(Route {
                endpoints: Endpoints {
                    responses: Some(self.endpoints.chatgpt.codex_base_url()),
                    ..Endpoints::default()
                },
                prefer: Prefer::Nothing,
                auth: None,
                backend: Backend::ChatGpt {
                    account_id: account.account_id.clone(),
                },
            });
        }
        let (mut endpoints, prefer, auth) = match (&provider.custom, catalog::vendor(&provider.id))
        {
            (Some(custom), _) => (
                Endpoints::default(),
                Prefer::Nothing,
                Some(match custom.auth_mode {
                    AssistantAuthMode::Bearer => AuthMode::Bearer,
                    AssistantAuthMode::XApiKey => AuthMode::XApiKey,
                }),
            ),
            (None, Some(vendor)) => (
                region_of(vendor, provider)?.endpoints.owned(),
                vendor.prefer,
                None,
            ),
            (None, None) => {
                return Err(not_configured(format!("unknown vendor: {}", provider.id)));
            }
        };
        // A vendor's overrides move the APIs its region serves; overrides of
        // other APIs are ignored.
        for protocol in PROTOCOLS {
            let served = provider.custom.is_some() || endpoints.get(protocol).is_some();
            if let Some(url) = provider
                .endpoints
                .get(settings_protocol(protocol))
                .filter(|_| served)
            {
                endpoints.set(protocol, Some(url.to_owned()));
            }
        }
        Ok(Route {
            endpoints,
            prefer,
            auth,
            backend: Backend::Standard,
        })
    }

    /// The stored credential, with an expiring ChatGPT sign-in renewed and
    /// persisted under the store's lock.
    fn fresh_credential(
        &self,
        id: &str,
        cancellation: &LlmCancellation,
    ) -> Result<Option<ProviderCredential>, AssistantUiError> {
        self.credentials.update(id, |credential| match credential {
            Some(ProviderCredential::ChatGpt(stored)) if tokens(&stored).needs_refresh(now()) => {
                let renewed =
                    chatgpt::refresh(&self.endpoints.chatgpt, &tokens(&stored), cancellation)
                        .map_err(|error| {
                            scrub(
                                error.into(),
                                Some(&ProviderCredential::ChatGpt(stored.clone())),
                            )
                        })?;
                Ok(Some(ProviderCredential::ChatGpt(account(renewed))))
            }
            other => Ok(other),
        })
    }

    /// Stores a sign-in's credential and enables its provider, creating the
    /// provider's entry when it has none.
    fn store_sign_in(
        &self,
        id: &str,
        credential: ProviderCredential,
    ) -> Result<CredentialSummary, AssistantUiError> {
        let summary = CredentialSummary::of(Some(&credential));
        self.credentials.set(id, Some(credential))?;
        let current = self.settings.reload()?;
        self.settings
            .edit(&SettingsService::revision_of(&current), |settings| {
                let providers = &mut settings.assistant.providers;
                match providers.iter_mut().find(|provider| provider.id == id) {
                    Some(provider) => provider.enabled = true,
                    None => providers.push(ProviderSettings::new(id)),
                }
                Ok(())
            })?;
        Ok(summary)
    }
}

enum Flow {
    ChatGpt(ChatGptSignIn),
    OpenRouter(OpenRouterSignIn),
}

/// A browser sign-in waiting for its callback: open `url`, then `finish`.
pub struct PendingSignIn {
    pub url: String,
    id: String,
    flow: Flow,
    service: ProviderService,
}

impl PendingSignIn {
    pub fn provider(&self) -> &str {
        &self.id
    }

    /// Waits for the browser, trades the code, stores the credential and
    /// enables the provider. A cancelled wait is `is_cancelled()`.
    pub fn finish(
        self,
        cancellation: &LlmCancellation,
    ) -> Result<CredentialSummary, AssistantUiError> {
        self.finish_within(cancellation, SIGN_IN_TIMEOUT)
    }

    fn finish_within(
        self,
        cancellation: &LlmCancellation,
        timeout: Duration,
    ) -> Result<CredentialSummary, AssistantUiError> {
        let credential = match self.flow {
            Flow::ChatGpt(sign_in) => {
                ProviderCredential::ChatGpt(account(sign_in.finish(cancellation, timeout)?))
            }
            Flow::OpenRouter(sign_in) => ProviderCredential::ApiKey {
                key: sign_in.finish(cancellation, timeout)?,
            },
        };
        self.service.store_sign_in(&self.id, credential)
    }
}

/// The models the picker offers: `Vendor` is the live list (at most 24)
/// then the added ones, `Picked` the picked then the added ones, and
/// `LatestPerFamily` the newest of each family among the live and added
/// ones. Duplicates are dropped, first wins.
pub fn visible_models(selection: &ModelSelection, live: &[String]) -> Vec<String> {
    match selection.mode {
        ModelListMode::Vendor => {
            union(&live[..live.len().min(MAX_VENDOR_MODELS)], &selection.added)
        }
        ModelListMode::Picked => union(&selection.picked, &selection.added),
        ModelListMode::LatestPerFamily => best_in_family(&union(live, &selection.added)),
    }
}

/// The choice `choice` is upgraded to under `LatestPerFamily`: its
/// family's newest among the live and added models, when newer than
/// `choice`, on the API that model resolves to.
pub fn upgraded_choice(
    view: &ProviderView,
    live: &[ListedModel],
    choice: &ModelChoice,
) -> Option<ModelChoice> {
    let selection = &view.settings.models;
    (selection.mode == ModelListMode::LatestPerFamily)
        .then(|| union(&model_ids(live), &selection.added))
        .and_then(|models| {
            newest_in_family(&models, &choice.model).map(|model| model_choice(view, live, model))
        })
}

/// `model` of `view` as the Assistant's choice, on the API its listing in
/// `live` resolves to (the native rule alone for a model not listed).
pub fn model_choice(view: &ProviderView, live: &[ListedModel], model: &str) -> ModelChoice {
    let listed = live
        .iter()
        .find(|listed| listed.id == model)
        .map_or(&[][..], |listed| &listed.protocols);
    ModelChoice {
        provider: view.id.clone(),
        model: model.to_owned(),
        protocol: model_protocol(&view.endpoints, view.prefer, listed, model)
            .map(settings_protocol),
    }
}

/// The API `model` is best asked on at `endpoints`: magpie's native rule,
/// among the APIs `listed` names unless none of them is served. `None`
/// when `endpoints` serve no API.
pub fn model_protocol(
    endpoints: &Endpoints,
    prefer: Prefer,
    listed: &[LlmProtocol],
    model: &str,
) -> Option<LlmProtocol> {
    let available = endpoints.protocols();
    native(&available, prefer, listed, model).or_else(|| native(&available, prefer, &[], model))
}

/// The ids of a merged model list, in its order.
pub fn model_ids(live: &[ListedModel]) -> Vec<String> {
    live.iter().map(|model| model.id.clone()).collect()
}

/// The region `provider` names, else `vendor`'s first.
fn region_of(
    vendor: &'static Vendor,
    provider: &ProviderSettings,
) -> Result<&'static Region, AssistantUiError> {
    match &provider.region {
        None => Ok(vendor.default_region()),
        Some(label) => vendor
            .region(label)
            .ok_or_else(|| not_configured(format!("{} has no region {label}", vendor.name))),
    }
}

/// `protocol` as the llm crate names it.
pub fn llm_protocol(protocol: settings::LlmProtocol) -> LlmProtocol {
    match protocol {
        settings::LlmProtocol::OpenAiChatCompletions => LlmProtocol::OpenAiChatCompletions,
        settings::LlmProtocol::OpenAiResponses => LlmProtocol::OpenAiResponses,
        settings::LlmProtocol::AnthropicMessages => LlmProtocol::AnthropicMessages,
    }
}

/// `protocol` as settings name it.
pub fn settings_protocol(protocol: LlmProtocol) -> settings::LlmProtocol {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => settings::LlmProtocol::OpenAiChatCompletions,
        LlmProtocol::OpenAiResponses => settings::LlmProtocol::OpenAiResponses,
        LlmProtocol::AnthropicMessages => settings::LlmProtocol::AnthropicMessages,
    }
}

fn union(first: &[String], second: &[String]) -> Vec<String> {
    let mut models = Vec::with_capacity(first.len() + second.len());
    for model in first.iter().chain(second) {
        if !models.contains(model) {
            models.push(model.clone());
        }
    }
    models
}

fn tokens(account: &ChatGptAccount) -> ChatGptTokens {
    ChatGptTokens {
        access_token: account.access_token.clone(),
        refresh_token: account.refresh_token.clone(),
        id_token: account.id_token.clone(),
        account_id: account.account_id.clone(),
        email: account.email.clone(),
        plan: account.plan.clone(),
        expires_at: account.expires_at,
        subscription_until: account.subscription_until,
    }
}

fn account(tokens: ChatGptTokens) -> ChatGptAccount {
    ChatGptAccount {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        account_id: tokens.account_id,
        email: tokens.email,
        plan: tokens.plan,
        expires_at: tokens.expires_at,
        subscription_until: tokens.subscription_until,
    }
}

fn api_key(credential: Option<&ProviderCredential>) -> Option<Credential> {
    match credential {
        Some(ProviderCredential::ApiKey { key }) => Some(Credential::new(key.clone())),
        _ => None,
    }
}

fn scrub(mut error: AssistantUiError, credential: Option<&ProviderCredential>) -> AssistantUiError {
    for secret in credential.into_iter().flat_map(ProviderCredential::secrets) {
        if !secret.is_empty() {
            error.message = error.message.replace(secret, "[REDACTED]");
        }
    }
    error
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn invalid(message: impl Into<String>) -> AssistantUiError {
    AssistantUiError::from_settings(&SettingsError::Invalid(message.into()))
}

fn not_configured(message: impl Into<String>) -> AssistantUiError {
    AssistantUiError::from(LlmError::new(
        LlmErrorKind::ConnectionNotConfigured,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn selection(mode: ModelListMode, picked: &[&str], added: &[&str]) -> ModelSelection {
        ModelSelection {
            mode,
            picked: ids(picked),
            added: ids(added),
        }
    }

    #[test]
    fn visible_models_follow_the_list_mode() {
        let live = ids(&["gpt-5", "gpt-5.1", "gpt-4o", "o3"]);
        for (selection, expected) in [
            (
                selection(ModelListMode::Vendor, &["o3"], &["my-model", "gpt-5"]),
                vec!["gpt-5", "gpt-5.1", "gpt-4o", "o3", "my-model"],
            ),
            (
                selection(ModelListMode::Picked, &["o3", "gpt-5"], &["my-model", "o3"]),
                vec!["o3", "gpt-5", "my-model"],
            ),
            (
                selection(ModelListMode::Picked, &[], &[]),
                Vec::<&str>::new(),
            ),
            (
                selection(ModelListMode::LatestPerFamily, &[], &["gpt-5.2"]),
                vec!["gpt-4o", "o3", "gpt-5.2"],
            ),
        ] {
            assert_eq!(
                visible_models(&selection, &live),
                ids(&expected),
                "{selection:?}"
            );
        }
    }

    #[test]
    fn the_vendor_list_is_capped_but_added_models_stay() {
        let live = (0..30)
            .map(|index| format!("m-{index}"))
            .collect::<Vec<_>>();
        let models = visible_models(
            &selection(ModelListMode::Vendor, &[], &["mine", "m-29"]),
            &live,
        );
        assert_eq!(models.len(), 26);
        assert_eq!(models[23], "m-23");
        assert_eq!(&models[24..], ["mine", "m-29"]);
    }

    fn listed(id: &str, protocols: &[LlmProtocol]) -> ListedModel {
        ListedModel {
            id: id.into(),
            protocols: protocols.to_vec(),
        }
    }

    /// OpenAI's view: Chat and Responses, Responses for its own models.
    fn openai(models: ModelSelection) -> ProviderView {
        let vendor = catalog::vendor("openai").unwrap();
        ProviderView {
            id: "openai".into(),
            vendor: Some(vendor),
            name: vendor.name.into(),
            configured: true,
            enabled: true,
            region: Some("Global"),
            endpoints: vendor.default_region().endpoints.owned(),
            prefer: vendor.prefer,
            credential: CredentialSummary::None,
            settings: ProviderSettings {
                models,
                ..ProviderSettings::new("openai")
            },
        }
    }

    fn choice(model: &str, protocol: Option<settings::LlmProtocol>) -> ModelChoice {
        ModelChoice {
            provider: "openai".into(),
            model: model.into(),
            protocol,
        }
    }

    #[test]
    fn only_latest_per_family_upgrades_a_choice_on_the_new_models_api() {
        use settings::LlmProtocol::{OpenAiChatCompletions as Chat, OpenAiResponses as Responses};
        let live = [
            listed("gpt-5", &[]),
            listed("gpt-5.1", &[LlmProtocol::OpenAiChatCompletions]),
            listed("o3", &[]),
        ];
        let latest = openai(selection(ModelListMode::LatestPerFamily, &[], &[]));
        assert_eq!(
            upgraded_choice(&latest, &live, &choice("gpt-5", Some(Responses))),
            Some(choice("gpt-5.1", Some(Chat)))
        );
        assert_eq!(
            upgraded_choice(&latest, &live, &choice("gpt-5.1", Some(Chat))),
            None
        );
        assert_eq!(upgraded_choice(&latest, &live, &choice("o3", None)), None);
        let added = openai(selection(ModelListMode::LatestPerFamily, &[], &["gpt-5.2"]));
        assert_eq!(
            upgraded_choice(&added, &live, &choice("gpt-5", Some(Chat))),
            Some(choice("gpt-5.2", Some(Responses)))
        );
        for mode in [ModelListMode::Vendor, ModelListMode::Picked] {
            assert_eq!(
                upgraded_choice(
                    &openai(selection(mode, &[], &[])),
                    &live,
                    &choice("gpt-5", None)
                ),
                None
            );
        }
    }

    #[test]
    fn a_models_api_comes_from_its_listing_unless_none_of_it_is_served() {
        use LlmProtocol::{
            AnthropicMessages as Messages, OpenAiChatCompletions as Chat,
            OpenAiResponses as Responses,
        };
        let magpie = catalog::vendor("magpie").unwrap();
        let gateway = magpie.default_region().endpoints.owned();
        for (listed, model, expected) in [
            (&[][..], "claude-opus-5-5", Some(Messages)),
            (&[], "gpt-5", Some(Responses)),
            (&[], "kimi-k2", Some(Chat)),
            (&[Chat], "claude-opus-5-5", Some(Chat)),
            (&[Messages, Responses], "kimi-k2", Some(Responses)),
        ] {
            assert_eq!(
                model_protocol(&gateway, magpie.prefer, listed, model),
                expected,
                "{listed:?} {model}"
            );
        }
        let chat_only = Endpoints {
            chat: Some("https://a.example/v1".into()),
            ..Endpoints::default()
        };
        assert_eq!(
            model_protocol(&chat_only, Prefer::Nothing, &[Messages], "claude-x"),
            Some(Chat)
        );
        assert_eq!(
            model_protocol(&Endpoints::default(), Prefer::Nothing, &[], "x"),
            None
        );
        let view = openai(ModelSelection::default());
        assert_eq!(
            model_choice(&view, &[listed("o3", &[Chat])], "o3").protocol,
            Some(settings::LlmProtocol::OpenAiChatCompletions)
        );
        assert_eq!(
            model_choice(&view, &[], "o3").protocol,
            Some(settings::LlmProtocol::OpenAiResponses)
        );
    }

    #[test]
    fn protocols_convert_both_ways() {
        for protocol in PROTOCOLS {
            assert_eq!(llm_protocol(settings_protocol(protocol)), protocol);
        }
    }

    #[test]
    fn a_summary_shows_only_the_last_four_of_a_long_key() {
        let key = |key: &str| {
            CredentialSummary::of(Some(&ProviderCredential::ApiKey { key: key.into() }))
        };
        assert_eq!(
            key("sk-abcdef123456"),
            CredentialSummary::ApiKey {
                last4: "3456".into()
            }
        );
        assert_eq!(
            key("短密钥一二三四五六"),
            CredentialSummary::ApiKey {
                last4: "三四五六".into()
            }
        );
        assert_eq!(
            key("sk-1234"),
            CredentialSummary::ApiKey {
                last4: String::new()
            }
        );
        assert_eq!(CredentialSummary::of(None), CredentialSummary::None);
    }

    #[test]
    fn secrets_are_scrubbed_from_messages() {
        let error = scrub(
            AssistantUiError::unavailable("bad key sk-secret-1 for sk-secret-1"),
            Some(&ProviderCredential::ApiKey {
                key: "sk-secret-1".into(),
            }),
        );
        assert_eq!(error.message, "bad key [REDACTED] for [REDACTED]");
    }
}
