use contracts::{AgentWriteMode, ComputeDevice, DiarizationStageConfiguration, ModelRef};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use url::Url;

mod credentials;

pub use credentials::{ChatGptAccount, CredentialStore, ProviderCredential};

const SETTINGS_SCHEMA_VERSION: &str = "meetnote.settings.v2";
const SETTINGS_SCHEMA_V1: &str = "meetnote.settings.v1";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn default_ui_locale() -> String {
    "zh-CN".into()
}

fn default_speaker_library_path() -> String {
    "data/speakers/library.json".into()
}

fn default_documents_root() -> String {
    "data/documents".into()
}

fn default_max_tool_rounds() -> u32 {
    8
}

fn default_true() -> bool {
    true
}

fn default_max_validation_retries() -> u32 {
    3
}

fn default_timeout_ms() -> u64 {
    120_000
}

fn default_max_output_tokens() -> u32 {
    4_096
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LlmProtocol {
    #[serde(rename = "openaiChatCompletions")]
    OpenAiChatCompletions,
    #[serde(rename = "anthropicMessages")]
    AnthropicMessages,
    #[serde(rename = "openaiResponses")]
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssistantAuthMode {
    Bearer,
    XApiKey,
}

/// The provider and model the built-in Assistant runs on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelChoice {
    pub provider: String,
    pub model: String,
    /// The API the model was resolved to when it was picked from its
    /// provider's list; `None` leaves the choice to the native rule.
    #[serde(default)]
    pub protocol: Option<LlmProtocol>,
}

/// Which of a provider's models the model picker offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelListMode {
    /// Whatever the vendor lists.
    #[default]
    Vendor,
    /// Only the models picked by hand.
    Picked,
    /// The newest model of each family the vendor lists.
    LatestPerFamily,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    #[serde(default)]
    pub mode: ModelListMode,
    /// The models `Picked` offers.
    #[serde(default)]
    pub picked: Vec<String>,
    /// Model ids typed in by hand, offered in every mode.
    #[serde(default)]
    pub added: Vec<String>,
}

/// A custom provider's name and the header its key goes in; its APIs are
/// its `endpoints`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomEndpoint {
    pub name: String,
    pub auth_mode: AssistantAuthMode,
}

/// A base URL per API, version included (`…/v1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointUrls {
    #[serde(default)]
    pub chat: Option<String>,
    #[serde(default)]
    pub responses: Option<String>,
    #[serde(default)]
    pub messages: Option<String>,
}

impl EndpointUrls {
    pub fn get(&self, protocol: LlmProtocol) -> Option<&str> {
        match protocol {
            LlmProtocol::OpenAiChatCompletions => self.chat.as_deref(),
            LlmProtocol::OpenAiResponses => self.responses.as_deref(),
            LlmProtocol::AnthropicMessages => self.messages.as_deref(),
        }
    }

    pub fn set(&mut self, protocol: LlmProtocol, url: Option<String>) {
        *match protocol {
            LlmProtocol::OpenAiChatCompletions => &mut self.chat,
            LlmProtocol::OpenAiResponses => &mut self.responses,
            LlmProtocol::AnthropicMessages => &mut self.messages,
        } = url;
    }

    fn named(&self) -> [(&'static str, Option<&str>); 3] {
        [
            ("chat", self.chat.as_deref()),
            ("responses", self.responses.as_deref()),
            ("messages", self.messages.as_deref()),
        ]
    }

    pub fn is_empty(&self) -> bool {
        self.named().iter().all(|(_, url)| url.is_none())
    }
}

/// A configured connection: a catalog vendor (`id` is the vendor's id) or a
/// custom endpoint (`id` starts with `custom-`). Credentials live in
/// `credentials.json`, never here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettings {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// A catalog vendor's region label; its first region when `None`.
    #[serde(default)]
    pub region: Option<String>,
    /// A vendor's per-API overrides of its region; a custom provider's
    /// APIs, at least one.
    #[serde(default)]
    pub endpoints: EndpointUrls,
    #[serde(default)]
    pub custom: Option<CustomEndpoint>,
    #[serde(default)]
    pub models: ModelSelection,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
}

impl ProviderSettings {
    /// An enabled provider with every default.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            region: None,
            endpoints: EndpointUrls::default(),
            custom: None,
            models: ModelSelection::default(),
            context_window_tokens: None,
            timeout_ms: default_timeout_ms(),
            max_output_tokens: default_max_output_tokens(),
        }
    }

    pub fn is_custom(&self) -> bool {
        self.id.starts_with("custom-")
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        let invalid = |message: &str| {
            Err(SettingsError::Invalid(format!(
                "provider {}: {message}",
                self.id
            )))
        };
        if self.id.trim().is_empty() || self.id.chars().any(char::is_whitespace) {
            return invalid("id must be non-empty without whitespace");
        }
        match (&self.custom, self.is_custom()) {
            (Some(_), false) => {
                return invalid("only custom- providers describe a custom endpoint")
            }
            (None, true) => return invalid("a custom provider needs its endpoint"),
            (Some(custom), true) if custom.name.trim().is_empty() => {
                return invalid("a custom provider needs a name");
            }
            (Some(_), true) if self.endpoints.is_empty() => {
                return invalid("a custom provider needs an endpoint URL");
            }
            (Some(_), true) if self.region.is_some() => {
                return invalid("only catalog vendors have a region");
            }
            _ => {}
        }
        if self
            .region
            .as_deref()
            .is_some_and(|region| region.trim().is_empty())
        {
            return invalid("region must not be empty");
        }
        for (name, url) in self.endpoints.named() {
            if let Some(Err(message)) = url.map(validate_base_url) {
                return invalid(&format!("endpoints.{name} {message}"));
            }
        }
        if self.timeout_ms == 0 {
            return invalid("timeoutMs must be greater than zero");
        }
        if self.max_output_tokens == 0 {
            return invalid("maxOutputTokens must be greater than zero");
        }
        if self
            .context_window_tokens
            .is_some_and(|limit| limit <= self.max_output_tokens)
        {
            return invalid("contextWindowTokens must exceed maxOutputTokens");
        }
        if self
            .models
            .picked
            .iter()
            .chain(&self.models.added)
            .any(|model| model.trim().is_empty())
        {
            return invalid("model ids must not be empty");
        }
        Ok(())
    }
}

/// HTTPS unless the host is this machine, without credentials, query or
/// fragment.
fn validate_base_url(base_url: &str) -> Result<(), String> {
    let url = Url::parse(base_url).map_err(|error| format!("is invalid: {error}"))?;
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("cannot contain credentials, query, or fragment".into());
    }
    let host = url.host_str().ok_or("must contain a host")?;
    if url.scheme() != "https" && !(url.scheme() == "http" && is_loopback_host(host)) {
        return Err("must use HTTPS unless it targets localhost".into());
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub write_mode: AgentWriteMode,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_max_validation_retries")]
    pub max_validation_retries: u32,
    #[serde(default)]
    pub model: Option<ModelChoice>,
    #[serde(default)]
    pub providers: Vec<ProviderSettings>,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            write_mode: AgentWriteMode::Automatic,
            max_tool_rounds: default_max_tool_rounds(),
            max_validation_retries: default_max_validation_retries(),
            model: None,
            providers: Vec::new(),
        }
    }
}

impl AssistantSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.max_tool_rounds == 0 {
            return Err(SettingsError::Invalid(
                "assistant maxToolRounds must be greater than zero".into(),
            ));
        }
        if self.max_validation_retries > self.max_tool_rounds {
            return Err(SettingsError::Invalid(
                "assistant maxValidationRetries cannot exceed maxToolRounds".into(),
            ));
        }
        for (index, provider) in self.providers.iter().enumerate() {
            provider.validate()?;
            if self.providers[..index]
                .iter()
                .any(|other| other.id == provider.id)
            {
                return Err(SettingsError::Invalid(format!(
                    "provider id is duplicated: {}",
                    provider.id
                )));
            }
        }
        if let Some(choice) = &self.model {
            if choice.model.trim().is_empty() {
                return Err(SettingsError::Invalid(
                    "assistant model must not be empty".into(),
                ));
            }
            if self.provider(&choice.provider).is_none() {
                return Err(SettingsError::Invalid(format!(
                    "assistant model names a missing provider: {}",
                    choice.provider
                )));
            }
        }
        Ok(())
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderSettings> {
        self.providers.iter().find(|provider| provider.id == id)
    }
}

/// Light or dark: follow the system appearance or pin one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// The window chrome material. Glass is used only while the system does not
/// ask to reduce transparency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaterialPreference {
    #[default]
    Glass,
    Opaque,
}

/// Desktop appearance. It belongs to the desktop UI alone and is not part of
/// the Settings capability projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub material: MaterialPreference,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: String,
    pub asr_backend_id: Option<String>,
    pub asr_model: Option<ModelRef>,
    pub diarization_backend_id: Option<String>,
    pub diarization_model: Option<ModelRef>,
    #[serde(default = "default_ui_locale")]
    pub ui_locale: String,
    pub compute_device: ComputeDevice,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    #[serde(default = "default_speaker_library_path")]
    pub speaker_library_path: String,
    /// The directory holding the document packages. A desktop Workspace
    /// setting: it is not part of the Settings capability projection, and the
    /// desktop saves it only after the workspace switched to it.
    #[serde(default = "default_documents_root")]
    pub documents_root: String,
    #[serde(default)]
    pub diarization_stages: DiarizationStageConfiguration,
    #[serde(default)]
    pub assistant: AssistantSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION.into(),
            asr_backend_id: Some("whisper-rs".into()),
            asr_model: None,
            diarization_backend_id: Some("diarization-native".into()),
            diarization_model: None,
            ui_locale: "zh-CN".into(),
            compute_device: ComputeDevice::Auto,
            language: None,
            expected_speakers: None,
            speaker_library_path: default_speaker_library_path(),
            documents_root: default_documents_root(),
            diarization_stages: DiarizationStageConfiguration::default(),
            assistant: AssistantSettings::default(),
            appearance: AppearanceSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::Invalid(format!(
                "unsupported settings schema version: {}",
                self.schema_version
            )));
        }
        if self.asr_backend_id.as_deref() == Some("")
            || self.diarization_backend_id.as_deref() == Some("")
        {
            return Err(SettingsError::Invalid(
                "backend ids must be non-empty when configured".into(),
            ));
        }
        if let (Some(backend_id), Some(model)) = (&self.asr_backend_id, &self.asr_model) {
            if backend_id != &model.backend_id {
                return Err(SettingsError::Invalid(
                    "ASR backend id does not match ASR model backend id".into(),
                ));
            }
        }
        if let (Some(backend_id), Some(model)) =
            (&self.diarization_backend_id, &self.diarization_model)
        {
            if backend_id != &model.backend_id {
                return Err(SettingsError::Invalid(
                    "diarization backend id does not match diarization model backend id".into(),
                ));
            }
        }
        if self.expected_speakers == Some(0) {
            return Err(SettingsError::Invalid(
                "expected speakers must be greater than zero".into(),
            ));
        }
        if self.ui_locale.trim().is_empty() {
            return Err(SettingsError::Invalid("UI locale must not be empty".into()));
        }
        if self.speaker_library_path.trim().is_empty() {
            return Err(SettingsError::Invalid(
                "speaker library path must not be empty".into(),
            ));
        }
        if self.documents_root.trim().is_empty() {
            return Err(SettingsError::Invalid(
                "documents root must not be empty".into(),
            ));
        }
        if let Some(model) = &self.diarization_stages.segmentation_model {
            if model.backend_id != "diarization-native" {
                return Err(SettingsError::Invalid(
                    "segmentation stage model must use diarization-native".into(),
                ));
            }
        }
        if let Some(model) = &self.diarization_stages.embedding_model {
            if model.backend_id != "diarization-native" {
                return Err(SettingsError::Invalid(
                    "embedding stage model must use diarization-native".into(),
                ));
            }
        }
        self.assistant.validate()?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("failed to read settings {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write settings {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse settings: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("settings revision changed before update")]
    RevisionConflict,
    #[error("invalid settings: {0}")]
    Invalid(String),
    #[error("settings lock poisoned")]
    LockPoisoned,
}

pub trait SettingsRepository: Send + Sync {
    fn load(&self) -> Result<AppSettings, SettingsError>;
    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError>;

    fn save_if_equal(
        &self,
        expected: &AppSettings,
        settings: &AppSettings,
    ) -> Result<(), SettingsError> {
        let current = self.load()?;
        if &current != expected {
            return Err(SettingsError::RevisionConflict);
        }
        self.save(settings)
    }
}

#[derive(Debug, Clone)]
pub struct JsonSettingsRepository {
    path: PathBuf,
}

impl JsonSettingsRepository {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_unlocked(&self) -> Result<AppSettings, SettingsError> {
        if !self.path.is_file() {
            return Ok(AppSettings::default());
        }
        let content = fs::read(&self.path).map_err(|source| SettingsError::Read {
            path: self.path.display().to_string(),
            source,
        })?;
        let settings: AppSettings =
            serde_json::from_value(migrate(serde_json::from_slice(&content)?))?;
        settings.validate()?;
        Ok(settings)
    }

    fn save_unlocked(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        write_atomically(&self.path, &serde_json::to_vec_pretty(settings)?)
    }
}

/// v1 kept connection profiles with their keys in settings; v2 keeps
/// providers here and their credentials in `credentials.json`. The profiles
/// are dropped, every other field is kept.
fn migrate(mut value: serde_json::Value) -> serde_json::Value {
    if value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_str)
        == Some(SETTINGS_SCHEMA_V1)
    {
        if let Some(assistant) = value
            .get_mut("assistant")
            .and_then(serde_json::Value::as_object_mut)
        {
            assistant.remove("activeConnectionId");
            assistant.remove("connections");
        }
        value["schemaVersion"] = SETTINGS_SCHEMA_VERSION.into();
    }
    if let Some(providers) = value
        .pointer_mut("/assistant/providers")
        .and_then(serde_json::Value::as_array_mut)
    {
        providers
            .iter_mut()
            .filter_map(serde_json::Value::as_object_mut)
            .for_each(migrate_base_url);
    }
    value
}

/// v2 first gave each provider one `baseUrl`, and a custom provider its one
/// `custom.protocol`; providers now keep a URL per API in `endpoints`. A
/// custom URL moves to its protocol's API; a vendor override, which moved
/// the vendor's only API, moves every API (resolution ignores the APIs the
/// vendor's region does not serve).
fn migrate_base_url(provider: &mut serde_json::Map<String, serde_json::Value>) {
    let base_url = provider.remove("baseUrl");
    let protocol = provider
        .get_mut("custom")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|custom| {
            let api = match custom.get("protocol")?.as_str()? {
                "openaiChatCompletions" => "chat",
                "openaiResponses" => "responses",
                "anthropicMessages" => "messages",
                _ => return None,
            };
            custom.remove("protocol");
            Some(api)
        });
    let Some(url) = base_url.filter(|url| !url.is_null()) else {
        return;
    };
    let apis: &[&str] = match protocol {
        Some(api) => &[api],
        None => &["chat", "responses", "messages"],
    };
    let endpoints = provider
        .entry("endpoints")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(endpoints) = endpoints.as_object_mut() {
        for api in apis {
            endpoints.insert((*api).into(), url.clone());
        }
    }
}

impl SettingsRepository for JsonSettingsRepository {
    fn load(&self) -> Result<AppSettings, SettingsError> {
        let _lock = lock_beside(&self.path)?;
        self.load_unlocked()
    }

    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        let _lock = lock_beside(&self.path)?;
        self.save_unlocked(settings)
    }

    fn save_if_equal(
        &self,
        expected: &AppSettings,
        settings: &AppSettings,
    ) -> Result<(), SettingsError> {
        let _lock = lock_beside(&self.path)?;
        let current = self.load_unlocked()?;
        if &current != expected {
            return Err(SettingsError::RevisionConflict);
        }
        self.save_unlocked(settings)
    }
}

fn write_error(path: &Path) -> impl FnOnce(std::io::Error) -> SettingsError + '_ {
    move |source| SettingsError::Write {
        path: path.display().to_string(),
        source,
    }
}

/// Holds `<path>.lock` exclusively until the returned file is dropped.
fn lock_beside(path: &Path) -> Result<File, SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(write_error(parent))?;
    }
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(write_error(&lock_path))?;
    file.lock_exclusive().map_err(write_error(&lock_path))?;
    Ok(file)
}

/// Replaces `path` with `content` through an owner-only temporary file that
/// is synced before the rename, then syncs the directory.
fn write_atomically(path: &Path, content: &[u8]) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(write_error(parent))?;
    }
    let temporary = unique_temporary_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temporary).map_err(write_error(&temporary))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(write_error(&temporary))?;
        }
        file.write_all(content).map_err(write_error(&temporary))?;
        file.sync_all().map_err(write_error(&temporary))?;
        drop(file);
        fs::rename(&temporary, path).map_err(write_error(path))?;
        if let Some(parent) = path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(write_error(parent))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn unique_temporary_path(path: &Path) -> PathBuf {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings");
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ))
}

#[derive(Clone)]
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
    current: Arc<RwLock<AppSettings>>,
    #[cfg(test)]
    edit_barrier: Option<Arc<std::sync::Barrier>>,
}

impl SettingsService {
    pub fn open(repository: Arc<dyn SettingsRepository>) -> Result<Self, SettingsError> {
        let settings = repository.load()?;
        Ok(Self {
            repository,
            current: Arc::new(RwLock::new(settings)),
            #[cfg(test)]
            edit_barrier: None,
        })
    }

    pub fn snapshot(&self) -> Result<AppSettings, SettingsError> {
        self.current
            .read()
            .map_err(|_| SettingsError::LockPoisoned)
            .map(|settings| settings.clone())
    }

    /// Re-read the repository and adopt it as the snapshot, so a caller whose
    /// write conflicted with another writer sees that value and can retry
    /// against its revision. A file that does not load or validate leaves the
    /// snapshot unchanged and returns the error.
    pub fn reload(&self) -> Result<AppSettings, SettingsError> {
        let mut current = self
            .current
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let settings = self.repository.load()?;
        settings.validate()?;
        *current = settings.clone();
        Ok(settings)
    }

    pub fn revision(&self) -> Result<String, SettingsError> {
        Ok(Self::revision_of(&self.snapshot()?))
    }

    pub fn revision_of(settings: &AppSettings) -> String {
        settings_revision(settings)
    }

    pub fn update(&self, settings: AppSettings) -> Result<AppSettings, SettingsError> {
        let mut current = self
            .current
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        settings.validate()?;
        self.repository.save_if_equal(&current, &settings)?;
        *current = settings.clone();
        Ok(settings)
    }

    /// Atomically validate and persist an update only if the in-memory snapshot
    /// is still equal to the caller's expected value.
    pub fn update_if_equal(
        &self,
        expected: &AppSettings,
        settings: AppSettings,
    ) -> Result<AppSettings, SettingsError> {
        let mut current = self
            .current
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        if &*current != expected {
            return Err(SettingsError::RevisionConflict);
        }
        settings.validate()?;
        self.repository.save_if_equal(&current, &settings)?;
        *current = settings.clone();
        Ok(settings)
    }

    /// Applies `change` to the settings at `expected_revision` and commits
    /// the result with one compare-and-swap; a write in between makes it a
    /// `RevisionConflict`.
    pub fn edit(
        &self,
        expected_revision: &str,
        change: impl FnOnce(&mut AppSettings) -> Result<(), SettingsError>,
    ) -> Result<AppSettings, SettingsError> {
        let current = self.snapshot()?;
        if settings_revision(&current) != expected_revision {
            return Err(SettingsError::RevisionConflict);
        }
        let mut next = current.clone();
        change(&mut next)?;
        #[cfg(test)]
        if let Some(barrier) = &self.edit_barrier {
            barrier.wait();
            barrier.wait();
        }
        self.update_if_equal(&current, next)
    }

    pub fn reset(&self) -> Result<AppSettings, SettingsError> {
        self.update(AppSettings::default())
    }
}

fn settings_revision(settings: &AppSettings) -> String {
    let bytes = serde_json::to_vec(settings).expect("settings must serialize");
    bytes
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::ModelRef;

    #[test]
    fn settings_round_trip_without_a_database() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-settings-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        let repository = Arc::new(JsonSettingsRepository::open(&path));
        let service = SettingsService::open(repository).unwrap();
        let settings = AppSettings {
            asr_backend_id: Some("whisper-rs".into()),
            asr_model: Some(ModelRef {
                backend_id: "whisper-rs".into(),
                model_id: "whisper-base-en-ggml".into(),
                revision: None,
                artifact_path: "models/asr/base.bin".into(),
                artifacts: Vec::new(),
            }),
            ..AppSettings::default()
        };

        service.update(settings.clone()).unwrap();
        let reopened =
            SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();

        assert_eq!(reopened.snapshot().unwrap(), settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_zero_expected_speakers() {
        let settings = AppSettings {
            expected_speakers: Some(0),
            ..AppSettings::default()
        };
        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("greater than zero")
        ));
    }

    #[test]
    fn legacy_settings_use_the_default_speaker_library_path() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-legacy-settings-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        fs::write(
            &path,
            r#"{
                "schemaVersion": "meetnote.settings.v1",
                "asrBackendId": null,
                "asrModel": null,
                "diarizationBackendId": null,
                "diarizationModel": null,
                "uiLocale": "zh-CN",
                "computeDevice": "Auto",
                "language": null,
                "expectedSpeakers": null
            }"#,
        )
        .unwrap();

        let settings = JsonSettingsRepository::open(&path).load().unwrap();

        assert_eq!(settings.speaker_library_path, "data/speakers/library.json");
        assert_eq!(settings.assistant, AssistantSettings::default());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn services_reject_a_stale_disk_version_before_overwriting_it() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-settings-concurrency-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        let first = SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();
        let second = SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();

        let mut changed = first.snapshot().unwrap();
        changed.ui_locale = "en-US".into();
        first.update(changed.clone()).unwrap();

        let mut stale = second.snapshot().unwrap();
        stale.ui_locale = "ja-JP".into();
        assert!(matches!(
            second.update(stale),
            Err(SettingsError::RevisionConflict)
        ));
        assert_eq!(
            JsonSettingsRepository::open(&path)
                .load()
                .unwrap()
                .ui_locale,
            "en-US"
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(format!("{}.lock", path.display()));
    }

    struct TestRepository(std::sync::Mutex<AppSettings>);

    impl TestRepository {
        fn new(settings: AppSettings) -> Self {
            Self(std::sync::Mutex::new(settings))
        }
    }

    impl SettingsRepository for TestRepository {
        fn load(&self) -> Result<AppSettings, SettingsError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
            *self.0.lock().unwrap() = settings.clone();
            Ok(())
        }
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn settings_without_appearance_load_the_default_appearance() {
        let settings: AppSettings = serde_json::from_str(
            r#"{
                "schemaVersion": "meetnote.settings.v2",
                "asrBackendId": null,
                "asrModel": null,
                "diarizationBackendId": null,
                "diarizationModel": null,
                "computeDevice": "Auto",
                "language": null,
                "expectedSpeakers": null
            }"#,
        )
        .unwrap();

        assert_eq!(
            settings.appearance,
            AppearanceSettings {
                theme: ThemePreference::System,
                material: MaterialPreference::Glass,
            }
        );
        assert_eq!(AppSettings::default().appearance, settings.appearance);
    }

    #[test]
    fn appearance_round_trips_with_camel_case_values() {
        let settings = AppSettings {
            appearance: AppearanceSettings {
                theme: ThemePreference::Light,
                material: MaterialPreference::Opaque,
            },
            ..AppSettings::default()
        };

        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            value["appearance"],
            serde_json::json!({"theme": "light", "material": "opaque"})
        );
        let partial: AppearanceSettings =
            serde_json::from_value(serde_json::json!({"theme": "dark"})).unwrap();
        assert_eq!(partial.material, MaterialPreference::Glass);
        assert_eq!(
            serde_json::from_value::<AppSettings>(value).unwrap(),
            settings
        );
    }

    #[test]
    fn settings_without_a_documents_root_load_the_default_root() {
        let settings: AppSettings = serde_json::from_str(
            r#"{
                "schemaVersion": "meetnote.settings.v2",
                "asrBackendId": null,
                "asrModel": null,
                "diarizationBackendId": null,
                "diarizationModel": null,
                "computeDevice": "Auto",
                "language": null,
                "expectedSpeakers": null
            }"#,
        )
        .unwrap();

        assert_eq!(settings.documents_root, "data/documents");
        assert_eq!(AppSettings::default().documents_root, "data/documents");
        settings.validate().unwrap();
    }

    #[test]
    fn documents_root_round_trips_as_camel_case() {
        let settings = AppSettings {
            documents_root: "/Users/me/Notes/meetnote".into(),
            ..AppSettings::default()
        };

        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["documentsRoot"], "/Users/me/Notes/meetnote");
        assert!(value.get("documents_root").is_none());
        assert_eq!(
            serde_json::from_value::<AppSettings>(value).unwrap(),
            settings
        );
    }

    #[test]
    fn rejects_an_empty_documents_root() {
        for root in ["", "   "] {
            let settings = AppSettings {
                documents_root: root.into(),
                ..AppSettings::default()
            };
            assert!(matches!(
                settings.validate(),
                Err(SettingsError::Invalid(message)) if message.contains("documents root")
            ));
        }
    }

    /// `settings.json` in a directory of its own, which the test removes.
    fn settings_file(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "meetnote-settings-{label}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        (root, path)
    }

    #[test]
    fn reload_adopts_an_external_change_so_the_retry_succeeds() {
        let (root, path) = settings_file("reload");
        let service = SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();
        let other = SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();
        let mut external = other.snapshot().unwrap();
        external.ui_locale = "en-US".into();
        other.update(external.clone()).unwrap();
        let retry = |expected: &AppSettings| {
            let mut next = expected.clone();
            next.language = Some("en".into());
            service.update_if_equal(expected, next)
        };

        // The snapshot cannot see the other writer, so retrying against it
        // conflicts every time.
        let stale = service.snapshot().unwrap();
        assert!(matches!(
            retry(&stale),
            Err(SettingsError::RevisionConflict)
        ));
        assert_eq!(service.snapshot().unwrap(), stale);

        let reloaded = service.reload().unwrap();
        assert_eq!(reloaded, external);
        assert_eq!(service.snapshot().unwrap(), external);
        assert_eq!(
            service.revision().unwrap(),
            SettingsService::revision_of(&external)
        );
        let saved = retry(&reloaded).unwrap();
        assert_eq!(saved.ui_locale, "en-US");
        assert_eq!(saved.language.as_deref(), Some("en"));
        assert_eq!(JsonSettingsRepository::open(&path).load().unwrap(), saved);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reload_of_an_invalid_file_keeps_the_snapshot_and_returns_the_error() {
        let (root, path) = settings_file("reload-invalid");
        let service = SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();
        let mut loaded = service.snapshot().unwrap();
        loaded.ui_locale = "en-US".into();
        service.update(loaded.clone()).unwrap();

        fs::write(&path, "{ not json").unwrap();
        let unparsable = service.reload();
        assert!(
            matches!(unparsable, Err(SettingsError::Parse(_))),
            "{unparsable:?}"
        );
        assert_eq!(service.snapshot().unwrap(), loaded);

        let mut invalid = serde_json::to_value(&loaded).unwrap();
        invalid["expectedSpeakers"] = serde_json::json!(0);
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let rejected = service.reload();
        assert!(
            matches!(&rejected, Err(SettingsError::Invalid(message)) if message.contains("expected speakers")),
            "{rejected:?}"
        );
        assert_eq!(service.snapshot().unwrap(), loaded);
        fs::remove_dir_all(root).unwrap();

        // Settings validates what any repository returns before adopting it.
        let repository = Arc::new(TestRepository::new(AppSettings::default()));
        let service = SettingsService::open(repository.clone()).unwrap();
        *repository.0.lock().unwrap() = AppSettings {
            expected_speakers: Some(0),
            ..AppSettings::default()
        };
        assert!(matches!(service.reload(), Err(SettingsError::Invalid(_))));
        assert_eq!(service.snapshot().unwrap(), AppSettings::default());
    }

    fn custom(id: &str, messages: &str) -> ProviderSettings {
        ProviderSettings {
            endpoints: EndpointUrls {
                messages: Some(messages.into()),
                ..EndpointUrls::default()
            },
            custom: Some(CustomEndpoint {
                name: "Gateway".into(),
                auth_mode: AssistantAuthMode::XApiKey,
            }),
            ..ProviderSettings::new(id)
        }
    }

    #[test]
    fn a_v1_file_loads_as_v2_without_its_connections() {
        let (root, path) = settings_file("migrate");
        fs::write(
            &path,
            r#"{
                "schemaVersion": "meetnote.settings.v1",
                "asrBackendId": "whisper-rs",
                "asrModel": null,
                "diarizationBackendId": "diarization-native",
                "diarizationModel": null,
                "uiLocale": "en-US",
                "computeDevice": "Auto",
                "language": "en",
                "expectedSpeakers": 3,
                "speakerLibraryPath": "speakers.json",
                "documentsRoot": "/notes",
                "assistant": {
                    "enabled": false,
                    "activeConnectionId": "local",
                    "writeMode": "confirm_each",
                    "maxToolRounds": 5,
                    "maxValidationRetries": 2,
                    "connections": [{
                        "id": "local", "name": "Local", "protocol": "openaiChatCompletions",
                        "baseUrl": "http://127.0.0.1:4000", "model": "m", "authMode": "bearer",
                        "apiKey": "sk-old", "contextWindowTokens": null
                    }]
                },
                "appearance": {"theme": "dark", "material": "opaque"}
            }"#,
        )
        .unwrap();

        let settings = JsonSettingsRepository::open(&path).load().unwrap();

        assert_eq!(settings.schema_version, "meetnote.settings.v2");
        assert_eq!(settings.ui_locale, "en-US");
        assert_eq!(settings.language.as_deref(), Some("en"));
        assert_eq!(settings.expected_speakers, Some(3));
        assert_eq!(settings.speaker_library_path, "speakers.json");
        assert_eq!(settings.documents_root, "/notes");
        assert_eq!(settings.appearance.theme, ThemePreference::Dark);
        assert_eq!(
            settings.assistant,
            AssistantSettings {
                enabled: false,
                write_mode: AgentWriteMode::ConfirmEach,
                max_tool_rounds: 5,
                max_validation_retries: 2,
                model: None,
                providers: Vec::new(),
            }
        );
        JsonSettingsRepository::open(&path).save(&settings).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("meetnote.settings.v2") && !saved.contains("sk-old"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_v2_file_with_base_urls_loads_its_providers_as_endpoints() {
        let (root, path) = settings_file("base-url");
        fs::write(
            &path,
            r#"{
                "schemaVersion": "meetnote.settings.v2",
                "asrBackendId": null,
                "asrModel": null,
                "diarizationBackendId": null,
                "diarizationModel": null,
                "computeDevice": "Auto",
                "language": null,
                "expectedSpeakers": null,
                "assistant": {
                    "model": {"provider": "custom-gw", "model": "m"},
                    "providers": [
                        {"id": "openai", "enabled": true, "baseUrl": null, "custom": null,
                         "models": {"mode": "vendor", "picked": [], "added": []},
                         "contextWindowTokens": null, "timeoutMs": 120000, "maxOutputTokens": 4096},
                        {"id": "magpie", "enabled": true, "baseUrl": null, "custom": null,
                         "models": {"mode": "vendor", "picked": [], "added": []},
                         "contextWindowTokens": null, "timeoutMs": 120000, "maxOutputTokens": 4096},
                        {"id": "deepseek", "enabled": false, "baseUrl": "https://proxy.example.com/v1",
                         "custom": null, "models": {"mode": "picked", "picked": ["deepseek-chat"], "added": []},
                         "contextWindowTokens": null, "timeoutMs": 60000, "maxOutputTokens": 2048},
                        {"id": "custom-gw", "enabled": true, "baseUrl": "https://gw.example.com/v1",
                         "custom": {"name": "Gateway", "protocol": "anthropicMessages", "authMode": "xApiKey"},
                         "models": {"mode": "vendor", "picked": [], "added": ["m"]},
                         "contextWindowTokens": 200000, "timeoutMs": 120000, "maxOutputTokens": 4096}
                    ]
                }
            }"#,
        )
        .unwrap();
        let repository = JsonSettingsRepository::open(&path);

        let settings = repository.load().unwrap();

        let providers = &settings.assistant.providers;
        assert_eq!(providers.len(), 4);
        assert!(providers.iter().all(|provider| provider.region.is_none()));
        for provider in &providers[..2] {
            assert_eq!(provider.endpoints, EndpointUrls::default());
        }
        let proxy = Some("https://proxy.example.com/v1".to_owned());
        assert_eq!(
            providers[2].endpoints,
            EndpointUrls {
                chat: proxy.clone(),
                responses: proxy.clone(),
                messages: proxy,
            }
        );
        assert!(!providers[2].enabled);
        assert_eq!(providers[2].models.picked, ["deepseek-chat"]);
        assert_eq!(providers[3], {
            let mut gateway = custom("custom-gw", "https://gw.example.com/v1");
            gateway.models.added = vec!["m".into()];
            gateway.context_window_tokens = Some(200_000);
            gateway
        });
        repository.save(&settings).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let saved = &saved["assistant"]["providers"];
        assert!(saved[0].get("baseUrl").is_none() && saved[3].get("baseUrl").is_none());
        assert_eq!(
            saved[3]["custom"],
            serde_json::json!({"name": "Gateway", "authMode": "xApiKey"})
        );
        assert_eq!(
            saved[3]["endpoints"],
            serde_json::json!({"chat": null, "responses": null, "messages": "https://gw.example.com/v1"})
        );
        assert_eq!(repository.load().unwrap(), settings);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn providers_round_trip_in_camel_case() {
        let mut deepseek = ProviderSettings::new("deepseek");
        deepseek.region = Some("Global".into());
        deepseek.endpoints.chat = Some("https://proxy.example.com/v1".into());
        deepseek.models = ModelSelection {
            mode: ModelListMode::LatestPerFamily,
            picked: vec!["deepseek-chat".into()],
            added: vec!["deepseek-reasoner".into()],
        };
        let settings = AppSettings {
            assistant: AssistantSettings {
                model: Some(ModelChoice {
                    provider: "custom-gw".into(),
                    model: "claude".into(),
                    protocol: Some(LlmProtocol::AnthropicMessages),
                }),
                providers: vec![deepseek, custom("custom-gw", "https://gw.example.com/v1")],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        };
        settings.validate().unwrap();

        let value = serde_json::to_value(&settings).unwrap();
        let assistant = &value["assistant"];
        assert_eq!(
            assistant["model"],
            serde_json::json!({"provider": "custom-gw", "model": "claude", "protocol": "anthropicMessages"})
        );
        assert_eq!(assistant["providers"][0]["region"], "Global");
        assert_eq!(
            assistant["providers"][0]["endpoints"],
            serde_json::json!({"chat": "https://proxy.example.com/v1", "responses": null, "messages": null})
        );
        assert_eq!(
            assistant["providers"][0]["models"]["mode"],
            "latestPerFamily"
        );
        assert_eq!(assistant["providers"][0]["timeoutMs"], 120_000);
        assert_eq!(assistant["providers"][0]["maxOutputTokens"], 4_096);
        assert_eq!(
            assistant["providers"][1]["custom"],
            serde_json::json!({"name": "Gateway", "authMode": "xApiKey"})
        );
        assert_eq!(
            assistant["providers"][1]["endpoints"]["messages"],
            "https://gw.example.com/v1"
        );
        assert_eq!(
            serde_json::from_value::<AppSettings>(value).unwrap(),
            settings
        );
        let minimal: ProviderSettings =
            serde_json::from_value(serde_json::json!({"id": "openai"})).unwrap();
        assert_eq!(minimal, ProviderSettings::new("openai"));
        let unresolved: ModelChoice =
            serde_json::from_value(serde_json::json!({"provider": "openai", "model": "gpt"}))
                .unwrap();
        assert_eq!(unresolved.protocol, None);
        assert!(serde_json::from_value::<ProviderSettings>(
            serde_json::json!({"id": "openai", "apiKey": "sk"})
        )
        .is_err());
    }

    #[test]
    fn provider_rules_are_enforced() {
        let invalid = |provider: ProviderSettings, needle: &str| {
            let settings = AssistantSettings {
                providers: vec![provider],
                ..AssistantSettings::default()
            };
            match settings.validate() {
                Err(SettingsError::Invalid(message)) => {
                    assert!(message.contains(needle), "{message} lacks {needle}")
                }
                other => panic!("expected {needle}, got {other:?}"),
            }
        };
        invalid(ProviderSettings::new(""), "id");
        invalid(ProviderSettings::new("custom-x"), "endpoint");
        invalid(
            ProviderSettings {
                custom: custom("custom-x", "https://a.example").custom,
                ..ProviderSettings::new("openai")
            },
            "only custom-",
        );
        invalid(
            ProviderSettings {
                endpoints: EndpointUrls::default(),
                ..custom("custom-x", "https://a.example")
            },
            "endpoint URL",
        );
        invalid(
            ProviderSettings {
                region: Some("Global".into()),
                ..custom("custom-x", "https://a.example")
            },
            "region",
        );
        invalid(
            ProviderSettings {
                region: Some(" ".into()),
                ..ProviderSettings::new("moonshot")
            },
            "region",
        );
        for url in [
            "http://remote.example.com/v1",
            "https://user:pw@a.example/v1",
            "https://a.example/v1?x=1",
            "https://a.example/v1#f",
            "not a url",
        ] {
            invalid(custom("custom-x", url), "endpoints.messages");
            let mut overridden = ProviderSettings::new("deepseek");
            overridden.endpoints.responses = Some(url.into());
            invalid(overridden, "endpoints.responses");
        }
        for url in [
            "http://127.0.0.1:4000/v1",
            "http://localhost/v1",
            "http://[::1]:9/v1",
        ] {
            AssistantSettings {
                providers: vec![custom("custom-x", url)],
                ..AssistantSettings::default()
            }
            .validate()
            .unwrap();
        }
        invalid(
            ProviderSettings {
                timeout_ms: 0,
                ..ProviderSettings::new("openai")
            },
            "timeoutMs",
        );
        invalid(
            ProviderSettings {
                max_output_tokens: 0,
                ..ProviderSettings::new("openai")
            },
            "maxOutputTokens",
        );
        invalid(
            ProviderSettings {
                context_window_tokens: Some(4_096),
                ..ProviderSettings::new("openai")
            },
            "contextWindowTokens",
        );

        let duplicated = AssistantSettings {
            providers: vec![
                ProviderSettings::new("openai"),
                ProviderSettings::new("openai"),
            ],
            ..AssistantSettings::default()
        };
        assert!(
            matches!(duplicated.validate(), Err(SettingsError::Invalid(message)) if message.contains("duplicated"))
        );
        let dangling = AssistantSettings {
            model: Some(ModelChoice {
                provider: "openai".into(),
                model: "gpt".into(),
                protocol: None,
            }),
            ..AssistantSettings::default()
        };
        assert!(
            matches!(dangling.validate(), Err(SettingsError::Invalid(message)) if message.contains("missing provider"))
        );
        let budgets = AssistantSettings {
            max_tool_rounds: 2,
            max_validation_retries: 3,
            ..AssistantSettings::default()
        };
        assert!(budgets.validate().is_err());
    }

    #[test]
    fn edit_rejects_a_stale_snapshot_after_a_concurrent_commit() {
        let mut service =
            SettingsService::open(Arc::new(TestRepository::new(AppSettings::default()))).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        service.edit_barrier = Some(barrier.clone());
        let revision = service.revision().unwrap();
        let editor = service.clone();
        let edit = std::thread::spawn(move || {
            editor.edit(&revision, |settings| {
                settings.assistant.max_tool_rounds = 4;
                Ok(())
            })
        });
        barrier.wait();

        let mut concurrent = service.snapshot().unwrap();
        concurrent.assistant.max_tool_rounds = 5;
        service.update(concurrent).unwrap();
        barrier.wait();

        let result = edit.join().unwrap();
        assert!(
            matches!(result, Err(SettingsError::RevisionConflict)),
            "{result:?}"
        );
        assert_eq!(service.snapshot().unwrap().assistant.max_tool_rounds, 5);
    }

    #[test]
    fn edit_checks_the_revision_and_validates_the_result() {
        let service =
            SettingsService::open(Arc::new(TestRepository::new(AppSettings::default()))).unwrap();
        assert!(matches!(
            service.edit("stale", |_| Ok(())),
            Err(SettingsError::RevisionConflict)
        ));
        let revision = service.revision().unwrap();
        assert!(matches!(
            service.edit(&revision, |settings| {
                settings.assistant.max_tool_rounds = 0;
                Ok(())
            }),
            Err(SettingsError::Invalid(_))
        ));
        let saved = service
            .edit(&revision, |settings| {
                settings.assistant.enabled = false;
                Ok(())
            })
            .unwrap();
        assert!(!saved.assistant.enabled);
        assert_eq!(service.snapshot().unwrap(), saved);
    }
}
