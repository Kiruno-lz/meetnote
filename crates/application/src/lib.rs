use contracts::{
    AsrRequest, AsrResult, BackendError, BackendTask, DiarizationRequest, DiarizationResult,
    DiarizationStageConfiguration, DocumentRecord, ModelRef,
};
use domain::{align_transcript_and_speakers, SpeakerTaggedSegment};
use runtime::JobContext;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

mod document_commands;
mod speaker_identity;
mod workspace;

pub use contracts::BackendDescriptor;
pub use document_commands::{DocumentCommandError, DocumentCommandService};
pub use speaker_identity::{
    apply_identity_matches, cosine_similarity, ClusterIdentityResolver, IdentityResolution,
    IdentityResolverConfig, InMemorySpeakerProfileStore, JsonSpeakerProfileStore,
    SpeakerLibraryError, SpeakerLibraryService, SpeakerProfileStore,
};
pub use workspace::{
    render_markdown, ProcessingContexts, ProcessingOptions, ProcessingRequest, ProcessingService,
    ProjectProcessingJob, WorkspaceService,
};

pub type BackendFuture<T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send>>;

pub fn migrate_file(source: &Path, destination: &Path) -> Result<(), String> {
    if source == destination {
        return Ok(());
    }
    if destination.exists() {
        return Err(format!(
            "migration destination already exists: {}",
            destination.display()
        ));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = destination.with_extension("migration.tmp");
    let bytes = fs::read(source).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

pub trait AsrBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &contracts::ModelRef) -> Result<(), BackendError>;
    fn transcribe(&self, request: AsrRequest, context: JobContext) -> BackendFuture<AsrResult>;
}

pub trait AsrBackendProvider: Send + Sync {
    fn available_backends(&self) -> Vec<BackendDescriptor>;
    fn backend(&self, backend_id: &str) -> Result<Arc<dyn AsrBackend>, BackendError>;
}

pub trait DiarizationBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &contracts::ModelRef) -> Result<(), BackendError>;
    fn validate_stage_configuration(
        &self,
        _model: &contracts::ModelRef,
        configuration: &DiarizationStageConfiguration,
    ) -> Result<(), BackendError> {
        if configuration.is_empty() {
            Ok(())
        } else {
            Err(BackendError {
                code: contracts::BackendErrorCode::ModelIncompatible,
                message: format!(
                    "{} does not support diarization stage overrides",
                    self.descriptor().id
                ),
            })
        }
    }
    fn probe_with_configuration(
        &self,
        model: &contracts::ModelRef,
        configuration: &DiarizationStageConfiguration,
    ) -> Result<(), BackendError> {
        self.probe(model)?;
        self.validate_stage_configuration(model, configuration)
    }
    fn stage_provenance(&self, _model: &contracts::ModelRef) -> DiarizationStageProvenance {
        DiarizationStageProvenance::default()
    }
    fn stage_provenance_with_configuration(
        &self,
        model: &contracts::ModelRef,
        _configuration: &DiarizationStageConfiguration,
    ) -> DiarizationStageProvenance {
        self.stage_provenance(model)
    }
    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult>;
}

pub trait DiarizationBackendProvider: Send + Sync {
    fn available_backends(&self) -> Vec<BackendDescriptor>;
    fn backend(&self, backend_id: &str) -> Result<Arc<dyn DiarizationBackend>, BackendError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub documents: Vec<domain::DocumentState>,
    pub issues: Vec<DocumentIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentChange {
    Changed,
}

pub trait DocumentRepository: Send + Sync {
    fn snapshot(&self) -> Result<DocumentSnapshot, String>;
    fn get(
        &self,
        document_id: &domain::DocumentId,
    ) -> Result<Option<domain::DocumentState>, String>;
    fn read(&self, document_id: &domain::DocumentId) -> Result<Option<DocumentRecord>, String>;
    fn watch(&self) -> Result<Receiver<DocumentChange>, String>;
    fn save(&self, document: domain::DocumentState, markdown: Option<String>)
        -> Result<(), String>;
}

#[derive(Clone)]
pub struct DocumentService {
    repository: Arc<dyn DocumentRepository>,
}

impl DocumentService {
    pub fn new(repository: Arc<dyn DocumentRepository>) -> Self {
        Self { repository }
    }

    pub fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        self.repository.snapshot()
    }

    pub fn list(&self) -> Result<Vec<domain::DocumentState>, String> {
        Ok(self.snapshot()?.documents)
    }

    pub fn get(
        &self,
        document_id: &domain::DocumentId,
    ) -> Result<Option<domain::DocumentState>, String> {
        self.repository.get(document_id)
    }

    pub fn read(&self, document_id: &domain::DocumentId) -> Result<Option<DocumentRecord>, String> {
        self.repository.read(document_id)
    }

    pub fn watch(&self) -> Result<Receiver<DocumentChange>, String> {
        self.repository.watch()
    }

    pub fn save(&self, document: domain::DocumentState) -> Result<domain::DocumentState, String> {
        let document = self.prepare(document)?;
        self.repository.save(document.clone(), None)?;
        Ok(document)
    }

    pub fn save_with_markdown(
        &self,
        document: domain::DocumentState,
        markdown: impl Into<String>,
    ) -> Result<domain::DocumentState, String> {
        let document = self.prepare(document)?;
        self.repository
            .save(document.clone(), Some(markdown.into()))?;
        Ok(document)
    }

    pub fn save_versioned(
        &self,
        mut document: domain::DocumentState,
        expected_revision: u64,
        markdown: impl Into<String>,
    ) -> Result<DocumentRecord, String> {
        let current = self
            .read(document.id())?
            .ok_or_else(|| "document does not exist".to_string())?;
        if current.state.revision() != expected_revision {
            return Err(format!(
                "document revision conflict: expected {}, found {}",
                expected_revision,
                current.state.revision()
            ));
        }
        document.schema.revision = expected_revision.saturating_add(1);
        document.schema.updated_at_ms = now_ms();
        self.repository
            .save(document.clone(), Some(markdown.into()))?;
        self.read(document.id())?
            .ok_or_else(|| "document disappeared after save".to_string())
    }

    fn prepare(
        &self,
        mut document: domain::DocumentState,
    ) -> Result<domain::DocumentState, String> {
        if let Some(current) = self.read(document.id())? {
            document.schema.revision = current.state.revision().saturating_add(1);
        }
        document.schema.updated_at_ms = now_ms();
        Ok(document)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInventory {
    pub asr: Vec<BackendDescriptor>,
    pub diarization: Vec<BackendDescriptor>,
}

impl BackendInventory {
    pub fn new(asr: Vec<BackendDescriptor>, diarization: Vec<BackendDescriptor>) -> Self {
        Self { asr, diarization }
    }
}

pub struct PipelineResult {
    pub asr: AsrResult,
    pub diarization: DiarizationResult,
    pub aligned: Vec<SpeakerTaggedSegment>,
    pub diarization_stage_provenance: DiarizationStageProvenance,
}

#[derive(Debug, Clone, Default)]
pub struct DiarizationStageProvenance {
    pub segmentation: Option<ModelRef>,
    pub embedding: Option<ModelRef>,
    pub clustering: Option<domain::AlgorithmProvenance>,
}

pub async fn run_pipeline(
    asr_backend: &dyn AsrBackend,
    diarization_backend: &dyn DiarizationBackend,
    asr_request: AsrRequest,
    diarization_request: DiarizationRequest,
    asr_context: JobContext,
    diarization_context: JobContext,
) -> Result<PipelineResult, BackendError> {
    let speaker_library_path = diarization_request.speaker_library_path.clone();
    let diarization_stage_provenance = diarization_backend.stage_provenance_with_configuration(
        &diarization_request.model,
        &diarization_request.stage_configuration,
    );
    let asr_future = asr_backend.transcribe(asr_request, asr_context);
    let diarization_future = diarization_backend.diarize(diarization_request, diarization_context);
    let (asr, diarization) = futures::join!(asr_future, diarization_future);
    let asr = asr?;
    let mut diarization = diarization?;
    if let Some(path) = speaker_library_path.as_deref() {
        let library = SpeakerLibraryService::new(Arc::new(JsonSpeakerProfileStore::new(path)));
        let resolver = ClusterIdentityResolver::new(IdentityResolverConfig::default());
        apply_identity_matches(&mut diarization, &library, &resolver).map_err(|error| {
            BackendError {
                code: contracts::BackendErrorCode::PersistenceFailed,
                message: error.to_string(),
            }
        })?;
    }
    let aligned = align_transcript_and_speakers(&asr.segments, &diarization.segments);

    Ok(PipelineResult {
        asr,
        diarization,
        aligned,
        diarization_stage_provenance,
    })
}

pub fn asr_descriptor(id: &str, capabilities: &[&str]) -> BackendDescriptor {
    BackendDescriptor {
        id: id.to_string(),
        task: BackendTask::Asr,
        capabilities: capabilities
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        configurable_diarization_stages: Vec::new(),
        diarization_stage_options: Vec::new(),
    }
}

pub fn diarization_descriptor(id: &str, capabilities: &[&str]) -> BackendDescriptor {
    BackendDescriptor {
        id: id.to_string(),
        task: BackendTask::Diarization,
        capabilities: capabilities
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        configurable_diarization_stages: Vec::new(),
        diarization_stage_options: Vec::new(),
    }
}

pub fn diarization_descriptor_with_stages(
    id: &str,
    capabilities: &[&str],
    stages: Vec<contracts::DiarizationStageKind>,
    options: Vec<contracts::DiarizationStageOption>,
) -> BackendDescriptor {
    let mut descriptor = diarization_descriptor(id, capabilities);
    descriptor.configurable_diarization_stages = stages;
    descriptor.diarization_stage_options = options;
    descriptor
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{AsrRequest, ComputeDevice, DiarizationRequest, ModelRef};
    use domain::{DiarizationSegment, TranscriptSegment};
    use runtime::JobRuntime;

    struct FakeAsr;

    impl AsrBackend for FakeAsr {
        fn descriptor(&self) -> BackendDescriptor {
            asr_descriptor("fake-asr", &["timestamps"])
        }

        fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(
            &self,
            _request: AsrRequest,
            _context: JobContext,
        ) -> BackendFuture<AsrResult> {
            Box::pin(async {
                Ok(AsrResult {
                    backend: asr_descriptor("fake-asr", &["timestamps"]),
                    model: ModelRef {
                        backend_id: "fake-asr".into(),
                        model_id: "fake".into(),
                        revision: None,
                        artifact_path: "fixture-model".into(),
                        artifacts: Vec::new(),
                    },
                    language: Some("zh".into()),
                    segments: vec![TranscriptSegment {
                        id: "t1".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "你好".into(),
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

        fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn diarize(
            &self,
            _request: DiarizationRequest,
            _context: JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(async {
                Ok(DiarizationResult {
                    backend: diarization_descriptor("fake-diarization", &["offline"]),
                    model: ModelRef {
                        backend_id: "fake-diarization".into(),
                        model_id: "fake".into(),
                        revision: None,
                        artifact_path: "fixture-model".into(),
                        artifacts: Vec::new(),
                    },
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
                    speaker_profile_candidates: Vec::new(),
                })
            })
        }
    }

    #[test]
    fn fake_backends_cover_the_first_pipeline() {
        let runtime = JobRuntime::default();
        let (asr_context, _) = runtime.create_job();
        let (diarization_context, _) = runtime.create_job();
        let request_model = ModelRef {
            backend_id: "fake".into(),
            model_id: "fake".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
            artifacts: Vec::new(),
        };
        let result = futures::executor::block_on(run_pipeline(
            &FakeAsr,
            &FakeDiarization,
            AsrRequest {
                audio_path: "fixture.wav".into(),
                model: request_model.clone(),
                language: Some("zh".into()),
                device: ComputeDevice::Cpu,
            },
            DiarizationRequest {
                audio_path: "fixture.wav".into(),
                model: request_model,
                expected_speakers: Some(1),
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
                stage_configuration: DiarizationStageConfiguration::default(),
            },
            asr_context,
            diarization_context,
        ))
        .unwrap();

        assert_eq!(result.aligned[0].speaker_id, "speaker-1");
    }

    #[test]
    fn migrate_file_copies_the_library_and_keeps_the_source() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("old/library.json");
        let destination = root.join("new/library.json");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, br#"{"profiles":[]}"#).unwrap();

        migrate_file(&source, &destination).unwrap();

        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&destination).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
