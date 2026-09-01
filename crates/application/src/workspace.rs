use crate::{
    run_pipeline, AsrBackend, AsrBackendProvider, BackendFuture, DiarizationBackend,
    DiarizationBackendProvider, DocumentService, PipelineResult,
};
use contracts::{AsrRequest, ComputeDevice, DiarizationRequest, ModelRef};
use domain::{
    reorder_project, reparent_project, AlgorithmProvenance, DiarizationProvenance, DocumentState,
    IdentityMatchingProvenance, ModelProvenance, ProcessingRecord, ProjectId, RuntimeProvenance,
    SpeakerTaggedSegment,
};
use futures::channel::mpsc::UnboundedReceiver;
use runtime::{JobContext, JobEvent, JobRuntime};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ProcessingRequest {
    pub audio_path: String,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    pub device: ComputeDevice,
    pub speaker_library_path: Option<String>,
}

#[derive(Clone)]
pub struct ProcessingService {
    asr_provider: Arc<dyn AsrBackendProvider>,
    diarization_provider: Arc<dyn DiarizationBackendProvider>,
    runtime: Arc<JobRuntime>,
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
        }
    }

    pub fn probe(&self, request: &ProcessingRequest) -> Result<(), contracts::BackendError> {
        self.asr_provider
            .backend(&request.asr_model.backend_id)?
            .probe(&request.asr_model)?;
        self.diarization_provider
            .backend(&request.diarization_model.backend_id)?
            .probe(&request.diarization_model)
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
        Box::pin(async move {
            run_pipeline(
                asr_backend.as_ref(),
                diarization_backend.as_ref(),
                AsrRequest {
                    audio_path: request.audio_path.clone(),
                    model: request.asr_model,
                    language: request.language,
                    device: request.device,
                },
                DiarizationRequest {
                    audio_path: request.audio_path,
                    model: request.diarization_model,
                    expected_speakers: request.expected_speakers,
                    device: request.device,
                    speaker_library_path: request.speaker_library_path,
                },
                asr_context,
                diarization_context,
            )
            .await
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
}

pub struct ProjectProcessingJob {
    pub future: BackendFuture<DocumentState>,
    pub asr_context: JobContext,
    pub diarization_context: JobContext,
    pub asr_events: UnboundedReceiver<JobEvent>,
    pub diarization_events: UnboundedReceiver<JobEvent>,
}

impl WorkspaceService {
    pub fn new(processing: ProcessingService, documents: DocumentService) -> Self {
        Self {
            processing,
            documents,
        }
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
        let mut project = self
            .documents
            .get(&project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| persistence_error("project does not exist"))?;
        let request = ProcessingRequest {
            audio_path: project.audio_path().to_string(),
            asr_model,
            diarization_model,
            language: None,
            expected_speakers: None,
            device: ComputeDevice::Auto,
            speaker_library_path,
        };
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
            segmentation: Some(model_provenance(&result.diarization.model)),
            embedding: Some(model_provenance(&result.diarization.model)),
            clustering: AlgorithmProvenance {
                id: "backend-clustering".into(),
                revision: Some(result.diarization.backend.id.clone()),
                parameters: None,
            },
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
    }
}

fn file_sha256(path: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let digest = Sha256::digest(bytes);
    Some(format!("{digest:x}"))
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
        hasher.update(fs::read(file).ok()?);
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
                    language: Some("en".into()),
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
                    speaker_count: Some(1),
                    embedding_observations: Vec::new(),
                })
            })
        }
    }

    #[test]
    fn workspace_processes_and_persists_a_project() {
        let repository = Arc::new(MemoryRepository::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            DocumentService::new(repository.clone()),
        );
        let project = workspace.import_audio("fixture.wav").unwrap();
        let asr_model = ModelRef {
            backend_id: "fake-asr".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
        };
        let diarization_model = ModelRef {
            backend_id: "fake-diarization".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
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
        assert_eq!(processing.alignment.id, "max-overlap-speaker-alignment");
        assert_eq!(processing.runtime.sample_rate_hz, Some(16_000));
        assert_eq!(repository.get(project.id()).unwrap(), Some(result.clone()));
    }
}
