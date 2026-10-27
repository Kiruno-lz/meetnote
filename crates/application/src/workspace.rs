use crate::documents_root::{same_location, validate_documents_root};
use crate::{
    run_pipeline, run_windowed_pipeline, AsrBackend, AsrBackendProvider, BackendFuture,
    DiarizationBackend, DiarizationBackendProvider, DocumentService, DocumentsRoot,
    DocumentsRootError, LongAudioConfig, PipelineResult,
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
use runtime::{JobContext, JobEvent, JobId, JobRuntime, JobRuntimeError, JobSnapshot, JobState};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

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

    /// Jobs of this service's runtime that are queued or running, including
    /// those of other services sharing the runtime.
    pub fn active_job_ids(&self) -> Result<Vec<JobId>, JobRuntimeError> {
        Ok(self
            .runtime
            .list()?
            .into_iter()
            .filter(|job| matches!(job.state, JobState::Queued | JobState::Running))
            .map(|job| job.job_id)
            .collect())
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
    /// Shared by every clone. Processing starts hold it for reading, a root
    /// switch for writing, so a job cannot start in the middle of a switch.
    documents_root: Arc<RwLock<Option<DocumentsRoot>>>,
    /// Jobs of processing futures that have not finished or been dropped.
    /// A cancelled job is no longer active in the runtime while its future
    /// still saves the outcome, so a root switch checks these as well.
    processing_in_flight: Arc<Mutex<BTreeSet<JobId>>>,
}

/// Registers a processing future's jobs until the future ends or is dropped.
struct InFlightProcessing {
    jobs: Arc<Mutex<BTreeSet<JobId>>>,
    ids: [JobId; 2],
}

impl InFlightProcessing {
    fn register(jobs: &Arc<Mutex<BTreeSet<JobId>>>, ids: [JobId; 2]) -> Self {
        jobs.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(ids);
        Self {
            jobs: jobs.clone(),
            ids,
        }
    }
}

impl Drop for InFlightProcessing {
    fn drop(&mut self) {
        let mut jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in &self.ids {
            jobs.remove(id);
        }
    }
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
            documents_root: Arc::new(RwLock::new(None)),
            processing_in_flight: Arc::default(),
        }
    }

    /// Lets this workspace switch documents roots; `root.root()` must be the
    /// root its `DocumentService` was opened on.
    pub fn with_documents_root(self, root: DocumentsRoot) -> Self {
        *self
            .documents_root
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(root);
        self
    }

    /// The documents root every clone currently reads and writes, if this
    /// workspace was given one.
    pub fn documents_root(&self) -> Option<PathBuf> {
        self.documents_root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|root| root.root.clone())
    }

    /// Opens `root` (creating the directory if needed) and makes every clone
    /// of this workspace and of its `DocumentService` read and write it.
    ///
    /// The root is checked by [`crate::validate_documents_root`] against the
    /// Speaker Library of the current [`ProcessingOptions`] and the model
    /// directory. While any job of the shared job runtime (processing or
    /// Assistant) is queued or running, or a processing future of this
    /// workspace has not ended, the switch fails with
    /// [`DocumentsRootError::WorkspaceBusy`] and changes nothing. A call
    /// already running on another thread against [`Self::documents`]
    /// finishes on the old root. After a switch, every change feed of the old root
    /// ([`Self::watch_documents`], [`Self::subscribe_document_changes`]) ends;
    /// callers subscribe again. Switching to the current root changes
    /// nothing. The caller saves the new root in the settings (with its
    /// revision check) once this returns `Ok`; blocking, call it off the UI
    /// thread.
    pub fn switch_documents_root(&self, root: impl AsRef<Path>) -> Result<(), DocumentsRootError> {
        let root = root.as_ref();
        let speaker_library = self
            .options
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .speaker_library_path
            .clone()
            .map(PathBuf::from);
        let mut current = self
            .documents_root
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = current.as_mut().ok_or(DocumentsRootError::Unsupported)?;
        validate_documents_root(root, speaker_library.as_deref(), &current.model_directory)?;
        if same_location(root, &current.root) {
            return Ok(());
        }
        let mut job_ids: BTreeSet<JobId> = self
            .processing
            .active_job_ids()
            .map_err(|error| DocumentsRootError::Jobs(error.to_string()))?
            .into_iter()
            .collect();
        job_ids.extend(
            self.processing_in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter(),
        );
        if !job_ids.is_empty() {
            return Err(DocumentsRootError::WorkspaceBusy {
                job_ids: job_ids.into_iter().collect(),
            });
        }
        fs::create_dir_all(root).map_err(|error| DocumentsRootError::Create {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        let repository = (current.factory)(root).map_err(|message| DocumentsRootError::Open {
            path: root.to_path_buf(),
            message,
        })?;
        self.documents.replace_repository(repository);
        current.root = root.to_path_buf();
        Ok(())
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
    /// [`crate::DocumentCommandError::RevisionConflict`]. The handle keeps the
    /// current repository if execution is queued across a documents-root switch.
    pub fn document_commands(&self) -> crate::DocumentCommandService {
        crate::DocumentCommandService::new(self.documents.frozen())
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

    /// Changes of the current documents root; the receiver disconnects after
    /// a documents-root switch.
    pub fn watch_documents(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
        self.documents.watch()
    }

    /// Document-directory changes as an async stream the watcher feeds
    /// directly; dropping it stops watching. It ends after a documents-root
    /// switch; subscribe again for the new root.
    pub fn subscribe_document_changes(&self) -> Result<crate::DocumentChanges, String> {
        self.documents.subscribe_changes()
    }

    pub fn import_audio(&self, audio_path: impl Into<String>) -> Result<DocumentState, String> {
        self.documents.import_audio(audio_path)
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
        // Request construction and job admission must read the same workspace.
        let _no_switch = self
            .documents_root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let request = self.processing_request(
            &project_id,
            asr_model,
            diarization_model,
            speaker_library_path,
        )?;
        self.start_project_processing_inner(project_id, request)
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
        // Until the jobs are running, a documents-root switch waits; after
        // that it sees them and refuses.
        let _no_switch = self
            .documents_root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.start_project_processing_inner(project_id, request)
    }

    // Both public admission paths hold documents_root across request reads,
    // probe and registration; do not acquire the read lock recursively here.
    fn start_project_processing_inner(
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
        let in_flight = InFlightProcessing::register(
            &self.processing_in_flight,
            [
                contexts.asr_context.job_id(),
                contexts.diarization_context.job_id(),
            ],
        );
        let future = Box::pin(async move {
            // Dropped with the future: when it ends, is dropped unpolled or
            // is dropped mid-way.
            let _in_flight = in_flight;
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
    let mut checksums = ProcessingChecksums::default();
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
        input_audio_checksum: checksums.file(Path::new(&request.audio_path)),
        asr: model_provenance(&result.asr.model, &mut checksums),
        diarization: DiarizationProvenance {
            base_model: Some(model_provenance(&result.diarization.model, &mut checksums)),
            backend_id: result.diarization.backend.id.clone(),
            model_revision: result.diarization.model.revision.clone(),
            segmentation: result
                .diarization_stage_provenance
                .segmentation
                .as_ref()
                .map(|model| model_provenance(model, &mut checksums)),
            embedding: result
                .diarization_stage_provenance
                .embedding
                .as_ref()
                .map(|model| model_provenance(model, &mut checksums)),
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

fn model_provenance(model: &ModelRef, checksums: &mut ProcessingChecksums) -> ModelProvenance {
    ModelProvenance {
        backend_id: model.backend_id.clone(),
        model_id: model.model_id.clone(),
        revision: model.revision.clone(),
        artifact_sha256: checksums.artifact(Path::new(&model.artifact_path)),
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
                    checksums.file(&artifact_path)
                },
                archive_member: artifact.archive_member.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
thread_local! {
    static CHECKSUM_FILE_READS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn open_checksum_file(path: &Path) -> std::io::Result<fs::File> {
    let file = fs::File::open(path)?;
    #[cfg(test)]
    CHECKSUM_FILE_READS.with(|reads| reads.borrow_mut().push(path.to_path_buf()));
    Ok(file)
}

/// The lowercase hex SHA-256 of the file at `path`, as recorded in
/// `processing.latest.input_audio_checksum`.
pub(crate) fn read_file_sha256(path: &Path) -> std::io::Result<String> {
    stream_file_sha256(path, None)
}

fn stream_file_sha256(path: &Path, mut aggregate: Option<&mut Sha256>) -> std::io::Result<String> {
    let file = open_checksum_file(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if let Some(aggregate) = aggregate.as_deref_mut() {
            aggregate.update(&buffer[..read]);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Reuse observations only while constructing one record. No process-wide cache:
/// later jobs must observe external edits, missing files and restored files afresh.
#[derive(Default)]
struct ProcessingChecksums {
    files: BTreeMap<PathBuf, Option<String>>,
    bundles: BTreeMap<PathBuf, Option<String>>,
}

impl ProcessingChecksums {
    fn file(&mut self, path: &Path) -> Option<String> {
        self.files
            .entry(path.to_path_buf())
            .or_insert_with(|| read_file_sha256(path).ok())
            .clone()
    }

    fn artifact(&mut self, root: &Path) -> Option<String> {
        if root.is_file() {
            return self.file(root);
        }
        if let Some(checksum) = self.bundles.get(root) {
            return checksum.clone();
        }
        let checksum = self.read_bundle(root);
        self.bundles.insert(root.to_path_buf(), checksum.clone());
        checksum
    }

    fn read_bundle(&mut self, root: &Path) -> Option<String> {
        if !root.is_dir() {
            return None;
        }
        let mut files = Vec::new();
        collect_files(root, &mut files).ok()?;
        files.sort();
        let mut aggregate = Sha256::new();
        for file in files {
            let relative = file.strip_prefix(root).ok()?;
            aggregate.update(relative.to_string_lossy().as_bytes());
            // Preserve the aggregate's sorted relative-name + raw-byte format.
            // Hashing file digests instead would silently change stored checksums.
            let checksum = stream_file_sha256(&file, Some(&mut aggregate)).ok();
            self.files.insert(file, checksum.clone());
            checksum?;
        }
        Some(format!("{:x}", aggregate.finalize()))
    }
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

impl DocumentService {
    /// Creates the document for an external audio path. Capture a frozen
    /// service before queueing the import to retain its chosen workspace.
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
        let project = self.save_with_markdown(
            project.clone(),
            render_markdown(
                Path::new(project.audio_path()),
                &project.results.speaker_segments,
            ),
        )?;
        Ok(project)
    }
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
        DocumentSnapshot, ReservedLocation,
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

    fn provenance_fixture_record(
        audio: &str,
        asr: ModelRef,
        diarization: ModelRef,
    ) -> ProcessingRecord {
        let request = ProcessingRequest {
            audio_path: audio.into(),
            asr_model: asr.clone(),
            diarization_model: diarization.clone(),
            language: Some("en".into()),
            expected_speakers: None,
            device: ComputeDevice::Auto,
            speaker_library_path: None,
            stage_configuration: Default::default(),
        };
        let result = PipelineResult {
            asr: AsrResult {
                backend: FakeAsr.descriptor(),
                model: asr,
                language: Some("en".into()),
                segments: vec![],
            },
            diarization: DiarizationResult {
                backend: FakeDiarization.descriptor(),
                model: diarization.clone(),
                segments: vec![],
                speaker_count: None,
                embedding_observations: vec![],
                speaker_profile_candidates: vec![],
            },
            aligned: vec![],
            windowing: None,
            diarization_stage_provenance: FakeDiarization.stage_provenance(&diarization),
        };
        processing_record(&request, &result, 1)
    }

    #[test]
    fn processing_provenance_reads_a_shared_bundle_once_and_refreshes_next_record() {
        // Real filesystem bytes, not model execution. The recorder must not repeatedly
        // stream one bundle for its base, segmentation, embedding and artifact entries.
        let root = std::env::temp_dir().join(format!("meetnote-checksum-{}", unique_suffix()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a.onnx"), b"graph").unwrap();
        std::fs::write(root.join("nested/weights.bin"), vec![19u8; 65_537]).unwrap();
        let model = ModelRef {
            backend_id: "fake-diarization".into(),
            model_id: "shared-bundle".into(),
            revision: None,
            artifact_path: root.display().to_string(),
            artifacts: vec![
                contracts::ModelArtifactRef {
                    filename: "a.onnx".into(),
                    expected_sha256: None,
                    archive_member: None,
                },
                contracts::ModelArtifactRef {
                    filename: "nested/weights.bin".into(),
                    expected_sha256: None,
                    archive_member: None,
                },
            ],
        };
        let audio = write_test_wav();
        CHECKSUM_FILE_READS.with(|reads| reads.borrow_mut().clear());
        let first =
            provenance_fixture_record(&audio.display().to_string(), model.clone(), model.clone());
        for path in [
            root.join("a.onnx"),
            root.join("nested/weights.bin"),
            audio.clone(),
        ] {
            let reads = CHECKSUM_FILE_READS
                .with(|reads| reads.borrow().iter().filter(|read| **read == path).count());
            assert_eq!(reads, 1, "stream each shared file once: {}", path.display());
        }
        let base = first.diarization.base_model.as_ref().unwrap();
        assert_eq!(
            base.artifact_sha256.as_deref(),
            Some("fdea8d895858cfeb7eadbba8b77ecb6a1bc90a98dc7a4814ef4c885df48f8171")
        );
        assert_eq!(
            base.artifacts[1].observed_sha256.as_deref(),
            Some("882eeb071086100fc0b9da9a6d168a7df202e4a405593a819df1e45bdf232446")
        );
        assert_eq!(first.asr, *base);
        assert_eq!(first.diarization.segmentation.as_ref(), Some(base));
        assert_eq!(first.diarization.embedding.as_ref(), Some(base));
        std::fs::write(root.join("nested/weights.bin"), b"edited weights").unwrap();
        let second =
            provenance_fixture_record(&audio.display().to_string(), model.clone(), model.clone());
        let refreshed = second.diarization.base_model.as_ref().unwrap();
        assert_ne!(base.artifact_sha256, refreshed.artifact_sha256);
        assert_ne!(
            base.artifacts[1].observed_sha256,
            refreshed.artifacts[1].observed_sha256
        );
        std::fs::remove_file(root.join("nested/weights.bin")).unwrap();
        let missing = provenance_fixture_record(&audio.display().to_string(), model.clone(), model);
        assert_eq!(
            missing.diarization.base_model.unwrap().artifacts[1].observed_sha256,
            None
        );
        std::fs::remove_file(audio).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "explicit MEETNOTE_PROVENANCE_RECORDS and MEETNOTE_PROVENANCE_MODEL_ROOT; hashes real model bytes, does not run inference"]
    fn benchmark_processing_provenance_with_explicit_real_model_files() {
        let root = PathBuf::from(
            std::env::var("MEETNOTE_PROVENANCE_MODEL_ROOT").expect("explicit model root"),
        );
        let records =
            std::env::var("MEETNOTE_PROVENANCE_RECORDS").expect("explicit saved processing rows");
        let text = std::fs::read_to_string(records).unwrap();
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 4);
        let mut output = Vec::new();
        for row in rows {
            let saved: DocumentState = serde_json::from_value(row["state"].clone()).unwrap();
            let original = saved.processing.latest.as_ref().unwrap();
            let model = |source: &ModelProvenance| ModelRef {
                backend_id: source.backend_id.clone(),
                model_id: source.model_id.clone(),
                revision: source.revision.clone(),
                artifact_path: source.artifact_path.clone(),
                artifacts: source
                    .artifacts
                    .iter()
                    .map(|a| contracts::ModelArtifactRef {
                        filename: a.filename.clone(),
                        expected_sha256: a.expected_sha256.clone(),
                        archive_member: a.archive_member.clone(),
                    })
                    .collect(),
            };
            let base = original.diarization.base_model.as_ref().unwrap();
            for source in [
                &original.asr,
                base,
                original.diarization.segmentation.as_ref().unwrap(),
                original.diarization.embedding.as_ref().unwrap(),
            ] {
                assert!(Path::new(&source.artifact_path).starts_with(&root));
            }
            let request = ProcessingRequest {
                audio_path: saved.audio_path().into(),
                asr_model: model(&original.asr),
                diarization_model: model(base),
                language: Some("en".into()),
                expected_speakers: None,
                device: ComputeDevice::Auto,
                speaker_library_path: None,
                stage_configuration: Default::default(),
            };
            let result = PipelineResult {
                asr: AsrResult {
                    backend: FakeAsr.descriptor(),
                    model: request.asr_model.clone(),
                    language: Some("en".into()),
                    segments: vec![],
                },
                diarization: DiarizationResult {
                    backend: contracts::BackendDescriptor {
                        id: original.diarization.backend_id.clone(),
                        ..FakeDiarization.descriptor()
                    },
                    model: request.diarization_model.clone(),
                    segments: vec![],
                    speaker_count: None,
                    embedding_observations: vec![],
                    speaker_profile_candidates: vec![],
                },
                aligned: vec![],
                windowing: None,
                diarization_stage_provenance: crate::DiarizationStageProvenance {
                    segmentation: original.diarization.segmentation.as_ref().map(model),
                    embedding: original.diarization.embedding.as_ref().map(model),
                    clustering: Some(original.diarization.clustering.clone()),
                },
            };
            CHECKSUM_FILE_READS.with(|reads| reads.borrow_mut().clear());
            let started = std::time::Instant::now();
            let record = processing_record(&request, &result, original.processing_revision);
            let seconds = started.elapsed().as_secs_f64();
            assert_eq!(record.input_audio_checksum, original.input_audio_checksum);
            assert_eq!(record.asr, original.asr);
            assert_eq!(
                record.diarization.base_model,
                original.diarization.base_model
            );
            assert_eq!(
                record.diarization.segmentation,
                original.diarization.segmentation
            );
            assert_eq!(record.diarization.embedding, original.diarization.embedding);
            assert_eq!(
                record.diarization.clustering,
                original.diarization.clustering
            );
            let reads = CHECKSUM_FILE_READS.with(|reads| reads.borrow().clone());
            let bytes: u64 = reads
                .iter()
                .map(|p| std::fs::metadata(p).unwrap().len())
                .sum();
            output.push(serde_json::json!({"profile":row["profile"],"seconds":seconds,"fileStreams":reads.len(),"bytesStreamed":bytes,"provenanceExact":true}));
        }
        println!(
            "provenance-benchmark: {}",
            serde_json::to_string(&output).unwrap()
        );
    }

    #[test]
    fn processing_provenance_keeps_the_base_bundle_and_external_sidecars() {
        // Byte fixtures and fake providers test persisted provenance, not model quality.
        struct UnstagedDiarization;
        impl DiarizationBackend for UnstagedDiarization {
            fn descriptor(&self) -> BackendDescriptor {
                FakeDiarization.descriptor()
            }
            fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
                FakeDiarization.probe(model)
            }
            fn diarize(
                &self,
                request: DiarizationRequest,
                context: JobContext,
            ) -> BackendFuture<DiarizationResult> {
                FakeDiarization.diarize(request, context)
            }
        }
        let root = std::env::temp_dir().join(format!("meetnote-base-bundle-{}", unique_suffix()));
        std::fs::create_dir_all(&root).unwrap();
        let files = [
            (
                "graph.onnx",
                "graph",
                "eef93e1d14482804277fca0172464032d1a4fdbcc338524059fa1e861454ad4d",
            ),
            (
                "graph.onnx.data",
                "external weights",
                "474a90a73b4c8dcefbf595d24cef79ce69a9eb252e8c10a405928967122ac368",
            ),
            (
                "plda.npy",
                "calibration",
                "e152337e4e85aa3e81482f0ce329aec7bfad531413fe53fef84f1f0d4165caee",
            ),
        ];
        for (name, bytes, _) in &files {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        let model = ModelRef {
            backend_id: "fake-diarization".into(),
            model_id: "fixture-bundle-profile".into(),
            revision: Some("bundle-revision".into()),
            artifact_path: root.display().to_string(),
            artifacts: files
                .iter()
                .map(|(name, _, sha)| contracts::ModelArtifactRef {
                    filename: (*name).into(),
                    expected_sha256: Some((*sha).into()),
                    archive_member: None,
                })
                .collect(),
        };
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(UnstagedDiarization)),
            DocumentService::new(Arc::new(MemoryRepository::default())),
        );
        let audio = write_test_wav();
        let project = workspace.import_audio(audio.display().to_string()).unwrap();
        let asr = ModelRef {
            backend_id: "fake-asr".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "not a model execution".into(),
            artifacts: vec![],
        };
        let output = futures::executor::block_on(workspace.process_project(
            project.id().clone(),
            asr,
            model,
        ))
        .unwrap();
        let json = serde_json::to_value(output.processing.latest.as_ref().unwrap()).unwrap();
        let base = &json["diarization"]["base_model"];
        assert_eq!(base["model_id"], "fixture-bundle-profile");
        assert_eq!(base["revision"], "bundle-revision");
        assert_eq!(base["artifact_path"], root.display().to_string());
        assert_eq!(base["artifacts"].as_array().unwrap().len(), files.len());
        for (name, _, sha) in files {
            let row = base["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["filename"] == name)
                .unwrap();
            assert_eq!(row["expected_sha256"], sha);
            assert_eq!(row["observed_sha256"], sha);
        }
        assert!(json["diarization"]["segmentation"].is_null());
        assert!(json["diarization"]["embedding"].is_null());
        std::fs::remove_file(audio).unwrap();
        std::fs::remove_dir_all(root).unwrap();
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

    /// Hands out one in-memory repository per documents root.
    #[derive(Clone, Default)]
    struct RootRepositories {
        roots: Arc<std::sync::Mutex<BTreeMap<PathBuf, Arc<MemoryRepository>>>>,
    }

    impl RootRepositories {
        fn repository(&self, root: &Path) -> Arc<MemoryRepository> {
            self.roots
                .lock()
                .unwrap()
                .entry(root.to_path_buf())
                .or_default()
                .clone()
        }

        fn opened(&self, root: &Path) -> bool {
            self.roots.lock().unwrap().contains_key(root)
        }

        fn ids(&self, root: &Path) -> Vec<String> {
            self.repository(root)
                .projects
                .read()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        }

        fn documents_root(&self, root: &Path, model_directory: &Path) -> DocumentsRoot {
            let roots = self.clone();
            DocumentsRoot::new(root, model_directory, move |root: &Path| {
                Ok(roots.repository(root) as Arc<dyn DocumentRepository>)
            })
        }
    }

    fn temp_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "meetnote-workspace-{label}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// A workspace at `<base>/old` with a shared job runtime, the model
    /// directory `<base>/models` and the Speaker Library
    /// `<base>/speakers/library.json`.
    struct RootFixture {
        base: PathBuf,
        old_root: PathBuf,
        models: PathBuf,
        speaker_library: PathBuf,
        roots: RootRepositories,
        jobs: Arc<JobRuntime>,
        workspace: WorkspaceService,
    }

    impl RootFixture {
        fn new(label: &str) -> Self {
            Self::with_repository(label, None)
        }

        fn with_repository(label: &str, old: Option<Arc<dyn DocumentRepository>>) -> Self {
            let base = temp_directory(label);
            let old_root = base.join("old");
            let models = base.join("models");
            let speaker_library = base.join("speakers/library.json");
            fs::create_dir_all(&old_root).unwrap();
            fs::create_dir_all(models.join("asr")).unwrap();
            fs::create_dir_all(speaker_library.parent().unwrap()).unwrap();
            fs::write(
                &speaker_library,
                serde_json::to_vec(&contracts::SpeakerLibraryDocument::default()).unwrap(),
            )
            .unwrap();
            let roots = RootRepositories::default();
            let old = old.unwrap_or_else(|| roots.repository(&old_root));
            let jobs = Arc::new(JobRuntime::default());
            let workspace = WorkspaceService::with_options(
                ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization))
                    .with_job_runtime(jobs.clone()),
                DocumentService::new(old),
                ProcessingOptions {
                    speaker_library_path: Some(speaker_library.display().to_string()),
                    ..ProcessingOptions::default()
                },
            )
            .with_documents_root(roots.documents_root(&old_root, &models));
            Self {
                base,
                old_root,
                models,
                speaker_library,
                roots,
                jobs,
                workspace,
            }
        }
    }

    impl Drop for RootFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn fixture_models() -> (ModelRef, ModelRef) {
        let model = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };
        (model("fake-asr"), model("fake-diarization"))
    }

    #[test]
    fn switching_the_documents_root_moves_every_clone_to_the_new_root() {
        let fixture = RootFixture::new("switch");
        let old = fixture.workspace.import_audio("old.wav").unwrap();
        let clone = fixture.workspace.clone();
        // Held the way the Assistant holds its document service.
        let documents = fixture.workspace.documents();
        let new_root = fixture.base.join("new/documents");

        fixture.workspace.switch_documents_root(&new_root).unwrap();

        assert!(new_root.is_dir());
        assert_eq!(clone.documents_root(), Some(new_root.clone()));
        assert!(clone.list_projects().unwrap().is_empty());
        assert_eq!(documents.get(old.id()).unwrap(), None);
        let imported = clone.import_audio("new.wav").unwrap();
        documents
            .save(DocumentState::new(
                ProjectId::new("assistant-write"),
                "Assistant",
                "a.wav",
            ))
            .unwrap();
        assert_eq!(
            fixture.roots.ids(&fixture.old_root),
            [old.id().as_str().to_string()]
        );
        let mut expected = vec![
            imported.id().as_str().to_string(),
            "assistant-write".to_string(),
        ];
        expected.sort();
        assert_eq!(fixture.roots.ids(&new_root), expected);

        // Switching back opens the old root again.
        fixture
            .workspace
            .switch_documents_root(&fixture.old_root)
            .unwrap();
        assert_eq!(documents.list().unwrap(), vec![old]);
    }

    /// Change feeds whose senders the test holds, so that only a switch can
    /// end them.
    #[derive(Default)]
    struct FeedRepository {
        watchers: std::sync::Mutex<Vec<std::sync::mpsc::Sender<crate::DocumentChange>>>,
        streams:
            std::sync::Mutex<Vec<futures::channel::mpsc::UnboundedSender<crate::DocumentChange>>>,
    }

    impl FeedRepository {
        fn change(&self) {
            for sender in self.watchers.lock().unwrap().iter() {
                let _ = sender.send(crate::DocumentChange::Changed);
            }
            for sender in self.streams.lock().unwrap().iter() {
                let _ = sender.unbounded_send(crate::DocumentChange::Changed);
            }
        }
    }

    impl DocumentRepository for FeedRepository {
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
            let (sender, receiver) = std::sync::mpsc::channel();
            self.watchers.lock().unwrap().push(sender);
            Ok(receiver)
        }

        fn subscribe_changes(&self) -> Result<crate::DocumentChanges, String> {
            let (sender, receiver) = futures::channel::mpsc::unbounded();
            self.streams.lock().unwrap().push(sender);
            Ok(crate::DocumentChanges::new(receiver, ()))
        }
    }

    #[test]
    fn a_documents_root_switch_ends_the_change_feeds_of_the_old_root() {
        use futures::{FutureExt, StreamExt};
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::Duration;
        let old = Arc::new(FeedRepository::default());
        let fixture = RootFixture::with_repository("feeds", Some(old.clone()));
        let new = Arc::new(FeedRepository::default());
        let opened = new.clone();
        let workspace = fixture
            .workspace
            .clone()
            .with_documents_root(DocumentsRoot::new(
                &fixture.old_root,
                &fixture.models,
                move |_: &Path| Ok(opened.clone() as Arc<dyn DocumentRepository>),
            ));
        let watched = workspace.watch_documents().unwrap();
        let mut stream = workspace.subscribe_document_changes().unwrap();
        old.change();
        assert_eq!(
            watched.recv_timeout(Duration::from_secs(5)),
            Ok(crate::DocumentChange::Changed)
        );
        assert_eq!(
            stream.next().now_or_never(),
            Some(Some(crate::DocumentChange::Changed))
        );

        // The current root, spelled differently: nothing changes.
        workspace
            .switch_documents_root(fixture.old_root.join("../old/."))
            .unwrap();
        old.change();
        assert_eq!(
            watched.recv_timeout(Duration::from_secs(5)),
            Ok(crate::DocumentChange::Changed)
        );
        assert_eq!(
            stream.next().now_or_never(),
            Some(Some(crate::DocumentChange::Changed))
        );
        assert_eq!(stream.next().now_or_never(), None);

        workspace
            .switch_documents_root(fixture.base.join("new"))
            .unwrap();

        assert_eq!(
            watched.recv_timeout(Duration::from_secs(5)),
            Err(RecvTimeoutError::Disconnected)
        );
        assert_eq!(stream.next().now_or_never(), Some(None));
        let mut resubscribed = workspace.subscribe_document_changes().unwrap();
        new.change();
        assert_eq!(
            resubscribed.next().now_or_never(),
            Some(Some(crate::DocumentChange::Changed))
        );
    }

    #[test]
    fn a_change_stream_outlives_the_document_service_it_came_from() {
        use futures::{FutureExt, StreamExt};
        let repository = Arc::new(FeedRepository::default());
        let mut stream = DocumentService::new(repository.clone())
            .subscribe_changes()
            .unwrap();

        repository.change();

        assert_eq!(
            stream.next().now_or_never(),
            Some(Some(crate::DocumentChange::Changed))
        );
        assert_eq!(stream.next().now_or_never(), None);
    }

    #[test]
    fn switching_while_a_job_is_queued_or_running_is_refused_as_busy() {
        let fixture = RootFixture::new("busy");
        let project = fixture.workspace.import_audio("kept.wav").unwrap();
        // An Assistant run on the shared runtime.
        let (running, _running_events) = fixture.jobs.create_job();
        running.started();
        let (queued, _queued_events) = fixture.jobs.create_job();
        let target = fixture.base.join("busy-target");

        assert_eq!(
            fixture.workspace.switch_documents_root(&target),
            Err(DocumentsRootError::WorkspaceBusy {
                job_ids: vec![running.job_id(), queued.job_id()]
            })
        );
        running.completed();
        assert_eq!(
            fixture.workspace.switch_documents_root(&target),
            Err(DocumentsRootError::WorkspaceBusy {
                job_ids: vec![queued.job_id()]
            })
        );
        assert!(!target.exists());
        assert!(!fixture.roots.opened(&target));
        assert_eq!(
            fixture.workspace.documents_root(),
            Some(fixture.old_root.clone())
        );
        assert_eq!(fixture.workspace.list_projects().unwrap(), vec![project]);

        queued.failed("stopped");
        fixture.workspace.switch_documents_root(&target).unwrap();
        assert_eq!(fixture.workspace.documents_root(), Some(target));
    }

    /// Tries to switch the documents root whenever processing results are
    /// saved, recording each outcome.
    struct SwitchOnResultSave {
        inner: MemoryRepository,
        workspace: std::sync::OnceLock<WorkspaceService>,
        target: PathBuf,
        attempts: std::sync::Mutex<Vec<Result<(), DocumentsRootError>>>,
    }

    impl DocumentRepository for SwitchOnResultSave {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            self.inner.snapshot()
        }

        fn save(&self, project: DocumentState, markdown: Option<String>) -> Result<(), String> {
            if project.processing.latest.is_some() {
                if let Some(workspace) = self.workspace.get() {
                    let attempt = workspace.switch_documents_root(&self.target);
                    self.attempts.lock().unwrap().push(attempt);
                }
            }
            self.inner.save(project, markdown)
        }

        fn get(&self, id: &ProjectId) -> Result<Option<DocumentState>, String> {
            self.inner.get(id)
        }

        fn read(&self, id: &ProjectId) -> Result<Option<DocumentRecord>, String> {
            self.inner.read(id)
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
            self.inner.watch()
        }
    }

    #[test]
    fn a_processing_job_keeps_the_workspace_busy_until_its_results_are_saved() {
        let base = temp_directory("result-save");
        let repository = Arc::new(SwitchOnResultSave {
            inner: MemoryRepository::default(),
            workspace: std::sync::OnceLock::new(),
            target: base.join("new"),
            attempts: std::sync::Mutex::new(Vec::new()),
        });
        let fixture = RootFixture::with_repository("result-save-root", Some(repository.clone()));
        assert!(repository.workspace.set(fixture.workspace.clone()).is_ok());
        let audio_path = write_test_wav();
        let project = fixture
            .workspace
            .import_audio(audio_path.display().to_string())
            .unwrap();
        let (asr_model, diarization_model) = fixture_models();
        let job = fixture
            .workspace
            .start_project_processing(project.id().clone(), asr_model, diarization_model)
            .unwrap();
        let (asr_job, diarization_job) = job.job_ids();
        assert_eq!(
            fixture.workspace.switch_documents_root(base.join("new")),
            Err(DocumentsRootError::WorkspaceBusy {
                job_ids: vec![asr_job, diarization_job]
            })
        );

        let processed = futures::executor::block_on(job.future).unwrap();

        let attempts = repository.attempts.lock().unwrap().clone();
        assert!(
            matches!(
                attempts.as_slice(),
                [Err(DocumentsRootError::WorkspaceBusy { .. })]
            ),
            "a switch during the result save must be refused: {attempts:?}"
        );
        assert_eq!(
            fixture.workspace.documents_root(),
            Some(fixture.old_root.clone())
        );
        assert_eq!(repository.inner.get(project.id()).unwrap(), Some(processed));
        fixture
            .workspace
            .switch_documents_root(base.join("new"))
            .unwrap();
        fs::remove_file(audio_path).unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_cancelled_processing_job_keeps_the_workspace_busy_until_its_future_ends() {
        let fixture = RootFixture::new("cancelled");
        let audio_path = write_test_wav();
        let project = fixture
            .workspace
            .import_audio(audio_path.display().to_string())
            .unwrap();
        let (asr_model, diarization_model) = fixture_models();
        let job = fixture
            .workspace
            .start_project_processing(project.id().clone(), asr_model, diarization_model)
            .unwrap();
        job.cancel();
        let target = fixture.base.join("after-cancel");

        assert!(matches!(
            fixture.workspace.switch_documents_root(&target),
            Err(DocumentsRootError::WorkspaceBusy { .. })
        ));
        // The future still writes the outcome into the root it started in.
        let _ = futures::executor::block_on(job.future);
        let written = fixture.roots.repository(&fixture.old_root);
        assert!(written.get(project.id()).unwrap().unwrap().revision() > project.revision());
        fixture.workspace.switch_documents_root(&target).unwrap();

        // A cancelled job dropped before it ran does not keep it busy.
        let imported = fixture
            .workspace
            .import_audio(audio_path.display().to_string())
            .unwrap();
        let (asr_model, diarization_model) = fixture_models();
        let dropped = fixture
            .workspace
            .start_project_processing(imported.id().clone(), asr_model, diarization_model)
            .unwrap();
        dropped.cancel();
        drop(dropped);
        fixture
            .workspace
            .switch_documents_root(&fixture.old_root)
            .unwrap();
        fs::remove_file(audio_path).unwrap();
    }

    #[test]
    fn invalid_documents_roots_are_rejected_without_changing_the_workspace() {
        let fixture = RootFixture::new("invalid");
        let project = fixture.workspace.import_audio("kept.wav").unwrap();
        let file = fixture.base.join("notes.txt");
        fs::write(&file, b"not a directory").unwrap();
        let speakers = fixture.speaker_library.parent().unwrap().to_path_buf();
        let reserved = |path: &Path, location: ReservedLocation| {
            matches!(
                fixture.workspace.switch_documents_root(path),
                Err(DocumentsRootError::Reserved { location: actual, .. }) if actual == location
            )
        };

        assert_eq!(
            fixture.workspace.switch_documents_root(""),
            Err(DocumentsRootError::Empty)
        );
        assert_eq!(
            fixture.workspace.switch_documents_root("   "),
            Err(DocumentsRootError::Empty)
        );
        assert_eq!(
            fixture.workspace.switch_documents_root(&file),
            Err(DocumentsRootError::NotADirectory { path: file.clone() })
        );
        assert!(reserved(
            &fixture.speaker_library,
            ReservedLocation::SpeakerLibrary
        ));
        assert!(reserved(&speakers, ReservedLocation::SpeakerLibrary));
        assert!(reserved(&fixture.base, ReservedLocation::SpeakerLibrary));
        assert!(reserved(&fixture.models, ReservedLocation::ModelDirectory));
        assert!(reserved(
            &fixture.models.join("asr/whisper"),
            ReservedLocation::ModelDirectory
        ));
        assert!(reserved(
            &fixture.base.join("old/../models/."),
            ReservedLocation::ModelDirectory
        ));
        assert!(matches!(
            fixture
                .workspace
                .switch_documents_root(file.join("documents")),
            Err(DocumentsRootError::Create { .. })
        ));

        let failing = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(Arc::new(MemoryRepository::default())),
        )
        .with_documents_root(DocumentsRoot::new(
            &fixture.old_root,
            &fixture.models,
            |_: &Path| Err("volume is offline".into()),
        ));
        let elsewhere = fixture.base.join("elsewhere");
        assert_eq!(
            failing.switch_documents_root(&elsewhere),
            Err(DocumentsRootError::Open {
                path: elsewhere,
                message: "volume is offline".into()
            })
        );
        let unsupported = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(Arc::new(MemoryRepository::default())),
        );
        assert_eq!(
            unsupported.switch_documents_root(&fixture.base.join("any")),
            Err(DocumentsRootError::Unsupported)
        );

        assert_eq!(
            fixture.workspace.documents_root(),
            Some(fixture.old_root.clone())
        );
        assert_eq!(fixture.workspace.list_projects().unwrap(), vec![project]);
    }
}
