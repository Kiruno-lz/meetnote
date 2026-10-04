use contracts::{AgentWriteMode, ComputeDevice, DiarizationStageConfiguration, ModelRef};
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use url::Url;

const SETTINGS_SCHEMA_VERSION: &str = "meetnote.settings.v1";
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

fn default_assistant_enabled() -> bool {
    true
}

fn default_max_validation_retries() -> u32 {
    3
}

fn default_connection_timeout_ms() -> u64 {
    120_000
}

fn default_max_output_tokens() -> u32 {
    4_096
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LlmProtocol {
    #[serde(rename = "openaiChatCompletions")]
    OpenAiChatCompletions,
    #[serde(rename = "anthropicMessages")]
    AnthropicMessages,
    #[serde(rename = "openaiResponses")]
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AssistantAuthMode {
    Bearer,
    XApiKey,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConnectionProfile {
    pub id: String,
    pub name: String,
    pub protocol: LlmProtocol,
    pub base_url: String,
    pub model: String,
    pub auth_mode: AssistantAuthMode,
    /// API key stored directly in settings. Mutually exclusive with `api_key_file`.
    #[serde(default, alias = "api-key")]
    pub api_key: Option<String>,
    /// Path to a file containing the API key. Mutually exclusive with `api_key`.
    #[serde(default, alias = "api-key-file")]
    pub api_key_file: Option<String>,
    pub context_window_tokens: Option<u32>,
    #[serde(default = "default_connection_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
}

impl fmt::Debug for AssistantConnectionProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssistantConnectionProfile")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("auth_mode", &self.auth_mode)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field(
                "api_key_file",
                &self.api_key_file.as_ref().map(|_| "<redacted>"),
            )
            .field("context_window_tokens", &self.context_window_tokens)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_output_tokens", &self.max_output_tokens)
            .finish()
    }
}

impl AssistantConnectionProfile {
    pub fn validate(&self) -> Result<(), SettingsError> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("name", self.name.as_str()),
            ("model", self.model.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(SettingsError::Invalid(format!(
                    "assistant connection {field} must not be empty"
                )));
            }
        }

        let url = Url::parse(&self.base_url).map_err(|error| {
            SettingsError::Invalid(format!("assistant connection baseUrl is invalid: {error}"))
        })?;
        if url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(SettingsError::Invalid(
                "assistant connection baseUrl cannot contain credentials, query, or fragment"
                    .into(),
            ));
        }
        let host = url.host_str().ok_or_else(|| {
            SettingsError::Invalid("assistant connection baseUrl must contain a host".into())
        })?;
        let local = is_loopback_host(host);
        if url.scheme() != "https" && !(url.scheme() == "http" && local) {
            return Err(SettingsError::Invalid(
                "assistant connection baseUrl must use HTTPS unless it targets localhost".into(),
            ));
        }
        if self.timeout_ms == 0 {
            return Err(SettingsError::Invalid(
                "assistant connection timeoutMs must be greater than zero".into(),
            ));
        }
        if self.max_output_tokens == 0 {
            return Err(SettingsError::Invalid(
                "assistant connection maxOutputTokens must be greater than zero".into(),
            ));
        }
        if self.context_window_tokens == Some(0) {
            return Err(SettingsError::Invalid(
                "assistant connection contextWindowTokens must be greater than zero".into(),
            ));
        }
        if self
            .api_key
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(SettingsError::Invalid(
                "assistant connection apiKey must not be empty".into(),
            ));
        }
        if self
            .api_key_file
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(SettingsError::Invalid(
                "assistant connection apiKeyFile must not be empty".into(),
            ));
        }
        if self.api_key.is_some() && self.api_key_file.is_some() {
            return Err(SettingsError::Invalid(
                "assistant connection must configure apiKey or apiKeyFile, not both".into(),
            ));
        }
        if self
            .context_window_tokens
            .is_some_and(|limit| limit <= self.max_output_tokens)
        {
            return Err(SettingsError::Invalid(
                "assistant connection contextWindowTokens must exceed maxOutputTokens".into(),
            ));
        }
        Ok(())
    }

    pub fn is_local(&self) -> bool {
        Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| is_loopback_host(&host))
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantSettings {
    #[serde(default = "default_assistant_enabled")]
    pub enabled: bool,
    pub active_connection_id: Option<String>,
    #[serde(default)]
    pub write_mode: AgentWriteMode,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_max_validation_retries")]
    pub max_validation_retries: u32,
    #[serde(default)]
    pub connections: Vec<AssistantConnectionProfile>,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            active_connection_id: None,
            write_mode: AgentWriteMode::Automatic,
            max_tool_rounds: default_max_tool_rounds(),
            max_validation_retries: default_max_validation_retries(),
            connections: Vec::new(),
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
        for connection in &self.connections {
            connection.validate()?;
        }
        for (index, connection) in self.connections.iter().enumerate() {
            if self.connections[..index]
                .iter()
                .any(|other| other.id == connection.id)
            {
                return Err(SettingsError::Invalid(format!(
                    "assistant connection id is duplicated: {}",
                    connection.id
                )));
            }
        }
        if let Some(active) = &self.active_connection_id {
            if !self
                .connections
                .iter()
                .any(|connection| &connection.id == active)
            {
                return Err(SettingsError::Invalid(format!(
                    "assistant activeConnectionId does not exist: {active}"
                )));
            }
        }
        Ok(())
    }

    pub fn active_connection(&self) -> Option<&AssistantConnectionProfile> {
        let active = self.active_connection_id.as_deref()?;
        self.connections
            .iter()
            .find(|connection| connection.id == active)
    }
}

/// Light or dark: follow the system appearance or pin one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// The window chrome material. Glass is used only while the system does not
/// ask to reduce transparency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MaterialPreference {
    #[default]
    Glass,
    Opaque,
}

/// Desktop appearance. It belongs to the desktop UI alone and is not part of
/// the Settings capability projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub material: MaterialPreference,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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

/// A credential change is explicit.  Omitting `credential` from a patch keeps
/// the existing credential; it is never represented by a redaction marker.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CredentialUpdate {
    Direct(String),
    File(String),
    Clear,
}

impl fmt::Debug for CredentialUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(_) => formatter.write_str("CredentialUpdate::Direct(<redacted>)"),
            Self::File(_) => formatter.write_str("CredentialUpdate::File(<redacted>)"),
            Self::Clear => formatter.write_str("CredentialUpdate::Clear"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConnectionPatch {
    pub id: String,
    pub name: Option<String>,
    pub protocol: Option<LlmProtocol>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub auth_mode: Option<AssistantAuthMode>,
    pub context_window_tokens: Option<Option<u32>>,
    pub timeout_ms: Option<u64>,
    pub max_output_tokens: Option<u32>,
    pub credential: Option<CredentialUpdate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub assistant_enabled: Option<bool>,
    pub active_connection_id: Option<Option<String>>,
    pub write_mode: Option<AgentWriteMode>,
    pub max_tool_rounds: Option<u32>,
    pub max_validation_retries: Option<u32>,
    pub connection: Option<AssistantConnectionPatch>,
    /// A profile added before `connection` is applied, which sets its
    /// credential under the edit rules; the profile carries none itself.
    /// Desktop Settings only: absent from the wire contract, so capability
    /// callers cannot create connections.
    #[serde(skip)]
    pub new_connection: Option<AssistantConnectionProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsGrant {
    read_connections: bool,
    read_connection_targets: bool,
    read_credentials: bool,
    read_write_policy: bool,
    write_connections: bool,
    write_connection_targets: bool,
    write_credentials: bool,
    write_write_policy: bool,
}

impl SettingsGrant {
    pub const fn read_only() -> Self {
        Self {
            read_connections: false,
            read_connection_targets: false,
            read_credentials: false,
            read_write_policy: false,
            write_connections: false,
            write_connection_targets: false,
            write_credentials: false,
            write_write_policy: false,
        }
    }

    pub const fn full() -> Self {
        Self {
            read_connections: true,
            read_connection_targets: true,
            read_credentials: true,
            read_write_policy: true,
            write_connections: true,
            write_connection_targets: true,
            write_credentials: true,
            write_write_policy: true,
        }
    }

    pub const fn assistant_connections() -> Self {
        Self {
            read_connections: true,
            write_connections: true,
            ..Self::read_only()
        }
    }

    pub const fn assistant_connection_targets() -> Self {
        Self {
            read_connection_targets: true,
            write_connection_targets: true,
            ..Self::read_only()
        }
    }

    pub const fn assistant_credentials() -> Self {
        Self {
            read_credentials: true,
            write_credentials: true,
            ..Self::read_only()
        }
    }

    pub const fn assistant_write_policy() -> Self {
        Self {
            read_write_policy: true,
            write_write_policy: true,
            ..Self::read_only()
        }
    }

    pub const fn with_connections(self) -> Self {
        Self {
            read_connections: true,
            write_connections: true,
            ..self
        }
    }

    pub const fn with_read_connections(self) -> Self {
        Self {
            read_connections: true,
            ..self
        }
    }

    pub const fn with_connection_targets(self) -> Self {
        Self {
            read_connection_targets: true,
            write_connection_targets: true,
            ..self
        }
    }

    pub const fn with_read_connection_targets(self) -> Self {
        Self {
            read_connection_targets: true,
            ..self
        }
    }

    pub const fn with_credentials(self) -> Self {
        Self {
            read_credentials: true,
            write_credentials: true,
            ..self
        }
    }

    pub const fn with_read_credentials(self) -> Self {
        Self {
            read_credentials: true,
            ..self
        }
    }

    pub const fn with_write_policy(self) -> Self {
        Self {
            read_write_policy: true,
            write_write_policy: true,
            ..self
        }
    }

    pub const fn with_read_write_policy(self) -> Self {
        Self {
            read_write_policy: true,
            ..self
        }
    }

    pub const fn can_read_connections(self) -> bool {
        self.read_connections
    }

    pub const fn can_read_connection_targets(self) -> bool {
        self.read_connection_targets
    }

    pub const fn can_read_credentials(self) -> bool {
        self.read_credentials
    }

    pub const fn can_read_write_policy(self) -> bool {
        self.read_write_policy
    }

    pub const fn can_write_connections(self) -> bool {
        self.write_connections
    }

    pub const fn can_write_connection_targets(self) -> bool {
        self.write_connection_targets
    }

    pub const fn can_write_credentials(self) -> bool {
        self.write_credentials
    }

    pub const fn can_write_write_policy(self) -> bool {
        self.write_write_policy
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self {
            read_connections: self.read_connections && other.read_connections,
            read_connection_targets: self.read_connection_targets && other.read_connection_targets,
            read_credentials: self.read_credentials && other.read_credentials,
            read_write_policy: self.read_write_policy && other.read_write_policy,
            write_connections: self.write_connections && other.write_connections,
            write_connection_targets: self.write_connection_targets
                && other.write_connection_targets,
            write_credentials: self.write_credentials && other.write_credentials,
            write_write_policy: self.write_write_policy && other.write_write_policy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConnectionSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConnectionTarget {
    pub id: String,
    pub protocol: LlmProtocol,
    pub base_url: String,
    pub model: String,
    pub auth_mode: AssistantAuthMode,
    pub context_window_tokens: Option<u32>,
    pub timeout_ms: u64,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub id: String,
    pub has_api_key: bool,
    pub has_api_key_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantWritePolicy {
    pub enabled: bool,
    pub active_connection_id: Option<String>,
    pub write_mode: AgentWriteMode,
    pub max_tool_rounds: u32,
    pub max_validation_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsReadResult<T> {
    pub revision: String,
    pub value: T,
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
    #[error("settings field is not granted: {field}")]
    FieldNotGranted { field: String },
    #[error("settings credential cannot be forwarded to a changed connection target")]
    CredentialTargetConflict,
    #[error("settings credential update cannot use [redacted]")]
    RedactedCredential,
    #[error("settings credential file must be an owner-only regular file under credentials/")]
    UnsafeCredentialFile,
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

    /// Open the file an `apiKeyFile` reference names, applying the same checks
    /// as load and save to the returned handle. Repositories without a
    /// settings directory have no credential files.
    fn open_credential_file(&self, _reference: &str) -> Result<File, SettingsError> {
        Err(SettingsError::Invalid(
            "settings repository has no credential directory".into(),
        ))
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

    fn lock_path(&self) -> PathBuf {
        let mut path = self.path.as_os_str().to_os_string();
        path.push(".lock");
        PathBuf::from(path)
    }

    fn lock(&self) -> Result<File, SettingsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| SettingsError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())
            .map_err(|source| SettingsError::Write {
                path: self.lock_path().display().to_string(),
                source,
            })?;
        file.lock_exclusive()
            .map_err(|source| SettingsError::Write {
                path: self.lock_path().display().to_string(),
                source,
            })?;
        Ok(file)
    }

    fn load_unlocked(&self) -> Result<AppSettings, SettingsError> {
        if !self.path.is_file() {
            return Ok(AppSettings::default());
        }
        let content = fs::read_to_string(&self.path).map_err(|source| SettingsError::Read {
            path: self.path.display().to_string(),
            source,
        })?;
        let settings: AppSettings = serde_json::from_str(&content)?;
        settings.validate()?;
        validate_settings_file_permissions(&settings, &self.path)?;
        validate_credential_file_references(&settings, &self.path)?;
        Ok(settings)
    }

    fn save_unlocked(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        validate_credential_file_references(settings, &self.path)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| SettingsError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let content = serde_json::to_vec_pretty(settings)?;
        let temporary = unique_temporary_path(&self.path);
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| SettingsError::Write {
                    path: temporary.display().to_string(),
                    source,
                })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|source| SettingsError::Write {
                        path: temporary.display().to_string(),
                        source,
                    })?;
            }
            file.write_all(&content)
                .map_err(|source| SettingsError::Write {
                    path: temporary.display().to_string(),
                    source,
                })?;
            file.sync_all().map_err(|source| SettingsError::Write {
                path: temporary.display().to_string(),
                source,
            })?;
            drop(file);
            fs::rename(&temporary, &self.path).map_err(|source| SettingsError::Write {
                path: self.path.display().to_string(),
                source,
            })?;
            if let Some(parent) = self.path.parent() {
                File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|source| SettingsError::Write {
                        path: parent.display().to_string(),
                        source,
                    })?;
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

impl SettingsRepository for JsonSettingsRepository {
    fn load(&self) -> Result<AppSettings, SettingsError> {
        let _lock = self.lock()?;
        self.load_unlocked()
    }

    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        let _lock = self.lock()?;
        self.save_unlocked(settings)
    }

    fn save_if_equal(
        &self,
        expected: &AppSettings,
        settings: &AppSettings,
    ) -> Result<(), SettingsError> {
        let _lock = self.lock()?;
        let current = self.load_unlocked()?;
        if &current != expected {
            return Err(SettingsError::RevisionConflict);
        }
        self.save_unlocked(settings)
    }

    fn open_credential_file(&self, reference: &str) -> Result<File, SettingsError> {
        open_credential(reference, &self.path)
    }
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

fn validate_credential_file_references(
    settings: &AppSettings,
    settings_path: &Path,
) -> Result<(), SettingsError> {
    for connection in &settings.assistant.connections {
        if let Some(reference) = connection.api_key_file.as_deref() {
            // A reference may be saved before its file is provisioned; any
            // other failure to open it safely is an invalid setting.
            if let Err(error) = open_credential(reference, settings_path) {
                if !matches!(&error, SettingsError::Read { source, .. }
                    if source.kind() == std::io::ErrorKind::NotFound)
                {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

/// The single rule for `apiKeyFile`: a relative path under `credentials/`
/// next to the settings file that names an owner-only regular file, with no
/// symlink at any component below the settings directory. The checks run on
/// the opened handle, which is returned so callers read exactly the verified
/// file.
fn open_credential(reference: &str, settings_path: &Path) -> Result<File, SettingsError> {
    let path = Path::new(reference);
    if reference.contains('\0')
        || path.is_absolute()
        || path.file_name().is_none()
        || path == settings_path
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(name)) if name == OsStr::new("credentials")
        )
    {
        return Err(SettingsError::UnsafeCredentialFile);
    }
    // `components()` drops interior `.`, so anything but a name here is `..`
    // or a root and would leave the settings directory.
    let names = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name),
            _ => Err(SettingsError::UnsafeCredentialFile),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let directory = match settings_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let resolved = directory.join(path);
    let file = open_credential_components(directory, &names, &resolved)?;
    let metadata = file.metadata().map_err(|source| SettingsError::Read {
        path: resolved.display().to_string(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(SettingsError::UnsafeCredentialFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SettingsError::UnsafeCredentialFile);
        }
    }
    Ok(file)
}

/// Walk `names` below `directory` one descriptor at a time, so no component
/// can be redirected between the check and the open.
#[cfg(unix)]
fn open_credential_components(
    directory: &Path,
    names: &[&OsStr],
    resolved: &Path,
) -> Result<File, SettingsError> {
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::fs::OpenOptionsExt;

    let component_error = |source: std::io::Error| match source.raw_os_error() {
        // O_NOFOLLOW reports a symlink as ELOOP (EMLINK on FreeBSD); a file or
        // symlink where a directory is required is ENOTDIR.
        Some(libc::ELOOP | libc::EMLINK | libc::ENOTDIR) => SettingsError::UnsafeCredentialFile,
        _ => SettingsError::Read {
            path: resolved.display().to_string(),
            source,
        },
    };
    let Some((file_name, parents)) = names.split_last() else {
        return Err(SettingsError::UnsafeCredentialFile);
    };
    // The settings directory is the configured root; only what lies below it
    // is constrained.
    let mut parent: OwnedFd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(directory)
        .map_err(|source| SettingsError::Read {
            path: directory.display().to_string(),
            source,
        })?
        .into();
    for name in parents {
        parent = open_at(
            parent.as_fd(),
            name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
        .map_err(component_error)?;
    }
    // O_NONBLOCK keeps a FIFO or device from blocking the open, and O_NOCTTY
    // keeps a terminal from becoming the controlling one; neither affects
    // reading the regular file that is the only kind accepted afterwards.
    open_at(
        parent.as_fd(),
        file_name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC,
    )
    .map(File::from)
    .map_err(component_error)
}

#[cfg(unix)]
fn open_at(
    parent: std::os::fd::BorrowedFd<'_>,
    name: &OsStr,
    flags: libc::c_int,
) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let name = std::ffi::CString::new(name.as_bytes())?;
    loop {
        // SAFETY: `parent` is a live descriptor and `name` a NUL-terminated
        // string; both outlive the call, which retains neither.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd >= 0 {
            // SAFETY: `openat` just returned this descriptor and nothing else
            // owns it.
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Best effort without descriptor-relative opens: every component is checked
/// by path before the open, so a component swapped in between is not caught.
#[cfg(not(unix))]
fn open_credential_components(
    directory: &Path,
    names: &[&OsStr],
    resolved: &Path,
) -> Result<File, SettingsError> {
    let read_error = |source| SettingsError::Read {
        path: resolved.display().to_string(),
        source,
    };
    let mut current = directory.to_path_buf();
    for (index, name) in names.iter().enumerate() {
        current.push(name);
        let metadata = fs::symlink_metadata(&current).map_err(read_error)?;
        let is_file_name = index + 1 == names.len();
        if metadata.file_type().is_symlink()
            || (is_file_name && !metadata.is_file())
            || (!is_file_name && !metadata.is_dir())
        {
            return Err(SettingsError::UnsafeCredentialFile);
        }
    }
    File::open(&current).map_err(read_error)
}

fn validate_settings_file_permissions(
    settings: &AppSettings,
    settings_path: &Path,
) -> Result<(), SettingsError> {
    let contains_secret = settings
        .assistant
        .connections
        .iter()
        .any(|connection| connection.api_key.is_some() || connection.api_key_file.is_some());
    if !contains_secret {
        return Ok(());
    }
    #[cfg(unix)]
    if let Ok(metadata) = fs::metadata(settings_path) {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SettingsError::Invalid(
                "settings file containing credentials must be owner-only".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
    current: Arc<RwLock<AppSettings>>,
    #[cfg(test)]
    patch_barrier: Option<Arc<std::sync::Barrier>>,
}

impl SettingsService {
    pub fn open(repository: Arc<dyn SettingsRepository>) -> Result<Self, SettingsError> {
        let settings = repository.load()?;
        Ok(Self {
            repository,
            current: Arc::new(RwLock::new(settings)),
            #[cfg(test)]
            patch_barrier: None,
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

    pub fn read_connections(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<AssistantConnectionSummary>>, SettingsError> {
        if !grant.read_connections {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.connections".into(),
            });
        }
        let settings = self.snapshot()?;
        Ok(SettingsReadResult {
            revision: settings_revision(&settings),
            value: settings
                .assistant
                .connections
                .iter()
                .map(|connection| AssistantConnectionSummary {
                    id: connection.id.clone(),
                    name: connection.name.clone(),
                })
                .collect(),
        })
    }

    pub fn read_connection_targets(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<AssistantConnectionTarget>>, SettingsError> {
        if !grant.read_connection_targets {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.connectionTargets".into(),
            });
        }
        let settings = self.snapshot()?;
        Ok(SettingsReadResult {
            revision: settings_revision(&settings),
            value: settings
                .assistant
                .connections
                .iter()
                .map(|connection| AssistantConnectionTarget {
                    id: connection.id.clone(),
                    protocol: connection.protocol,
                    base_url: connection.base_url.clone(),
                    model: connection.model.clone(),
                    auth_mode: connection.auth_mode,
                    context_window_tokens: connection.context_window_tokens,
                    timeout_ms: connection.timeout_ms,
                    max_output_tokens: connection.max_output_tokens,
                })
                .collect(),
        })
    }

    pub fn read_credentials(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<CredentialStatus>>, SettingsError> {
        if !grant.read_credentials {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.credentials".into(),
            });
        }
        let settings = self.snapshot()?;
        Ok(SettingsReadResult {
            revision: settings_revision(&settings),
            value: settings
                .assistant
                .connections
                .iter()
                .map(|connection| CredentialStatus {
                    id: connection.id.clone(),
                    has_api_key: connection.api_key.is_some(),
                    has_api_key_file: connection.api_key_file.is_some(),
                })
                .collect(),
        })
    }

    pub fn read_write_policy(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<AssistantWritePolicy>, SettingsError> {
        if !grant.read_write_policy {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.writePolicy".into(),
            });
        }
        let settings = self.snapshot()?;
        Ok(SettingsReadResult {
            revision: settings_revision(&settings),
            value: AssistantWritePolicy {
                enabled: settings.assistant.enabled,
                active_connection_id: settings.assistant.active_connection_id.clone(),
                write_mode: settings.assistant.write_mode,
                max_tool_rounds: settings.assistant.max_tool_rounds,
                max_validation_retries: settings.assistant.max_validation_retries,
            },
        })
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

    pub fn patch_if_equal(
        &self,
        expected_revision: &str,
        patch: SettingsPatch,
        grant: SettingsGrant,
    ) -> Result<AppSettings, SettingsError> {
        let current = self.snapshot()?;
        if settings_revision(&current) != expected_revision {
            return Err(SettingsError::RevisionConflict);
        }
        let mut next = current.clone();
        apply_patch(&mut next, patch, grant)?;
        #[cfg(test)]
        if let Some(barrier) = &self.patch_barrier {
            barrier.wait();
            barrier.wait();
        }
        self.update_if_equal(&current, next)
    }

    pub fn reset(&self) -> Result<AppSettings, SettingsError> {
        self.update(AppSettings::default())
    }

    /// Open an `apiKeyFile` reference through the repository that owns the
    /// settings location; callers read only from the returned handle.
    pub fn open_credential_file(&self, reference: &str) -> Result<File, SettingsError> {
        self.repository.open_credential_file(reference)
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

fn apply_patch(
    settings: &mut AppSettings,
    patch: SettingsPatch,
    grant: SettingsGrant,
) -> Result<(), SettingsError> {
    let has_write_policy = patch.assistant_enabled.is_some()
        || patch.active_connection_id.is_some()
        || patch.write_mode.is_some()
        || patch.max_tool_rounds.is_some()
        || patch.max_validation_retries.is_some();
    if has_write_policy && !grant.write_write_policy {
        return Err(SettingsError::FieldNotGranted {
            field: "assistant.writePolicy".into(),
        });
    }
    if let Some(value) = patch.assistant_enabled {
        settings.assistant.enabled = value;
    }
    if let Some(value) = patch.active_connection_id {
        settings.assistant.active_connection_id = value;
    }
    if let Some(value) = patch.write_mode {
        settings.assistant.write_mode = value;
    }
    if let Some(value) = patch.max_tool_rounds {
        settings.assistant.max_tool_rounds = value;
    }
    if let Some(value) = patch.max_validation_retries {
        settings.assistant.max_validation_retries = value;
    }
    if let Some(profile) = patch.new_connection {
        // A new profile is read back through both sections, so it needs both
        // grants. Its credential arrives only as `connection.credential`.
        if !grant.write_connections {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.connections".into(),
            });
        }
        if !grant.write_connection_targets {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.connectionTargets".into(),
            });
        }
        if profile.api_key.is_some() || profile.api_key_file.is_some() {
            return Err(SettingsError::Invalid(
                "a new assistant connection sets its credential through connection.credential"
                    .into(),
            ));
        }
        if settings
            .assistant
            .connections
            .iter()
            .any(|connection| connection.id == profile.id)
        {
            return Err(SettingsError::Invalid(format!(
                "assistant connection id is duplicated: {}",
                profile.id
            )));
        }
        settings.assistant.connections.push(profile);
    }

    let Some(connection_patch) = patch.connection else {
        return settings.validate();
    };
    let connection = settings
        .assistant
        .connections
        .iter_mut()
        .find(|connection| connection.id == connection_patch.id)
        .ok_or_else(|| SettingsError::Invalid("assistant connection does not exist".into()))?;
    // Each field is written under the grant whose section reads it back.
    // Budgets belong to connectionTargets but do not redirect the credential,
    // so only endpoint changes are bound to an explicit credential decision.
    let target_changed = connection_patch.protocol.is_some()
        || connection_patch.base_url.is_some()
        || connection_patch.model.is_some()
        || connection_patch.auth_mode.is_some();
    let budget_changed = connection_patch.context_window_tokens.is_some()
        || connection_patch.timeout_ms.is_some()
        || connection_patch.max_output_tokens.is_some();
    if connection_patch.name.is_some() && !grant.write_connections {
        return Err(SettingsError::FieldNotGranted {
            field: "assistant.connections".into(),
        });
    }
    if (target_changed || budget_changed) && !grant.write_connection_targets {
        return Err(SettingsError::FieldNotGranted {
            field: "assistant.connectionTargets".into(),
        });
    }
    if connection_patch.credential.is_some() && !grant.write_credentials {
        return Err(SettingsError::FieldNotGranted {
            field: "assistant.credentials".into(),
        });
    }
    if target_changed
        && (connection.api_key.is_some() || connection.api_key_file.is_some())
        && connection_patch.credential.is_none()
    {
        return Err(SettingsError::CredentialTargetConflict);
    }

    if let Some(value) = connection_patch.name {
        connection.name = value;
    }
    if let Some(value) = connection_patch.protocol {
        connection.protocol = value;
    }
    if let Some(value) = connection_patch.base_url {
        connection.base_url = value;
    }
    if let Some(value) = connection_patch.model {
        connection.model = value;
    }
    if let Some(value) = connection_patch.auth_mode {
        connection.auth_mode = value;
    }
    if let Some(value) = connection_patch.context_window_tokens {
        connection.context_window_tokens = value;
    }
    if let Some(value) = connection_patch.timeout_ms {
        connection.timeout_ms = value;
    }
    if let Some(value) = connection_patch.max_output_tokens {
        connection.max_output_tokens = value;
    }
    if let Some(value) = connection_patch.credential {
        connection.api_key = None;
        connection.api_key_file = None;
        match value {
            CredentialUpdate::Direct(value) => {
                if value == "[redacted]" {
                    return Err(SettingsError::RedactedCredential);
                }
                connection.api_key = Some(value);
            }
            CredentialUpdate::File(value) => {
                if value == "[redacted]" {
                    return Err(SettingsError::RedactedCredential);
                }
                let path = Path::new(&value);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                    || !matches!(
                        path.components().next(),
                        Some(std::path::Component::Normal(name))
                            if name == OsStr::new("credentials")
                    )
                {
                    return Err(SettingsError::UnsafeCredentialFile);
                }
                connection.api_key_file = Some(value);
            }
            CredentialUpdate::Clear => {}
        }
    }
    settings.validate()
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
    fn assistant_connection_round_trips_with_a_direct_api_key() {
        let settings = AppSettings {
            assistant: AssistantSettings {
                active_connection_id: Some("local-litellm".into()),
                connections: vec![AssistantConnectionProfile {
                    id: "local-litellm".into(),
                    name: "Local LiteLLM".into(),
                    protocol: LlmProtocol::OpenAiResponses,
                    base_url: "http://127.0.0.1:4321/proxy".into(),
                    model: "assistant-default".into(),
                    auth_mode: AssistantAuthMode::Bearer,
                    api_key: Some("local-key".into()),
                    api_key_file: None,
                    context_window_tokens: Some(128_000),
                    timeout_ms: 120_000,
                    max_output_tokens: 4_096,
                }],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        };

        settings.validate().unwrap();
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            value.pointer("/assistant/writeMode"),
            Some(&serde_json::json!("automatic"))
        );
        assert_eq!(
            value.pointer("/assistant/connections/0/protocol"),
            Some(&serde_json::json!("openaiResponses"))
        );
        assert_eq!(
            value.pointer("/assistant/connections/0/baseUrl"),
            Some(&serde_json::json!("http://127.0.0.1:4321/proxy"))
        );
        assert!(serde_json::to_string(&settings)
            .unwrap()
            .contains("local-key"));
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("actual-secret"));
        assert_eq!(
            serde_json::from_value::<AppSettings>(value).unwrap(),
            settings
        );
    }

    #[test]
    fn assistant_connection_rejects_insecure_remote_http() {
        let connection = AssistantConnectionProfile {
            id: "remote".into(),
            name: "Remote".into(),
            protocol: LlmProtocol::OpenAiChatCompletions,
            base_url: "http://example.com:4000".into(),
            model: "model".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: None,
            api_key_file: Some("/tmp/assistant-key".into()),
            context_window_tokens: Some(8_192),
            timeout_ms: 1_000,
            max_output_tokens: 1_024,
        };

        assert!(matches!(
            connection.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("HTTPS")
        ));
    }

    #[test]
    fn assistant_settings_reject_missing_active_connection() {
        let settings = AssistantSettings {
            active_connection_id: Some("missing".into()),
            ..AssistantSettings::default()
        };

        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("activeConnectionId")
        ));
    }

    #[test]
    fn assistant_settings_preserve_confirm_each_and_all_protocol_names() {
        let mut settings = AssistantSettings {
            write_mode: AgentWriteMode::ConfirmEach,
            ..AssistantSettings::default()
        };
        for (index, protocol) in [
            LlmProtocol::OpenAiChatCompletions,
            LlmProtocol::AnthropicMessages,
            LlmProtocol::OpenAiResponses,
        ]
        .into_iter()
        .enumerate()
        {
            settings.connections.push(AssistantConnectionProfile {
                id: format!("route-{index}"),
                name: format!("Route {index}"),
                protocol,
                base_url: format!("http://localhost:{}/gateway", 4100 + index),
                model: "assistant-default".into(),
                auth_mode: if protocol == LlmProtocol::AnthropicMessages {
                    AssistantAuthMode::XApiKey
                } else {
                    AssistantAuthMode::Bearer
                },
                api_key: None,
                api_key_file: None,
                context_window_tokens: None,
                timeout_ms: 2_000,
                max_output_tokens: 512,
            });
        }
        settings.active_connection_id = Some("route-1".into());

        settings.validate().unwrap();
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["writeMode"], "confirm_each");
        assert_eq!(value["connections"][0]["protocol"], "openaiChatCompletions");
        assert_eq!(value["connections"][1]["protocol"], "anthropicMessages");
        assert_eq!(value["connections"][2]["protocol"], "openaiResponses");
        assert_eq!(value["connections"][1]["authMode"], "xApiKey");
    }

    #[test]
    fn assistant_settings_reject_duplicate_routes_and_invalid_budgets() {
        let profile = AssistantConnectionProfile {
            id: "route".into(),
            name: "Route".into(),
            protocol: LlmProtocol::OpenAiChatCompletions,
            base_url: "https://example.com:443/gateway".into(),
            model: "assistant-default".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: Some("route-key".into()),
            api_key_file: None,
            context_window_tokens: Some(512),
            timeout_ms: 2_000,
            max_output_tokens: 512,
        };
        assert!(matches!(
            profile.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("exceed maxOutputTokens")
        ));

        let mut valid = profile;
        valid.context_window_tokens = Some(8_192);
        let duplicated = AssistantSettings {
            active_connection_id: Some("route".into()),
            connections: vec![valid.clone(), valid],
            ..AssistantSettings::default()
        };
        assert!(matches!(
            duplicated.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("duplicated")
        ));

        let invalid_loop_budget = AssistantSettings {
            max_tool_rounds: 2,
            max_validation_retries: 3,
            ..AssistantSettings::default()
        };
        assert!(matches!(
            invalid_loop_budget.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("maxValidationRetries")
        ));
    }

    #[test]
    fn assistant_connection_accepts_api_key_or_file_but_not_both() {
        let mut connection = AssistantConnectionProfile {
            id: "route".into(),
            name: "Route".into(),
            protocol: LlmProtocol::OpenAiChatCompletions,
            base_url: "http://localhost:4000".into(),
            model: "assistant-default".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: Some("key".into()),
            api_key_file: None,
            context_window_tokens: Some(8_192),
            timeout_ms: 2_000,
            max_output_tokens: 512,
        };
        connection.validate().unwrap();
        connection.api_key = None;
        connection.api_key_file = Some("/tmp/key".into());
        connection.validate().unwrap();
        connection.api_key = Some("key".into());
        assert!(matches!(
            connection.validate(),
            Err(SettingsError::Invalid(message)) if message.contains("apiKey or apiKeyFile")
        ));
    }

    #[test]
    fn assistant_connection_reads_the_legacy_hyphenated_api_key_field() {
        let value = serde_json::json!({
            "id": "route",
            "name": "Route",
            "protocol": "openaiChatCompletions",
            "baseUrl": "http://localhost:4000",
            "model": "assistant-default",
            "authMode": "bearer",
            "api-key": "legacy-key",
            "contextWindowTokens": 8192,
            "timeoutMs": 2000,
            "maxOutputTokens": 512
        });
        let connection: AssistantConnectionProfile = serde_json::from_value(value).unwrap();
        assert_eq!(connection.api_key.as_deref(), Some("legacy-key"));
        connection.validate().unwrap();
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

    #[test]
    fn patch_rejects_a_stale_snapshot_after_a_concurrent_commit() {
        let mut service =
            SettingsService::open(Arc::new(TestRepository::new(AppSettings::default()))).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        service.patch_barrier = Some(barrier.clone());
        let revision = service.revision().unwrap();
        let patcher = service.clone();
        let patch = std::thread::spawn(move || {
            patcher.patch_if_equal(
                &revision,
                SettingsPatch {
                    max_tool_rounds: Some(4),
                    ..Default::default()
                },
                SettingsGrant::assistant_write_policy(),
            )
        });
        barrier.wait();

        let mut concurrent = service.snapshot().unwrap();
        concurrent.assistant.max_tool_rounds = 5;
        service.update(concurrent).unwrap();
        barrier.wait();

        let result = patch.join().unwrap();
        assert!(
            matches!(result, Err(SettingsError::RevisionConflict)),
            "{result:?}"
        );
        assert_eq!(service.snapshot().unwrap().assistant.max_tool_rounds, 5);
    }

    #[test]
    fn typed_patch_separates_policy_target_and_credential_permissions() {
        let profile = AssistantConnectionProfile {
            id: "route".into(),
            name: "Route".into(),
            protocol: LlmProtocol::OpenAiResponses,
            base_url: "https://example.com/gateway".into(),
            model: "model".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: Some("secret".into()),
            api_key_file: None,
            context_window_tokens: Some(8_192),
            timeout_ms: 2_000,
            max_output_tokens: 512,
        };
        let service = SettingsService::open(Arc::new(TestRepository::new(AppSettings {
            assistant: AssistantSettings {
                active_connection_id: Some("route".into()),
                connections: vec![profile],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        })))
        .unwrap();

        assert!(service
            .read_credentials(SettingsGrant::assistant_connections())
            .is_err());
        let revision = service.revision().unwrap();
        assert!(matches!(
            service.patch_if_equal(
                &revision,
                SettingsPatch {
                    connection: Some(AssistantConnectionPatch {
                        id: "route".into(),
                        base_url: Some("https://new.example.com/gateway".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                SettingsGrant::full(),
            ),
            Err(SettingsError::CredentialTargetConflict)
        ));

        let revision = service.revision().unwrap();
        service
            .patch_if_equal(
                &revision,
                SettingsPatch {
                    write_mode: Some(AgentWriteMode::ConfirmEach),
                    ..Default::default()
                },
                SettingsGrant::assistant_write_policy(),
            )
            .unwrap();
        let revision = service.revision().unwrap();
        service
            .patch_if_equal(
                &revision,
                SettingsPatch {
                    connection: Some(AssistantConnectionPatch {
                        id: "route".into(),
                        base_url: Some("https://new.example.com/gateway".into()),
                        credential: Some(CredentialUpdate::Direct("new-secret".into())),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                SettingsGrant::full(),
            )
            .unwrap();
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .assistant
                .active_connection()
                .unwrap()
                .api_key
                .as_deref(),
            Some("new-secret")
        );
    }

    #[test]
    fn credential_paths_are_narrow_and_debug_is_redacted() {
        let mut connection = AssistantConnectionProfile {
            id: "route".into(),
            name: "Route".into(),
            protocol: LlmProtocol::OpenAiResponses,
            base_url: "https://example.com".into(),
            model: "model".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: None,
            api_key_file: None,
            context_window_tokens: None,
            timeout_ms: 2_000,
            max_output_tokens: 512,
        };
        let service = SettingsService::open(Arc::new(TestRepository::new(AppSettings {
            assistant: AssistantSettings {
                connections: vec![connection.clone()],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        })))
        .unwrap();
        let revision = service.revision().unwrap();
        assert!(matches!(
            service.patch_if_equal(
                &revision,
                SettingsPatch {
                    connection: Some(AssistantConnectionPatch {
                        id: "route".into(),
                        credential: Some(CredentialUpdate::File("../../etc/passwd".into())),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                SettingsGrant::full(),
            ),
            Err(SettingsError::UnsafeCredentialFile)
        ));
        connection.api_key = Some("do-not-log".into());
        let debug = format!("{connection:?}");
        assert!(!debug.contains("do-not-log"));
    }

    #[cfg(unix)]
    #[test]
    fn opened_credential_is_verified_on_the_handle_and_never_follows_symlinks() {
        use std::io::Read;
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "meetnote-settings-credential-handle-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let settings_dir = root.join("settings");
        let key = settings_dir.join("credentials/nested/key");
        fs::create_dir_all(key.parent().unwrap()).unwrap();
        fs::write(&key, "first-key\n").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let service = SettingsService::open(Arc::new(JsonSettingsRepository::open(
            settings_dir.join("settings.json"),
        )))
        .unwrap();

        // The returned handle is the verified file: replacing the path after
        // the check cannot change what the caller reads.
        let mut opened = service
            .open_credential_file("credentials/nested/key")
            .unwrap();
        fs::remove_file(&key).unwrap();
        fs::write(&key, "swapped-key\n").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let mut content = String::new();
        opened.read_to_string(&mut content).unwrap();
        assert_eq!(content.trim(), "first-key");

        // No component below the settings directory may be a symlink, even
        // when it leads to an owner-only regular file.
        let outside = root.join("outside");
        fs::create_dir_all(outside.join("nested")).unwrap();
        fs::write(outside.join("nested/key"), "outside-key").unwrap();
        fs::set_permissions(
            outside.join("nested/key"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::remove_dir_all(settings_dir.join("credentials")).unwrap();
        std::os::unix::fs::symlink(&outside, settings_dir.join("credentials")).unwrap();
        assert!(matches!(
            service.open_credential_file("credentials/nested/key"),
            Err(SettingsError::UnsafeCredentialFile)
        ));
        fs::remove_file(settings_dir.join("credentials")).unwrap();
        fs::create_dir_all(settings_dir.join("credentials")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("nested"),
            settings_dir.join("credentials/nested"),
        )
        .unwrap();
        assert!(matches!(
            service.open_credential_file("credentials/nested/key"),
            Err(SettingsError::UnsafeCredentialFile)
        ));

        // A FIFO is rejected without blocking on open.
        let fifo = settings_dir.join("credentials/fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        fs::set_permissions(&fifo, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            service.open_credential_file("credentials/fifo"),
            Err(SettingsError::UnsafeCredentialFile)
        ));

        // A missing file is an I/O read failure, not an unsafe reference.
        assert!(matches!(
            service.open_credential_file("credentials/missing"),
            Err(SettingsError::Read { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn credential_file_resolves_only_owner_only_files_under_the_settings_directory() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-settings-credential-file-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let credentials = root.join("settings/credentials");
        fs::create_dir_all(&credentials).unwrap();
        let service = SettingsService::open(Arc::new(JsonSettingsRepository::open(
            root.join("settings/settings.json"),
        )))
        .unwrap();
        let rejects = |reference: &str| {
            matches!(
                service.open_credential_file(reference),
                Err(SettingsError::UnsafeCredentialFile)
            )
        };
        let read = |reference: &str| {
            let mut content = String::new();
            std::io::Read::read_to_string(
                &mut service.open_credential_file(reference).unwrap(),
                &mut content,
            )
            .unwrap();
            content
        };

        // A missing file is a read failure for the caller to report.
        assert!(matches!(
            service.open_credential_file("credentials/key"),
            Err(SettingsError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound
        ));
        fs::write(credentials.join("key"), "secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(credentials.join("key"), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        assert_eq!(read("credentials/key"), "secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(credentials.join("shared"), "secret").unwrap();
            fs::set_permissions(
                credentials.join("shared"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert!(rejects("credentials/shared"));
            std::os::unix::fs::symlink(credentials.join("key"), credentials.join("link")).unwrap();
            assert!(rejects("credentials/link"));
        }
        assert!(rejects(&credentials.join("key").display().to_string()));
        assert!(rejects("credentials/../settings.json"));
        assert!(rejects("secrets/key"));
        assert!(matches!(
            SettingsService::open(Arc::new(TestRepository::new(AppSettings::default())))
                .unwrap()
                .open_credential_file("credentials/key"),
            Err(SettingsError::Invalid(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn patch_field_ownership_matches_read_projection_and_binds_credentials_to_targets() {
        let route = AssistantConnectionProfile {
            id: "route".into(),
            name: "Route".into(),
            protocol: LlmProtocol::OpenAiResponses,
            base_url: "https://example.com/gateway".into(),
            model: "model".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: Some("route-secret".into()),
            api_key_file: None,
            context_window_tokens: Some(8_192),
            timeout_ms: 2_000,
            max_output_tokens: 512,
        };
        let service = SettingsService::open(Arc::new(TestRepository::new(AppSettings {
            assistant: AssistantSettings {
                active_connection_id: Some("route".into()),
                connections: vec![route],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        })))
        .unwrap();
        let connection_patch = |patch: AssistantConnectionPatch| SettingsPatch {
            connection: Some(AssistantConnectionPatch {
                id: "route".into(),
                ..patch
            }),
            ..Default::default()
        };
        let budget = || AssistantConnectionPatch {
            timeout_ms: Some(5_000),
            max_output_tokens: Some(1_024),
            context_window_tokens: Some(Some(16_384)),
            ..Default::default()
        };

        // Budget fields are read through connectionTargets, so connections alone
        // cannot write them.
        let before = service.snapshot().unwrap();
        let denied = service.patch_if_equal(
            &service.revision().unwrap(),
            connection_patch(budget()),
            SettingsGrant::assistant_connections(),
        );
        assert!(
            matches!(
                &denied,
                Err(SettingsError::FieldNotGranted { field }) if field == "assistant.connectionTargets"
            ),
            "{denied:?}"
        );
        assert_eq!(service.snapshot().unwrap(), before);

        // The target owner can change budgets without forwarding or replacing
        // the credential, because budgets do not redirect where it is sent.
        service
            .patch_if_equal(
                &service.revision().unwrap(),
                connection_patch(budget()),
                SettingsGrant::assistant_connection_targets(),
            )
            .unwrap();
        let connection = service.snapshot().unwrap().assistant.connections[0].clone();
        assert_eq!(connection.timeout_ms, 5_000);
        assert_eq!(connection.max_output_tokens, 1_024);
        assert_eq!(connection.context_window_tokens, Some(16_384));
        assert_eq!(connection.api_key.as_deref(), Some("route-secret"));

        // A target owner without credential authority cannot move an existing
        // credential to a new endpoint, with or without an explicit credential.
        let before = service.snapshot().unwrap();
        for patch in [
            AssistantConnectionPatch {
                base_url: Some("https://moved.example.com/gateway".into()),
                ..Default::default()
            },
            AssistantConnectionPatch {
                base_url: Some("https://moved.example.com/gateway".into()),
                credential: Some(CredentialUpdate::Direct("moved-secret".into())),
                ..Default::default()
            },
        ] {
            let result = service.patch_if_equal(
                &service.revision().unwrap(),
                connection_patch(patch),
                SettingsGrant::assistant_connection_targets(),
            );
            assert!(
                matches!(
                    result,
                    Err(SettingsError::CredentialTargetConflict)
                        | Err(SettingsError::FieldNotGranted { .. })
                ),
                "{result:?}"
            );
            assert_eq!(service.snapshot().unwrap(), before);
        }

        // Connection display metadata stays with connections.
        service
            .patch_if_equal(
                &service.revision().unwrap(),
                connection_patch(AssistantConnectionPatch {
                    name: Some("Renamed".into()),
                    ..Default::default()
                }),
                SettingsGrant::assistant_connections(),
            )
            .unwrap();
        assert_eq!(
            service.snapshot().unwrap().assistant.connections[0].name,
            "Renamed"
        );

        // Target plus credential authority may move the endpoint only with an
        // explicit credential decision.
        service
            .patch_if_equal(
                &service.revision().unwrap(),
                connection_patch(AssistantConnectionPatch {
                    base_url: Some("https://moved.example.com/gateway".into()),
                    credential: Some(CredentialUpdate::Clear),
                    ..Default::default()
                }),
                SettingsGrant::assistant_connection_targets().with_credentials(),
            )
            .unwrap();
        let connection = service.snapshot().unwrap().assistant.connections[0].clone();
        assert_eq!(connection.base_url, "https://moved.example.com/gateway");
        assert_eq!(connection.api_key, None);
        assert_eq!(connection.api_key_file, None);
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
                "schemaVersion": "meetnote.settings.v1",
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
                "schemaVersion": "meetnote.settings.v1",
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

    /// Creates connection `created`, with `credential` when it is given.
    fn creation(credential: Option<CredentialUpdate>) -> SettingsPatch {
        SettingsPatch {
            new_connection: Some(AssistantConnectionProfile {
                id: "created".into(),
                name: "Created".into(),
                protocol: LlmProtocol::OpenAiResponses,
                base_url: "https://example.com/gateway".into(),
                model: "model".into(),
                auth_mode: AssistantAuthMode::Bearer,
                api_key: None,
                api_key_file: None,
                context_window_tokens: Some(8_192),
                timeout_ms: 2_000,
                max_output_tokens: 512,
            }),
            connection: credential.map(|credential| AssistantConnectionPatch {
                id: "created".into(),
                credential: Some(credential),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn creating_a_connection_with_a_stale_revision_writes_nothing() {
        let repository = Arc::new(TestRepository::new(AppSettings::default()));
        let service = SettingsService::open(repository.clone()).unwrap();
        let loaded_revision = service.revision().unwrap();
        service
            .patch_if_equal(
                &loaded_revision,
                SettingsPatch {
                    write_mode: Some(AgentWriteMode::ConfirmEach),
                    ..Default::default()
                },
                SettingsGrant::full(),
            )
            .unwrap();
        let before = repository.load().unwrap();

        let result = service.patch_if_equal(
            &loaded_revision,
            creation(Some(CredentialUpdate::Direct("created-secret".into()))),
            SettingsGrant::full(),
        );
        assert!(
            matches!(result, Err(SettingsError::RevisionConflict)),
            "{result:?}"
        );
        assert_eq!(repository.load().unwrap(), before);
        assert_eq!(service.snapshot().unwrap(), before);
    }

    #[test]
    fn creating_a_connection_persists_the_profile_and_its_credential() {
        for (credential, api_key, api_key_file) in [
            (
                Some(CredentialUpdate::Direct("created-secret".into())),
                Some("created-secret"),
                None,
            ),
            (
                Some(CredentialUpdate::File("credentials/created".into())),
                None,
                Some("credentials/created"),
            ),
            (None, None, None),
        ] {
            let repository = Arc::new(TestRepository::new(AppSettings::default()));
            let service = SettingsService::open(repository.clone()).unwrap();
            let saved = service
                .patch_if_equal(
                    &service.revision().unwrap(),
                    SettingsPatch {
                        active_connection_id: Some(Some("created".into())),
                        ..creation(credential)
                    },
                    SettingsGrant::full(),
                )
                .unwrap();

            let persisted = repository.load().unwrap();
            assert_eq!(persisted, saved);
            assert_eq!(service.snapshot().unwrap(), saved);
            assert_eq!(persisted.assistant.connections.len(), 1);
            let created = persisted.assistant.active_connection().unwrap();
            assert_eq!(created.id, "created");
            assert_eq!(created.name, "Created");
            assert_eq!(created.base_url, "https://example.com/gateway");
            assert_eq!(created.api_key.as_deref(), api_key);
            assert_eq!(created.api_key_file.as_deref(), api_key_file);
        }
    }

    #[test]
    fn a_new_connection_credential_breaks_a_rule_with_the_edit_error() {
        let service = SettingsService::open(Arc::new(TestRepository::new(AppSettings {
            assistant: AssistantSettings {
                connections: vec![AssistantConnectionProfile {
                    id: "route".into(),
                    name: "Route".into(),
                    protocol: LlmProtocol::OpenAiResponses,
                    base_url: "https://example.com/gateway".into(),
                    model: "model".into(),
                    auth_mode: AssistantAuthMode::Bearer,
                    api_key: None,
                    api_key_file: None,
                    context_window_tokens: None,
                    timeout_ms: 2_000,
                    max_output_tokens: 512,
                }],
                ..AssistantSettings::default()
            },
            ..AppSettings::default()
        })))
        .unwrap();
        let before = service.snapshot().unwrap();
        let revision = service.revision().unwrap();

        for credential in [
            CredentialUpdate::Direct("[redacted]".into()),
            CredentialUpdate::Direct("  ".into()),
            CredentialUpdate::File("[redacted]".into()),
            CredentialUpdate::File("/etc/meetnote-key".into()),
            CredentialUpdate::File("credentials/../../etc/passwd".into()),
            CredentialUpdate::File("secrets/key".into()),
        ] {
            let edited = service
                .patch_if_equal(
                    &revision,
                    SettingsPatch {
                        connection: Some(AssistantConnectionPatch {
                            id: "route".into(),
                            credential: Some(credential.clone()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    SettingsGrant::full(),
                )
                .unwrap_err();
            let created = service
                .patch_if_equal(
                    &revision,
                    creation(Some(credential.clone())),
                    SettingsGrant::full(),
                )
                .unwrap_err();

            assert_eq!(
                std::mem::discriminant(&created),
                std::mem::discriminant(&edited),
                "{credential:?}: {created:?} / {edited:?}"
            );
            assert_eq!(created.to_string(), edited.to_string());
            assert_eq!(service.snapshot().unwrap(), before);
        }
    }

    #[test]
    fn creating_a_connection_needs_the_grant_of_every_section_it_writes() {
        let service =
            SettingsService::open(Arc::new(TestRepository::new(AppSettings::default()))).unwrap();
        let create = |patch: SettingsPatch, grant: SettingsGrant| {
            service.patch_if_equal(&service.revision().unwrap(), patch, grant)
        };
        let before = service.snapshot().unwrap();
        for (grant, field) in [
            (
                SettingsGrant::assistant_connection_targets().with_credentials(),
                "assistant.connections",
            ),
            (
                SettingsGrant::assistant_connections().with_credentials(),
                "assistant.connectionTargets",
            ),
            (
                SettingsGrant::assistant_connections().with_connection_targets(),
                "assistant.credentials",
            ),
        ] {
            let result = create(
                creation(Some(CredentialUpdate::Direct("created-secret".into()))),
                grant,
            );
            assert!(
                matches!(&result, Err(SettingsError::FieldNotGranted { field: denied }) if denied == field),
                "{result:?}"
            );
            assert_eq!(service.snapshot().unwrap(), before);
        }

        // Without a credential, connections and targets are enough.
        create(
            creation(None),
            SettingsGrant::assistant_connections().with_connection_targets(),
        )
        .unwrap();
        let created = service.snapshot().unwrap();
        let duplicate = create(creation(None), SettingsGrant::full());
        assert!(
            matches!(&duplicate, Err(SettingsError::Invalid(message)) if message.contains("duplicated")),
            "{duplicate:?}"
        );

        // A profile cannot carry a credential past the credential rules.
        let mut inline = creation(None);
        let profile = inline.new_connection.as_mut().unwrap();
        profile.id = "inline".into();
        profile.api_key = Some("inline-secret".into());
        let inline = create(inline, SettingsGrant::full());
        assert!(
            matches!(&inline, Err(SettingsError::Invalid(message)) if message.contains("connection.credential")),
            "{inline:?}"
        );
        assert_eq!(service.snapshot().unwrap(), created);
    }
}
