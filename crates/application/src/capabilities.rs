use crate::{BackendFuture, ProcessingRequest, ProjectProcessingJob, WorkspaceService};
use contracts::{BackendError, ModelRef};
use domain::ProjectId;
use runtime::{JobContext, JobId, JobSnapshot, JobState};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::{
    AppSettings, AssistantConnectionSummary, AssistantConnectionTarget, AssistantWritePolicy,
    CredentialStatus, SettingsError, SettingsGrant, SettingsPatch, SettingsReadResult,
    SettingsService,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

/// A small, versioned description shared by built-in and external consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityOperation {
    pub name: String,
    pub description: String,
    pub side_effect: bool,
    pub long_running: bool,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub id: String,
    pub version: u32,
    pub read_only: bool,
    pub long_running: bool,
    pub operations: Vec<CapabilityOperation>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioModelArguments {
    pub project_id: String,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioQueryArguments {
    pub project_id: String,
    pub job_id: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioHandleArguments {
    pub project_id: String,
    pub asr_job_id: u64,
    pub diarization_job_id: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioProbeWire {
    pub project_id: String,
    pub probed: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioStartWire {
    pub project_id: String,
    pub asr_job_id: u64,
    pub diarization_job_id: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioCancelWire {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum AudioJobStateWire {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioProgressWire {
    pub completed: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioJobSnapshotWire {
    pub job_id: u64,
    pub state: AudioJobStateWire,
    pub progress: Option<AudioProgressWire>,
    pub error: Option<String>,
}

impl From<JobSnapshot> for AudioJobSnapshotWire {
    fn from(snapshot: JobSnapshot) -> Self {
        Self {
            job_id: snapshot.job_id.0,
            state: match snapshot.state {
                JobState::Queued => AudioJobStateWire::Queued,
                JobState::Running => AudioJobStateWire::Running,
                JobState::Completed => AudioJobStateWire::Completed,
                JobState::Failed => AudioJobStateWire::Failed,
                JobState::Cancelled => AudioJobStateWire::Cancelled,
            },
            progress: snapshot.progress.map(|progress| AudioProgressWire {
                completed: progress.completed,
                total: progress.total,
            }),
            error: snapshot.error,
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioResultWire {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsSnapshotArguments {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsReadArguments {
    pub section: SettingsSection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SettingsSection {
    Connections,
    ConnectionTargets,
    Credentials,
    WritePolicy,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatchArguments {
    pub expected_revision: String,
    pub patch: SettingsPatch,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPublicSnapshotWire {
    pub revision: String,
    pub connections: SettingsReadResult<Vec<AssistantConnectionSummary>>,
    pub connection_targets: SettingsReadResult<Vec<AssistantConnectionTarget>>,
    pub credentials: SettingsReadResult<Vec<CredentialStatus>>,
    pub write_policy: SettingsReadResult<AssistantWritePolicy>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsCapabilityMutation {
    pub revision: String,
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioProcessingConfigSnapshot {
    pub audio_path: String,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    pub device: contracts::ComputeDevice,
    pub speaker_library_path: Option<String>,
    pub stage_configuration: contracts::DiarizationStageConfiguration,
}

impl From<&ProcessingRequest> for AudioProcessingConfigSnapshot {
    fn from(request: &ProcessingRequest) -> Self {
        Self {
            audio_path: request.audio_path.clone(),
            asr_model: request.asr_model.clone(),
            diarization_model: request.diarization_model.clone(),
            language: request.language.clone(),
            expected_speakers: request.expected_speakers,
            device: request.device,
            speaker_library_path: request.speaker_library_path.clone(),
            stage_configuration: request.stage_configuration.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioJobHandle {
    pub asr_job_id: JobId,
    pub diarization_job_id: JobId,
}

impl AudioJobHandle {
    pub fn job_ids(self) -> (JobId, JobId) {
        (self.asr_job_id, self.diarization_job_id)
    }
}

struct AudioJobEntry {
    project_id: ProjectId,
    config: AudioProcessingConfigSnapshot,
    asr_context: Option<JobContext>,
    diarization_context: Option<JobContext>,
    terminal: BTreeMap<JobId, JobSnapshot>,
    result: Option<domain::DocumentState>,
}

struct AudioJobCompletion {
    handle: AudioJobHandle,
    snapshots: [JobSnapshot; 2],
    result: Option<domain::DocumentState>,
}

#[derive(Clone)]
pub struct AudioJobRegistry {
    jobs: Arc<Mutex<BTreeMap<AudioJobHandle, AudioJobEntry>>>,
    completion_sender: mpsc::Sender<AudioJobCompletion>,
    completion_receiver: Arc<Mutex<mpsc::Receiver<AudioJobCompletion>>>,
}

impl AudioJobRegistry {
    pub fn new() -> Self {
        let (completion_sender, completion_receiver) = mpsc::channel();
        Self {
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            completion_sender,
            completion_receiver: Arc::new(Mutex::new(completion_receiver)),
        }
    }

    fn drain_completions(&self) -> Result<(), BackendError> {
        let receiver = self
            .completion_receiver
            .lock()
            .map_err(|_| storage_error("audio completion channel lock poisoned".into()))?;
        let completions = receiver.try_iter().collect::<Vec<_>>();
        drop(receiver);
        if completions.is_empty() {
            return Ok(());
        }
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        for completion in completions {
            if let Some(entry) = jobs.get_mut(&completion.handle) {
                entry.asr_context = None;
                entry.diarization_context = None;
                entry.terminal.insert(
                    completion.snapshots[0].job_id,
                    completion.snapshots[0].clone(),
                );
                entry.terminal.insert(
                    completion.snapshots[1].job_id,
                    completion.snapshots[1].clone(),
                );
                entry.result = completion.result;
            }
        }
        Ok(())
    }

    fn register(
        &self,
        handle: AudioJobHandle,
        project_id: ProjectId,
        config: AudioProcessingConfigSnapshot,
        asr_context: JobContext,
        diarization_context: JobContext,
    ) -> Result<(), BackendError> {
        self.drain_completions()?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        if jobs.contains_key(&handle) {
            return Err(storage_error(
                "audio job handle is already registered".into(),
            ));
        }
        jobs.insert(
            handle,
            AudioJobEntry {
                project_id,
                config,
                asr_context: Some(asr_context),
                diarization_context: Some(diarization_context),
                terminal: BTreeMap::new(),
                result: None,
            },
        );
        Ok(())
    }

    fn abort_registration(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<(), BackendError> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        if let Some(entry) = jobs.get(&handle) {
            if entry.project_id != *project_id {
                return Err(storage_error(
                    "audio job is not owned by this project".into(),
                ));
            }
        }
        jobs.remove(&handle);
        Ok(())
    }

    fn complete(
        &self,
        handle: AudioJobHandle,
        snapshots: [JobSnapshot; 2],
        result: Option<domain::DocumentState>,
    ) {
        let _ = self.completion_sender.send(AudioJobCompletion {
            handle,
            snapshots,
            result,
        });
    }

    fn cancel(&self, project_id: &ProjectId, handle: AudioJobHandle) -> Result<(), BackendError> {
        self.drain_completions()?;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        let entry = jobs
            .get(&handle)
            .filter(|entry| entry.project_id == *project_id)
            .ok_or_else(|| access_denied("audio job is not owned by this project"))?;
        if let Some(context) = &entry.asr_context {
            context.cancel();
        }
        if let Some(context) = &entry.diarization_context {
            context.cancel();
        }
        Ok(())
    }

    fn snapshot(
        &self,
        project_id: &ProjectId,
        job_id: JobId,
    ) -> Result<Option<JobSnapshot>, BackendError> {
        self.drain_completions()?;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        let entry = jobs
            .iter()
            .find(|(handle, entry)| {
                entry.project_id == *project_id
                    && (handle.asr_job_id == job_id || handle.diarization_job_id == job_id)
            })
            .map(|(_, entry)| entry)
            .ok_or_else(|| access_denied("audio job is not owned by this project"))?;
        Ok(entry.terminal.get(&job_id).cloned())
    }

    fn ensure_owned_job(&self, project_id: &ProjectId, job_id: JobId) -> Result<(), BackendError> {
        self.drain_completions()?;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        if jobs.iter().any(|(handle, entry)| {
            entry.project_id == *project_id
                && (handle.asr_job_id == job_id || handle.diarization_job_id == job_id)
        }) {
            Ok(())
        } else {
            Err(access_denied("audio job is not owned by this project"))
        }
    }

    fn config(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<AudioProcessingConfigSnapshot, BackendError> {
        self.drain_completions()?;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        jobs.get(&handle)
            .filter(|entry| entry.project_id == *project_id)
            .map(|entry| entry.config.clone())
            .ok_or_else(|| access_denied("audio job is not owned by this project"))
    }

    fn result(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<Option<domain::DocumentState>, BackendError> {
        self.drain_completions()?;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        jobs.get(&handle)
            .filter(|entry| entry.project_id == *project_id)
            .map(|entry| entry.result.clone())
            .ok_or_else(|| access_denied("audio job is not owned by this project"))
    }

    fn release(&self, project_id: &ProjectId, handle: AudioJobHandle) -> Result<(), BackendError> {
        self.drain_completions()?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| storage_error("audio job registry lock poisoned".into()))?;
        let Some(entry) = jobs.get(&handle) else {
            return Err(access_denied("audio job is not owned by this project"));
        };
        if entry.project_id != *project_id {
            return Err(access_denied("audio job is not owned by this project"));
        }
        if entry.asr_context.is_some() || entry.diarization_context.is_some() {
            return Err(storage_error("active audio job cannot be released".into()));
        }
        jobs.remove(&handle);
        Ok(())
    }

    fn cancel_all(&self) {
        if let Ok(jobs) = self.jobs.lock() {
            for entry in jobs.values() {
                if let Some(context) = &entry.asr_context {
                    context.cancel();
                }
                if let Some(context) = &entry.diarization_context {
                    context.cancel();
                }
            }
        }
    }
}

impl Default for AudioJobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct AudioCapabilityPort {
    workspace: WorkspaceService,
    allowed_projects: Arc<Mutex<BTreeSet<ProjectId>>>,
    registry: AudioJobRegistry,
    lifecycle: Arc<Mutex<()>>,
}

impl AudioCapabilityPort {
    pub fn new(
        workspace: WorkspaceService,
        allowed_projects: impl IntoIterator<Item = ProjectId>,
    ) -> Self {
        Self {
            workspace,
            allowed_projects: Arc::new(Mutex::new(allowed_projects.into_iter().collect())),
            registry: AudioJobRegistry::default(),
            lifecycle: Arc::new(Mutex::new(())),
        }
    }

    pub fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: "audio.processing".into(),
            version: 1,
            read_only: false,
            long_running: true,
            operations: vec![
                operation(
                    "probe",
                    false,
                    true,
                    audio_input_schema(),
                    audio_probe_schema(),
                ),
                operation(
                    "start",
                    true,
                    true,
                    audio_input_schema(),
                    audio_job_schema(),
                ),
                operation(
                    "snapshot",
                    false,
                    false,
                    audio_query_schema(),
                    job_snapshot_schema(),
                ),
                operation(
                    "query",
                    false,
                    false,
                    audio_query_schema(),
                    job_snapshot_schema(),
                ),
                operation(
                    "cancel",
                    true,
                    false,
                    audio_cancel_schema(),
                    contracts::schema_for_arguments::<AudioCancelWire>(),
                ),
                operation(
                    "result",
                    false,
                    false,
                    audio_handle_schema(),
                    contracts::schema_for_arguments::<AudioResultWire>(),
                ),
            ],
        }
    }

    pub fn probe(
        &self,
        project_id: &ProjectId,
        asr_model: &ModelRef,
        diarization_model: &ModelRef,
    ) -> Result<(), BackendError> {
        self.ensure_project(project_id)?;
        let project = self
            .workspace
            .documents()
            .get(project_id)
            .map_err(storage_error)?
            .ok_or_else(|| missing_project(project_id))?;
        self.workspace
            .processing()
            .probe(&crate::ProcessingRequest {
                audio_path: project.audio_path().to_string(),
                asr_model: asr_model.clone(),
                diarization_model: diarization_model.clone(),
                language: None,
                expected_speakers: None,
                device: contracts::ComputeDevice::Auto,
                speaker_library_path: None,
                stage_configuration: contracts::DiarizationStageConfiguration::default(),
            })
    }

    pub fn start(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
    ) -> Result<AudioJobHandle, BackendError> {
        self.ensure_project(&project_id)?;
        let request =
            self.workspace
                .processing_request(&project_id, asr_model, diarization_model, None)?;
        self.start_request_async(project_id, request)
    }

    pub fn start_job(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
    ) -> Result<ProjectProcessingJob, BackendError> {
        self.ensure_project(&project_id)?;
        let request =
            self.workspace
                .processing_request(&project_id, asr_model, diarization_model, None)?;
        self.start_request(project_id, request)
    }

    pub fn start_with_request(
        &self,
        project_id: ProjectId,
        request: ProcessingRequest,
    ) -> Result<AudioJobHandle, BackendError> {
        self.ensure_project(&project_id)?;
        self.validate_request(&project_id, &request)?;
        self.start_request_async(project_id, request)
    }

    pub fn start_job_with_request(
        &self,
        project_id: ProjectId,
        request: crate::ProcessingRequest,
    ) -> Result<ProjectProcessingJob, BackendError> {
        self.ensure_project(&project_id)?;
        self.validate_request(&project_id, &request)?;
        self.start_request(project_id, request)
    }

    fn start_request_async(
        &self,
        project_id: ProjectId,
        request: ProcessingRequest,
    ) -> Result<AudioJobHandle, BackendError> {
        let job = self.start_request(project_id.clone(), request)?;
        let handle = AudioJobHandle {
            asr_job_id: job.asr_context.job_id(),
            diarization_job_id: job.diarization_context.job_id(),
        };
        let asr_context = job.asr_context.clone();
        let diarization_context = job.diarization_context.clone();
        let workspace = self.workspace.clone();
        let registry = self.registry.clone();
        let ProjectProcessingJob { future, .. } = job;
        thread::Builder::new()
            .name(format!("meetnote-audio-{}", handle.asr_job_id.0))
            .spawn(move || {
                let _ = futures::executor::block_on(future);
            })
            .map_err(|error| {
                asr_context.cancel();
                diarization_context.cancel();
                workspace.discard_processing_job_ids(handle.job_ids());
                let _ = registry.abort_registration(&project_id, handle);
                storage_error(format!("failed to spawn audio job worker: {error}"))
            })?;
        Ok(handle)
    }

    fn validate_request(
        &self,
        project_id: &ProjectId,
        request: &ProcessingRequest,
    ) -> Result<(), BackendError> {
        let project = self
            .workspace
            .documents()
            .get(project_id)
            .map_err(storage_error)?
            .ok_or_else(|| missing_project(project_id))?;
        if request.audio_path != project.audio_path() {
            return Err(BackendError {
                code: contracts::BackendErrorCode::InvalidAudio,
                message: "audio capability request must use the granted project's source audio"
                    .into(),
            });
        }
        validate_processing_resource_paths(request)
    }

    fn start_request(
        &self,
        project_id: ProjectId,
        request: ProcessingRequest,
    ) -> Result<ProjectProcessingJob, BackendError> {
        let config = AudioProcessingConfigSnapshot::from(&request);
        let job = self
            .workspace
            .start_project_processing_with_request(project_id.clone(), request)?;
        let handle = AudioJobHandle {
            asr_job_id: job.asr_context.job_id(),
            diarization_job_id: job.diarization_context.job_id(),
        };
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| storage_error("audio capability lifecycle lock poisoned".into()))?;
        if let Err(error) = self.ensure_project(&project_id) {
            job.cancel();
            self.workspace.discard_processing_job(&job);
            return Err(error);
        }
        if let Err(error) = self.registry.register(
            handle,
            project_id.clone(),
            config,
            job.asr_context.clone(),
            job.diarization_context.clone(),
        ) {
            job.cancel();
            self.workspace.discard_processing_job(&job);
            return Err(error);
        }

        let ProjectProcessingJob {
            future,
            asr_context,
            diarization_context,
            asr_events,
            diarization_events,
        } = job;
        let registry = self.registry.clone();
        let workspace = self.workspace.clone();
        let asr_context_for_terminal = asr_context.clone();
        let diarization_context_for_terminal = diarization_context.clone();
        let wrapped_future: BackendFuture<domain::DocumentState> = Box::pin(async move {
            let result = future.await;
            let snapshots = [
                workspace
                    .processing()
                    .job_snapshot(asr_context_for_terminal.job_id())
                    .unwrap_or(JobSnapshot {
                        job_id: asr_context_for_terminal.job_id(),
                        state: JobState::Failed,
                        progress: None,
                        error: Some("ASR terminal snapshot unavailable".into()),
                    }),
                workspace
                    .processing()
                    .job_snapshot(diarization_context_for_terminal.job_id())
                    .unwrap_or(JobSnapshot {
                        job_id: diarization_context_for_terminal.job_id(),
                        state: JobState::Failed,
                        progress: None,
                        error: Some("diarization terminal snapshot unavailable".into()),
                    }),
            ];
            registry.complete(handle, snapshots, result.as_ref().ok().cloned());
            workspace.discard_processing_job_ids(handle.job_ids());
            result
        });
        Ok(ProjectProcessingJob {
            future: wrapped_future,
            asr_context,
            diarization_context,
            asr_events,
            diarization_events,
        })
    }

    pub fn registry(&self) -> AudioJobRegistry {
        self.registry.clone()
    }

    pub fn revoke(&self) -> Result<(), BackendError> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| storage_error("audio capability lifecycle lock poisoned".into()))?;
        self.registry.cancel_all();
        self.allowed_projects
            .lock()
            .map_err(|_| storage_error("audio capability grant lock poisoned".into()))?
            .clear();
        Ok(())
    }

    pub fn release(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<(), BackendError> {
        self.registry.release(project_id, handle)
    }

    pub fn config_snapshot(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<AudioProcessingConfigSnapshot, BackendError> {
        self.registry.config(project_id, handle)
    }

    pub fn result_for(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<Option<domain::DocumentState>, BackendError> {
        self.registry.result(project_id, handle)
    }

    pub fn snapshot_for(
        &self,
        project_id: &ProjectId,
        job_id: JobId,
    ) -> Result<JobSnapshot, BackendError> {
        if let Some(snapshot) = self.registry.snapshot(project_id, job_id)? {
            return Ok(snapshot);
        }
        self.registry.ensure_owned_job(project_id, job_id)?;
        self.workspace
            .processing()
            .job_snapshot(job_id)
            .map_err(|error| storage_error(error.to_string()))
    }

    pub fn cancel_handle(
        &self,
        project_id: &ProjectId,
        handle: AudioJobHandle,
    ) -> Result<(), BackendError> {
        self.registry.cancel(project_id, handle)
    }

    fn ensure_project(&self, project_id: &ProjectId) -> Result<(), BackendError> {
        if self
            .allowed_projects
            .lock()
            .map_err(|_| storage_error("audio capability grant lock poisoned".into()))?
            .contains(project_id)
        {
            Ok(())
        } else {
            Err(BackendError {
                code: contracts::BackendErrorCode::AccessDenied,
                message: "audio capability grant does not include this project".into(),
            })
        }
    }
}

#[derive(Clone)]
pub struct SettingsCapabilityPort {
    settings: SettingsService,
    grant: SettingsGrant,
    #[cfg(test)]
    snapshot_barrier: Option<Arc<std::sync::Barrier>>,
}

impl SettingsCapabilityPort {
    pub fn new(settings: SettingsService, fields: impl IntoIterator<Item = String>) -> Self {
        Self {
            settings,
            grant: settings_grant_for_fields(fields),
            #[cfg(test)]
            snapshot_barrier: None,
        }
    }

    pub fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: "settings.app".into(),
            version: 1,
            read_only: !(self.grant.can_write_connections()
                || self.grant.can_write_connection_targets()
                || self.grant.can_write_credentials()
                || self.grant.can_write_write_policy()),
            long_running: false,
            operations: vec![
                operation(
                    "snapshot",
                    false,
                    false,
                    contracts::schema_for_arguments::<SettingsSnapshotArguments>(),
                    settings_public_snapshot_schema(),
                ),
                operation(
                    "read",
                    false,
                    false,
                    contracts::schema_for_arguments::<SettingsReadArguments>(),
                    settings_read_schema(),
                ),
                operation(
                    "patch",
                    true,
                    false,
                    settings_patch_input_schema(),
                    settings_mutation_schema(),
                ),
            ],
        }
    }

    pub fn read_connections(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<AssistantConnectionSummary>>, SettingsError> {
        self.settings.read_connections(self.grant.intersect(grant))
    }

    pub fn read_connection_targets(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<AssistantConnectionTarget>>, SettingsError> {
        self.settings
            .read_connection_targets(self.grant.intersect(grant))
    }

    pub fn read_credentials(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<Vec<CredentialStatus>>, SettingsError> {
        self.settings.read_credentials(self.grant.intersect(grant))
    }

    pub fn read_write_policy(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsReadResult<AssistantWritePolicy>, SettingsError> {
        self.settings.read_write_policy(self.grant.intersect(grant))
    }

    pub fn patch(
        &self,
        expected_revision: &str,
        patch: SettingsPatch,
        grant: SettingsGrant,
    ) -> Result<SettingsCapabilityMutation, SettingsError> {
        // Creating a connection is a desktop Settings action; capability
        // callers edit only existing profiles.
        if patch.new_connection.is_some() {
            return Err(SettingsError::FieldNotGranted {
                field: "assistant.connections".into(),
            });
        }
        let grant = self.grant.intersect(grant);
        let settings = self
            .settings
            .patch_if_equal(expected_revision, patch, grant)?;
        Ok(settings_mutation(&settings, grant))
    }

    pub fn public_snapshot(
        &self,
        grant: SettingsGrant,
    ) -> Result<SettingsPublicSnapshotWire, SettingsError> {
        let grant = self.grant.intersect(grant);
        ensure_all_settings_readable(grant)?;
        let settings = self.settings.snapshot()?;
        #[cfg(test)]
        if let Some(barrier) = &self.snapshot_barrier {
            barrier.wait();
            barrier.wait();
        }
        Ok(settings_public_snapshot_from(&settings))
    }
}

fn settings_public_snapshot_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<SettingsPublicSnapshotWire>()
}

fn settings_patch_input_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<SettingsPatchArguments>()
}

fn settings_read_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<SettingsReadResult<serde_json::Value>>()
}

fn settings_mutation_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<SettingsCapabilityMutation>()
}

fn validate_processing_resource_paths(request: &ProcessingRequest) -> Result<(), BackendError> {
    for (name, model) in [
        ("ASR", &request.asr_model),
        ("diarization", &request.diarization_model),
    ] {
        validate_model_resource_path(name, model)?;
    }
    for (name, model) in [
        (
            "segmentation",
            request.stage_configuration.segmentation_model.as_ref(),
        ),
        (
            "embedding",
            request.stage_configuration.embedding_model.as_ref(),
        ),
    ] {
        if let Some(model) = model {
            validate_model_resource_path(name, model)?;
        }
    }
    if let Some(path) = request.speaker_library_path.as_deref() {
        validate_managed_relative_path("Speaker Library", path)?;
    }
    Ok(())
}

fn validate_model_resource_path(name: &str, model: &ModelRef) -> Result<(), BackendError> {
    validate_managed_relative_path(name, &model.artifact_path)?;
    for artifact in &model.artifacts {
        validate_managed_relative_path(name, &artifact.filename)?;
        if let Some(member) = artifact.archive_member.as_deref() {
            validate_managed_relative_path(name, member)?;
        }
    }
    Ok(())
}

fn validate_managed_relative_path(name: &str, value: &str) -> Result<(), BackendError> {
    if value.is_empty() {
        return Ok(());
    }
    let path = std::path::Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(BackendError {
            code: contracts::BackendErrorCode::AccessDenied,
            message: format!("{name} resource path is outside the managed root"),
        });
    }
    Ok(())
}

/// The one mapping from trusted or requested settings field names to a grant.
/// Unknown names grant nothing.
pub(crate) fn settings_grant_for_fields(
    fields: impl IntoIterator<Item = impl AsRef<str>>,
) -> SettingsGrant {
    fields
        .into_iter()
        .fold(SettingsGrant::read_only(), |grant, field| {
            match field.as_ref() {
                "assistant" => SettingsGrant::full(),
                "assistant.connections" => grant.with_connections(),
                "assistant.connectionTargets" => grant.with_connection_targets(),
                "assistant.credentials" => grant.with_credentials(),
                "assistant.writePolicy" => grant.with_write_policy(),
                _ => grant,
            }
        })
}

fn ensure_all_settings_readable(grant: SettingsGrant) -> Result<(), SettingsError> {
    for (field, readable) in [
        ("assistant.connections", grant.can_read_connections()),
        (
            "assistant.connectionTargets",
            grant.can_read_connection_targets(),
        ),
        ("assistant.credentials", grant.can_read_credentials()),
        ("assistant.writePolicy", grant.can_read_write_policy()),
    ] {
        if !readable {
            return Err(SettingsError::FieldNotGranted {
                field: field.into(),
            });
        }
    }
    Ok(())
}

/// Project a committed configuration through the caller's grant. Even a full
/// Assistant grant sees only `assistant` sections and credential status.
fn settings_mutation(settings: &AppSettings, grant: SettingsGrant) -> SettingsCapabilityMutation {
    let mut assistant = serde_json::Map::new();
    if grant.can_read_write_policy() {
        assistant.insert(
            "enabled".into(),
            serde_json::json!(settings.assistant.enabled),
        );
        assistant.insert(
            "activeConnectionId".into(),
            serde_json::json!(settings.assistant.active_connection_id),
        );
        assistant.insert(
            "writeMode".into(),
            serde_json::json!(settings.assistant.write_mode),
        );
        assistant.insert(
            "maxToolRounds".into(),
            serde_json::json!(settings.assistant.max_tool_rounds),
        );
        assistant.insert(
            "maxValidationRetries".into(),
            serde_json::json!(settings.assistant.max_validation_retries),
        );
    }
    if grant.can_read_connections()
        || grant.can_read_connection_targets()
        || grant.can_read_credentials()
    {
        assistant.insert(
            "connections".into(),
            serde_json::to_value(
                settings
                    .assistant
                    .connections
                    .iter()
                    .map(|connection| {
                        let mut value = serde_json::Map::new();
                        value.insert("id".into(), serde_json::json!(connection.id));
                        if grant.can_read_connections() {
                            value.insert("name".into(), serde_json::json!(connection.name));
                        }
                        if grant.can_read_connection_targets() {
                            value.insert("protocol".into(), serde_json::json!(connection.protocol));
                            value.insert("baseUrl".into(), serde_json::json!(connection.base_url));
                            value.insert("model".into(), serde_json::json!(connection.model));
                            value
                                .insert("authMode".into(), serde_json::json!(connection.auth_mode));
                            value.insert(
                                "contextWindowTokens".into(),
                                serde_json::json!(connection.context_window_tokens),
                            );
                            value.insert(
                                "timeoutMs".into(),
                                serde_json::json!(connection.timeout_ms),
                            );
                            value.insert(
                                "maxOutputTokens".into(),
                                serde_json::json!(connection.max_output_tokens),
                            );
                        }
                        if grant.can_read_credentials() {
                            value.insert(
                                "credentialStatus".into(),
                                serde_json::json!({
                                    "hasApiKey": connection.api_key.is_some(),
                                    "hasApiKeyFile": connection.api_key_file.is_some()
                                }),
                            );
                        }
                        serde_json::Value::Object(value)
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("settings connection projection serializes"),
        );
    }
    let mut projected = serde_json::Map::new();
    projected.insert("assistant".into(), serde_json::Value::Object(assistant));
    SettingsCapabilityMutation {
        revision: SettingsService::revision_of(settings),
        settings: serde_json::Value::Object(projected),
    }
}

fn settings_public_snapshot_from(settings: &AppSettings) -> SettingsPublicSnapshotWire {
    let revision = SettingsService::revision_of(settings);
    SettingsPublicSnapshotWire {
        revision: revision.clone(),
        connections: SettingsReadResult {
            revision: revision.clone(),
            value: settings
                .assistant
                .connections
                .iter()
                .map(|connection| AssistantConnectionSummary {
                    id: connection.id.clone(),
                    name: connection.name.clone(),
                })
                .collect(),
        },
        connection_targets: SettingsReadResult {
            revision: revision.clone(),
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
        },
        credentials: SettingsReadResult {
            revision: revision.clone(),
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
        },
        write_policy: SettingsReadResult {
            revision,
            value: AssistantWritePolicy {
                enabled: settings.assistant.enabled,
                active_connection_id: settings.assistant.active_connection_id.clone(),
                write_mode: settings.assistant.write_mode,
                max_tool_rounds: settings.assistant.max_tool_rounds,
                max_validation_retries: settings.assistant.max_validation_retries,
            },
        },
    }
}

fn storage_error(message: String) -> BackendError {
    BackendError {
        code: contracts::BackendErrorCode::PersistenceFailed,
        message,
    }
}

/// Decode typed capability arguments. They are always JSON objects: serde would
/// otherwise accept a JSON array positionally for the same struct. Each caller
/// maps the message to its owning service's error code.
pub(crate) fn decode_object_arguments<T: serde::de::DeserializeOwned>(
    arguments: &serde_json::Value,
) -> Result<T, String> {
    if !arguments.is_object() {
        return Err("arguments must be a JSON object".into());
    }
    serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())
}

fn operation(
    name: &str,
    side_effect: bool,
    long_running: bool,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
) -> CapabilityOperation {
    CapabilityOperation {
        name: name.into(),
        description: name.into(),
        side_effect,
        long_running,
        input_schema,
        output_schema,
    }
}

fn audio_input_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioModelArguments>()
}

fn audio_probe_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioProbeWire>()
}

fn audio_snapshot_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioQueryArguments>()
}

fn audio_query_schema() -> serde_json::Value {
    audio_snapshot_schema()
}

fn audio_cancel_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioHandleArguments>()
}

fn audio_handle_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioHandleArguments>()
}

fn audio_job_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioStartWire>()
}

fn job_snapshot_schema() -> serde_json::Value {
    contracts::schema_for_arguments::<AudioJobSnapshotWire>()
}

fn missing_project(project_id: &ProjectId) -> BackendError {
    BackendError {
        code: contracts::BackendErrorCode::InvalidAudio,
        message: format!("project does not exist: {}", project_id.as_str()),
    }
}

fn access_denied(message: impl Into<String>) -> BackendError {
    BackendError {
        code: contracts::BackendErrorCode::AccessDenied,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsrBackend, BackendFuture, DiarizationBackend};
    use contracts::{
        AsrRequest, AsrResult, BackendDescriptor, BackendTask, DiarizationRequest,
        DiarizationResult,
    };
    use futures::future;
    use settings::SettingsRepository;
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::Receiver,
        Arc, Barrier, Mutex,
    };

    struct MemorySettings(Mutex<AppSettings>);
    impl SettingsRepository for MemorySettings {
        fn load(&self) -> Result<AppSettings, SettingsError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, value: &AppSettings) -> Result<(), SettingsError> {
            *self.0.lock().unwrap() = value.clone();
            Ok(())
        }
    }

    struct MemoryDocuments(Mutex<BTreeMap<String, contracts::DocumentRecord>>);
    impl crate::DocumentRepository for MemoryDocuments {
        fn snapshot(&self) -> Result<crate::DocumentSnapshot, String> {
            Ok(crate::DocumentSnapshot {
                documents: self
                    .0
                    .lock()
                    .unwrap()
                    .values()
                    .map(|record| record.state.clone())
                    .collect(),
                issues: Vec::new(),
            })
        }
        fn get(&self, id: &domain::DocumentId) -> Result<Option<domain::DocumentState>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(id.as_str())
                .map(|record| record.state.clone()))
        }
        fn read(
            &self,
            id: &domain::DocumentId,
        ) -> Result<Option<contracts::DocumentRecord>, String> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }
        fn watch(&self) -> Result<Receiver<crate::DocumentChange>, String> {
            Ok(std::sync::mpsc::channel().1)
        }
        fn save(
            &self,
            state: domain::DocumentState,
            markdown: Option<String>,
        ) -> Result<(), String> {
            let id = state.id().as_str().to_string();
            self.0
                .lock()
                .unwrap()
                .entry(id)
                .and_modify(|record| {
                    record.state = state.clone();
                    if let Some(markdown) = markdown.clone() {
                        record.markdown = markdown;
                    }
                })
                .or_insert_with(|| contracts::DocumentRecord {
                    state,
                    markdown: markdown.unwrap_or_default(),
                });
            Ok(())
        }
    }

    struct NoopAsr;
    impl AsrBackend for NoopAsr {
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: "noop-asr".into(),
                task: BackendTask::Asr,
                capabilities: Vec::new(),
                configurable_diarization_stages: Vec::new(),
                diarization_stage_options: Vec::new(),
            }
        }
        fn probe(&self, _: &ModelRef) -> Result<(), contracts::BackendError> {
            Ok(())
        }
        fn transcribe(&self, _: AsrRequest, _: runtime::JobContext) -> BackendFuture<AsrResult> {
            Box::pin(future::ready(Err(contracts::BackendError {
                code: contracts::BackendErrorCode::NotImplemented,
                message: "noop".into(),
            })))
        }
    }

    struct NoopDiarization;
    impl DiarizationBackend for NoopDiarization {
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: "noop-diarization".into(),
                task: BackendTask::Diarization,
                capabilities: Vec::new(),
                configurable_diarization_stages: Vec::new(),
                diarization_stage_options: Vec::new(),
            }
        }
        fn probe(&self, _: &ModelRef) -> Result<(), contracts::BackendError> {
            Ok(())
        }
        fn diarize(
            &self,
            _: DiarizationRequest,
            _: runtime::JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(future::ready(Err(contracts::BackendError {
                code: contracts::BackendErrorCode::NotImplemented,
                message: "noop".into(),
            })))
        }
    }

    struct EchoBinding {
        dispatch_count: Arc<AtomicUsize>,
    }

    impl crate::CapabilityBinding for EchoBinding {
        fn descriptor(&self, _: &crate::AgentHost) -> CapabilityDescriptor {
            CapabilityDescriptor {
                id: "test.echo".into(),
                version: 7,
                read_only: false,
                long_running: false,
                operations: vec![CapabilityOperation {
                    name: "echo".into(),
                    description: "Return the supplied test payload.".into(),
                    side_effect: false,
                    long_running: false,
                    input_schema: serde_json::json!({
                        "type": "object",
                        "required": ["value"],
                        "properties": {"value": {"type": "string"}}
                    }),
                    output_schema: serde_json::json!({
                        "type": "object",
                        "required": ["binding", "value"],
                        "properties": {
                            "binding": {"const": "echo"},
                            "value": {"type": "string"}
                        }
                    }),
                }],
            }
        }

        fn dispatch(
            &self,
            _: &crate::AgentHost,
            request: contracts::CapabilityCallRequest,
        ) -> contracts::CapabilityCallResponse {
            self.dispatch_count.fetch_add(1, Ordering::SeqCst);
            if request.operation != "echo" {
                return contracts::CapabilityCallResponse::failure(
                    &request,
                    contracts::CapabilityServiceError::new(
                        "TEST_OPERATION_UNKNOWN",
                        "test binding operation is not supported",
                        false,
                    )
                    .with_context(contracts::CapabilityErrorContext::new(
                        contracts::CapabilityService::Workspace,
                        request.operation.clone(),
                    )),
                );
            }
            contracts::CapabilityCallResponse::success(
                &request,
                serde_json::json!({
                    "binding": "echo",
                    "value": request.arguments["value"]
                }),
            )
        }
    }

    #[test]
    fn settings_capability_requires_revision_and_explicit_field_grant() {
        let service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let port = SettingsCapabilityPort::new(service.clone(), ["assistant".into()]);
        let rounds = |value| SettingsPatch {
            max_tool_rounds: Some(value),
            ..Default::default()
        };
        let revision = service.revision().unwrap();
        let updated = port
            .patch(&revision, rounds(4), SettingsGrant::full())
            .unwrap();
        assert_ne!(revision, updated.revision);
        assert!(matches!(
            port.patch(&revision, rounds(5), SettingsGrant::full()),
            Err(SettingsError::RevisionConflict)
        ));
        let forbidden = port.patch(
            &updated.revision,
            rounds(5),
            SettingsGrant::assistant_connections(),
        );
        assert!(
            matches!(
                &forbidden,
                Err(SettingsError::FieldNotGranted { field }) if field == "assistant.writePolicy"
            ),
            "{forbidden:?}"
        );
        assert_eq!(service.snapshot().unwrap().assistant.max_tool_rounds, 4);
    }

    #[test]
    fn capability_wire_schemas_match_their_host_outputs() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings_service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let all_settings = [
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]),
            SettingsCapabilityPort::new(settings_service, all_settings.clone()),
            all_settings.clone(),
        );
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "noop".into(),
            revision: None,
            artifact_path: String::new(),
            artifacts: Vec::new(),
        };
        let project_authorization = contracts::CapabilityAuthorization {
            project_ids: vec!["allowed".into()],
            ..Default::default()
        };
        let probe = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-probe",
            "wire-probe",
            "audio.processing",
            1,
            "probe",
            project_authorization.clone(),
            serde_json::json!({
                "projectId": "allowed",
                "asrModel": model("noop-asr"),
                "diarizationModel": model("noop-diarization")
            }),
        ));
        assert!(probe.ok, "{probe:?}");
        let start = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-start",
            "wire-start",
            "audio.processing",
            1,
            "start",
            project_authorization.clone(),
            serde_json::json!({
                "projectId": "allowed",
                "asrModel": model("noop-asr"),
                "diarizationModel": model("noop-diarization")
            }),
        ));
        assert!(start.ok, "{start:?}");
        let operation_schema = |capability: &str, operation: &str| {
            host.discover()
                .into_iter()
                .find(|descriptor| descriptor.id == capability)
                .unwrap()
                .operations
                .into_iter()
                .find(|candidate| candidate.name == operation)
                .unwrap()
                .output_schema
        };
        let assert_required = |schema: serde_json::Value, value: &serde_json::Value| {
            for field in schema["required"].as_array().unwrap() {
                assert!(value.get(field.as_str().unwrap()).is_some(), "{value}");
            }
        };
        assert_required(
            operation_schema("audio.processing", "probe"),
            probe.result.as_ref().unwrap(),
        );
        assert_required(
            operation_schema("audio.processing", "start"),
            start.result.as_ref().unwrap(),
        );
        let asr_job_id = start.result.as_ref().unwrap()["asrJobId"].as_u64().unwrap();
        let diarization_job_id = start.result.as_ref().unwrap()["diarizationJobId"]
            .as_u64()
            .unwrap();
        let query = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-query",
            "wire-query",
            "audio.processing",
            1,
            "query",
            project_authorization,
            serde_json::json!({"projectId": "allowed", "jobId": asr_job_id}),
        ));
        assert!(query.ok, "{query:?}");
        assert!(query.result.as_ref().unwrap()["jobId"].is_number());
        assert!(query.result.as_ref().unwrap()["job_id"].is_null());
        assert_required(
            operation_schema("audio.processing", "query"),
            query.result.as_ref().unwrap(),
        );

        let cancelled = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-cancel",
            "wire-cancel",
            "audio.processing",
            1,
            "cancel",
            contracts::CapabilityAuthorization {
                project_ids: vec!["allowed".into()],
                ..Default::default()
            },
            serde_json::json!({
                "projectId": "allowed",
                "asrJobId": asr_job_id,
                "diarizationJobId": diarization_job_id
            }),
        ));
        assert!(cancelled.ok, "{cancelled:?}");
        assert_required(
            operation_schema("audio.processing", "cancel"),
            cancelled.result.as_ref().unwrap(),
        );
        let result = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-result",
            "wire-result",
            "audio.processing",
            1,
            "result",
            contracts::CapabilityAuthorization {
                project_ids: vec!["allowed".into()],
                ..Default::default()
            },
            serde_json::json!({
                "projectId": "allowed",
                "asrJobId": asr_job_id,
                "diarizationJobId": diarization_job_id
            }),
        ));
        assert!(result.ok, "{result:?}");
        assert_required(
            operation_schema("audio.processing", "result"),
            result.result.as_ref().unwrap(),
        );

        let settings_snapshot = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-settings",
            "wire-settings",
            "settings.app",
            1,
            "snapshot",
            contracts::CapabilityAuthorization {
                settings_fields: all_settings.to_vec(),
                ..Default::default()
            },
            serde_json::json!({}),
        ));
        assert!(settings_snapshot.ok, "{settings_snapshot:?}");
        assert!(settings_snapshot.result.as_ref().unwrap()["writePolicy"]["value"].is_object());
        assert_required(
            operation_schema("settings.app", "snapshot"),
            settings_snapshot.result.as_ref().unwrap(),
        );
        assert!(operation_schema("settings.app", "snapshot")
            .to_string()
            .contains("\"required\":[\"revision\",\"value\"]"));
        let settings_authorization = contracts::CapabilityAuthorization {
            settings_fields: all_settings.to_vec(),
            ..Default::default()
        };
        let read = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-read",
            "wire-read",
            "settings.app",
            1,
            "read",
            settings_authorization.clone(),
            serde_json::json!({"section": "writePolicy"}),
        ));
        assert!(read.ok, "{read:?}");
        assert_required(
            operation_schema("settings.app", "read"),
            read.result.as_ref().unwrap(),
        );
        let patched = host.dispatch(contracts::CapabilityCallRequest::new(
            "wire-patch",
            "wire-patch",
            "settings.app",
            1,
            "patch",
            settings_authorization,
            serde_json::json!({
                "expectedRevision": settings_snapshot.result.as_ref().unwrap()["revision"],
                "patch": {"writeMode": "confirm_each"}
            }),
        ));
        assert!(patched.ok, "{patched:?}");
        assert_required(
            operation_schema("settings.app", "patch"),
            patched.result.as_ref().unwrap(),
        );
    }

    #[test]
    fn settings_host_snapshot_is_one_consistent_configuration_version() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings_service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let fields = vec![
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        let snapshot_barrier = Arc::new(Barrier::new(2));
        let mut settings_port =
            SettingsCapabilityPort::new(settings_service.clone(), fields.clone());
        settings_port.snapshot_barrier = Some(snapshot_barrier.clone());
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, std::iter::empty()),
            settings_port,
            fields.clone(),
        );
        let host_for_thread = host.clone();
        let snapshot = std::thread::spawn(move || {
            host_for_thread.dispatch(contracts::CapabilityCallRequest::new(
                "settings-consistent-snapshot",
                "settings-consistent-snapshot",
                "settings.app",
                1,
                "snapshot",
                contracts::CapabilityAuthorization {
                    settings_fields: fields,
                    ..Default::default()
                },
                serde_json::json!({}),
            ))
        });
        snapshot_barrier.wait();
        let mut next = settings_service.snapshot().unwrap();
        next.assistant.max_tool_rounds = 5;
        settings_service.update(next).unwrap();
        snapshot_barrier.wait();
        let response = snapshot.join().unwrap();
        assert!(response.ok, "{response:?}");
        let result = response.result.unwrap();
        let revision = result["revision"].as_str().unwrap();
        for section in [
            "connections",
            "connectionTargets",
            "credentials",
            "writePolicy",
        ] {
            assert_eq!(result[section]["revision"].as_str(), Some(revision));
        }
    }

    #[test]
    fn settings_capability_port_cannot_expand_or_leak_a_write_policy_grant() {
        let mut configured = AppSettings::default();
        configured
            .assistant
            .connections
            .push(settings::AssistantConnectionProfile {
                id: "primary".into(),
                name: "Primary".into(),
                protocol: settings::LlmProtocol::OpenAiChatCompletions,
                base_url: "http://localhost:1234".into(),
                model: "fixture".into(),
                auth_mode: settings::AssistantAuthMode::Bearer,
                api_key: Some("secret".into()),
                api_key_file: None,
                context_window_tokens: Some(8_192),
                timeout_ms: 1_000,
                max_output_tokens: 128,
            });
        let service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(configured)))).unwrap();
        let port = SettingsCapabilityPort::new(service.clone(), ["assistant.writePolicy".into()]);
        assert!(port.read_write_policy(SettingsGrant::full()).is_ok());
        assert!(port.read_connections(SettingsGrant::full()).is_err());
        assert!(port.read_connection_targets(SettingsGrant::full()).is_err());
        assert!(port.read_credentials(SettingsGrant::full()).is_err());
        assert!(port.public_snapshot(SettingsGrant::full()).is_err());
        let patched = port
            .patch(
                &service.revision().unwrap(),
                SettingsPatch {
                    write_mode: Some(contracts::AgentWriteMode::ConfirmEach),
                    ..Default::default()
                },
                SettingsGrant::full(),
            )
            .unwrap();
        assert_eq!(patched.settings["assistant"]["writeMode"], "confirm_each");
        assert!(patched.settings["assistant"].get("connections").is_none());
        assert!(!patched.settings.to_string().contains("secret"));

        let revision = service.revision().unwrap();
        let patch = port.patch(
            &revision,
            SettingsPatch {
                connection: Some(settings::AssistantConnectionPatch {
                    id: "primary".into(),
                    name: Some("Changed".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            SettingsGrant::full(),
        );
        assert!(patch.is_err());
        assert_eq!(
            service.snapshot().unwrap().assistant.connections[0].name,
            "Primary"
        );
    }

    #[test]
    fn settings_capability_cannot_create_a_connection_with_its_credential() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let fields = vec!["assistant".to_string()];
        let port = SettingsCapabilityPort::new(service.clone(), fields.clone());
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, std::iter::empty()),
            port.clone(),
            fields.clone(),
        );
        let profile = settings::AssistantConnectionProfile {
            id: "created".into(),
            name: "Created".into(),
            protocol: settings::LlmProtocol::OpenAiChatCompletions,
            base_url: "http://localhost:1234".into(),
            model: "fixture".into(),
            auth_mode: settings::AssistantAuthMode::Bearer,
            api_key: None,
            api_key_file: None,
            context_window_tokens: Some(8_192),
            timeout_ms: 1_000,
            max_output_tokens: 128,
        };
        let credential = settings::AssistantConnectionPatch {
            id: "created".into(),
            credential: Some(settings::CredentialUpdate::Direct("created-secret".into())),
            ..Default::default()
        };

        // Even a caller holding every Assistant field cannot pass a typed
        // creation through the port.
        let typed = port.patch(
            &service.revision().unwrap(),
            SettingsPatch {
                active_connection_id: Some(Some("created".into())),
                connection: Some(credential.clone()),
                new_connection: Some(profile.clone()),
                ..Default::default()
            },
            SettingsGrant::full(),
        );
        assert!(
            matches!(
                &typed,
                Err(SettingsError::FieldNotGranted { field }) if field == "assistant.connections"
            ),
            "{typed:?}"
        );
        assert_eq!(service.snapshot().unwrap(), AppSettings::default());

        // The wire contract has no creation field, and the Host rejects one.
        let patch_schema = port
            .descriptor()
            .operations
            .into_iter()
            .find(|operation| operation.name == "patch")
            .unwrap()
            .input_schema
            .to_string();
        assert!(
            !patch_schema.contains("newConnection") && !patch_schema.contains("new_connection"),
            "{patch_schema}"
        );
        let response = host.dispatch(contracts::CapabilityCallRequest::new(
            "settings-create-connection",
            "settings-create-connection",
            "settings.app",
            1,
            "patch",
            contracts::CapabilityAuthorization {
                settings_fields: fields,
                ..Default::default()
            },
            serde_json::json!({
                "expectedRevision": service.revision().unwrap(),
                "patch": {
                    "activeConnectionId": "created",
                    "connection": credential,
                    "newConnection": profile,
                }
            }),
        ));
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "SETTINGS_ARGUMENTS_INVALID");
        assert_eq!(service.snapshot().unwrap(), AppSettings::default());
    }

    #[test]
    fn settings_host_rejects_unknown_arguments_before_reading_state() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let fields = vec![
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, std::iter::empty()),
            SettingsCapabilityPort::new(settings, fields.clone()),
            fields.clone(),
        );
        let authorization = contracts::CapabilityAuthorization {
            settings_fields: fields,
            ..Default::default()
        };
        let snapshot = host.dispatch(contracts::CapabilityCallRequest::new(
            "settings-unknown-snapshot",
            "settings-unknown-snapshot",
            "settings.app",
            1,
            "snapshot",
            authorization.clone(),
            serde_json::json!({"unexpected": true}),
        ));
        assert!(!snapshot.ok);
        assert_eq!(snapshot.error.unwrap().code, "SETTINGS_ARGUMENTS_INVALID");
        let read = host.dispatch(contracts::CapabilityCallRequest::new(
            "settings-unknown-read",
            "settings-unknown-read",
            "settings.app",
            1,
            "read",
            authorization.clone(),
            serde_json::json!({"section": "writePolicy", "unexpected": true}),
        ));
        assert!(!read.ok);
        assert_eq!(read.error.unwrap().code, "SETTINGS_ARGUMENTS_INVALID");
        let unknown_section = host.dispatch(contracts::CapabilityCallRequest::new(
            "settings-unknown-section",
            "settings-unknown-section",
            "settings.app",
            1,
            "read",
            authorization,
            serde_json::json!({"section": "assistant.writePolicy"}),
        ));
        assert!(!unknown_section.ok);
        let error = unknown_section.error.unwrap();
        assert_eq!(error.code, "SETTINGS_ARGUMENTS_INVALID");
        let context = error.context.unwrap();
        assert_eq!(
            context.service,
            Some(contracts::CapabilityService::Settings)
        );
        assert_eq!(context.operation.as_deref(), Some("read"));
    }

    #[test]
    fn capability_descriptors_expose_typed_required_fields_for_audio_and_settings() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents,
        );
        let audio = AudioCapabilityPort::new(workspace, std::iter::empty());
        let start = audio
            .descriptor()
            .operations
            .into_iter()
            .find(|operation| operation.name == "start")
            .unwrap();
        assert_eq!(
            start.input_schema["required"],
            serde_json::json!(["projectId", "asrModel", "diarizationModel"])
        );
        assert_eq!(
            start.output_schema["required"],
            serde_json::json!(["projectId", "asrJobId", "diarizationJobId"])
        );

        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let settings = SettingsCapabilityPort::new(settings, std::iter::empty::<String>());
        let patch = settings
            .descriptor()
            .operations
            .into_iter()
            .find(|operation| operation.name == "patch")
            .unwrap();
        assert_eq!(
            patch.input_schema["required"],
            serde_json::json!(["expectedRevision", "patch"])
        );
    }

    fn sentinel_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.asr_backend_id = Some("whisper-rs".into());
        settings.asr_model = Some(ModelRef {
            backend_id: "whisper-rs".into(),
            model_id: "SENTINEL_ASR_MODEL".into(),
            revision: None,
            artifact_path: "models/SENTINEL_ASR_PATH".into(),
            artifacts: Vec::new(),
        });
        settings.diarization_backend_id = Some("diarization-native".into());
        settings.diarization_model = Some(ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "SENTINEL_DIARIZATION_MODEL".into(),
            revision: None,
            artifact_path: "models/SENTINEL_DIARIZATION_PATH".into(),
            artifacts: Vec::new(),
        });
        settings.language = Some("SENTINEL_LANGUAGE".into());
        settings.speaker_library_path = "data/speakers/SENTINEL_LIBRARY.json".into();
        let connection = |id: &str, api_key: Option<&str>, api_key_file: Option<&str>| {
            settings::AssistantConnectionProfile {
                id: id.into(),
                name: format!("Connection {id}"),
                protocol: settings::LlmProtocol::OpenAiChatCompletions,
                base_url: "http://localhost:1234".into(),
                model: "fixture".into(),
                auth_mode: settings::AssistantAuthMode::Bearer,
                api_key: api_key.map(str::to_owned),
                api_key_file: api_key_file.map(str::to_owned),
                context_window_tokens: Some(8_192),
                timeout_ms: 1_000,
                max_output_tokens: 128,
            }
        };
        settings.assistant.active_connection_id = Some("direct".into());
        settings.assistant.connections = vec![
            connection("direct", Some("SENTINEL_DIRECT_KEY"), None),
            connection("file", None, Some("credentials/SENTINEL_KEY_FILE")),
        ];
        // Desktop-only: never part of the Settings capability projection.
        settings.appearance = settings::AppearanceSettings {
            theme: settings::ThemePreference::Light,
            material: settings::MaterialPreference::Opaque,
        };
        settings
    }

    #[test]
    fn assistant_grants_never_project_unrelated_settings_or_credentials() {
        const SENTINEL: &str = "SENTINEL";
        let all_fields: Vec<String> = vec![
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        for (trusted, requested) in [
            (all_fields.clone(), all_fields.clone()),
            (vec!["assistant".to_string()], all_fields.clone()),
        ] {
            let initial = sentinel_settings();
            let settings_service =
                SettingsService::open(Arc::new(MemorySettings(Mutex::new(initial.clone()))))
                    .unwrap();
            let documents =
                crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
            let workspace = crate::WorkspaceService::new(
                crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
                documents.clone(),
            );
            let host = crate::AgentHost::new(
                crate::WorkspaceCapabilityPort::new(
                    documents,
                    crate::WorkspaceAccessPolicy::AllDocuments,
                ),
                AudioCapabilityPort::new(workspace, std::iter::empty()),
                SettingsCapabilityPort::new(settings_service.clone(), trusted.clone()),
                trusted.clone(),
            );
            let call = |operation: &str, arguments: serde_json::Value| {
                host.dispatch(contracts::CapabilityCallRequest::new(
                    format!("sentinel-{operation}"),
                    format!("sentinel-{operation}"),
                    "settings.app",
                    1,
                    operation,
                    contracts::CapabilityAuthorization {
                        settings_fields: requested.clone(),
                        ..Default::default()
                    },
                    arguments,
                ))
            };
            let descriptor = host
                .discover()
                .into_iter()
                .find(|descriptor| descriptor.id == "settings.app")
                .unwrap();
            let operations = descriptor
                .operations
                .iter()
                .map(|operation| operation.name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(operations, ["snapshot", "read", "patch"]);
            for operation in &descriptor.operations {
                for schema in [&operation.input_schema, &operation.output_schema] {
                    assert!(
                        !schema.to_string().contains("appearance"),
                        "{} schema exposes appearance: {schema}",
                        operation.name
                    );
                }
            }

            let mut public_results = Vec::new();
            let snapshot = call("snapshot", serde_json::json!({}));
            assert!(snapshot.ok, "{snapshot:?}");
            let revision = snapshot.result.as_ref().unwrap()["revision"]
                .as_str()
                .unwrap()
                .to_owned();
            public_results.push(serde_json::to_string(&snapshot).unwrap());
            for section in [
                "connections",
                "connectionTargets",
                "credentials",
                "writePolicy",
            ] {
                let read = call("read", serde_json::json!({"section": section}));
                assert!(read.ok, "{read:?}");
                public_results.push(serde_json::to_string(&read).unwrap());
            }
            let patched = call(
                "patch",
                serde_json::json!({
                    "expectedRevision": revision,
                    "patch": {"maxToolRounds": 6}
                }),
            );
            assert!(patched.ok, "{patched:?}");
            let projection = &patched.result.as_ref().unwrap()["settings"];
            assert_eq!(
                projection.as_object().unwrap().keys().collect::<Vec<_>>(),
                ["assistant"],
                "patch projection must contain only granted assistant sections: {projection}"
            );
            assert_eq!(projection["assistant"]["maxToolRounds"], 6);
            public_results.push(serde_json::to_string(&patched).unwrap());

            let removed = call(
                "update",
                serde_json::json!({
                    "expectedRevision": patched.result.as_ref().unwrap()["revision"],
                    "settings": initial
                }),
            );
            assert!(!removed.ok);
            assert_eq!(
                removed.error.as_ref().unwrap().code,
                "SETTINGS_OPERATION_UNKNOWN"
            );

            for result in &public_results {
                assert!(!result.contains(SENTINEL), "public result leaked: {result}");
                assert!(
                    !result.contains("appearance"),
                    "public result exposes appearance: {result}"
                );
            }
            let mut expected = initial;
            expected.assistant.max_tool_rounds = 6;
            assert_eq!(settings_service.snapshot().unwrap(), expected);
        }
    }

    #[test]
    fn settings_capability_port_neither_projects_nor_patches_the_documents_root() {
        const ROOT: &str = "/Volumes/SENTINEL_DOCUMENTS_ROOT";
        let fields: Vec<String> = vec![
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        let initial = AppSettings {
            documents_root: ROOT.into(),
            ..sentinel_settings()
        };
        let settings_service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(initial.clone())))).unwrap();
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, std::iter::empty()),
            SettingsCapabilityPort::new(settings_service.clone(), fields.clone()),
            fields.clone(),
        );
        let call = |operation: &str, arguments: serde_json::Value| {
            host.dispatch(contracts::CapabilityCallRequest::new(
                format!("root-{operation}"),
                format!("root-{operation}"),
                "settings.app",
                1,
                operation,
                contracts::CapabilityAuthorization {
                    settings_fields: fields.clone(),
                    ..Default::default()
                },
                arguments,
            ))
        };
        let descriptor = host
            .discover()
            .into_iter()
            .find(|descriptor| descriptor.id == "settings.app")
            .unwrap();
        for operation in &descriptor.operations {
            for schema in [&operation.input_schema, &operation.output_schema] {
                assert!(
                    !schema.to_string().contains("documentsRoot"),
                    "{} schema exposes the documents root: {schema}",
                    operation.name
                );
            }
        }

        let snapshot = call("snapshot", serde_json::json!({}));
        assert!(snapshot.ok, "{snapshot:?}");
        let revision = snapshot.result.as_ref().unwrap()["revision"].clone();
        let mut results = vec![serde_json::to_string(&snapshot).unwrap()];
        for section in [
            "connections",
            "connectionTargets",
            "credentials",
            "writePolicy",
        ] {
            let read = call("read", serde_json::json!({"section": section}));
            assert!(read.ok, "{read:?}");
            results.push(serde_json::to_string(&read).unwrap());
        }
        let rejected = call(
            "patch",
            serde_json::json!({
                "expectedRevision": revision,
                "patch": {"documentsRoot": "/tmp/elsewhere"}
            }),
        );
        assert!(!rejected.ok);
        assert_eq!(
            rejected.error.as_ref().unwrap().code,
            "SETTINGS_ARGUMENTS_INVALID"
        );
        let patched = call(
            "patch",
            serde_json::json!({
                "expectedRevision": revision,
                "patch": {"maxToolRounds": 5}
            }),
        );
        assert!(patched.ok, "{patched:?}");
        results.push(serde_json::to_string(&patched).unwrap());

        for result in &results {
            assert!(!result.contains(ROOT), "public result leaked: {result}");
            assert!(
                !result.contains("documentsRoot"),
                "public result exposes the documents root: {result}"
            );
        }
        let stored = settings_service.snapshot().unwrap();
        assert_eq!(stored.documents_root, ROOT);
        assert_eq!(stored.assistant.max_tool_rounds, 5);
    }

    #[test]
    fn builtin_capabilities_reject_non_object_arguments_with_service_codes() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings_service =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let fields: Vec<String> = vec![
            "assistant.connections".into(),
            "assistant.connectionTargets".into(),
            "assistant.credentials".into(),
            "assistant.writePolicy".into(),
        ];
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]),
            SettingsCapabilityPort::new(settings_service.clone(), fields.clone()),
            fields.clone(),
        );
        let revision = settings_service.revision().unwrap();
        let before = settings_service.snapshot().unwrap();
        for (capability, operation, authorization, arguments, code) in [
            (
                "workspace.documents",
                "read_document",
                contracts::CapabilityAuthorization {
                    document_ids: vec!["allowed".into()],
                    ..Default::default()
                },
                serde_json::json!(["allowed", "metadata"]),
                "TOOL_ARGUMENTS_INVALID",
            ),
            (
                "audio.processing",
                "query",
                contracts::CapabilityAuthorization {
                    project_ids: vec!["allowed".into()],
                    ..Default::default()
                },
                serde_json::json!(["allowed", 1]),
                "AUDIO_ARGUMENTS_INVALID",
            ),
            (
                "audio.processing",
                "query",
                contracts::CapabilityAuthorization {
                    project_ids: vec!["allowed".into()],
                    ..Default::default()
                },
                serde_json::json!({"projectId": "allowed"}),
                "AUDIO_ARGUMENTS_INVALID",
            ),
            (
                "settings.app",
                "snapshot",
                contracts::CapabilityAuthorization {
                    settings_fields: fields.clone(),
                    ..Default::default()
                },
                serde_json::json!([]),
                "SETTINGS_ARGUMENTS_INVALID",
            ),
            (
                "settings.app",
                "read",
                contracts::CapabilityAuthorization {
                    settings_fields: fields.clone(),
                    ..Default::default()
                },
                serde_json::json!(["writePolicy"]),
                "SETTINGS_ARGUMENTS_INVALID",
            ),
            (
                "settings.app",
                "patch",
                contracts::CapabilityAuthorization {
                    settings_fields: fields.clone(),
                    ..Default::default()
                },
                serde_json::json!([revision, {"maxToolRounds": 5}]),
                "SETTINGS_ARGUMENTS_INVALID",
            ),
        ] {
            let response = host.dispatch(contracts::CapabilityCallRequest::new(
                format!("non-object-{capability}-{operation}"),
                format!("non-object-{capability}-{operation}"),
                capability,
                1,
                operation,
                authorization,
                arguments.clone(),
            ));
            assert!(
                !response.ok,
                "{capability}.{operation} accepted {arguments}"
            );
            let error = response.error.unwrap();
            assert_eq!(error.code, code, "{capability}.{operation}: {error:?}");
            assert_eq!(error.context.unwrap().operation.as_deref(), Some(operation));
        }
        assert_eq!(settings_service.snapshot().unwrap(), before);
    }

    #[test]
    fn settings_read_descriptor_enumerates_the_only_readable_sections() {
        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let read = SettingsCapabilityPort::new(settings, std::iter::empty::<String>())
            .descriptor()
            .operations
            .into_iter()
            .find(|operation| operation.name == "read")
            .unwrap();
        let mut section = read.input_schema["properties"]["section"].clone();
        if let Some(reference) = section["$ref"].as_str() {
            let name = reference.trim_start_matches("#/$defs/");
            section = read.input_schema["$defs"][name].clone();
        }
        assert_eq!(
            section["enum"],
            serde_json::json!([
                "connections",
                "connectionTargets",
                "credentials",
                "writePolicy"
            ]),
            "read.section must declare exactly the sections the port accepts: {}",
            read.input_schema
        );
    }

    struct RecordingProbeProvider(Mutex<Vec<llm::LlmRequest>>);

    impl llm::LlmProvider for RecordingProbeProvider {
        fn descriptor(&self) -> llm::LlmProviderDescriptor {
            llm::LlmProviderDescriptor {
                id: "recording".into(),
                local: true,
                protocol: llm::LlmProtocol::OpenAiChatCompletions,
            }
        }

        fn complete(
            &self,
            request: &llm::LlmRequest,
            _: &llm::LlmCancellation,
        ) -> Result<llm::LlmOutput, llm::LlmError> {
            self.0.lock().unwrap().push(request.clone());
            Ok(llm::LlmOutput {
                blocks: vec![llm::LlmOutputBlock::Text {
                    text: "ready".into(),
                }],
                stop_reason: llm::LlmStopReason::Complete,
                usage: None,
                requested_model: "requested".into(),
                resolved_model: "resolved".into(),
            })
        }

        fn probe(&self, _: &llm::LlmCancellation) -> Result<llm::ProbeResult, llm::LlmError> {
            unreachable!("Host probe must use one minimal completion")
        }
    }

    #[test]
    fn host_provider_probe_sends_one_minimal_toolless_request() {
        let documents =
            crate::DocumentService::new(Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new()))));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, std::iter::empty()),
            SettingsCapabilityPort::new(settings, std::iter::empty::<String>()),
            std::iter::empty::<String>(),
        );
        let provider = RecordingProbeProvider(Mutex::new(Vec::new()));
        let output = host
            .provider_probe(&provider, &llm::LlmCancellation::new())
            .unwrap();
        assert_eq!(output.resolved_model, "resolved");
        let requests = provider.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.tools.is_empty());
        assert_eq!(request.tool_choice, llm::LlmToolChoice::None);
        assert_eq!(request.max_output_tokens, Some(16));
        assert_eq!(request.messages.len(), 1);
    }

    #[test]
    fn audio_capability_is_a_granted_project_port_and_rejects_path_escape() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents,
        );
        let port = AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]);
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "noop".into(),
            revision: None,
            artifact_path: String::new(),
            artifacts: Vec::new(),
        };
        let denied = match port.start(
            ProjectId::new("denied"),
            model("noop-asr"),
            model("noop-diarization"),
        ) {
            Ok(_) => panic!("unexpected audio job"),
            Err(error) => error,
        };
        assert_eq!(denied.code, contracts::BackendErrorCode::AccessDenied);
        let escaped = match port.start_with_request(
            ProjectId::new("allowed"),
            crate::ProcessingRequest {
                audio_path: "other.wav".into(),
                asr_model: model("noop-asr"),
                diarization_model: model("noop-diarization"),
                language: Some("zh".into()),
                expected_speakers: Some(1),
                device: contracts::ComputeDevice::Cpu,
                speaker_library_path: None,
                stage_configuration: contracts::DiarizationStageConfiguration::default(),
            },
        ) {
            Ok(_) => panic!("unexpected audio job"),
            Err(error) => error,
        };
        assert_eq!(escaped.code, contracts::BackendErrorCode::InvalidAudio);

        let escaped_model = match port.start_with_request(
            ProjectId::new("allowed"),
            crate::ProcessingRequest {
                audio_path: "a.wav".into(),
                asr_model: ModelRef {
                    artifact_path: "../outside/model.bin".into(),
                    ..model("noop-asr")
                },
                diarization_model: model("noop-diarization"),
                language: None,
                expected_speakers: None,
                device: contracts::ComputeDevice::Auto,
                speaker_library_path: None,
                stage_configuration: contracts::DiarizationStageConfiguration::default(),
            },
        ) {
            Ok(_) => panic!("unexpected escaped model job"),
            Err(error) => error,
        };
        assert_eq!(
            escaped_model.code,
            contracts::BackendErrorCode::AccessDenied
        );

        let escaped_library = match port.start_with_request(
            ProjectId::new("allowed"),
            crate::ProcessingRequest {
                audio_path: "a.wav".into(),
                asr_model: model("noop-asr"),
                diarization_model: model("noop-diarization"),
                language: None,
                expected_speakers: None,
                device: contracts::ComputeDevice::Auto,
                speaker_library_path: Some("../outside/library.json".into()),
                stage_configuration: contracts::DiarizationStageConfiguration::default(),
            },
        ) {
            Ok(_) => panic!("unexpected escaped speaker library job"),
            Err(error) => error,
        };
        assert_eq!(
            escaped_library.code,
            contracts::BackendErrorCode::AccessDenied
        );
    }

    #[test]
    fn agent_host_audio_lifecycle_reuses_one_registry_and_keeps_result_state() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]),
            SettingsCapabilityPort::new(settings, std::iter::empty::<String>()),
            std::iter::empty::<String>(),
        );
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "noop".into(),
            revision: None,
            artifact_path: String::new(),
            artifacts: Vec::new(),
        };
        let authorization = contracts::CapabilityAuthorization {
            project_ids: vec!["allowed".into()],
            ..Default::default()
        };
        let request = |call_id: &str, operation: &str, arguments: serde_json::Value| {
            contracts::CapabilityCallRequest::new(
                format!("audio-{call_id}"),
                call_id,
                "audio.processing",
                1,
                operation,
                authorization.clone(),
                arguments,
            )
        };
        let started = host.dispatch(request(
            "start",
            "start",
            serde_json::json!({
                "projectId": "allowed",
                "asrModel": model("noop-asr"),
                "diarizationModel": model("noop-diarization")
            }),
        ));
        assert!(started.ok, "{started:?}");
        let asr_job_id = started.result.as_ref().unwrap()["asrJobId"]
            .as_u64()
            .unwrap();
        let diarization_job_id = started.result.as_ref().unwrap()["diarizationJobId"]
            .as_u64()
            .unwrap();
        let handle_arguments = serde_json::json!({
            "projectId": "allowed",
            "asrJobId": asr_job_id,
            "diarizationJobId": diarization_job_id
        });
        let queried = host.dispatch(request(
            "query",
            "query",
            serde_json::json!({"projectId": "allowed", "jobId": asr_job_id}),
        ));
        assert!(queried.ok, "{queried:?}");
        let cancelled = host.dispatch(request("cancel", "cancel", handle_arguments.clone()));
        assert!(cancelled.ok, "{cancelled:?}");
        let result = host.dispatch(request("result", "result", handle_arguments));
        assert!(result.ok, "{result:?}");
        assert_eq!(result.result.unwrap()["ready"], false);
    }

    #[test]
    fn capability_binding_is_replaceable_without_changing_builtin_dispatch() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents.clone(),
        );
        let settings =
            SettingsService::open(Arc::new(MemorySettings(Mutex::new(AppSettings::default()))))
                .unwrap();
        let host = crate::AgentHost::new(
            crate::WorkspaceCapabilityPort::new(
                documents,
                crate::WorkspaceAccessPolicy::AllDocuments,
            ),
            AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]),
            SettingsCapabilityPort::new(settings, std::iter::empty::<String>()),
            std::iter::empty::<String>(),
        );
        let dispatch_count = Arc::new(AtomicUsize::new(0));
        let host = host.with_binding(
            "test.echo",
            Arc::new(EchoBinding {
                dispatch_count: dispatch_count.clone(),
            }),
        );

        let descriptor = host
            .discover()
            .into_iter()
            .find(|descriptor| descriptor.id == "test.echo")
            .expect("custom binding must be discoverable");
        assert_eq!(descriptor.version, 7);
        assert_eq!(descriptor.operations[0].name, "echo");

        let request = |version: u32, operation: &str, arguments: serde_json::Value| {
            contracts::CapabilityCallRequest::new(
                format!("binding-{operation}"),
                format!("call-{operation}"),
                "test.echo",
                version,
                operation,
                contracts::CapabilityAuthorization::default(),
                arguments,
            )
        };
        let response = host.dispatch(request(7, "echo", serde_json::json!({"value": "proof"})));
        assert!(response.ok, "custom binding response: {response:?}");
        assert_eq!(response.result.unwrap()["value"], "proof");
        assert_eq!(dispatch_count.load(Ordering::SeqCst), 1);

        let unsupported = host.dispatch(request(8, "echo", serde_json::json!({"value": "proof"})));
        assert!(
            !unsupported.ok,
            "unsupported custom version must be rejected"
        );
        assert_eq!(
            unsupported.error.as_ref().unwrap().code,
            "CAPABILITY_VERSION_UNSUPPORTED"
        );
        assert_eq!(dispatch_count.load(Ordering::SeqCst), 1);

        let error = host.dispatch(request(7, "missing", serde_json::json!({})));
        assert!(!error.ok);
        assert_eq!(error.error.as_ref().unwrap().code, "TEST_OPERATION_UNKNOWN");
        assert_eq!(
            error
                .error
                .as_ref()
                .unwrap()
                .context
                .as_ref()
                .unwrap()
                .operation
                .as_deref(),
            Some("missing")
        );

        let builtin = host.dispatch(contracts::CapabilityCallRequest::new(
            "builtin-list",
            "builtin-list-call",
            "workspace.documents",
            1,
            "list_documents",
            contracts::CapabilityAuthorization {
                all_documents: true,
                ..Default::default()
            },
            serde_json::json!({}),
        ));
        assert!(builtin.ok, "builtin dispatch changed: {builtin:?}");
        assert_eq!(builtin.result.unwrap()["type"], "documents");
    }

    #[test]
    fn audio_capability_exposes_job_ids_and_cancellation_to_a_consumer() {
        let audio_path = std::env::temp_dir().join(format!(
            "meetnote-capability-audio-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&audio_path, spec).unwrap();
        writer.write_sample(0_i16).unwrap();
        writer.finalize().unwrap();

        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("processable"),
                "Processable",
                audio_path.display().to_string(),
            ))
            .unwrap();
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents,
        );
        let port = AudioCapabilityPort::new(workspace.clone(), [ProjectId::new("processable")]);
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "noop".into(),
            revision: None,
            artifact_path: String::new(),
            artifacts: Vec::new(),
        };
        let job = port
            .start_job(
                ProjectId::new("processable"),
                model("noop-asr"),
                model("noop-diarization"),
            )
            .unwrap();
        let (asr_job, diarization_job) = job.job_ids();
        assert_eq!(
            port.snapshot_for(&ProjectId::new("processable"), asr_job)
                .unwrap()
                .state,
            runtime::JobState::Running
        );
        port.cancel_handle(
            &ProjectId::new("processable"),
            job_handle(asr_job, diarization_job),
        )
        .unwrap();
        assert_eq!(
            port.snapshot_for(&ProjectId::new("processable"), asr_job)
                .unwrap()
                .state,
            runtime::JobState::Cancelled
        );
        assert_eq!(
            port.snapshot_for(&ProjectId::new("processable"), diarization_job)
                .unwrap()
                .state,
            runtime::JobState::Cancelled
        );
        let result = futures::executor::block_on(job.future);
        assert_eq!(
            result.unwrap_err().code,
            contracts::BackendErrorCode::NotImplemented
        );
        let terminal = port
            .snapshot_for(&ProjectId::new("processable"), asr_job)
            .unwrap();
        assert_eq!(terminal.state, runtime::JobState::Cancelled);
        assert!(workspace.processing().job_snapshot(asr_job).is_err());
        assert_eq!(
            port.config_snapshot(
                &ProjectId::new("processable"),
                job_handle(asr_job, diarization_job)
            )
            .unwrap()
            .language,
            None
        );
        let config = port
            .config_snapshot(
                &ProjectId::new("processable"),
                job_handle(asr_job, diarization_job),
            )
            .unwrap();
        assert_eq!(config.audio_path, audio_path.display().to_string());
        assert_eq!(config.asr_model.backend_id, "noop-asr");
        assert_eq!(config.diarization_model.backend_id, "noop-diarization");
        port.revoke().unwrap();
        assert!(port
            .snapshot_for(&ProjectId::new("processable"), asr_job)
            .is_ok());
        let denied_after_revoke = port.start(
            ProjectId::new("processable"),
            model("noop-asr"),
            model("noop-diarization"),
        );
        assert_eq!(
            denied_after_revoke.unwrap_err().code,
            contracts::BackendErrorCode::AccessDenied
        );
        port.release(
            &ProjectId::new("processable"),
            job_handle(asr_job, diarization_job),
        )
        .unwrap();
        assert!(port
            .snapshot_for(&ProjectId::new("processable"), asr_job)
            .is_err());
        std::fs::remove_file(audio_path).unwrap();
    }

    #[test]
    fn audio_job_handles_cannot_cross_project_ownership_boundaries() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        for project_id in ["allowed", "other"] {
            documents
                .save(domain::DocumentState::new(
                    ProjectId::new(project_id),
                    project_id,
                    "a.wav",
                ))
                .unwrap();
        }
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
            documents,
        );
        let allowed = AudioCapabilityPort::new(workspace.clone(), [ProjectId::new("allowed")]);
        let other = AudioCapabilityPort::new(workspace, [ProjectId::new("other")]);
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "noop".into(),
            revision: None,
            artifact_path: String::new(),
            artifacts: Vec::new(),
        };
        let allowed_job = allowed
            .start_job(
                ProjectId::new("allowed"),
                model("noop-asr"),
                model("noop-diarization"),
            )
            .unwrap();
        let other_job = other
            .start_job(
                ProjectId::new("other"),
                model("noop-asr"),
                model("noop-diarization"),
            )
            .unwrap();
        let (allowed_asr, _) = allowed_job.job_ids();
        let (other_asr, other_diarization) = other_job.job_ids();

        assert_eq!(
            allowed
                .snapshot_for(&ProjectId::new("allowed"), allowed_asr)
                .unwrap()
                .state,
            runtime::JobState::Running
        );
        let crossed = allowed.snapshot_for(&ProjectId::new("allowed"), other_asr);
        assert_eq!(
            crossed.unwrap_err().code,
            contracts::BackendErrorCode::AccessDenied
        );

        assert_eq!(
            allowed
                .cancel_handle(
                    &ProjectId::new("allowed"),
                    job_handle(other_asr, other_diarization),
                )
                .unwrap_err()
                .code,
            contracts::BackendErrorCode::AccessDenied
        );
        assert_eq!(
            other
                .snapshot_for(&ProjectId::new("other"), other_asr)
                .unwrap()
                .state,
            runtime::JobState::Running
        );
        allowed
            .cancel_handle(
                &ProjectId::new("allowed"),
                job_handle(allowed_asr, allowed_job.job_ids().1),
            )
            .unwrap();
        assert_eq!(
            allowed
                .snapshot_for(&ProjectId::new("allowed"), allowed_asr)
                .unwrap()
                .state,
            runtime::JobState::Cancelled
        );
        assert_eq!(
            other
                .snapshot_for(&ProjectId::new("other"), other_diarization)
                .unwrap()
                .state,
            runtime::JobState::Running
        );
    }

    struct BlockingProbeAsr {
        entered: Arc<Barrier>,
        proceed: Arc<Barrier>,
    }

    impl AsrBackend for BlockingProbeAsr {
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: "blocking-asr".into(),
                task: BackendTask::Asr,
                capabilities: Vec::new(),
                configurable_diarization_stages: Vec::new(),
                diarization_stage_options: Vec::new(),
            }
        }

        fn probe(&self, _: &ModelRef) -> Result<(), contracts::BackendError> {
            self.entered.wait();
            self.proceed.wait();
            Ok(())
        }

        fn transcribe(&self, _: AsrRequest, _: runtime::JobContext) -> BackendFuture<AsrResult> {
            Box::pin(future::ready(Err(contracts::BackendError {
                code: contracts::BackendErrorCode::NotImplemented,
                message: "blocking probe test does not transcribe".into(),
            })))
        }
    }

    #[test]
    fn revoke_cannot_finish_before_a_checked_start_registers_a_job() {
        let repository = Arc::new(MemoryDocuments(Mutex::new(BTreeMap::new())));
        let documents = crate::DocumentService::new(repository);
        documents
            .save(domain::DocumentState::new(
                ProjectId::new("allowed"),
                "Allowed",
                "a.wav",
            ))
            .unwrap();
        let entered = Arc::new(Barrier::new(2));
        let proceed = Arc::new(Barrier::new(2));
        let workspace = crate::WorkspaceService::new(
            crate::ProcessingService::new(
                Arc::new(BlockingProbeAsr {
                    entered: entered.clone(),
                    proceed: proceed.clone(),
                }),
                Arc::new(NoopDiarization),
            ),
            documents,
        );
        let port = AudioCapabilityPort::new(workspace, [ProjectId::new("allowed")]);
        let start_port = port.clone();
        let start = std::thread::spawn(move || {
            start_port.start(
                ProjectId::new("allowed"),
                ModelRef {
                    backend_id: "blocking-asr".into(),
                    model_id: "probe".into(),
                    revision: None,
                    artifact_path: String::new(),
                    artifacts: Vec::new(),
                },
                ModelRef {
                    backend_id: "noop-diarization".into(),
                    model_id: "probe".into(),
                    revision: None,
                    artifact_path: String::new(),
                    artifacts: Vec::new(),
                },
            )
        });
        entered.wait();
        port.revoke().unwrap();
        proceed.wait();
        let started = start.join().unwrap();
        assert_eq!(
            started.unwrap_err().code,
            contracts::BackendErrorCode::AccessDenied
        );
        assert!(port.registry().jobs.lock().unwrap().is_empty());
    }

    fn job_handle(asr_job: JobId, diarization_job: JobId) -> AudioJobHandle {
        AudioJobHandle {
            asr_job_id: asr_job,
            diarization_job_id: diarization_job,
        }
    }
}
