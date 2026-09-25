use crate::{
    run_pipeline, run_windowed_pipeline, AsrBackend, AsrBackendProvider, BackendFuture,
    DiarizationBackend, DiarizationBackendProvider, DocumentService, LongAudioConfig,
    PipelineResult,
};
use contracts::{
    AsrRequest, ComputeDevice, DiarizationRequest, DiarizationStageConfiguration, ModelRef,
};
use domain::{
    reorder_project, reparent_project, AlgorithmProvenance, DiarizationProvenance, DocumentState,
    IdentityMatchingProvenance, ModelArtifactProvenance, ModelProvenance, ProcessingRecord,
    ProjectId, RuntimeProvenance, SpeakerTaggedSegment,
};
use futures::channel::mpsc::UnboundedReceiver;
use runtime::{JobContext, JobEvent, JobId, JobRuntime, JobRuntimeError, JobSnapshot};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct ProcessingRequest {
    pub audio_path: String,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    pub device: ComputeDevice,
    pub speaker_library_path: Option<String>,
    pub stage_configuration: DiarizationStageConfiguration,
}

#[derive(Clone)]
pub struct ProcessingService {
    asr_provider: Arc<dyn AsrBackendProvider>,
    diarization_provider: Arc<dyn DiarizationBackendProvider>,
    runtime: Arc<JobRuntime>,
    long_audio: LongAudioConfig,
}

pub struct ProcessingContexts {
    pub asr_context: JobContext,
    pub diarization_context: JobContext,
    pub asr_events: UnboundedReceiver<JobEvent>,
    pub diarization_events: UnboundedReceiver<JobEvent>,
}

impl ProcessingService {
    pub fn new(
        asr_backend: Arc<dyn AsrBackend>,
        diarization_backend: Arc<dyn DiarizationBackend>,
    ) -> Self {
        Self::new_with_providers(
            Arc::new(SingleAsrBackendProvider {
                backend: asr_backend,
            }),
            Arc::new(SingleDiarizationBackendProvider {
                backend: diarization_backend,
            }),
        )
    }

    pub fn new_with_providers(
        asr_provider: Arc<dyn AsrBackendProvider>,
        diarization_provider: Arc<dyn DiarizationBackendProvider>,
    ) -> Self {
        Self {
            asr_provider,
            diarization_provider,
            runtime: Arc::new(JobRuntime::default()),
            long_audio: LongAudioConfig::default(),
        }
    }

    pub fn with_long_audio_config(mut self, config: LongAudioConfig) -> Self {
        self.long_audio = config;
        self
    }

    /// Runs this service's jobs on `runtime`, so they share its worker limit
    /// with every other user of that runtime.
    pub fn with_job_runtime(mut self, runtime: Arc<JobRuntime>) -> Self {
        self.runtime = runtime;
        self
    }

    pub fn probe(&self, request: &ProcessingRequest) -> Result<(), contracts::BackendError> {
        self.asr_provider
            .backend(&request.asr_model.backend_id)?
            .probe(&request.asr_model)?;
        self.diarization_provider
            .backend(&request.diarization_model.backend_id)?
            .probe_with_configuration(&request.diarization_model, &request.stage_configuration)
    }

    pub fn backend_ids(&self) -> (String, String) {
        (
            self.asr_provider
                .available_backends()
                .first()
                .map(|backend| backend.id.clone())
                .unwrap_or_default(),
            self.diarization_provider
                .available_backends()
                .first()
                .map(|backend| backend.id.clone())
                .unwrap_or_default(),
        )
    }

    pub fn contexts(&self) -> ProcessingContexts {
        let (asr_context, asr_events) = self.runtime.create_job();
        let (diarization_context, diarization_events) = self.runtime.create_job();
        ProcessingContexts {
            asr_context,
            diarization_context,
            asr_events,
            diarization_events,
        }
    }

    pub fn job_snapshot(&self, job_id: JobId) -> Result<JobSnapshot, JobRuntimeError> {
        self.runtime.snapshot(job_id)
    }

    pub fn remove_job(&self, job_id: JobId) -> Result<JobSnapshot, JobRuntimeError> {
        self.runtime.remove(job_id)
    }

    pub fn process(
        &self,
        request: ProcessingRequest,
        asr_context: JobContext,
        diarization_context: JobContext,
    ) -> BackendFuture<PipelineResult> {
        let asr_backend = match self.asr_provider.backend(&request.asr_model.backend_id) {
            Ok(backend) => backend,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let diarization_backend = match self
            .diarization_provider
            .backend(&request.diarization_model.backend_id)
        {
            Ok(backend) => backend,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let long_audio = self.long_audio;
        Box::pin(async move {
            let source =
                audio::AudioChunkReader::open(&request.audio_path, long_audio.decode_chunk_samples)
                    .map_err(|error| contracts::BackendError {
                        code: contracts::BackendErrorCode::InvalidAudio,
                        message: error.to_string(),
                    })?;
            let use_windows = long_audio.is_long(source.source_info().estimated_output_samples);
            drop(source);
            let asr_request = AsrRequest {
                audio_path: request.audio_path.clone(),
                model: request.asr_model,
                language: request.language,
                device: request.device,
            };
            let diarization_request = DiarizationRequest {
                audio_path: request.audio_path,
                model: request.diarization_model,
                expected_speakers: request.expected_speakers,
                device: request.device,
                speaker_library_path: request.speaker_library_path,
                stage_configuration: request.stage_configuration,
            };
            if use_windows {
                if !asr_backend.supports_windowed_audio()
                    || !diarization_backend.supports_windowed_audio()
                {
                    return Err(contracts::BackendError {
                        code: contracts::BackendErrorCode::NotImplemented,
                        message: format!(
                            "long audio requires bounded-window ASR and unclustered global diarization; selected backends are ASR={} (windowed={}) and diarization={} (windowed={})",
                            asr_backend.descriptor().id,
                            asr_backend.supports_windowed_audio(),
                            diarization_backend.descriptor().id,
                            diarization_backend.supports_windowed_audio()
                        ),
                    });
                }
                run_windowed_pipeline(
                    asr_backend,
                    diarization_backend,
                    asr_request,
                    diarization_request,
                    asr_context,
                    diarization_context,
                    long_audio,
                )
                .await
            } else {
                run_pipeline(
                    asr_backend.as_ref(),
                    diarization_backend.as_ref(),
                    asr_request,
                    diarization_request,
                    asr_context,
                    diarization_context,
                )
                .await
            }
        })
    }
}

struct SingleAsrBackendProvider {
    backend: Arc<dyn AsrBackend>,
}

impl AsrBackendProvider for SingleAsrBackendProvider {
    fn available_backends(&self) -> Vec<crate::BackendDescriptor> {
        vec![self.backend.descriptor()]
    }

    fn backend(&self, backend_id: &str) -> Result<Arc<dyn AsrBackend>, contracts::BackendError> {
        if backend_id == self.backend.descriptor().id {
            Ok(self.backend.clone())
        } else {
            Err(contracts::BackendError {
                code: contracts::BackendErrorCode::BackendUnavailable,
                message: format!("ASR backend not registered: {backend_id}"),
            })
        }
    }
}

struct SingleDiarizationBackendProvider {
    backend: Arc<dyn DiarizationBackend>,
}

impl DiarizationBackendProvider for SingleDiarizationBackendProvider {
    fn available_backends(&self) -> Vec<crate::BackendDescriptor> {
        vec![self.backend.descriptor()]
    }

    fn backend(
        &self,
        backend_id: &str,
    ) -> Result<Arc<dyn DiarizationBackend>, contracts::BackendError> {
        if backend_id == self.backend.descriptor().id {
            Ok(self.backend.clone())
        } else {
            Err(contracts::BackendError {
                code: contracts::BackendErrorCode::BackendUnavailable,
                message: format!("Diarization backend not registered: {backend_id}"),
            })
        }
    }
}

#[derive(Clone)]
pub struct WorkspaceService {
    processing: ProcessingService,
    documents: DocumentService,
    options: Arc<RwLock<ProcessingOptions>>,
}

#[derive(Debug, Clone)]
pub struct ProcessingOptions {
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    pub device: ComputeDevice,
    pub speaker_library_path: Option<String>,
    pub diarization_stages: DiarizationStageConfiguration,
}

impl Default for ProcessingOptions {
    fn default() -> Self {
        Self {
            language: None,
            expected_speakers: None,
            device: ComputeDevice::Auto,
            speaker_library_path: None,
            diarization_stages: DiarizationStageConfiguration::default(),
        }
    }
}

pub struct ProjectProcessingJob {
    pub future: BackendFuture<DocumentState>,
    pub asr_context: JobContext,
    pub diarization_context: JobContext,
    pub asr_events: UnboundedReceiver<JobEvent>,
    pub diarization_events: UnboundedReceiver<JobEvent>,
}

impl ProjectProcessingJob {
    pub fn job_ids(&self) -> (JobId, JobId) {
        (self.asr_context.job_id(), self.diarization_context.job_id())
    }

    pub fn cancel(&self) {
        self.asr_context.cancel();
        self.diarization_context.cancel();
    }
}

impl WorkspaceService {
    pub fn new(processing: ProcessingService, documents: DocumentService) -> Self {
        Self::with_options(processing, documents, ProcessingOptions::default())
    }

    pub fn with_options(
        processing: ProcessingService,
        documents: DocumentService,
        options: ProcessingOptions,
    ) -> Self {
        Self {
            processing,
            documents,
            options: Arc::new(RwLock::new(options)),
        }
    }

    pub fn set_processing_options(&self, options: ProcessingOptions) -> Result<(), String> {
        *self
            .options
            .write()
            .map_err(|_| "processing options lock poisoned".to_string())? = options;
        Ok(())
    }

    pub fn documents(&self) -> DocumentService {
        self.documents.clone()
    }

    /// Revision-checked update, tree move and speaker correction commands on
    /// this workspace's documents; a stale revision fails with
    /// [`crate::DocumentCommandError::RevisionConflict`].
    pub fn document_commands(&self) -> crate::DocumentCommandService {
        crate::DocumentCommandService::new(self.documents.clone())
    }

    pub fn processing(&self) -> ProcessingService {
        self.processing.clone()
    }

    pub fn list_projects(&self) -> Result<Vec<DocumentState>, String> {
        self.documents.list()
    }

    pub fn document_snapshot(&self) -> Result<crate::DocumentSnapshot, String> {
        self.documents.snapshot()
    }

    pub fn watch_documents(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
        self.documents.watch()
    }

    /// Document-directory changes as an async stream the watcher feeds
    /// directly; dropping it stops watching.
    pub fn subscribe_document_changes(&self) -> Result<crate::DocumentChanges, String> {
        self.documents.subscribe_changes()
    }

    pub fn import_audio(&self, audio_path: impl Into<String>) -> Result<DocumentState, String> {
        let audio_path = audio_path.into();
        let path = Path::new(&audio_path);
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Untitled meeting")
            .to_string();
        let id = ProjectId::new(format!("project-{}", unique_suffix()));
        let project = DocumentState::new(id, title, audio_path);
        let project = self.documents.save_with_markdown(
            project.clone(),
            render_markdown(
                Path::new(project.audio_path()),
                &project.results.speaker_segments,
            ),
        )?;
        Ok(project)
    }

    pub fn project(&self, project_id: &ProjectId) -> Result<Option<DocumentState>, String> {
        self.documents.get(project_id)
    }

    /// Deprecated: rewrites every document without revision checks. Use
    /// [`Self::document_commands`] and
    /// [`crate::DocumentCommandService::reparent_document`].
    pub fn reparent_project(
        &self,
        project_id: &ProjectId,
        new_parent_id: Option<&ProjectId>,
        sibling_order: u32,
    ) -> Result<Vec<DocumentState>, String> {
        let mut projects = self.documents.list()?;
        reparent_project(&mut projects, project_id, new_parent_id, sibling_order)
            .map_err(|error| format!("cannot move project: {error:?}"))?;
        for project in projects.iter().cloned() {
            self.documents.save(project)?;
        }
        Ok(projects)
    }

    /// Deprecated: rewrites every document without revision checks. Use
    /// [`Self::document_commands`] and
    /// [`crate::DocumentCommandService::reorder_document`].
    pub fn reorder_project(
        &self,
        project_id: &ProjectId,
        target_id: &ProjectId,
        before: bool,
    ) -> Result<Vec<DocumentState>, String> {
        let mut projects = self.documents.list()?;
        reorder_project(&mut projects, project_id, target_id, before)
            .map_err(|error| format!("cannot reorder project: {error:?}"))?;
        for project in projects.iter().cloned() {
            self.documents.save(project)?;
        }
        Ok(projects)
    }

    pub fn create_semantic_child(
        &self,
        parent_id: &ProjectId,
        title: impl Into<String>,
        start_ms: u64,
        end_ms: u64,
        tags: Vec<String>,
    ) -> Result<DocumentState, String> {
        let projects = self.documents.list()?;
        let parent = projects
            .iter()
            .find(|project| project.id() == parent_id)
            .ok_or_else(|| "project does not exist".to_string())?;
        if parent.hierarchy.depth >= 2 {
            return Err("project tree cannot exceed three levels".into());
        }
        if start_ms >= end_ms {
            return Err("semantic project range must be non-empty".into());
        }
        let child = parent.semantic_child(ProjectId::new_uuid(), title, start_ms, end_ms, tags);
        self.documents.save_with_markdown(
            child.clone(),
            render_markdown(
                Path::new(child.audio_path()),
                &child.results.speaker_segments,
            ),
        )?;
        Ok(child)
    }

    /// Deprecated: overwrites without a revision check. Use
    /// [`Self::document_commands`] and
    /// [`crate::DocumentCommandService::update_document`].
    pub fn update_summary(
        &self,
        project_id: &ProjectId,
        summary: impl Into<String>,
    ) -> Result<DocumentState, String> {
        let mut project = self
            .documents
            .get(project_id)?
            .ok_or_else(|| "project does not exist".to_string())?;
        project.content.summary = Some(summary.into());
        self.documents.save(project.clone())?;
        Ok(project)
    }

    /// Deprecated: no revision check, accepts unknown speaker ids and
    /// replaces an edited body. Use [`Self::document_commands`] and
    /// [`crate::DocumentCommandService::merge_speaker`].
    pub fn merge_speaker(
        &self,
        project_id: &ProjectId,
        from: &str,
        into: &str,
    ) -> Result<DocumentState, String> {
        let mut project = self
            .documents
            .get(project_id)?
            .ok_or_else(|| "project does not exist".to_string())?;
        let changed = project.merge_speaker(from, into);
        if changed == 0 {
            return Ok(project);
        }
        project = self.documents.save_with_markdown(
            project.clone(),
            render_markdown(
                Path::new(project.audio_path()),
                &project.results.speaker_segments,
            ),
        )?;
        Ok(project)
    }

    pub fn start_project_processing(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
    ) -> Result<ProjectProcessingJob, contracts::BackendError> {
        self.start_project_processing_with_speaker_library(
            project_id,
            asr_model,
            diarization_model,
            None,
        )
    }

    pub fn start_project_processing_with_speaker_library(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
        speaker_library_path: Option<String>,
    ) -> Result<ProjectProcessingJob, contracts::BackendError> {
        let request = self.processing_request(
            &project_id,
            asr_model,
            diarization_model,
            speaker_library_path,
        )?;
        self.start_project_processing_with_request(project_id, request)
    }

    pub fn processing_request(
        &self,
        project_id: &ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
        speaker_library_path: Option<String>,
    ) -> Result<ProcessingRequest, contracts::BackendError> {
        let project = self
            .documents
            .get(project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| persistence_error("project does not exist"))?;
        let options = self
            .options
            .read()
            .map_err(|_| persistence_error("processing options lock poisoned"))?
            .clone();
        Ok(ProcessingRequest {
            audio_path: project.audio_path().to_string(),
            asr_model,
            diarization_model,
            language: options.language,
            expected_speakers: options.expected_speakers,
            device: options.device,
            speaker_library_path: speaker_library_path.or(options.speaker_library_path),
            stage_configuration: options.diarization_stages,
        })
    }

    pub fn start_project_processing_with_request(
        &self,
        project_id: ProjectId,
        request: ProcessingRequest,
    ) -> Result<ProjectProcessingJob, contracts::BackendError> {
        let mut project = self
            .documents
            .get(&project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| persistence_error("project does not exist"))?;
        self.processing.probe(&request).inspect_err(|error| {
            project.set_processing_error(error.message.clone());
            let _ = self.documents.save(project.clone());
        })?;

        let contexts = self.processing.contexts();
        contexts.asr_context.started();
        contexts.diarization_context.started();
        let asr_context = contexts.asr_context.clone();
        let diarization_context = contexts.diarization_context.clone();
        let asr_status_context = contexts.asr_context.clone();
        let diarization_status_context = contexts.diarization_context.clone();
        let workspace = self.clone();
        let request_for_record = request.clone();
        let future = Box::pin(async move {
            let result = workspace
                .processing
                .process(request, asr_context, diarization_context)
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    if error.code == contracts::BackendErrorCode::Cancelled {
                        asr_status_context.cancel();
                        diarization_status_context.cancel();
                    } else {
                        asr_status_context.failed(error.message.clone());
                        diarization_status_context.failed(error.message.clone());
                    }
                    project.set_processing_error(error.message.clone());
                    let _ = workspace.documents.save(project);
                    return Err(error);
                }
            };

            asr_status_context.completed();
            diarization_status_context.completed();

            project.results.transcript = result.asr.segments.clone();
            project.results.diarization = result.diarization.segments.clone();
            project.results.speaker_segments = result.aligned.clone();
            let processing_revision = project
                .processing
                .latest
                .as_ref()
                .map(|record| record.processing_revision.saturating_add(1))
                .unwrap_or(1);
            project.record_processing(processing_record(
                &request_for_record,
                &result,
                processing_revision,
            ));
            project = workspace
                .documents
                .save_with_markdown(
                    project.clone(),
                    render_markdown(
                        Path::new(project.audio_path()),
                        &project.results.speaker_segments,
                    ),
                )
                .map_err(persistence_error)?;
            Ok(project)
        });
        Ok(ProjectProcessingJob {
            future,
            asr_context: contexts.asr_context,
            diarization_context: contexts.diarization_context,
            asr_events: contexts.asr_events,
            diarization_events: contexts.diarization_events,
        })
    }

    pub fn discard_processing_job(&self, job: &ProjectProcessingJob) {
        self.discard_processing_job_ids(job.job_ids());
    }

    pub fn discard_processing_job_ids(&self, (asr_job, diarization_job): (JobId, JobId)) {
        let _ = self.processing.remove_job(asr_job);
        let _ = self.processing.remove_job(diarization_job);
    }

    pub fn process_project(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
    ) -> BackendFuture<DocumentState> {
        let workspace = self.clone();
        Box::pin(async move {
            let job =
                workspace.start_project_processing(project_id, asr_model, diarization_model)?;
            job.future.await
        })
    }
}

fn persistence_error(message: impl Into<String>) -> contracts::BackendError {
    contracts::BackendError {
        code: contracts::BackendErrorCode::PersistenceFailed,
        message: message.into(),
    }
}

fn processing_record(
    request: &ProcessingRequest,
    result: &PipelineResult,
    processing_revision: u64,
) -> ProcessingRecord {
    let identity_matching = result
        .diarization
        .embedding_observations
        .first()
        .map(|observation| IdentityMatchingProvenance {
            embedding_space: Some(observation.embedding_space.model_id.clone()),
            profile_library_schema_version: request
                .speaker_library_path
                .as_ref()
                .map(|_| contracts::SPEAKER_LIBRARY_SCHEMA_VERSION),
            threshold_milli: None,
        });
    ProcessingRecord {
        processing_revision,
        completed_at_ms: now_ms(),
        input_audio_checksum: file_sha256(&request.audio_path),
        asr: model_provenance(&result.asr.model),
        diarization: DiarizationProvenance {
            backend_id: result.diarization.backend.id.clone(),
            model_revision: result.diarization.model.revision.clone(),
            segmentation: result
                .diarization_stage_provenance
                .segmentation
                .as_ref()
                .map(model_provenance),
            embedding: result
                .diarization_stage_provenance
                .embedding
                .as_ref()
                .map(model_provenance),
            clustering: result
                .diarization_stage_provenance
                .clustering
                .clone()
                .unwrap_or_else(|| AlgorithmProvenance {
                    id: "backend-clustering".into(),
                    revision: Some(result.diarization.backend.id.clone()),
                    parameters: None,
                }),
            identity_matching,
        },
        alignment: AlgorithmProvenance {
            id: "max-overlap-speaker-alignment".into(),
            revision: Some("1".into()),
            parameters: None,
        },
        runtime: RuntimeProvenance {
            device: format!("{:?}", request.device),
            language: result
                .asr
                .language
                .clone()
                .or_else(|| request.language.clone()),
            sample_rate_hz: Some(16_000),
            threads: None,
            expected_speakers: request.expected_speakers,
            windowing: result
                .windowing
                .map(|windowing| domain::AudioWindowingProvenance {
                    sample_rate_hz: windowing.sample_rate_hz,
                    core_samples: windowing.core_samples,
                    context_samples: windowing.context_samples,
                    window_count: windowing.window_count,
                    source_samples: windowing.source_samples,
                }),
        },
    }
}

fn model_provenance(model: &ModelRef) -> ModelProvenance {
    ModelProvenance {
        backend_id: model.backend_id.clone(),
        model_id: model.model_id.clone(),
        revision: model.revision.clone(),
        artifact_sha256: artifact_sha256(&model.artifact_path),
        artifact_path: model.artifact_path.clone(),
        artifacts: model
            .artifacts
            .iter()
            .map(|artifact| ModelArtifactProvenance {
                filename: artifact.filename.clone(),
                expected_sha256: artifact.expected_sha256.clone(),
                observed_sha256: {
                    let root = Path::new(&model.artifact_path);
                    let artifact_path = if root.is_file() {
                        root.to_path_buf()
                    } else {
                        root.join(&artifact.filename)
                    };
                    file_sha256(&artifact_path.display().to_string())
                },
                archive_member: artifact.archive_member.clone(),
            })
            .collect(),
    }
}

fn file_sha256(path: &str) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn artifact_sha256(path: &str) -> Option<String> {
    let root = Path::new(path);
    if root.is_file() {
        return file_sha256(path);
    }
    if !root.is_dir() {
        return None;
    }
    let mut files = Vec::new();
    collect_files(root, &mut files).ok()?;
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(root).ok()?;
        hasher.update(relative.to_string_lossy().as_bytes());
        let file = fs::File::open(file).ok()?;
        let mut reader = BufReader::new(file);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, files: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub fn render_markdown(audio_path: &Path, segments: &[SpeakerTaggedSegment]) -> String {
    let mut output = format!(
        "# Meeting transcript\n\nAudio: `{}`\n\n",
        audio_path.display()
    );
    for segment in segments {
        output.push_str(&format!(
            "- **[{}–{}] {}** {}\n",
            timestamp(segment.start_ms),
            timestamp(segment.end_ms),
            segment
                .speaker_label
                .as_deref()
                .unwrap_or(&segment.speaker_id),
            segment.text.replace('\n', " ")
        ));
    }
    output
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let seconds = seconds % 60;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % 1_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asr_descriptor, diarization_descriptor, DocumentRepository, DocumentService,
        DocumentSnapshot,
    };
    use contracts::{
        AsrResult, BackendDescriptor, BackendError, DiarizationResult, DocumentRecord,
    };
    use domain::{DiarizationSegment, TranscriptSegment};
    use std::collections::BTreeMap;
    use std::sync::RwLock;

    #[derive(Default)]
    struct MemoryRepository {
        projects: RwLock<BTreeMap<String, DocumentState>>,
    }

    impl DocumentRepository for MemoryRepository {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            Ok(DocumentSnapshot {
                documents: self.projects.read().unwrap().values().cloned().collect(),
                issues: Vec::new(),
            })
        }

        fn save(&self, project: DocumentState, _markdown: Option<String>) -> Result<(), String> {
            self.projects
                .write()
                .unwrap()
                .insert(project.id().as_str().into(), project);
            Ok(())
        }

        fn get(&self, id: &ProjectId) -> Result<Option<DocumentState>, String> {
            Ok(self.projects.read().unwrap().get(id.as_str()).cloned())
        }

        fn read(&self, id: &ProjectId) -> Result<Option<DocumentRecord>, String> {
            Ok(self.get(id)?.map(|state| DocumentRecord {
                markdown: format!("# {}\n\n", state.title()),
                state,
            }))
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
            let (_sender, receiver) = std::sync::mpsc::channel();
            Ok(receiver)
        }
    }

    struct FakeAsr;

    impl AsrBackend for FakeAsr {
        fn descriptor(&self) -> BackendDescriptor {
            asr_descriptor("fake-asr", &["timestamps"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(&self, request: AsrRequest, _: JobContext) -> BackendFuture<AsrResult> {
            Box::pin(async move {
                Ok(AsrResult {
                    backend: asr_descriptor("fake-asr", &["timestamps"]),
                    model: request.model,
                    language: request.language,
                    segments: vec![TranscriptSegment {
                        id: "t1".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "hello".into(),
                    }],
                })
            })
        }
    }

    struct FakeDiarization;

    impl DiarizationBackend for FakeDiarization {
        fn descriptor(&self) -> BackendDescriptor {
            diarization_descriptor("fake-diarization", &["offline"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn stage_provenance(&self, model: &ModelRef) -> crate::DiarizationStageProvenance {
            crate::DiarizationStageProvenance {
                segmentation: Some(model.clone()),
                embedding: Some(model.clone()),
                clustering: Some(AlgorithmProvenance {
                    id: "fake-clustering".into(),
                    revision: Some("1".into()),
                    parameters: None,
                }),
            }
        }

        fn diarize(
            &self,
            request: DiarizationRequest,
            _: JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(async move {
                Ok(DiarizationResult {
                    backend: diarization_descriptor("fake-diarization", &["offline"]),
                    model: request.model,
                    segments: vec![DiarizationSegment {
                        id: "d1".into(),
                        speaker_id: "speaker-1".into(),
                        speaker_profile_id: None,
                        speaker_label: None,
                        start_ms: 0,
                        end_ms: 1_000,
                        confidence: Some(1.0),
                    }],
                    speaker_count: request.expected_speakers,
                    embedding_observations: Vec::new(),
                    speaker_profile_candidates: Vec::new(),
                })
            })
        }
    }

    fn write_test_wav() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "meetnote-application-{}-{}.wav",
            std::process::id(),
            unique_suffix()
        ));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn workspace_processes_and_persists_a_project() {
        let repository = Arc::new(MemoryRepository::default());
        let workspace = WorkspaceService::with_options(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(repository.clone()),
            ProcessingOptions {
                language: Some("zh".into()),
                expected_speakers: Some(2),
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
                diarization_stages: DiarizationStageConfiguration::default(),
            },
        );
        let audio_path = write_test_wav();
        let project = workspace
            .import_audio(audio_path.display().to_string())
            .unwrap();
        let asr_model = ModelRef {
            backend_id: "fake-asr".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };
        let diarization_model = ModelRef {
            backend_id: "fake-diarization".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };

        let result = futures::executor::block_on(workspace.process_project(
            project.id().clone(),
            asr_model,
            diarization_model,
        ))
        .unwrap();

        assert_eq!(result.processing_status(), domain::ProjectStatus::Ready);
        assert_eq!(result.results.speaker_segments[0].speaker_id, "speaker-1");
        let processing = result.processing.latest.as_ref().unwrap();
        assert_eq!(processing.asr.backend_id, "fake-asr");
        assert_eq!(processing.diarization.backend_id, "fake-diarization");
        assert_eq!(
            processing
                .diarization
                .segmentation
                .as_ref()
                .map(|model| model.model_id.as_str()),
            Some("fixture")
        );
        assert_eq!(processing.diarization.clustering.id, "fake-clustering");
        assert_eq!(processing.alignment.id, "max-overlap-speaker-alignment");
        assert_eq!(processing.runtime.sample_rate_hz, Some(16_000));
        assert_eq!(processing.runtime.language.as_deref(), Some("zh"));
        assert_eq!(processing.runtime.device, "Cpu");
        assert_eq!(processing.runtime.expected_speakers, Some(2));
        assert_eq!(processing.runtime.windowing, None);
        assert_eq!(repository.get(project.id()).unwrap(), Some(result.clone()));
        std::fs::remove_file(audio_path).unwrap();
    }

    #[test]
    fn processing_job_keeps_the_settings_snapshot_from_start() {
        let repository = Arc::new(MemoryRepository::default());
        let workspace = WorkspaceService::with_options(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(repository),
            ProcessingOptions {
                language: Some("zh".into()),
                expected_speakers: Some(2),
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
                diarization_stages: DiarizationStageConfiguration::default(),
            },
        );
        let audio_path = write_test_wav();
        let project = workspace
            .import_audio(audio_path.display().to_string())
            .unwrap();
        let asr_model = ModelRef {
            backend_id: "fake-asr".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };
        let diarization_model = ModelRef {
            backend_id: "fake-diarization".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };
        let job = workspace
            .start_project_processing(project.id().clone(), asr_model, diarization_model)
            .unwrap();
        workspace
            .set_processing_options(ProcessingOptions {
                language: Some("en".into()),
                expected_speakers: Some(4),
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
                diarization_stages: DiarizationStageConfiguration::default(),
            })
            .unwrap();
        let result = futures::executor::block_on(job.future).unwrap();
        let processing = result.processing.latest.as_ref().unwrap();
        assert_eq!(processing.runtime.language.as_deref(), Some("zh"));
        assert_eq!(processing.runtime.expected_speakers, Some(2));
        std::fs::remove_file(audio_path).unwrap();
    }

    #[test]
    fn document_commands_write_the_workspace_documents_with_revisions() {
        let repository = Arc::new(MemoryRepository::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(repository.clone()),
        );
        let project = workspace.import_audio("meeting.wav").unwrap();
        let commands = workspace.document_commands();

        let updated = commands
            .update_document(
                project.id(),
                project.revision(),
                contracts::DocumentPatch {
                    title: Some("Renamed".into()),
                    ..contracts::DocumentPatch::default()
                },
            )
            .unwrap();

        assert_eq!(updated.state.revision(), project.revision() + 1);
        assert_eq!(
            workspace.project(project.id()).unwrap().unwrap().title(),
            "Renamed"
        );
        assert!(matches!(
            commands.update_document(
                project.id(),
                project.revision(),
                contracts::DocumentPatch::default()
            ),
            Err(crate::DocumentCommandError::RevisionConflict { .. })
        ));
    }

    #[test]
    fn processing_jobs_run_on_the_given_job_runtime() {
        let runtime = Arc::new(JobRuntime::with_max_concurrency(1));
        let processing = ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization))
            .with_job_runtime(runtime.clone());

        let contexts = processing.contexts();

        let jobs: Vec<_> = runtime
            .list()
            .unwrap()
            .into_iter()
            .map(|job| job.job_id)
            .collect();
        assert_eq!(
            jobs,
            vec![
                contexts.asr_context.job_id(),
                contexts.diarization_context.job_id()
            ]
        );
    }

    /// Hands out one prepared change stream.
    #[derive(Default)]
    struct ChangingRepository {
        changes: std::sync::Mutex<Option<crate::DocumentChanges>>,
    }

    impl DocumentRepository for ChangingRepository {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            Ok(DocumentSnapshot {
                documents: Vec::new(),
                issues: Vec::new(),
            })
        }

        fn save(&self, _: DocumentState, _: Option<String>) -> Result<(), String> {
            Ok(())
        }

        fn get(&self, _: &ProjectId) -> Result<Option<DocumentState>, String> {
            Ok(None)
        }

        fn read(&self, _: &ProjectId) -> Result<Option<DocumentRecord>, String> {
            Ok(None)
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
            Err("not used".into())
        }

        fn subscribe_changes(&self) -> Result<crate::DocumentChanges, String> {
            self.changes
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| "already subscribed".to_string())
        }
    }

    struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn document_changes_stream_from_the_repository_and_stop_its_watcher_on_drop() {
        use futures::StreamExt;
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let watcher_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let repository = ChangingRepository {
            changes: std::sync::Mutex::new(Some(crate::DocumentChanges::new(
                receiver,
                DropFlag(watcher_dropped.clone()),
            ))),
        };
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(Arc::new(repository)),
        );

        let mut changes = workspace.subscribe_document_changes().unwrap();
        sender
            .unbounded_send(crate::DocumentChange::Changed)
            .unwrap();
        drop(sender);

        assert_eq!(
            futures::executor::block_on(changes.next()),
            Some(crate::DocumentChange::Changed)
        );
        assert_eq!(futures::executor::block_on(changes.next()), None);
        assert!(!watcher_dropped.load(std::sync::atomic::Ordering::SeqCst));
        drop(changes);
        assert!(watcher_dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_repository_without_change_subscriptions_reports_an_error() {
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(Arc::new(MemoryRepository::default())),
        );

        assert!(workspace.subscribe_document_changes().is_err());
    }
}
