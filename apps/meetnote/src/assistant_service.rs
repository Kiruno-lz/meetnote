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
    SettingsError, SettingsGrant, SettingsPatch, SettingsRepository, SettingsService,
};
use std::io::Read;
use std::sync::{Arc, Mutex};
use ui::assistant_ui::{
    AssistantDocumentChange, AssistantFieldDiff, AssistantMutationSummary, AssistantPendingReview,
    AssistantProgress, AssistantProgressHandler, AssistantRangePreview, AssistantRunDiagnostics,
    AssistantRunRequest, AssistantRunResult, AssistantSettingsSave, AssistantTreeChange,
    AssistantUiError, AssistantUiService, AssistantUsage, ConnectionDiagnostic,
    ConnectionProbeKind,
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

    fn gateway(
        &self,
        document_ids: &[String],
        write_mode: contracts::AgentWriteMode,
    ) -> Result<WorkspaceCapabilityPort, AssistantUiError> {
        if document_ids.is_empty() || document_ids.iter().any(|id| id.trim().is_empty()) {
            return Err(ui_error(AgentToolError::new(
                AgentErrorCode::DocumentAccessDenied,
                "Assistant requires an explicit non-empty document grant",
                false,
            )));
        }
        Ok(WorkspaceCapabilityPort::new(
            self.documents.clone(),
            WorkspaceAccessPolicy::documents(
                document_ids
                    .iter()
                    .cloned()
                    .map(DocumentId::new)
                    .collect::<Vec<_>>(),
            ),
        )
        .with_actor(AgentActor::Assistant)
        .with_write_mode(write_mode))
    }

    fn settings_snapshot(&self) -> Result<AppSettings, AssistantUiError> {
        self.settings
            .snapshot()
            .map_err(|error| settings_ui_error(error.to_string()))
    }

    fn configured_profile(
        &self,
        connection_id: &str,
    ) -> Result<(AssistantSettings, AssistantConnectionProfile), AssistantUiError> {
        let assistant = self.settings_snapshot()?.assistant;
        if !assistant.enabled {
            return Err(AssistantUiError {
                code: "ASSISTANT_DISABLED".into(),
                message: "Assistant is disabled in Settings".into(),
                retryable: false,
            });
        }
        if assistant.active_connection_id.as_deref() != Some(connection_id) {
            return Err(ui_error(AgentToolError::new(
                AgentErrorCode::ConnectionNotConfigured,
                "requested connection is not the active Assistant connection",
                false,
            )));
        }
        let profile = assistant
            .connections
            .iter()
            .find(|profile| profile.id == connection_id)
            .cloned()
            .ok_or_else(|| {
                ui_error(AgentToolError::new(
                    AgentErrorCode::ConnectionNotConfigured,
                    "active Assistant connection was not found",
                    false,
                ))
            })?;
        Ok((assistant, profile))
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

    /// Adds `profile` without a credential, applies `patch` (write policy and
    /// credential) to that candidate under the same typed rules as
    /// `patch_if_equal`, and commits the result with one compare-and-swap.
    fn create_connection(
        &self,
        expected_revision: &str,
        profile: AssistantConnectionProfile,
        patch: SettingsPatch,
    ) -> Result<AppSettings, SettingsError> {
        let current = self.settings.snapshot()?;
        if SettingsService::revision_of(&current) != expected_revision {
            return Err(SettingsError::RevisionConflict);
        }
        let mut candidate = current.clone();
        candidate
            .assistant
            .connections
            .push(AssistantConnectionProfile {
                api_key: None,
                api_key_file: None,
                ..profile
            });
        let revision = SettingsService::revision_of(&candidate);
        let next = SettingsService::open(Arc::new(CandidateSettings(Mutex::new(candidate))))?
            .patch_if_equal(&revision, patch, SettingsGrant::full())?;
        self.settings.update_if_equal(&current, next)
    }

    fn verify_source_revisions(
        &self,
        request: &AssistantRunRequest,
    ) -> Result<Vec<String>, AssistantUiError> {
        let mut ids = Vec::with_capacity(request.authorized_documents.len());
        for document in &request.authorized_documents {
            let id = DocumentId::new(document.document_id.clone());
            let current = self
                .documents
                .get(&id)
                .map_err(|message| {
                    ui_error(AgentToolError::new(
                        AgentErrorCode::WriteTransactionFailed,
                        message,
                        false,
                    ))
                })?
                .ok_or_else(|| {
                    let mut error = AgentToolError::new(
                        AgentErrorCode::DocumentNotFound,
                        "authorized document no longer exists",
                        true,
                    );
                    error.document_id = Some(document.document_id.clone());
                    ui_error(error)
                })?;
            if current.revision() != document.revision {
                let mut error = AgentToolError::new(
                    AgentErrorCode::DocumentRevisionConflict,
                    "authorized document changed before the Assistant task started",
                    true,
                );
                error.document_id = Some(document.document_id.clone());
                error.expected_revision = Some(document.revision);
                error.actual_revision = Some(current.revision());
                return Err(ui_error(error));
            }
            ids.push(document.document_id.clone());
        }
        Ok(ids)
    }

    fn run_inner(
        &self,
        request: AssistantRunRequest,
        cancellation: AgentCancellation,
        progress_handler: AssistantProgressHandler,
    ) -> Result<AssistantRunResult, AssistantUiError> {
        let document_ids = self.verify_source_revisions(&request)?;
        let (assistant, profile) = self.configured_profile(&request.connection_id)?;
        let (provider, secret) = self.provider(&profile)?;
        let gateway = self.gateway(&document_ids, assistant.write_mode)?;
        let runtime = AgentRuntime::new(
            Arc::new(provider),
            gateway,
            AgentRuntimeConfig {
                max_tool_rounds: request.max_tool_rounds.min(assistant.max_tool_rounds),
                max_validation_retries: request
                    .max_validation_retries
                    .min(assistant.max_validation_retries),
                max_output_tokens: Some(profile.max_output_tokens),
                context_window_tokens: profile.context_window_tokens,
            },
        );
        let result = runtime.run_with_progress(
            &AgentTask {
                task_id: request.request_id,
                scene: request.scene,
                skill: request.skill,
                prompt: request.prompt,
                authorized_document_ids: document_ids,
                projection: request.projection,
            },
            &cancellation,
            |progress| progress_handler(map_progress(progress)),
        );
        map_run_result(result).map_err(|error| redact_ui_error(error, secret.as_deref()))
    }
}

/// Settings held only in memory: the candidate a new connection is patched in
/// before it is committed.
struct CandidateSettings(Mutex<AppSettings>);

impl SettingsRepository for CandidateSettings {
    fn load(&self) -> Result<AppSettings, SettingsError> {
        self.0
            .lock()
            .map(|settings| settings.clone())
            .map_err(|_| SettingsError::LockPoisoned)
    }

    fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        *self.0.lock().map_err(|_| SettingsError::LockPoisoned)? = settings.clone();
        Ok(())
    }
}

impl AssistantUiService for DesktopAssistantService {
    fn save_assistant_settings(
        &self,
        expected_revision: String,
        save: AssistantSettingsSave,
    ) -> Result<AppSettings, AssistantUiError> {
        match save.new_connection {
            None => {
                self.settings
                    .patch_if_equal(&expected_revision, save.patch, SettingsGrant::full())
            }
            Some(profile) => self.create_connection(&expected_revision, profile, save.patch),
        }
        .map_err(|error| AssistantUiError::from_settings(&error))
    }

    fn probe_connection(
        &self,
        profile: AssistantConnectionProfile,
        kind: ConnectionProbeKind,
        cancellation: AgentCancellation,
    ) -> Result<ConnectionDiagnostic, AssistantUiError> {
        let (provider, secret) = self.provider(&profile)?;
        let (job, _events) = self.jobs.create_job();
        job.started();
        cancellation.link_job(&job.cancellation_token());
        let secret_for_job = secret.clone();
        let profile_for_job = profile.clone();
        let result: Result<ConnectionDiagnostic, AssistantUiError> =
            futures::executor::block_on(job.spawn_blocking(move || {
                let descriptor = provider.descriptor();
                let result = match kind {
                    ConnectionProbeKind::Readiness => {
                        provider.readiness(cancellation.llm()).map_err(|error| {
                            redact_ui_error(
                                ui_error(agent_error_from_llm(error)),
                                secret_for_job.as_deref(),
                            )
                        })?;
                        ConnectionDiagnostic {
                            kind,
                            ready: true,
                            local: descriptor.local,
                            models: None,
                            text: None,
                            client_tools: None,
                            requested_model: profile_for_job.model.clone(),
                            resolved_model: None,
                            context_window_tokens: profile_for_job.context_window_tokens,
                            message: "LiteLLM readiness endpoint is reachable".into(),
                        }
                    }
                    ConnectionProbeKind::Model => {
                        let probe = provider.probe(cancellation.llm()).map_err(|error| {
                            redact_ui_error(
                                ui_error(agent_error_from_llm(error)),
                                secret_for_job.as_deref(),
                            )
                        })?;
                        ConnectionDiagnostic {
                            kind,
                            ready: probe.ready,
                            local: descriptor.local,
                            models: probe.models,
                            text: Some(probe.capabilities.text),
                            client_tools: Some(probe.capabilities.client_tools),
                            requested_model: probe.requested_model,
                            resolved_model: probe.resolved_model,
                            context_window_tokens: probe.context_window_tokens,
                            message: "Text and client-tool capability probe completed".into(),
                        }
                    }
                };
                Ok(result)
            }));
        match &result {
            Ok(_) => job.completed(),
            Err(error) if error.code == "TASK_CANCELLED" => job.cancel(),
            Err(error) => job.failed(error.to_string()),
        }
        result
    }

    fn run(
        &self,
        request: AssistantRunRequest,
        cancellation: AgentCancellation,
    ) -> Result<AssistantRunResult, AssistantUiError> {
        self.run_with_progress(request, cancellation, Arc::new(|_| {}))
    }

    fn run_with_progress(
        &self,
        request: AssistantRunRequest,
        cancellation: AgentCancellation,
        progress_handler: AssistantProgressHandler,
    ) -> Result<AssistantRunResult, AssistantUiError> {
        let (job, _events) = self.jobs.create_job();
        job.started();
        cancellation.link_job(&job.cancellation_token());
        job.report(Progress {
            completed: 0,
            total: None,
        });
        let service = self.clone();
        let job_for_worker = job.clone();
        let result = futures::executor::block_on(job.spawn_blocking(move || {
            if job_for_worker.is_cancelled() {
                return Err(AssistantUiError {
                    code: "TASK_CANCELLED".into(),
                    message: "Assistant task was cancelled before execution".into(),
                    retryable: true,
                });
            }
            service.run_inner(request, cancellation, progress_handler)
        }));
        match &result {
            Ok(_) => {
                job.report(Progress {
                    completed: 1,
                    total: Some(1),
                });
                job.completed();
            }
            Err(error) if error.code == "TASK_CANCELLED" => job.cancel(),
            Err(error) => job.failed(error.to_string()),
        }
        result
    }

    fn confirm(
        &self,
        confirmation_id: String,
        authorized_document_ids: Vec<String>,
    ) -> Result<AssistantRunResult, AssistantUiError> {
        let assistant = self.settings_snapshot()?.assistant;
        let response = self
            .gateway(&authorized_document_ids, assistant.write_mode)?
            .confirm(&confirmation_id, &format!("confirm:{confirmation_id}"));
        map_tool_response(response)
    }

    fn reject(
        &self,
        confirmation_id: String,
        authorized_document_ids: Vec<String>,
    ) -> Result<(), AssistantUiError> {
        let assistant = self.settings_snapshot()?.assistant;
        let response = self
            .gateway(&authorized_document_ids, assistant.write_mode)?
            .reject(&confirmation_id, &format!("reject:{confirmation_id}"));
        match response.error.as_ref() {
            Some(error) if error.code == AgentErrorCode::WritePolicyRejected => Ok(()),
            Some(error) => Err(ui_error(error.clone())),
            None if response.ok => Ok(()),
            None => Err(AssistantUiError::unavailable(
                "reject returned neither a result nor an error",
            )),
        }
    }

    fn undo(
        &self,
        undo_token: String,
        authorized_document_ids: Vec<String>,
    ) -> Result<AssistantMutationSummary, AssistantUiError> {
        let assistant = self.settings_snapshot()?.assistant;
        let response = self
            .gateway(&authorized_document_ids, assistant.write_mode)?
            .undo(&undo_token, &format!("undo:{undo_token}"));
        if let Some(AgentToolResult::Mutation(receipt)) = response.result.as_ref() {
            if response.ok {
                return Ok(map_mutation(receipt.clone()));
            }
        }
        Err(ui_error(response.error.clone().unwrap_or_else(|| {
            AgentToolError::new(
                AgentErrorCode::WriteTransactionFailed,
                "undo returned an unexpected response",
                false,
            )
        })))
    }
}

fn map_progress(progress: &AgentProgress) -> AssistantProgress {
    let stage = match progress.stage {
        AgentProgressStage::AssemblingContext => "assembling_context",
        AgentProgressStage::RequestingModel => "requesting_model",
        AgentProgressStage::ExecutingTool => "executing_tool",
        AgentProgressStage::Completed => "completed",
        AgentProgressStage::Failed => "failed",
        AgentProgressStage::Cancelled => "cancelled",
    };
    AssistantProgress {
        stage: stage.into(),
        round: progress.round,
        tool_name: progress.tool_name.clone(),
        completed: progress.completed,
        total: progress.total,
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

fn map_run_result(result: AgentRunResult) -> Result<AssistantRunResult, AssistantUiError> {
    if let Some(error) = result.error.as_ref() {
        if result.mutation.is_some() || !result.mutations.is_empty() || result.pending.is_some() {
            let warning = ui_error(error.clone());
            let mut partial = map_run_result_without_error(result);
            partial.warning = Some(warning.clone());
            partial.warnings.push(warning);
            return Ok(partial);
        }
        return Err(ui_error(error.clone()));
    }
    Ok(map_run_result_without_error(result))
}

fn map_run_result_without_error(result: AgentRunResult) -> AssistantRunResult {
    let requested_model = result.diagnostics.requested_model.clone();
    let resolved_model = result.diagnostics.resolved_model.clone();
    let pending = result.pending.map(map_pending);
    let diagnostics = AssistantRunDiagnostics {
        path: match result.diagnostics.path {
            application::AgentRunPath::ShortDocument => "short_document",
        }
        .into(),
        tool_rounds: result.diagnostics.tool_rounds,
        validation_retries: result.diagnostics.validation_retries,
        input_segments: result.diagnostics.input_segments,
        input_bytes: result.diagnostics.input_bytes,
        elapsed_ms: result.diagnostics.elapsed_ms,
    };
    AssistantRunResult {
        answer: result.answer.map(|answer| answer.text),
        warning: None,
        warnings: Vec::new(),
        evidence_segment_ids: result.evidence_segment_ids,
        usage: result.usage.map(|usage| AssistantUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cost_microusd: usage.cost_microusd,
        }),
        requested_model,
        resolved_model,
        mutation: result.mutation.map(map_mutation),
        mutations: result.mutations.into_iter().map(map_mutation).collect(),
        pending: pending.clone(),
        pending_reviews: pending.into_iter().collect(),
        diagnostics: Some(diagnostics),
    }
}

fn map_tool_response(
    response: contracts::CapabilityResponse,
) -> Result<AssistantRunResult, AssistantUiError> {
    if let Some(error) = response.error.as_ref() {
        return Err(ui_error(error.clone()));
    }
    match response.result.as_ref() {
        Some(AgentToolResult::Mutation(receipt)) => Ok(AssistantRunResult {
            answer: None,
            warning: None,
            warnings: Vec::new(),
            evidence_segment_ids: Vec::new(),
            usage: None,
            requested_model: None,
            resolved_model: None,
            mutation: Some(map_mutation(receipt.clone())),
            mutations: vec![map_mutation(receipt.clone())],
            pending: None,
            pending_reviews: Vec::new(),
            diagnostics: None,
        }),
        _ => Err(AssistantUiError::unavailable(
            "confirmation returned an unexpected response",
        )),
    }
}

fn map_mutation(receipt: AgentMutationReceipt) -> AssistantMutationSummary {
    AssistantMutationSummary {
        mutation_id: receipt.mutation_id,
        affected_documents: receipt
            .affected_documents
            .into_iter()
            .map(map_revision)
            .collect(),
        undo_token: receipt.undo_token,
        uncovered_segment_ids: receipt.uncovered_segment_ids,
    }
}

fn map_revision(revision: AgentDocumentRevision) -> AssistantDocumentChange {
    AssistantDocumentChange {
        document_id: revision.document_id,
        previous_revision: revision.previous_revision,
        revision: revision.revision,
        deleted: revision.deleted,
    }
}

fn map_pending(pending: AgentPendingMutation) -> AssistantPendingReview {
    AssistantPendingReview {
        confirmation_id: pending.confirmation_id,
        expires_at_ms: pending.expires_at_ms,
        affected_document_ids: pending.affected_document_ids,
        field_diffs: pending
            .field_diffs
            .into_iter()
            .map(|diff| AssistantFieldDiff {
                document_id: diff.document_id,
                field: diff.field,
                before: diff.before,
                after: diff.after,
            })
            .collect(),
        tree_changes: pending
            .tree_changes
            .into_iter()
            .map(|change| AssistantTreeChange {
                document_id: change.document_id,
                previous_parent_id: change.previous_parent_id,
                new_parent_id: change.new_parent_id,
                previous_sibling_order: change.previous_sibling_order,
                new_sibling_order: change.new_sibling_order,
            })
            .collect(),
        ranges: pending
            .ranges
            .into_iter()
            .map(|range| AssistantRangePreview {
                document_id: range.document_id,
                start_segment_id: range.start_segment_id,
                end_segment_id: range.end_segment_id,
                evidence_segment_ids: range.evidence_segment_ids,
            })
            .collect(),
        uncovered_segment_ids: pending.uncovered_segment_ids,
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
    use settings::{
        AssistantConnectionPatch, CredentialUpdate, SettingsError, SettingsGrant, SettingsPatch,
        SettingsRepository,
    };
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

    /// The settings form's save for a new `profile()` with its direct key.
    fn new_connection_save(credential: CredentialUpdate) -> AssistantSettingsSave {
        AssistantSettingsSave {
            patch: SettingsPatch {
                active_connection_id: Some(Some("local".into())),
                connection: Some(AssistantConnectionPatch {
                    id: "local".into(),
                    credential: Some(credential),
                    ..Default::default()
                }),
                ..Default::default()
            },
            new_connection: Some(AssistantConnectionProfile {
                api_key: None,
                ..profile()
            }),
        }
    }

    fn save(
        service: &DesktopAssistantService,
        save: AssistantSettingsSave,
    ) -> Result<AppSettings, AssistantUiError> {
        service.save_assistant_settings(service.settings.revision().unwrap(), save)
    }

    fn rename(name: &str) -> AssistantSettingsSave {
        AssistantSettingsSave {
            patch: SettingsPatch {
                connection: Some(AssistantConnectionPatch {
                    id: "local".into(),
                    name: Some(name.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            new_connection: None,
        }
    }

    #[test]
    fn saves_profile_metadata_and_api_key_in_settings() {
        let (service, _) = service();
        let saved = save(
            &service,
            new_connection_save(CredentialUpdate::Direct("private-token".into())),
        )
        .unwrap();

        let expected = AssistantSettings {
            active_connection_id: Some("local".into()),
            connections: vec![profile()],
            ..Default::default()
        };
        assert_eq!(saved.assistant, expected);
        assert_eq!(service.settings_snapshot().unwrap().assistant, expected);
    }

    #[test]
    fn a_new_connection_credential_follows_the_typed_patch_rules() {
        let (service, _) = service();
        let error = save(
            &service,
            new_connection_save(CredentialUpdate::File("/etc/meetnote-key".into())),
        )
        .unwrap_err();

        assert_eq!(error.code, "ASSISTANT_SETTINGS_INVALID");
        assert!(!error.retryable);
        assert!(service
            .settings_snapshot()
            .unwrap()
            .assistant
            .connections
            .is_empty());
    }

    /// Desktop service whose active connection `local` stores `private-token`.
    fn service_with_stored_key() -> DesktopAssistantService {
        let (service, _) = service();
        let mut settings = service.settings_snapshot().unwrap();
        settings.assistant.active_connection_id = Some("local".into());
        settings.assistant.connections = vec![profile()];
        service.settings.update(settings).unwrap();
        service
    }

    #[test]
    fn an_unchanged_credential_keeps_the_stored_key() {
        let service = service_with_stored_key();
        save(&service, rename("Renamed")).unwrap();

        let saved = &service.settings_snapshot().unwrap().assistant.connections[0];
        assert_eq!(saved.name, "Renamed");
        assert_eq!(saved.api_key.as_deref(), Some("private-token"));
    }

    #[test]
    fn a_stale_revision_is_a_retryable_conflict_that_keeps_the_external_change() {
        let service = service_with_stored_key();
        let loaded_revision = service.settings.revision().unwrap();
        service
            .settings
            .patch_if_equal(
                &loaded_revision,
                SettingsPatch {
                    write_mode: Some(contracts::AgentWriteMode::ConfirmEach),
                    ..Default::default()
                },
                SettingsGrant::full(),
            )
            .unwrap();
        let error = service
            .save_assistant_settings(loaded_revision, rename("Renamed"))
            .unwrap_err();

        assert_eq!(error.code, "SETTINGS_REVISION_CONFLICT");
        assert!(error.retryable);
        let persisted = service.settings_snapshot().unwrap().assistant;
        assert_eq!(persisted.write_mode, contracts::AgentWriteMode::ConfirmEach);
        assert_eq!(persisted.connections[0].name, "Local LiteLLM");

        let saved = save(&service, rename("Renamed")).unwrap().assistant;
        assert_eq!(saved.write_mode, contracts::AgentWriteMode::ConfirmEach);
        assert_eq!(saved.connections[0].name, "Renamed");
        assert_eq!(
            saved.connections[0].api_key.as_deref(),
            Some("private-token")
        );
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

    #[test]
    fn run_rejects_a_stale_ui_document_before_contacting_the_provider() {
        let (service, documents) = service();
        save(
            &service,
            new_connection_save(CredentialUpdate::Direct("private-token".into())),
        )
        .unwrap();
        documents
            .save(
                DocumentState::new(DocumentId::new("doc"), "Document", "audio.wav"),
                Some("# Document\n".into()),
            )
            .unwrap();
        let error = service
            .run(
                AssistantRunRequest {
                    request_id: "request".into(),
                    scene: contracts::AgentScene::Ask,
                    skill: contracts::AgentSkillSelector::for_scene(contracts::AgentScene::Ask),
                    prompt: "question".into(),
                    authorized_documents: vec![ui::assistant_ui::AssistantDocumentContext {
                        document_id: "doc".into(),
                        title: "Document".into(),
                        revision: 99,
                    }],
                    projection: contracts::DocumentProjection::Metadata,
                    write_mode: contracts::AgentWriteMode::Automatic,
                    connection_id: "local".into(),
                    max_tool_rounds: 8,
                    max_validation_retries: 3,
                },
                AgentCancellation::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_REVISION_CONFLICT");
    }

    #[test]
    fn maps_every_pending_preview_field_for_the_confirmation_ui() {
        let pending = map_pending(AgentPendingMutation {
            confirmation_id: "confirmation".into(),
            expires_at_ms: 10,
            affected_document_ids: vec!["doc".into()],
            field_diffs: vec![AgentFieldDiff {
                document_id: "doc".into(),
                field: "title".into(),
                before: Some("Before".into()),
                after: Some("After".into()),
            }],
            tree_changes: vec![AgentTreeChange {
                document_id: "doc".into(),
                previous_parent_id: None,
                new_parent_id: Some("parent".into()),
                previous_sibling_order: Some(0),
                new_sibling_order: Some(1),
            }],
            ranges: vec![AgentRangePreview {
                document_id: Some("child".into()),
                start_segment_id: "s1".into(),
                end_segment_id: "s2".into(),
                evidence_segment_ids: vec!["s1".into()],
            }],
            uncovered_segment_ids: vec!["s3".into()],
        });
        assert_eq!(pending.field_diffs.len(), 1);
        assert_eq!(pending.tree_changes.len(), 1);
        assert_eq!(pending.ranges.len(), 1);
        assert_eq!(pending.uncovered_segment_ids, vec!["s3"]);
    }

    #[test]
    fn maps_committed_partial_results_to_a_warning_terminal_state() {
        let receipt = AgentMutationReceipt {
            mutation_id: "mutation-partial".into(),
            affected_documents: vec![],
            undo_token: "undo-partial".into(),
            provenance: AgentMutationProvenance {
                actor: contracts::AgentActor::Assistant,
                mutation_id: "mutation-partial".into(),
                skill: None,
                connection_id: Some("local".into()),
                protocol: Some("openaiResponses".into()),
                requested_model: Some("fixture".into()),
                resolved_model: Some("fixture".into()),
                source_revisions: vec![],
                applied_at_ms: 1,
            },
            uncovered_segment_ids: vec!["gap-1".into()],
        };
        let result = map_run_result(AgentRunResult {
            answer: None,
            evidence_segment_ids: vec!["s1".into()],
            mutation: Some(receipt.clone()),
            mutations: vec![receipt],
            pending: None,
            results: vec![],
            usage: Some(AgentUsage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
                cost_microusd: None,
            }),
            diagnostics: AgentRunDiagnostics {
                path: application::AgentRunPath::ShortDocument,
                tool_rounds: 2,
                validation_retries: 1,
                input_segments: 1,
                input_bytes: 12,
                elapsed_ms: 9,
                requested_model: Some("fixture".into()),
                resolved_model: Some("fixture".into()),
            },
            progress: vec![],
            error: Some(AgentToolError::new(
                AgentErrorCode::DocumentRevisionConflict,
                "document changed after the committed mutation",
                true,
            )),
        })
        .unwrap();
        assert_eq!(
            result.warning.as_ref().unwrap().code,
            "DOCUMENT_REVISION_CONFLICT"
        );
        assert_eq!(result.mutations.len(), 1);
        assert_eq!(result.usage.unwrap().total_tokens, 5);
        assert_eq!(result.diagnostics.unwrap().tool_rounds, 2);
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
