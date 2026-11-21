//! The Providers page's state: the provider views, their live model lists,
//! quota and test results, the detail sheet's draft, and the background
//! work behind them. Settings writes run one at a time; network work runs
//! at most once per provider and kind; a browser sign-in is its own
//! cancellable wait.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use application::desktop_assistant::AssistantUiError;
use application::{
    CredentialChange, CredentialSummary, ProviderService, ProviderView, model_choice, model_ids,
    settings_protocol, upgraded_choice,
};
use futures::channel::oneshot;
use llm::vendors::catalog::{self, Region};
use llm::vendors::endpoints::{Endpoints, PROTOCOLS};
use llm::vendors::models::ListedModel;
use llm::vendors::oauth::chatgpt::ChatGptEndpoints;
use llm::vendors::quota::{Quota, QuotaWindow};
use llm::{LlmCancellation, LlmProtocol, ProbeResult};
use rune::{Key, KeyEvent, TextEdit, Tx};
use settings::{
    AssistantAuthMode, AssistantSettings, CustomEndpoint, EndpointUrls, ModelChoice, ModelListMode,
    ProviderCredential, ProviderSettings,
};

use super::{Task, WorkspaceHost, processing::ProcessingConfig};
use crate::MeetNote;

/// The vendor search's field and the button that opens it.
pub(crate) const SEARCH: &str = "providers-search";
pub(crate) const SEARCH_BUTTON: &str = "providers-search-button";

/// The providers as settings last held them.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub revision: String,
    pub views: Vec<ProviderView>,
    /// The Assistant's model choice.
    pub model: Option<ModelChoice>,
}

impl Snapshot {
    pub fn view(&self, id: &str) -> Option<&ProviderView> {
        self.views.iter().find(|view| view.id == id)
    }
}

/// What the network said about one provider; derived, never persisted.
#[derive(Default)]
pub(crate) struct Live {
    pub models: Option<Vec<ListedModel>>,
    /// The app clock when `models` arrived.
    pub fetched_at: f64,
    pub models_error: Option<String>,
    pub quota: Option<Quota>,
    pub quota_error: Option<String>,
    /// The last tested model and whether it answered.
    pub test: Option<(String, Result<(), String>)>,
}

impl Live {
    /// The listed models' ids, once the list arrived.
    pub fn ids(&self) -> Option<Vec<String>> {
        self.models.as_deref().map(model_ids)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Work {
    Models,
    Quota,
    Test,
}

struct Job {
    id: String,
    work: Work,
    cancellation: LlmCancellation,
    task: Task<Fetched>,
}

enum Fetched {
    Models(Result<Vec<ListedModel>, AssistantUiError>),
    Quota(Result<Option<Quota>, AssistantUiError>),
    Test(String, Result<ProbeResult, AssistantUiError>),
}

/// A change to the providers' settings or credentials.
#[derive(Clone)]
pub(crate) enum Write {
    Load,
    /// The sheet's draft, against the revision it was opened at.
    Save {
        revision: String,
        settings: ProviderSettings,
        credential: CredentialChange,
    },
    Toggle {
        id: String,
        enabled: bool,
    },
    Remove(String),
    SetModel(Option<ModelChoice>),
    SignOut(String),
    ImportCodex,
}

impl Write {
    fn run(&self, service: &ProviderService) -> Result<(), AssistantUiError> {
        let revision = || {
            service
                .settings()
                .revision()
                .map_err(AssistantUiError::from)
        };
        match self {
            Self::Load => Ok(()),
            Self::Save {
                revision,
                settings,
                credential,
            } => service
                .save(revision, settings.clone(), credential.clone())
                .map(drop),
            Self::Toggle { id, enabled } => {
                let (revision, views) = service.views()?;
                let mut settings = views
                    .into_iter()
                    .find(|view| view.id == *id)
                    .map(|view| view.settings)
                    .unwrap_or_else(|| ProviderSettings::new(id.as_str()));
                settings.enabled = *enabled;
                service
                    .save(&revision, settings, CredentialChange::Keep)
                    .map(drop)
            }
            Self::Remove(id) => service.remove(&revision()?, id).map(drop),
            Self::SetModel(choice) => service.set_model(&revision()?, choice.clone()).map(drop),
            Self::SignOut(id) => service.sign_out(&revision()?, id).map(drop),
            Self::ImportCodex => service.import_codex_login().map(drop),
        }
    }

    /// The same change made to the sample's in-memory providers.
    fn apply_sample(self, snapshot: &mut Snapshot) {
        let view = |snapshot: &mut Snapshot, id: &str| {
            snapshot.views.iter_mut().position(|view| view.id == id)
        };
        match self {
            Self::Load | Self::ImportCodex => {}
            Self::Save {
                settings,
                credential,
                ..
            } => {
                let index = view(snapshot, &settings.id).unwrap_or_else(|| {
                    snapshot.views.push(sample_view(settings.clone()));
                    snapshot.views.len() - 1
                });
                let target = &mut snapshot.views[index];
                *target = ProviderView {
                    configured: true,
                    enabled: settings.enabled,
                    credential: target.credential.clone(),
                    ..sample_view(settings)
                };
                match credential {
                    CredentialChange::Keep => {}
                    CredentialChange::ApiKey(key) => {
                        target.credential =
                            CredentialSummary::of(Some(&ProviderCredential::ApiKey { key }));
                    }
                    CredentialChange::Clear => target.credential = CredentialSummary::None,
                }
            }
            Self::Toggle { id, enabled } => {
                if let Some(index) = view(snapshot, &id) {
                    let target = &mut snapshot.views[index];
                    target.configured = true;
                    target.enabled = enabled;
                    target.settings.enabled = enabled;
                }
            }
            Self::Remove(id) => {
                if let Some(index) = view(snapshot, &id) {
                    if snapshot.views[index].vendor.is_none() {
                        snapshot.views.remove(index);
                    } else {
                        snapshot.views[index] = sample_view(ProviderSettings::new(id.as_str()));
                    }
                }
                if snapshot
                    .model
                    .as_ref()
                    .is_some_and(|choice| choice.provider == id)
                {
                    snapshot.model = None;
                }
            }
            Self::SetModel(choice) => snapshot.model = choice,
            Self::SignOut(id) => {
                if let Some(index) = view(snapshot, &id) {
                    let target = &mut snapshot.views[index];
                    target.enabled = false;
                    target.settings.enabled = false;
                    target.credential = CredentialSummary::None;
                }
            }
        }
    }
}

struct Written {
    write: Write,
    result: Result<(), AssistantUiError>,
    snapshot: Result<(Snapshot, AssistantSettings), AssistantUiError>,
}

/// A browser sign-in: its URL once the callback listens, then the wait.
pub(crate) struct SignIn {
    pub id: String,
    pub url: Option<String>,
    url_rx: Option<oneshot::Receiver<String>>,
    cancellation: LlmCancellation,
    task: Task<Result<CredentialSummary, AssistantUiError>>,
}

impl Drop for SignIn {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// The detail sheet's draft; nothing is written until it is saved.
pub(crate) struct Sheet {
    pub id: String,
    pub revision: String,
    pub draft: ProviderSettings,
    pub key: TextEdit,
    pub name: TextEdit,
    /// Each API's address in [`PROTOCOLS`] order: a vendor's override of
    /// its region's, a custom provider's own.
    pub urls: [TextEdit; 3],
    pub context: TextEdit,
    pub timeout: TextEdit,
    pub max_output: TextEdit,
    pub new_model: TextEdit,
    pub advanced: bool,
    /// The ui-text key of what the draft lacks, set by a save, with the
    /// text its fields held then.
    invalid: Option<(&'static str, Vec<String>)>,
    pub error: Option<String>,
}

impl Sheet {
    fn open(settings: &ProviderSettings, revision: &str) -> Self {
        let name = settings.custom.as_ref().map_or("", |custom| &custom.name);
        Self {
            id: settings.id.clone(),
            revision: revision.to_owned(),
            draft: settings.clone(),
            key: TextEdit::new(""),
            name: TextEdit::new(name),
            urls: PROTOCOLS.map(|protocol| {
                TextEdit::new(
                    settings
                        .endpoints
                        .get(settings_protocol(protocol))
                        .unwrap_or_default(),
                )
            }),
            context: TextEdit::new(
                settings
                    .context_window_tokens
                    .map(|tokens| tokens.to_string())
                    .unwrap_or_default(),
            ),
            timeout: TextEdit::new(settings.timeout_ms.to_string()),
            max_output: TextEdit::new(settings.max_output_tokens.to_string()),
            new_model: TextEdit::new(""),
            advanced: false,
            invalid: None,
            error: None,
        }
    }

    /// The typed address of `protocol`, trimmed.
    pub fn url(&self, protocol: LlmProtocol) -> &str {
        self.urls[slot(protocol)].text.trim()
    }

    pub fn url_mut(&mut self, protocol: LlmProtocol) -> &mut TextEdit {
        &mut self.urls[slot(protocol)]
    }

    /// A vendor's region the draft names, else its first; `None` for a
    /// custom provider.
    pub fn region(&self) -> Option<&'static Region> {
        let vendor = catalog::vendor(&self.id)?;
        Some(
            self.draft
                .region
                .as_deref()
                .and_then(|label| vendor.region(label))
                .unwrap_or(vendor.default_region()),
        )
    }

    /// The APIs the sheet takes an address for: the region's, or all three
    /// for a custom provider.
    pub fn protocols(&self) -> Vec<LlmProtocol> {
        self.region()
            .map_or(PROTOCOLS.to_vec(), |region| region.endpoints.protocols())
    }

    /// Where each API would be asked once saved: the region's, with the
    /// typed overrides.
    pub fn endpoints(&self) -> Endpoints {
        let mut endpoints = self
            .region()
            .map_or_else(Endpoints::default, |region| region.endpoints.owned());
        for protocol in self.protocols() {
            if let Some(url) = Some(self.url(protocol)).filter(|url| !url.is_empty()) {
                endpoints.set(protocol, Some(url.to_owned()));
            }
        }
        endpoints
    }

    /// What the last save found the draft lacks, until one of the fields
    /// it is about is edited.
    pub fn invalid(&self) -> Option<&'static str> {
        let (key, held) = self.invalid.as_ref()?;
        (&self.hinted(key) == held).then_some(*key)
    }

    /// The text of the fields the ui-text key `invalid` is about.
    fn hinted(&self, invalid: &str) -> Vec<String> {
        let fields: Vec<&TextEdit> = match invalid {
            "provider_name_required" => vec![&self.name],
            "provider_url_required" => self.urls.iter().collect(),
            "provider_invalid_number" => vec![&self.context, &self.timeout, &self.max_output],
            _ => Vec::new(),
        };
        fields.into_iter().map(|edit| edit.text.clone()).collect()
    }

    /// The draft as settings, or the ui-text key of what is wrong with it.
    pub fn settings(&self) -> Result<ProviderSettings, &'static str> {
        fn number<T: FromStr>(edit: &TextEdit) -> Result<T, &'static str> {
            edit.text
                .trim()
                .parse()
                .map_err(|_| "provider_invalid_number")
        }
        let context = self.context.text.trim();
        let mut settings = ProviderSettings {
            context_window_tokens: if context.is_empty() {
                None
            } else {
                Some(number(&self.context)?)
            },
            timeout_ms: number(&self.timeout)?,
            max_output_tokens: number(&self.max_output)?,
            endpoints: EndpointUrls::default(),
            ..self.draft.clone()
        };
        for protocol in self.protocols() {
            let url = self.url(protocol);
            settings.endpoints.set(
                settings_protocol(protocol),
                (!url.is_empty()).then(|| url.to_owned()),
            );
        }
        if let Some(custom) = &mut settings.custom {
            custom.name = self.name.text.trim().to_owned();
            if custom.name.is_empty() {
                return Err("provider_name_required");
            }
            if settings.endpoints.is_empty() {
                return Err("provider_url_required");
            }
        }
        Ok(settings)
    }

    fn credential(&self) -> CredentialChange {
        match self.key.text.trim() {
            "" => CredentialChange::Keep,
            key => CredentialChange::ApiKey(key.to_owned()),
        }
    }

    pub fn set_mode(&mut self, mode: ModelListMode) {
        self.draft.models.mode = mode;
    }

    /// Picks or unpicks `model` in `Picked` mode; a hand-added model is dropped.
    pub fn toggle_model(&mut self, model: &str) {
        let models = &mut self.draft.models;
        if let Some(index) = models.added.iter().position(|added| added == model) {
            models.added.remove(index);
        } else if let Some(index) = models.picked.iter().position(|picked| picked == model) {
            models.picked.remove(index);
        } else {
            models.picked.push(model.to_owned());
        }
    }

    pub fn add_model(&mut self) {
        let model = self.new_model.text.trim().to_owned();
        let models = &mut self.draft.models;
        if !model.is_empty() && !models.added.contains(&model) {
            models.added.push(model);
        }
        self.new_model.clear();
    }
}

#[derive(Default)]
pub(crate) struct ProviderHost {
    pub snapshot: Option<Snapshot>,
    pub live: BTreeMap<String, Live>,
    pub sheet: Option<Sheet>,
    /// Whether the add list shows every vendor.
    pub more: bool,
    /// Whether the vendor search's field shows in place of its button.
    pub searching: bool,
    /// The vendor search's query.
    pub search: TextEdit,
    pub error: Option<String>,
    pub sign_in: Option<SignIn>,
    sample: bool,
    reload: bool,
    refresh_all: bool,
    refresh: BTreeSet<String>,
    queue: VecDeque<Write>,
    write: Option<Task<Written>>,
    jobs: Vec<Job>,
}

impl ProviderHost {
    pub fn working(&self, id: &str, work: Work) -> bool {
        self.jobs.iter().any(|job| job.id == id && job.work == work)
    }

    pub fn writing(&self) -> bool {
        self.write.is_some() || !self.queue.is_empty()
    }

    /// The page or the Assistant opened: read settings again and ask the
    /// network for fresh lists and quota.
    pub fn page_opened(&mut self) {
        if !self.sample {
            self.reload = true;
            self.refresh_all = true;
        }
    }

    /// A workspace opened: providers are read from settings from now on.
    pub(super) fn open_live(&mut self) {
        if self.sample {
            *self = Self::default();
        }
        self.reload = true;
    }

    fn spawn(
        &mut self,
        config: &ProcessingConfig,
        id: &str,
        work: Work,
        model: Option<ModelChoice>,
    ) {
        if self.working(id, work) {
            return;
        }
        let cancellation = LlmCancellation::new();
        let (config, provider, token) = (config.clone(), id.to_owned(), cancellation.clone());
        let task: Task<Fetched> = Box::pin(runtime::spawn_blocking(move || {
            let service = config.providers()?;
            Ok(match (work, model) {
                (Work::Models, _) => Fetched::Models(service.models(&provider, &token)),
                (Work::Quota, _) => Fetched::Quota(service.quota(&provider, &token)),
                (Work::Test, Some(choice)) => {
                    let result = service.test(&choice, &token);
                    Fetched::Test(choice.model, result)
                }
                (Work::Test, None) => Fetched::Test(
                    String::new(),
                    Err(AssistantUiError::unavailable("no model to test")),
                ),
            })
        }));
        self.jobs.push(Job {
            id: id.to_owned(),
            work,
            cancellation,
            task,
        });
    }

    /// Lists and quota for the providers due a refresh, once nothing is
    /// being written.
    fn start_refresh(&mut self, config: &ProcessingConfig) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let mut ids = std::mem::take(&mut self.refresh);
        if std::mem::take(&mut self.refresh_all) {
            ids.extend(
                snapshot
                    .views
                    .iter()
                    .filter(|view| view.enabled)
                    .map(|view| view.id.clone()),
            );
        }
        ids.retain(|id| snapshot.view(id).is_some_and(|view| view.enabled));
        for id in ids {
            self.spawn(config, &id, Work::Models, None);
            self.spawn(config, &id, Work::Quota, None);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn snapshot(service: &ProviderService) -> Result<(Snapshot, AssistantSettings), AssistantUiError> {
    let (revision, views) = service.views()?;
    let assistant = service.settings().snapshot()?.assistant;
    Ok((
        Snapshot {
            revision,
            views,
            model: assistant.model.clone(),
        },
        assistant,
    ))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// `protocol`'s place in [`PROTOCOLS`].
pub(crate) fn slot(protocol: LlmProtocol) -> usize {
    PROTOCOLS
        .iter()
        .position(|candidate| *candidate == protocol)
        .unwrap_or_default()
}

/// A sample provider as `settings` describe it: its region's APIs with
/// the saved overrides, or a custom provider's own.
fn sample_view(settings: ProviderSettings) -> ProviderView {
    let vendor = catalog::vendor(&settings.id);
    let region = vendor.map(|vendor| {
        settings
            .region
            .as_deref()
            .and_then(|label| vendor.region(label))
            .unwrap_or(vendor.default_region())
    });
    let mut endpoints = region.map_or_else(Endpoints::default, |region| region.endpoints.owned());
    for protocol in PROTOCOLS {
        let served = settings.custom.is_some() || endpoints.get(protocol).is_some();
        if let Some(url) = settings
            .endpoints
            .get(settings_protocol(protocol))
            .filter(|_| served)
        {
            endpoints.set(protocol, Some(url.to_owned()));
        }
    }
    ProviderView {
        id: settings.id.clone(),
        vendor,
        name: match (&settings.custom, vendor) {
            (Some(custom), _) => custom.name.clone(),
            (None, Some(vendor)) => vendor.name.into(),
            (None, None) => settings.id.clone(),
        },
        configured: false,
        enabled: false,
        region: region.map(|region| region.label),
        endpoints,
        prefer: vendor.map_or_else(Default::default, |vendor| vendor.prefer),
        credential: CredentialSummary::None,
        settings,
    }
}

impl ProviderHost {
    /// The sample's providers: OpenAI signed in, DeepSeek with a key and
    /// the magpie gateway on this machine, with lists and quota as if just
    /// fetched.
    pub fn sample() -> Self {
        use LlmProtocol::{
            AnthropicMessages as Messages, OpenAiChatCompletions as Chat,
            OpenAiResponses as Responses,
        };
        let now = now();
        let listed = |models: &[(&str, &[LlmProtocol])]| {
            models
                .iter()
                .map(|(model, protocols)| ListedModel {
                    id: (*model).to_owned(),
                    protocols: protocols.to_vec(),
                })
                .collect()
        };
        let on = |protocols: &'static [LlmProtocol], models: &[&'static str]| {
            listed(
                &models
                    .iter()
                    .map(|model| (*model, protocols))
                    .collect::<Vec<_>>(),
            )
        };
        let mut views = catalog::catalog()
            .iter()
            .map(|vendor| sample_view(ProviderSettings::new(vendor.id)))
            .collect::<Vec<_>>();
        for view in &mut views {
            let credential = match view.id.as_str() {
                "openai" => {
                    view.settings.models.mode = ModelListMode::LatestPerFamily;
                    view.endpoints = Endpoints {
                        responses: Some(ChatGptEndpoints::default().codex_base_url()),
                        ..Endpoints::default()
                    };
                    CredentialSummary::Account {
                        email: "morgan@example.com".into(),
                        plan: Some("plus".into()),
                        subscription_until: Some(now + 23 * 86_400),
                    }
                }
                "deepseek" => CredentialSummary::ApiKey {
                    last4: "9f3a".into(),
                },
                "magpie" => CredentialSummary::None,
                _ => continue,
            };
            view.configured = true;
            view.enabled = true;
            view.credential = credential;
        }
        let mut live = BTreeMap::new();
        live.insert(
            "openai".to_owned(),
            Live {
                models: Some(on(
                    &[Responses],
                    &[
                        "gpt-5.1",
                        "gpt-5",
                        "gpt-5-mini",
                        "gpt-4.1",
                        "gpt-4o",
                        "o3",
                        "o4-mini",
                    ],
                )),
                quota: Some(Quota {
                    plan: Some("plus".into()),
                    windows: vec![
                        QuotaWindow {
                            span_seconds: Some(5 * 3_600),
                            used_percent: 18.0,
                            resets_at: Some(now + 2 * 3_600),
                        },
                        QuotaWindow {
                            span_seconds: Some(7 * 86_400),
                            used_percent: 41.0,
                            resets_at: Some(now + 3 * 86_400),
                        },
                    ],
                    reset_cards: Some(2),
                    subscription_until: Some(now + 23 * 86_400),
                    balance: None,
                }),
                ..Live::default()
            },
        );
        live.insert(
            "deepseek".to_owned(),
            Live {
                models: Some(on(
                    &[Chat, Responses, Messages],
                    &["deepseek-chat", "deepseek-reasoner"],
                )),
                quota: Some(Quota {
                    balance: Some("¥110.00".into()),
                    ..Quota::default()
                }),
                ..Live::default()
            },
        );
        live.insert(
            "magpie".to_owned(),
            Live {
                models: Some(listed(&[
                    ("claude-sonnet-4-5", &[Chat, Messages]),
                    ("gpt-5.1", &[Chat, Responses]),
                    ("kimi-k2", &[Chat]),
                    ("glm-4.6", &[Chat, Messages]),
                ])),
                ..Live::default()
            },
        );
        Self {
            snapshot: Some(Snapshot {
                revision: "sample".into(),
                views,
                model: Some(ModelChoice {
                    provider: "openai".into(),
                    model: "gpt-5.1".into(),
                    protocol: Some(settings::LlmProtocol::OpenAiResponses),
                }),
            }),
            live,
            sample: true,
            ..Self::default()
        }
    }
}

impl MeetNote {
    pub(crate) fn provider_write(&mut self, write: Write, tx: &mut Tx) {
        let host = &mut self.workspace_host.providers;
        if host.sample {
            if let Some(snapshot) = &mut host.snapshot {
                let closes = matches!(&write, Write::Save { .. } | Write::Remove(_));
                write.apply_sample(snapshot);
                if closes {
                    host.sheet = None;
                }
            }
            return;
        }
        host.queue.push_back(write);
        tx.wake_after(Duration::ZERO);
    }

    /// Asks the network again; the sample has no network.
    pub(crate) fn provider_work(&mut self, id: &str, work: Work, tx: &mut Tx) {
        if self.sample_mode {
            self.notify(self.tr("provider_sample_offline"), tx);
            return;
        }
        let host = &mut self.workspace_host.providers;
        let model = (work == Work::Test).then(|| host.test_model(id)).flatten();
        if work == Work::Test && model.is_none() {
            return;
        }
        if let Some(live) = host.live.get_mut(id) {
            if work == Work::Test {
                live.test = None;
            }
        }
        host.spawn(&self.workspace_host.processing_config, id, work, model);
        tx.wake_after(Duration::ZERO);
    }

    pub(crate) fn open_provider_sheet(&mut self, id: &str) {
        let host = &mut self.workspace_host.providers;
        if let Some(snapshot) = &host.snapshot
            && let Some(view) = snapshot.view(id)
        {
            host.sheet = Some(Sheet::open(&view.settings, &snapshot.revision));
            if view.enabled && !host.sample {
                host.refresh.insert(id.to_owned());
            }
        }
    }

    /// A new custom endpoint, named `custom-N` after the first free number.
    pub(crate) fn open_custom_sheet(&mut self) {
        let host = &mut self.workspace_host.providers;
        let Some(snapshot) = &host.snapshot else {
            return;
        };
        let id = (1..)
            .map(|index| format!("custom-{index}"))
            .find(|id| snapshot.view(id).is_none())
            .unwrap_or_default();
        let settings = ProviderSettings {
            custom: Some(CustomEndpoint {
                name: String::new(),
                auth_mode: AssistantAuthMode::Bearer,
            }),
            ..ProviderSettings::new(id)
        };
        host.sheet = Some(Sheet::open(&settings, &snapshot.revision));
    }

    pub(crate) fn close_provider_sheet(&mut self) {
        let host = &mut self.workspace_host.providers;
        if let Some(sheet) = host.sheet.take()
            && host
                .sign_in
                .as_ref()
                .is_some_and(|sign_in| sign_in.id == sheet.id)
        {
            host.sign_in = None;
        }
    }

    pub(crate) fn save_provider_sheet(&mut self, tx: &mut Tx) {
        let Some(sheet) = &mut self.workspace_host.providers.sheet else {
            return;
        };
        sheet.invalid = None;
        match sheet.settings() {
            Ok(settings) => {
                sheet.error = None;
                let write = Write::Save {
                    revision: sheet.revision.clone(),
                    settings,
                    credential: sheet.credential(),
                };
                self.provider_write(write, tx);
            }
            Err(key) => sheet.invalid = Some((key, sheet.hinted(key))),
        }
    }

    /// Opens the vendor search in place of its button, focused.
    pub(crate) fn open_provider_search(&mut self, tx: &mut Tx) {
        let host = &mut self.workspace_host.providers;
        host.searching = true;
        host.search = TextEdit::new("");
        tx.focus(SEARCH, tx.focus_visible());
    }

    /// Clears and closes the vendor search; focus returns to its button.
    pub(crate) fn close_provider_search(&mut self, tx: &mut Tx) {
        let host = &mut self.workspace_host.providers;
        host.searching = false;
        host.search.clear();
        tx.focus(SEARCH_BUTTON, tx.focus_visible());
    }

    /// Escape in the vendor search closes it.
    pub(crate) fn provider_search_key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        if event.key != Key::Escape {
            return false;
        }
        self.close_provider_search(tx);
        true
    }

    pub(crate) fn set_assistant_model(&mut self, choice: ModelChoice, tx: &mut Tx) {
        self.provider_write(Write::SetModel(Some(choice)), tx);
    }

    pub(crate) fn start_sign_in(&mut self, id: &str, tx: &mut Tx) {
        if self.sample_mode {
            self.notify(self.tr("provider_sample_offline"), tx);
            return;
        }
        let (url_tx, url_rx) = oneshot::channel();
        let cancellation = LlmCancellation::new();
        let (config, provider, token) = (
            self.workspace_host.processing_config.clone(),
            id.to_owned(),
            cancellation.clone(),
        );
        let task: Task<Result<CredentialSummary, AssistantUiError>> =
            Box::pin(runtime::spawn_blocking(move || {
                let result = config
                    .providers()
                    .map_err(AssistantUiError::unavailable)
                    .and_then(|service| {
                        let pending = service.begin_sign_in(&provider)?;
                        let _ = url_tx.send(pending.url.clone());
                        pending.finish(&token)
                    });
                Ok(result)
            }));
        self.workspace_host.providers.sign_in = Some(SignIn {
            id: id.to_owned(),
            url: None,
            url_rx: Some(url_rx),
            cancellation,
            task,
        });
        if let Some(sheet) = &mut self.workspace_host.providers.sheet {
            sheet.error = None;
        }
        tx.wake_after(Duration::ZERO);
    }

    /// Stops the browser wait; its callback listener closes with it.
    pub(crate) fn cancel_sign_in(&mut self) {
        self.workspace_host.providers.sign_in = None;
    }

    pub(crate) fn copy_sign_in_link(&mut self, tx: &mut Tx) {
        if let Some(url) = self
            .workspace_host
            .providers
            .sign_in
            .as_ref()
            .and_then(|sign_in| sign_in.url.clone())
        {
            tx.copy(url);
            self.notify(self.tr("provider_link_copied"), tx);
        }
    }

    pub(super) fn poll_providers(&mut self, tx: &mut Tx) -> bool {
        let WorkspaceHost {
            providers: host,
            waker,
            processing_config: config,
            ..
        } = &mut self.workspace_host;
        let waker = waker.as_ref();
        let mut changed = false;
        if host.reload && !host.sample {
            host.reload = false;
            if !host.queue.iter().any(|write| matches!(write, Write::Load)) {
                host.queue.push_back(Write::Load);
            }
        }
        let written = host
            .write
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, waker, tx));
        if written.is_some() {
            host.write = None;
        }
        if host.write.is_none()
            && let Some(write) = host.queue.pop_front()
        {
            let config = config.clone();
            host.write = Some(Box::pin(runtime::spawn_blocking(move || {
                let service = config.providers().map_err(AssistantUiError::unavailable);
                let result = service
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|service| write.run(service));
                let snapshot = service.and_then(|service| snapshot(&service));
                Ok(Written {
                    write,
                    result,
                    snapshot,
                })
            })));
            tx.wake_after(Duration::ZERO);
        }
        let mut fetched = Vec::new();
        host.jobs
            .retain_mut(|job| match WorkspaceHost::poll(&mut job.task, waker, tx) {
                Some(result) => {
                    fetched.push((job.id.clone(), result));
                    false
                }
                None => true,
            });
        let signed_in = host.sign_in.as_mut().and_then(|sign_in| {
            if let Some(rx) = &mut sign_in.url_rx {
                let mut context =
                    Context::from_waker(waker.unwrap_or_else(|| std::task::Waker::noop()));
                match Pin::new(rx).poll(&mut context) {
                    Poll::Ready(Ok(url)) => {
                        let _ = tx.open_url(&url);
                        sign_in.url = Some(url);
                        sign_in.url_rx = None;
                        changed = true;
                    }
                    Poll::Ready(Err(_)) => sign_in.url_rx = None,
                    Poll::Pending => {}
                }
            }
            WorkspaceHost::poll(&mut sign_in.task, waker, tx)
                .map(|result| (sign_in.id.clone(), result))
        });
        if signed_in.is_some() {
            host.sign_in = None;
        }
        changed |= written.is_some() || !fetched.is_empty() || signed_in.is_some();
        if let Some(written) = written {
            self.finish_provider_write(written, tx);
        }
        for (id, result) in fetched {
            self.finish_provider_work(&id, result, tx);
        }
        if let Some((id, result)) = signed_in {
            self.finish_sign_in(
                &id,
                result.and_then(|result| result.map_err(|error| error.message)),
                tx,
            );
        }
        let WorkspaceHost {
            providers: host,
            processing_config: config,
            ..
        } = &mut self.workspace_host;
        if !host.writing() && (host.refresh_all || !host.refresh.is_empty()) {
            host.start_refresh(config);
            changed = true;
        }
        changed
    }

    fn finish_provider_write(&mut self, written: Result<Written, String>, tx: &mut Tx) {
        let Written {
            write,
            result,
            snapshot,
        } = match written {
            Ok(written) => written,
            Err(error) => {
                self.notify(error, tx);
                return;
            }
        };
        let host = &mut self.workspace_host.providers;
        match snapshot {
            Ok((snapshot, assistant)) => {
                if result.is_ok()
                    && !matches!(write, Write::Save { .. })
                    && let Some(sheet) = &mut host.sheet
                {
                    sheet.revision = snapshot.revision.clone();
                    if let Some(view) = snapshot.view(&sheet.id) {
                        sheet.draft.enabled = view.settings.enabled;
                    }
                }
                host.snapshot = Some(snapshot);
                host.error = None;
                self.workspace_host.assistant.settings = assistant;
            }
            Err(error) => host.error = Some(error.message),
        }
        let locale = self.locale();
        let host = &mut self.workspace_host.providers;
        let notice = match (write, result) {
            (Write::Save { settings, .. }, Ok(())) => {
                if host
                    .sheet
                    .as_ref()
                    .is_some_and(|sheet| sheet.id == settings.id)
                {
                    host.sheet = None;
                }
                host.refresh.insert(settings.id);
                Some(self.tr("provider_saved").to_owned())
            }
            (Write::Save { settings, .. }, Err(error)) => {
                let conflict = error.is_settings_conflict();
                match &mut host.sheet {
                    Some(sheet) if sheet.id == settings.id => {
                        if conflict && let Some(snapshot) = &host.snapshot {
                            let reloaded = snapshot
                                .view(&settings.id)
                                .map_or(settings.clone(), |view| view.settings.clone());
                            *sheet = Sheet::open(&reloaded, &snapshot.revision);
                        }
                        sheet.error = Some(if conflict {
                            ui_text::text(locale, "settings_conflict").to_owned()
                        } else {
                            error.message
                        });
                        None
                    }
                    _ => Some(error.message),
                }
            }
            (Write::Toggle { id, enabled }, Ok(())) => {
                if enabled {
                    host.refresh.insert(id);
                }
                None
            }
            (Write::Remove(id), Ok(())) => {
                if host.sheet.as_ref().is_some_and(|sheet| sheet.id == id) {
                    host.sheet = None;
                }
                host.live.remove(&id);
                Some(self.tr("provider_removed").to_owned())
            }
            (Write::SignOut(id), Ok(())) => {
                host.live.remove(&id);
                None
            }
            (Write::ImportCodex, Ok(())) => {
                host.refresh.insert("openai".into());
                Some(self.tr("provider_signed_in").to_owned())
            }
            (Write::ImportCodex | Write::SignOut(_), Err(error)) => match &mut host.sheet {
                Some(sheet) => {
                    sheet.error = Some(error.message);
                    None
                }
                None => Some(error.message),
            },
            (Write::Load | Write::SetModel(_), Ok(())) => None,
            (_, Err(error)) => Some(ui_text::message(
                self.locale(),
                "provider_failed",
                &error.message,
            )),
        };
        if let Some(notice) = notice {
            self.notify(notice, tx);
        }
    }

    fn finish_provider_work(&mut self, id: &str, fetched: Result<Fetched, String>, tx: &mut Tx) {
        let (now, locale) = (self.now, self.locale());
        let host = &mut self.workspace_host.providers;
        let live = host.live.entry(id.to_owned()).or_default();
        let message = |error: AssistantUiError| (!error.is_cancelled()).then_some(error.message);
        match fetched {
            Ok(Fetched::Models(Ok(models))) => {
                live.models_error = None;
                live.fetched_at = now;
                let upgrade = host.snapshot.as_ref().and_then(|snapshot| {
                    let choice = snapshot
                        .model
                        .as_ref()
                        .filter(|choice| choice.provider == id)?;
                    upgraded_choice(snapshot.view(id)?, &models, choice)
                });
                live.models = Some(models);
                if let Some(choice) = upgrade {
                    self.provider_write(Write::SetModel(Some(choice)), tx);
                }
            }
            Ok(Fetched::Models(Err(error))) => live.models_error = message(error),
            Ok(Fetched::Quota(Ok(quota))) => {
                live.quota = quota;
                live.quota_error = None;
            }
            Ok(Fetched::Quota(Err(error))) => live.quota_error = message(error),
            Ok(Fetched::Test(model, result)) => {
                live.test = Some((
                    model,
                    match result {
                        Ok(probe) if probe.ready => Ok(()),
                        Ok(_) => Err(ui_text::text(locale, "provider_test_not_ready").to_owned()),
                        Err(error) => Err(error.message),
                    },
                ));
            }
            Err(error) => live.models_error = Some(error),
        }
    }

    fn finish_sign_in(&mut self, id: &str, result: Result<CredentialSummary, String>, tx: &mut Tx) {
        match result {
            Ok(_) => {
                let host = &mut self.workspace_host.providers;
                host.refresh.insert(id.to_owned());
                self.provider_write(Write::Load, tx);
                self.notify(self.tr("provider_signed_in"), tx);
            }
            Err(message) => {
                let message = ui_text::message(self.locale(), "provider_sign_in_failed", &message);
                match &mut self.workspace_host.providers.sheet {
                    Some(sheet) if sheet.id == id => sheet.error = Some(message),
                    _ => self.notify(message, tx),
                }
            }
        }
    }
}

impl ProviderHost {
    /// The model "测试模型" tries: the Assistant's when it is from `id`,
    /// else the first the provider offers.
    pub fn test_model(&self, id: &str) -> Option<ModelChoice> {
        let snapshot = self.snapshot.as_ref()?;
        if let Some(choice) = snapshot
            .model
            .as_ref()
            .filter(|choice| choice.provider == id)
        {
            return Some(choice.clone());
        }
        let view = snapshot.view(id)?;
        let live = self
            .live
            .get(id)
            .and_then(|live| live.models.as_deref())
            .unwrap_or_default();
        application::visible_models(&view.settings.models, &model_ids(live))
            .first()
            .map(|model| model_choice(view, live, model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };

    fn custom() -> Sheet {
        Sheet::open(
            &ProviderSettings {
                custom: Some(CustomEndpoint {
                    name: String::new(),
                    auth_mode: AssistantAuthMode::XApiKey,
                }),
                ..ProviderSettings::new("custom-1")
            },
            "r1",
        )
    }

    #[test]
    fn a_custom_endpoint_needs_a_name_then_at_least_one_url() {
        let mut sheet = custom();
        assert_eq!(sheet.protocols(), PROTOCOLS);
        assert_eq!(sheet.settings(), Err("provider_name_required"));
        sheet.name = TextEdit::new("Gateway");
        assert_eq!(sheet.settings(), Err("provider_url_required"));
        *sheet.url_mut(Messages) = TextEdit::new("  https://gw.example/v1 ");
        let settings = sheet.settings().unwrap();
        assert_eq!(
            settings.endpoints,
            EndpointUrls {
                messages: Some("https://gw.example/v1".into()),
                ..EndpointUrls::default()
            }
        );
        assert_eq!(
            settings.custom.map(|custom| custom.auth_mode),
            Some(AssistantAuthMode::XApiKey)
        );
        assert_eq!(sheet.endpoints().protocols(), [Messages]);
    }

    #[test]
    fn a_hint_lasts_until_a_field_it_is_about_is_edited() {
        let mut sheet = custom();
        sheet.name = TextEdit::new("Gateway");
        *sheet.url_mut(Chat) = TextEdit::new("https://gw.example/v1");
        sheet.timeout = TextEdit::new("soon");
        let key = sheet.settings().unwrap_err();
        sheet.invalid = Some((key, sheet.hinted(key)));
        assert_eq!(sheet.invalid(), Some("provider_invalid_number"));
        sheet.name.insert(" 2");
        *sheet.url_mut(Messages) = TextEdit::new("x");
        assert_eq!(sheet.invalid(), Some("provider_invalid_number"));
        sheet.max_output.insert("0");
        assert_eq!(sheet.invalid(), None);

        assert_eq!(sheet.hinted("provider_name_required"), ["Gateway 2"]);
        assert_eq!(sheet.hinted("provider_url_required").len(), 3);
        assert!(sheet.hinted("provider_sheet_error").is_empty());
    }

    #[test]
    fn endpoint_rows_follow_the_region_and_the_overrides_it_serves() {
        let mut sheet = Sheet::open(&ProviderSettings::new("moonshot"), "r1");
        assert_eq!(sheet.protocols(), [Chat, Messages]);
        assert_eq!(
            sheet.endpoints().iter().collect::<Vec<_>>(),
            [
                (Chat, "https://api.moonshot.ai/v1"),
                (Messages, "https://api.moonshot.ai/anthropic/v1"),
            ]
        );
        sheet.draft.region = Some("China".into());
        *sheet.url_mut(Chat) = TextEdit::new("http://127.0.0.1:9/v1");
        // Kimi serves no Responses: an address typed for it is not kept
        *sheet.url_mut(Responses) = TextEdit::new("https://elsewhere.example/v1");
        assert_eq!(
            sheet.endpoints().iter().collect::<Vec<_>>(),
            [
                (Chat, "http://127.0.0.1:9/v1"),
                (Messages, "https://api.moonshot.cn/anthropic/v1"),
            ]
        );
        let settings = sheet.settings().unwrap();
        assert_eq!(settings.region.as_deref(), Some("China"));
        assert_eq!(
            settings.endpoints,
            EndpointUrls {
                chat: Some("http://127.0.0.1:9/v1".into()),
                ..EndpointUrls::default()
            }
        );
    }

    #[test]
    fn a_saved_override_opens_in_its_apis_field() {
        let mut settings = ProviderSettings::new("deepseek");
        settings.endpoints.responses = Some("http://127.0.0.1:9/v1".into());
        let sheet = Sheet::open(&settings, "r1");
        assert_eq!(sheet.url(Responses), "http://127.0.0.1:9/v1");
        assert_eq!(sheet.url(Chat), "");
        assert_eq!(sheet.settings().unwrap().endpoints, settings.endpoints);
    }
}
