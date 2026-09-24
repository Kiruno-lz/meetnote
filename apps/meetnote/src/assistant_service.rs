use application::{
    agent_error_from_llm, AgentCancellation, AgentProgress, AgentProgressStage, AgentRunResult,
    AgentRuntime, AgentRuntimeConfig, AgentTask, DocumentService, WorkspaceAccessPolicy,
    WorkspaceCapabilityPort,
};
use contracts::{
    AgentActor, AgentDocumentRevision, AgentErrorCode, AgentMutationReceipt, AgentPendingMutation,
    AgentToolError, AgentToolResult,
};
use domain::DocumentId;
use llm::{
    AuthMode, ConnectionConfig, Credential, LiteLlmProvider, LlmProtocol as ProviderProtocol,
    LlmProvider,
};
use runtime::{JobRuntime, Progress};
use settings::{
    AppSettings, AssistantAuthMode, AssistantConnectionProfile, AssistantSettings, LlmProtocol,
    SettingsError, SettingsService,
};
use std::io::Read;
use std::sync::Arc;
use ui::assistant_ui::{
    AssistantDocumentChange, AssistantFieldDiff, AssistantMutationSummary, AssistantPendingReview,
    AssistantProgress, AssistantProgressHandler, AssistantRangePreview, AssistantRunDiagnostics,
    AssistantRunRequest, AssistantRunResult, AssistantTreeChange, AssistantUiError,
    AssistantUiService, AssistantUsage, ConnectionDiagnostic, ConnectionProbeKind,
};

#[derive(Clone)]
pub struct DesktopAssistantService {
    settings: SettingsService,
    documents: DocumentService,
    jobs: Arc<JobRuntime>,
}

impl DesktopAssistantService {
    pub fn new(settings: SettingsService, documents: DocumentService) -> Self {
        Self {
            settings,
            documents,
            jobs: Arc::new(JobRuntime::default()),
        }
    }

    fn settings_snapshot(&self) -> Result<AppSettings, AssistantUiError> {
        self.settings
            .snapshot()
            .map_err(|error| settings_ui_error(error.to_string()))
    }

    fn provider(
        &self,
        profile: &AssistantConnectionProfile,
    ) -> Result<(LiteLlmProvider, Option<String>), AssistantUiError> {
        profile
            .validate()
            .map_err(|error| settings_ui_error(error.to_string()))?;
        let owned_secret = self.resolve_api_key(profile)?;
        let provider = LiteLlmProvider::new(
            connection_config(profile),
            owned_secret.clone().map(Credential::new),
        )
        .map_err(|error| {
            redact_ui_error(
                ui_error(agent_error_from_llm(error)),
                owned_secret.as_deref(),
            )
        })?;
        Ok((provider, owned_secret))
    }

    /// `apiKeyFile` is opened only through Settings, and the key is read from
    /// the handle Settings verified; the key itself never enters an error.
    fn resolve_api_key(
        &self,
        profile: &AssistantConnectionProfile,
    ) -> Result<Option<String>, AssistantUiError> {
        if let Some(api_key) = profile.api_key.as_deref() {
            return Ok(Some(api_key.to_owned()));
        }
        let Some(reference) = profile.api_key_file.as_deref() else {
            return Ok(None);
        };
        let read_failed = |message: String| AssistantUiError {
            code: "ASSISTANT_API_KEY_FILE_READ_FAILED".into(),
            message,
            retryable: false,
        };
        let opened = self.settings.open_credential_file(reference);
        let mut file = opened.map_err(|error| match error {
            SettingsError::Read { .. } => read_failed(error.to_string()),
            error => settings_ui_error(error.to_string()),
        })?;
        let mut key = String::new();
        file.read_to_string(&mut key).map_err(|error| {
            read_failed(format!("failed to read apiKeyFile {reference}: {error}"))
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(AssistantUiError {
                code: "ASSISTANT_API_KEY_FILE_EMPTY".into(),
                message: format!("apiKeyFile {reference} is empty"),
                retryable: false,
            });
        }
        Ok(Some(key.to_owned()))
    }

    /// Provider for the active connection, built exactly as desktop runs build
    /// it. `None` means no active connection is configured.
    pub(crate) fn active_provider(
        &self,
    ) -> Result<Option<(LiteLlmProvider, Option<String>)>, AssistantUiError> {
        let Some(profile) = self
            .settings_snapshot()?
            .assistant
            .active_connection()
            .cloned()
        else {
            return Ok(None);
        };
        self.provider(&profile).map(Some)
    }
}

fn connection_config(profile: &AssistantConnectionProfile) -> ConnectionConfig {
    ConnectionConfig {
        id: profile.id.clone(),
        protocol: match profile.protocol {
            LlmProtocol::OpenAiChatCompletions => ProviderProtocol::OpenAiChatCompletions,
            LlmProtocol::AnthropicMessages => ProviderProtocol::AnthropicMessages,
            LlmProtocol::OpenAiResponses => ProviderProtocol::OpenAiResponses,
        },
        base_url: profile.base_url.clone(),
        model: profile.model.clone(),
        auth_mode: match profile.auth_mode {
            AssistantAuthMode::Bearer => AuthMode::Bearer,
            AssistantAuthMode::XApiKey => AuthMode::XApiKey,
        },
        timeout_ms: profile.timeout_ms,
        context_window_tokens: profile.context_window_tokens,
        max_output_tokens: profile.max_output_tokens,
    }
}

fn ui_error(error: AgentToolError) -> AssistantUiError {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "PROVIDER_RESPONSE_INVALID".into());
    AssistantUiError {
        code,
        message: error.message,
        retryable: error.retryable,
    }
}

fn redact_ui_error(mut error: AssistantUiError, secret: Option<&str>) -> AssistantUiError {
    redact_secret(&mut error.message, secret);
    error
}

pub(crate) fn redact_secret(message: &mut String, secret: Option<&str>) {
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        *message = message.replace(secret, "[REDACTED]");
    }
}

fn settings_ui_error(message: String) -> AssistantUiError {
    AssistantUiError {
        code: "ASSISTANT_SETTINGS_INVALID".into(),
        message,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::{AgentRunDiagnostics, AgentUsage, DocumentRepository, DocumentService};
    use contracts::{
        AgentErrorCode, AgentFieldDiff, AgentMutationProvenance, AgentMutationReceipt,
        AgentRangePreview, AgentToolError, AgentTreeChange,
    };
    use domain::DocumentState;
    use persistence::InMemoryDocumentRepository;
    use settings::{SettingsError, SettingsRepository};
    use std::fs;
    use std::sync::Mutex;

    struct MemorySettingsRepository(Mutex<AppSettings>);

    impl SettingsRepository for MemorySettingsRepository {
        fn load(&self) -> Result<AppSettings, SettingsError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
            *self.0.lock().unwrap() = settings.clone();
            Ok(())
        }
    }

    fn profile() -> AssistantConnectionProfile {
        AssistantConnectionProfile {
            id: "local".into(),
            name: "Local LiteLLM".into(),
            protocol: LlmProtocol::OpenAiChatCompletions,
            base_url: "http://127.0.0.1:4000".into(),
            model: "assistant-default".into(),
            auth_mode: AssistantAuthMode::Bearer,
            api_key: Some("private-token".into()),
            api_key_file: None,
            context_window_tokens: Some(8_192),
            timeout_ms: 1_000,
            max_output_tokens: 512,
        }
    }

    fn service() -> (DesktopAssistantService, Arc<InMemoryDocumentRepository>) {
        let settings = SettingsService::open(Arc::new(MemorySettingsRepository(Mutex::new(
            AppSettings::default(),
        ))))
        .unwrap();
        let documents = Arc::new(InMemoryDocumentRepository::default());
        (
            DesktopAssistantService::new(settings, DocumentService::new(documents.clone())),
            documents,
        )
    }

    #[test]
    fn resolves_api_key_file_from_the_settings_directory() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-assistant-key-root-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let key_path = root.join("settings/credentials/key");
        fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        fs::write(&key_path, "file-token\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut profile = profile();
        profile.api_key = None;
        profile.api_key_file = Some("credentials/key".into());
        let mut settings = AppSettings::default();
        settings.assistant.active_connection_id = Some(profile.id.clone());
        settings.assistant.connections = vec![profile];
        let repository =
            settings::JsonSettingsRepository::open(root.join("settings/settings.json"));
        repository.save(&settings).unwrap();
        let service = DesktopAssistantService::new(
            SettingsService::open(Arc::new(repository)).unwrap(),
            DocumentService::new(Arc::new(InMemoryDocumentRepository::default())),
        );

        let (_, secret) = service
            .active_provider()
            .unwrap_or_else(|error| panic!("settings-validated key file must resolve: {error:?}"))
            .expect("active connection is configured");
        assert_eq!(secret.as_deref(), Some("file-token"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_an_absolute_api_key_file_before_reading_it() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-assistant-absolute-key-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let key_path = root.join("key");
        fs::create_dir_all(&root).unwrap();
        fs::write(&key_path, "file-token\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let service = DesktopAssistantService::new(
            SettingsService::open(Arc::new(settings::JsonSettingsRepository::open(
                root.join("settings/settings.json"),
            )))
            .unwrap(),
            DocumentService::new(Arc::new(InMemoryDocumentRepository::default())),
        );
        let mut profile = profile();
        profile.api_key = None;
        profile.api_key_file = Some(key_path.display().to_string());

        let error = service.resolve_api_key(&profile).unwrap_err();
        assert_eq!(error.code, "ASSISTANT_SETTINGS_INVALID");
        assert!(!error.message.contains("file-token"));
        fs::remove_dir_all(root).unwrap();
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
