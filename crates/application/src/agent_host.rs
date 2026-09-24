//! The service-neutral Agent Host dispatch boundary.
//!
//! Host owns routing and the caller grant. Workspace, Audio, and Settings
//! ports continue to own validation and their native result/error contracts.

use crate::capabilities::{
    decode_object_arguments, settings_grant_for_fields, AudioCancelWire, AudioHandleArguments,
    AudioJobHandle, AudioJobSnapshotWire, AudioModelArguments, AudioProbeWire, AudioQueryArguments,
    AudioResultWire, AudioStartWire, SettingsPatchArguments, SettingsReadArguments,
    SettingsSection, SettingsSnapshotArguments,
};
use crate::{
    AudioCapabilityPort, CapabilityDescriptor, CapabilityOperation, SettingsCapabilityPort,
    WorkspaceCapabilityPort,
};
use contracts::{
    AgentScopeSelector, AgentToolError, CapabilityCallRequest, CapabilityCallResponse,
    CapabilityService, CapabilityServiceError, ConfirmationMutationArguments,
    UndoMutationArguments,
};
use domain::ProjectId;
use runtime::JobId;
use serde::de::DeserializeOwned;
use serde_json::Value;
use settings::SettingsGrant;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const AGENT_HOST_CAPABILITY_ID: &str = "agent.host";
pub const AGENT_HOST_CAPABILITY_VERSION: u32 = 1;

/// A capability binding is the only extension point needed by Host. The Host
/// loop does not need a new routing branch when a test or plugin capability is
/// assembled alongside the built-ins.
pub trait CapabilityBinding: Send + Sync {
    fn descriptor(&self, host: &AgentHost) -> CapabilityDescriptor;
    fn dispatch(&self, host: &AgentHost, request: CapabilityCallRequest) -> CapabilityCallResponse;
}

#[derive(Debug, Clone, Copy)]
enum BuiltinBinding {
    Workspace,
    Audio,
    Settings,
}

impl CapabilityBinding for BuiltinBinding {
    fn descriptor(&self, host: &AgentHost) -> CapabilityDescriptor {
        match self {
            Self::Workspace => host.workspace_descriptor(),
            Self::Audio => host.audio.descriptor(),
            Self::Settings => host.settings.descriptor(),
        }
    }

    fn dispatch(&self, host: &AgentHost, request: CapabilityCallRequest) -> CapabilityCallResponse {
        match self {
            Self::Workspace => host.dispatch_workspace(request),
            Self::Audio => host.dispatch_audio(request),
            Self::Settings => host.dispatch_settings(request),
        }
    }
}

#[derive(Clone)]
pub struct AgentHost {
    workspace: WorkspaceCapabilityPort,
    audio: AudioCapabilityPort,
    settings: SettingsCapabilityPort,
    settings_fields: BTreeSet<String>,
    bindings: BTreeMap<String, Arc<dyn CapabilityBinding>>,
}

impl AgentHost {
    pub fn new(
        workspace: WorkspaceCapabilityPort,
        audio: AudioCapabilityPort,
        settings: SettingsCapabilityPort,
        settings_fields: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut bindings: BTreeMap<String, Arc<dyn CapabilityBinding>> = BTreeMap::new();
        bindings.insert(
            "workspace.documents".into(),
            Arc::new(BuiltinBinding::Workspace),
        );
        bindings.insert("audio.processing".into(), Arc::new(BuiltinBinding::Audio));
        bindings.insert("settings.app".into(), Arc::new(BuiltinBinding::Settings));
        Self {
            workspace,
            audio,
            settings,
            settings_fields: settings_fields.into_iter().collect(),
            bindings,
        }
    }

    pub fn with_binding(
        mut self,
        capability_id: impl Into<String>,
        binding: Arc<dyn CapabilityBinding>,
    ) -> Self {
        self.bindings.insert(capability_id.into(), binding);
        self
    }

    pub fn discover(&self) -> Vec<CapabilityDescriptor> {
        self.bindings
            .values()
            .map(|binding| binding.descriptor(self))
            .collect()
    }

    pub fn workspace(&self) -> &WorkspaceCapabilityPort {
        &self.workspace
    }

    pub fn dispatch(&self, request: CapabilityCallRequest) -> CapabilityCallResponse {
        if request.schema_version != contracts::CAPABILITY_CALL_SCHEMA_VERSION {
            return self.failure(
                &request,
                "CAPABILITY_SCHEMA_UNSUPPORTED",
                "unsupported capability call schema version",
                false,
            );
        }
        if request.request_id.trim().is_empty() || request.call_id.trim().is_empty() {
            return self.failure(
                &request,
                "CAPABILITY_REQUEST_INVALID",
                "requestId and callId cannot be empty",
                false,
            );
        }
        let Some(binding) = self.bindings.get(&request.capability_id).cloned() else {
            return self.failure(
                &request,
                "CAPABILITY_UNKNOWN",
                "capability is not installed in this Host",
                false,
            );
        };
        let descriptor = binding.descriptor(self);
        if request.capability_version != descriptor.version {
            return self.failure(
                &request,
                "CAPABILITY_VERSION_UNSUPPORTED",
                "capability version is not supported",
                false,
            );
        }
        binding.dispatch(self, request)
    }

    fn dispatch_workspace(&self, request: CapabilityCallRequest) -> CapabilityCallResponse {
        let description = self.workspace.describe();
        if request.capability_version != description.capability_version {
            return self.failure(
                &request,
                "CAPABILITY_VERSION_UNSUPPORTED",
                "workspace capability version is not supported",
                false,
            );
        }
        let scope = AgentScopeSelector {
            all_documents: request.authorization.all_documents,
            document_ids: request.authorization.document_ids.clone(),
        };
        let workspace_request = contracts::CapabilityRequest::new(
            request.request_id.clone(),
            request.call_id.clone(),
            scope.clone(),
            request.arguments.clone(),
        );
        let response = match request.operation.as_str() {
            "confirm_mutation" => match parse_confirmation_arguments(&request) {
                Ok(confirmation_id) => {
                    self.workspace
                        .confirm_scoped(&confirmation_id, &request.request_id, &scope)
                }
                Err(error) => return CapabilityCallResponse::failure(&request, error),
            },
            "reject_mutation" => match parse_confirmation_arguments(&request) {
                Ok(confirmation_id) => {
                    self.workspace
                        .reject_scoped(&confirmation_id, &request.request_id, &scope)
                }
                Err(error) => return CapabilityCallResponse::failure(&request, error),
            },
            "undo_mutation" => match parse_undo_arguments(&request) {
                Ok(undo_token) => {
                    self.workspace
                        .undo_scoped(&undo_token, &request.request_id, &scope)
                }
                Err(error) => return CapabilityCallResponse::failure(&request, error),
            },
            operation => self.workspace.execute(operation, workspace_request),
        };
        if let Some(result) = response.result.clone() {
            return CapabilityCallResponse::success(
                &request,
                serde_json::to_value(result).expect("workspace result must serialize"),
            );
        }
        let error = response.error.clone().unwrap_or_else(|| {
            AgentToolError::new(
                contracts::AgentErrorCode::ProviderResponseInvalid,
                "workspace returned neither a result nor an error",
                false,
            )
        });
        CapabilityCallResponse::failure(&request, workspace_error(error, &request))
    }

    fn dispatch_audio(&self, request: CapabilityCallRequest) -> CapabilityCallResponse {
        if request.capability_version != self.audio.descriptor().version {
            return self.failure(
                &request,
                "CAPABILITY_VERSION_UNSUPPORTED",
                "audio capability version is not supported",
                false,
            );
        }
        let project_id = request
            .arguments
            .get("projectId")
            .and_then(Value::as_str)
            .map(ProjectId::new);
        let Some(project_id) = project_id else {
            return self.failure(
                &request,
                "AUDIO_ARGUMENTS_INVALID",
                "audio operation requires projectId",
                false,
            );
        };
        if !request.authorization.project_ids.is_empty()
            && !request
                .authorization
                .project_ids
                .iter()
                .any(|id| id == project_id.as_str())
        {
            return self.failure(
                &request,
                "AUDIO_ACCESS_DENIED",
                "project is outside the audio grant",
                false,
            );
        }
        let result = match request.operation.as_str() {
            "probe" => self.audio_probe(&project_id, &request.arguments),
            "start" => self.audio_start(project_id, &request.arguments),
            "snapshot" | "query" => self.audio_snapshot(&project_id, &request.arguments),
            "cancel" => self.audio_cancel(&project_id, &request.arguments),
            "result" => self.audio_result(&project_id, &request.arguments),
            _ => Err(service_error(
                CapabilityService::Audio,
                &request,
                "AUDIO_OPERATION_UNKNOWN",
                "audio operation is not supported",
                false,
            )),
        };
        match result {
            Ok(value) => CapabilityCallResponse::success(&request, value),
            Err(error) => CapabilityCallResponse::failure(&request, error),
        }
    }

    fn audio_probe(
        &self,
        project_id: &ProjectId,
        arguments: &Value,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments: AudioModelArguments = parse_audio_arguments(arguments, "probe")?;
        ensure_audio_project(project_id, &arguments.project_id, "probe")?;
        self.audio
            .probe(
                project_id,
                &arguments.asr_model,
                &arguments.diarization_model,
            )
            .map(|_| {
                serde_json::to_value(AudioProbeWire {
                    project_id: project_id.as_str().into(),
                    probed: true,
                })
                .expect("audio probe result serializes")
            })
            .map_err(|error| backend_error(CapabilityService::Audio, "probe", project_id, error))
    }

    fn audio_start(
        &self,
        project_id: ProjectId,
        arguments: &Value,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments: AudioModelArguments = parse_audio_arguments(arguments, "start")?;
        ensure_audio_project(&project_id, &arguments.project_id, "start")?;
        let handle = self
            .audio
            .start(
                project_id.clone(),
                arguments.asr_model,
                arguments.diarization_model,
            )
            .map_err(|error| {
                backend_error(CapabilityService::Audio, "start", &project_id, error)
            })?;
        Ok(serde_json::to_value(AudioStartWire {
            project_id: project_id.as_str().into(),
            asr_job_id: handle.asr_job_id.0,
            diarization_job_id: handle.diarization_job_id.0,
        })
        .expect("audio start result serializes"))
    }

    fn audio_snapshot(
        &self,
        project_id: &ProjectId,
        arguments: &Value,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments: AudioQueryArguments = parse_audio_arguments(arguments, "query")?;
        ensure_audio_project(project_id, &arguments.project_id, "query")?;
        self.audio
            .snapshot_for(project_id, JobId(arguments.job_id))
            .map(|snapshot| {
                serde_json::to_value(AudioJobSnapshotWire::from(snapshot))
                    .expect("job snapshot serializes")
            })
            .map_err(|error| backend_error(CapabilityService::Audio, "query", project_id, error))
    }

    fn audio_cancel(
        &self,
        project_id: &ProjectId,
        arguments: &Value,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments: AudioHandleArguments = parse_audio_arguments(arguments, "cancel")?;
        ensure_audio_project(project_id, &arguments.project_id, "cancel")?;
        self.audio
            .cancel_handle(
                project_id,
                AudioJobHandle {
                    asr_job_id: JobId(arguments.asr_job_id),
                    diarization_job_id: JobId(arguments.diarization_job_id),
                },
            )
            .map(|_| {
                serde_json::to_value(AudioCancelWire { cancelled: true })
                    .expect("audio cancel result serializes")
            })
            .map_err(|error| backend_error(CapabilityService::Audio, "cancel", project_id, error))
    }

    fn audio_result(
        &self,
        project_id: &ProjectId,
        arguments: &Value,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments: AudioHandleArguments = parse_audio_arguments(arguments, "result")?;
        ensure_audio_project(project_id, &arguments.project_id, "result")?;
        let result = self
            .audio
            .result_for(
                project_id,
                AudioJobHandle {
                    asr_job_id: JobId(arguments.asr_job_id),
                    diarization_job_id: JobId(arguments.diarization_job_id),
                },
            )
            .map_err(|error| {
                backend_error(CapabilityService::Audio, "result", project_id, error)
            })?;
        Ok(serde_json::to_value(AudioResultWire {
            ready: result.is_some(),
            document: result.map(|document| {
                serde_json::to_value(document).expect("audio result document serializes")
            }),
        })
        .expect("audio result serializes"))
    }

    fn dispatch_settings(&self, request: CapabilityCallRequest) -> CapabilityCallResponse {
        if request.capability_version != self.settings.descriptor().version {
            return self.failure(
                &request,
                "CAPABILITY_VERSION_UNSUPPORTED",
                "settings capability version is not supported",
                false,
            );
        }
        if let Some(field) = request
            .authorization
            .settings_fields
            .iter()
            .find(|field| !self.settings_field_granted(field))
        {
            return self.failure(
                &request,
                "SETTINGS_FIELD_NOT_GRANTED",
                format!("settings field is not granted: {field}"),
                false,
            );
        }
        let grant = settings_grant_for_fields(&request.authorization.settings_fields);
        let result = match request.operation.as_str() {
            "snapshot" => {
                parse_settings_arguments::<SettingsSnapshotArguments>(&request, "snapshot")
                    .and_then(|_| self.settings_public_snapshot(&request, grant))
            }
            "read" => self.settings_read(&request, grant),
            "patch" => self.settings_patch(&request, grant),
            _ => Err(settings_error(
                &request,
                "SETTINGS_OPERATION_UNKNOWN",
                "settings operation is not supported",
                false,
            )),
        };
        match result {
            Ok(value) => CapabilityCallResponse::success(&request, value),
            Err(error) => CapabilityCallResponse::failure(&request, error),
        }
    }

    fn settings_public_snapshot(
        &self,
        request: &CapabilityCallRequest,
        grant: SettingsGrant,
    ) -> Result<Value, CapabilityServiceError> {
        self.settings
            .public_snapshot(grant)
            .map(|snapshot| serde_json::to_value(snapshot).expect("settings snapshot serializes"))
            .map_err(|error| map_settings_error(request, "snapshot", error))
    }

    fn settings_read(
        &self,
        request: &CapabilityCallRequest,
        grant: SettingsGrant,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments = parse_settings_arguments::<SettingsReadArguments>(request, "read")?;
        let result = match arguments.section {
            SettingsSection::Connections => self
                .settings
                .read_connections(grant)
                .map(|value| serde_json::to_value(value).unwrap()),
            SettingsSection::ConnectionTargets => self
                .settings
                .read_connection_targets(grant)
                .map(|value| serde_json::to_value(value).unwrap()),
            SettingsSection::Credentials => self
                .settings
                .read_credentials(grant)
                .map(|value| serde_json::to_value(value).unwrap()),
            SettingsSection::WritePolicy => self
                .settings
                .read_write_policy(grant)
                .map(|value| serde_json::to_value(value).unwrap()),
        };
        result.map_err(|error| map_settings_error(request, "read", error))
    }

    fn settings_patch(
        &self,
        request: &CapabilityCallRequest,
        grant: SettingsGrant,
    ) -> Result<Value, CapabilityServiceError> {
        let arguments = parse_settings_arguments::<SettingsPatchArguments>(request, "patch")?;
        self.settings
            .patch(&arguments.expected_revision, arguments.patch, grant)
            .map(|mutation| serde_json::to_value(mutation).expect("settings mutation serializes"))
            .map_err(|error| map_settings_error(request, "patch", error))
    }

    fn failure(
        &self,
        request: &CapabilityCallRequest,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> CapabilityCallResponse {
        CapabilityCallResponse::failure(
            request,
            service_error(service_for(request), request, code, message, retryable),
        )
    }

    fn settings_field_granted(&self, field: &str) -> bool {
        self.settings_fields.contains(field) || self.settings_fields.contains("assistant")
    }

    fn workspace_descriptor(&self) -> CapabilityDescriptor {
        let description = self.workspace.describe();
        let mut operations = description
            .tools
            .into_iter()
            .map(|tool| CapabilityOperation {
                name: tool.name,
                description: tool.description,
                side_effect: tool.side_effect,
                long_running: false,
                input_schema: tool.input_schema,
                output_schema: workspace_result_schema(),
            })
            .collect::<Vec<_>>();
        for name in ["confirm_mutation", "reject_mutation", "undo_mutation"] {
            let (input_schema, description) = match name {
                "undo_mutation" => (
                    contracts::schema_for_arguments::<UndoMutationArguments>(),
                    "Undo a previously committed mutation within the request scope.".into(),
                ),
                _ => (
                    contracts::schema_for_arguments::<ConfirmationMutationArguments>(),
                    format!("{name} a pending mutation within the request scope."),
                ),
            };
            operations.push(CapabilityOperation {
                name: name.into(),
                description,
                side_effect: true,
                long_running: false,
                input_schema,
                output_schema: workspace_result_schema(),
            });
        }
        CapabilityDescriptor {
            id: description.capability_id,
            version: description.capability_version,
            read_only: false,
            long_running: false,
            operations,
        }
    }
}

fn workspace_result_schema() -> Value {
    contracts::schema_for_arguments::<contracts::AgentToolResult>()
}

fn service_for(request: &CapabilityCallRequest) -> CapabilityService {
    match request.capability_id.as_str() {
        "audio.processing" => CapabilityService::Audio,
        "settings.app" => CapabilityService::Settings,
        _ => CapabilityService::Workspace,
    }
}

fn parse_audio_arguments<T: DeserializeOwned>(
    arguments: &Value,
    operation: &str,
) -> Result<T, CapabilityServiceError> {
    decode_object_arguments(arguments).map_err(|error| {
        CapabilityServiceError::new(
            "AUDIO_ARGUMENTS_INVALID",
            format!("invalid typed audio arguments: {error}"),
            false,
        )
        .with_context(contracts::CapabilityErrorContext::new(
            CapabilityService::Audio,
            operation,
        ))
    })
}

fn parse_settings_arguments<T: DeserializeOwned>(
    request: &CapabilityCallRequest,
    operation: &str,
) -> Result<T, CapabilityServiceError> {
    decode_object_arguments(&request.arguments).map_err(|error| {
        settings_error(
            request,
            "SETTINGS_ARGUMENTS_INVALID",
            format!("invalid typed settings arguments: {error}"),
            false,
        )
        .with_context(contracts::CapabilityErrorContext::new(
            CapabilityService::Settings,
            operation,
        ))
    })
}

fn ensure_audio_project(
    expected: &ProjectId,
    actual: &str,
    operation: &str,
) -> Result<(), CapabilityServiceError> {
    if expected.as_str() == actual {
        Ok(())
    } else {
        Err(CapabilityServiceError::new(
            "AUDIO_ARGUMENTS_INVALID",
            "projectId must match the selected audio project",
            false,
        )
        .with_context(contracts::CapabilityErrorContext::new(
            CapabilityService::Audio,
            operation,
        )))
    }
}

fn service_error(
    service: CapabilityService,
    request: &CapabilityCallRequest,
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> CapabilityServiceError {
    CapabilityServiceError::new(code, message, retryable).with_context(
        contracts::CapabilityErrorContext::new(service, request.operation.clone())
            .with_resource("capability", request.capability_id.clone()),
    )
}

fn parse_confirmation_arguments(
    request: &CapabilityCallRequest,
) -> Result<String, CapabilityServiceError> {
    let arguments: ConfirmationMutationArguments =
        parse_control_arguments(request, "confirmationId")?;
    non_empty_control_value(request, "confirmationId", arguments.confirmation_id)
}

fn parse_undo_arguments(request: &CapabilityCallRequest) -> Result<String, CapabilityServiceError> {
    let arguments: UndoMutationArguments = parse_control_arguments(request, "undoToken")?;
    non_empty_control_value(request, "undoToken", arguments.undo_token)
}

fn parse_control_arguments<T: DeserializeOwned>(
    request: &CapabilityCallRequest,
    field: &str,
) -> Result<T, CapabilityServiceError> {
    decode_object_arguments(&request.arguments).map_err(|error| {
        CapabilityServiceError::new(
            "TOOL_ARGUMENTS_INVALID",
            format!("invalid {} arguments: {error}", request.operation),
            false,
        )
        .with_context(
            contracts::CapabilityErrorContext::new(
                CapabilityService::Workspace,
                request.operation.clone(),
            )
            .with_field(field),
        )
    })
}

fn non_empty_control_value(
    request: &CapabilityCallRequest,
    field: &str,
    value: String,
) -> Result<String, CapabilityServiceError> {
    if value.trim().is_empty() {
        return Err(CapabilityServiceError::new(
            "TOOL_ARGUMENTS_INVALID",
            format!("{field} is required"),
            false,
        )
        .with_context(
            contracts::CapabilityErrorContext::new(
                CapabilityService::Workspace,
                request.operation.clone(),
            )
            .with_field(field),
        ));
    }
    Ok(value)
}

fn settings_error(
    request: &CapabilityCallRequest,
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> CapabilityServiceError {
    service_error(
        CapabilityService::Settings,
        request,
        code,
        message,
        retryable,
    )
}

fn backend_error(
    service: CapabilityService,
    operation: &str,
    project_id: &ProjectId,
    error: contracts::BackendError,
) -> CapabilityServiceError {
    CapabilityServiceError::new(
        audio_error_code(error.code),
        error.message,
        audio_error_retryable(error.code),
    )
    .with_context(
        contracts::CapabilityErrorContext::new(service, operation)
            .with_resource("project", project_id.as_str().to_owned()),
    )
}

fn audio_error_code(code: contracts::BackendErrorCode) -> &'static str {
    match code {
        contracts::BackendErrorCode::BackendUnavailable => "AUDIO_BACKEND_UNAVAILABLE",
        contracts::BackendErrorCode::ModelMissing => "AUDIO_MODEL_MISSING",
        contracts::BackendErrorCode::ModelIncompatible => "AUDIO_MODEL_INCOMPATIBLE",
        contracts::BackendErrorCode::ModelLoadFailed => "AUDIO_MODEL_LOAD_FAILED",
        contracts::BackendErrorCode::DeviceUnavailable => "AUDIO_DEVICE_UNAVAILABLE",
        contracts::BackendErrorCode::AccessDenied => "AUDIO_ACCESS_DENIED",
        contracts::BackendErrorCode::InvalidAudio => "AUDIO_INVALID_AUDIO",
        contracts::BackendErrorCode::InferenceFailed => "AUDIO_INFERENCE_FAILED",
        contracts::BackendErrorCode::PersistenceFailed => "AUDIO_PERSISTENCE_FAILED",
        contracts::BackendErrorCode::NotImplemented => "AUDIO_NOT_IMPLEMENTED",
        contracts::BackendErrorCode::Cancelled => "AUDIO_CANCELLED",
        contracts::BackendErrorCode::OutputValidationFailed => "AUDIO_OUTPUT_VALIDATION_FAILED",
    }
}

fn audio_error_retryable(code: contracts::BackendErrorCode) -> bool {
    matches!(
        code,
        contracts::BackendErrorCode::BackendUnavailable
            | contracts::BackendErrorCode::DeviceUnavailable
            | contracts::BackendErrorCode::InferenceFailed
            | contracts::BackendErrorCode::PersistenceFailed
    )
}

fn map_settings_error(
    request: &CapabilityCallRequest,
    operation: &str,
    error: settings::SettingsError,
) -> CapabilityServiceError {
    let (code, retryable, field) = match &error {
        settings::SettingsError::RevisionConflict => ("SETTINGS_REVISION_CONFLICT", true, None),
        settings::SettingsError::FieldNotGranted { field } => {
            ("SETTINGS_FIELD_NOT_GRANTED", false, Some(field.clone()))
        }
        settings::SettingsError::CredentialTargetConflict => {
            ("SETTINGS_CREDENTIAL_TARGET_CONFLICT", false, None)
        }
        settings::SettingsError::RedactedCredential => {
            ("SETTINGS_REDACTED_CREDENTIAL", false, None)
        }
        settings::SettingsError::UnsafeCredentialFile => {
            ("SETTINGS_UNSAFE_CREDENTIAL_FILE", false, None)
        }
        settings::SettingsError::Invalid(_) => ("SETTINGS_INVALID", false, None),
        settings::SettingsError::Read { .. } => ("SETTINGS_READ_FAILED", true, None),
        settings::SettingsError::Write { .. } => ("SETTINGS_WRITE_FAILED", true, None),
        settings::SettingsError::Parse(_) => ("SETTINGS_PARSE_FAILED", false, None),
        settings::SettingsError::LockPoisoned => ("SETTINGS_LOCK_POISONED", true, None),
    };
    let mut context =
        contracts::CapabilityErrorContext::new(CapabilityService::Settings, operation)
            .with_resource("capability", request.capability_id.clone());
    if let Some(field) = field {
        context = context.with_field(field);
    }
    CapabilityServiceError::new(code, error.to_string(), retryable).with_context(context)
}

fn workspace_error(
    error: AgentToolError,
    request: &CapabilityCallRequest,
) -> CapabilityServiceError {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "WORKSPACE_ERROR".into());
    let mut context = contracts::CapabilityErrorContext::new(
        CapabilityService::Workspace,
        request.operation.clone(),
    )
    .with_resource("capability", request.capability_id.clone());
    if let Some(document_id) = error.document_id {
        context = context.with_resource("document", document_id);
    }
    if let Some(field) = error.field {
        context = context.with_field(field);
    }
    if let (Some(expected), Some(actual)) = (error.expected_revision, error.actual_revision) {
        context = context.with_revisions(expected.to_string(), actual.to_string());
    }
    context.extend(
        error
            .context
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    CapabilityServiceError::new(code, error.message, error.retryable).with_context(context)
}
