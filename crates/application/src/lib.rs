use contracts::{
    AgentMutationProvenance, AsrRequest, AsrResult, BackendError, BackendTask, DiarizationRequest,
    DiarizationResult, DiarizationStageConfiguration, DocumentRecord, ModelRef,
};
use domain::{align_transcript_and_speakers, SpeakerTaggedSegment};
use runtime::JobContext;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

mod agent_host;
mod capabilities;
mod document_commands;
mod long_audio;
mod speaker_identity;
mod workspace;
mod workspace_capability;

pub use agent_host::{
    AgentHost, CapabilityBinding, AGENT_HOST_CAPABILITY_ID, AGENT_HOST_CAPABILITY_VERSION,
};
pub use capabilities::{
    AudioCapabilityPort, CapabilityDescriptor, CapabilityOperation, SettingsCapabilityPort,
    SettingsPatchArguments, SettingsReadArguments, SettingsSection,
};
pub use contracts::BackendDescriptor;
pub use document_commands::{DocumentCommandError, DocumentCommandService};
pub use long_audio::{
    run_windowed_pipeline, AsrWindowSession, DiarizationWindowSession, LongAudioConfig,
};
pub use speaker_identity::{
    apply_identity_matches, cosine_similarity, ClusterIdentityResolver, IdentityResolution,
    IdentityResolverConfig, InMemorySpeakerProfileStore, JsonSpeakerProfileStore,
    SpeakerLibraryError, SpeakerLibraryService, SpeakerProfileStore,
};
pub use workspace::{
    render_markdown, ProcessingContexts, ProcessingOptions, ProcessingRequest, ProcessingService,
    ProjectProcessingJob, WorkspaceService,
};
pub use workspace_capability::{
    CapabilityProjectionPolicy, ToolRegistry, WorkspaceAccessPolicy, WorkspaceCapabilityPort,
    MAX_CAPABILITY_RESULT_BYTES,
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
    fn supports_windowed_audio(&self) -> bool {
        false
    }
    fn create_window_session(
        &self,
        _request: AsrRequest,
    ) -> Result<Box<dyn AsrWindowSession>, BackendError> {
        Err(BackendError::not_implemented(format!(
            "{} does not support bounded offline audio windows",
            self.descriptor().id
        )))
    }
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
    fn supports_windowed_audio(&self) -> bool {
        false
    }
    fn create_window_session(
        &self,
        _request: DiarizationRequest,
    ) -> Result<Box<dyn DiarizationWindowSession>, BackendError> {
        Err(BackendError::not_implemented(format!(
            "{} does not expose unclustered observations for bounded offline audio windows",
            self.descriptor().id
        )))
    }
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

/// The bounded projection used by list/read capability consumers. It excludes
/// transcript, diarization, and processing payloads so repositories can emit
/// list rows without materializing complete document states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub document_id: domain::DocumentId,
    pub revision: u64,
    pub title: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub document_status: String,
    pub parent_id: Option<domain::DocumentId>,
    pub depth: u8,
    pub sibling_order: u32,
    pub audio_asset_id: Option<domain::DocumentId>,
}

impl From<&domain::DocumentState> for DocumentMetadata {
    fn from(document: &domain::DocumentState) -> Self {
        Self {
            document_id: document.identity.id.clone(),
            revision: document.revision(),
            title: document.content.title.clone(),
            tags: document.content.tags.clone(),
            summary: document.content.summary.clone(),
            document_status: document.document_status.value.clone(),
            parent_id: document.hierarchy.parent_id.clone(),
            depth: document.hierarchy.depth,
            sibling_order: document.hierarchy.sibling_order,
            audio_asset_id: document.source.audio_asset_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentChange {
    Changed,
}

pub trait DocumentRepository: Send + Sync {
    fn snapshot(&self) -> Result<DocumentSnapshot, String>;

    /// Visit metadata one row at a time. Implementations should avoid loading
    /// complete document states; the default preserves compatibility for small
    /// or legacy repositories that have no specialized scanner yet.
    fn scan_metadata(
        &self,
        visitor: &mut dyn FnMut(DocumentMetadata) -> Result<bool, String>,
    ) -> Result<(), String> {
        for document in self.snapshot()?.documents {
            if !visitor(DocumentMetadata::from(&document))? {
                break;
            }
        }
        Ok(())
    }

    fn get(
        &self,
        document_id: &domain::DocumentId,
    ) -> Result<Option<domain::DocumentState>, String>;
    fn read(&self, document_id: &domain::DocumentId) -> Result<Option<DocumentRecord>, String>;
    fn watch(&self) -> Result<Receiver<DocumentChange>, String>;
    fn save(&self, document: domain::DocumentState, markdown: Option<String>)
        -> Result<(), String>;

    fn commit_agent_mutation(
        &self,
        _batch: DocumentMutationBatch,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        Err(DocumentMutationStoreError::Transaction(
            "repository does not support agent mutation transactions".into(),
        ))
    }

    fn find_agent_mutation(
        &self,
        _mutation_id: &str,
        _fingerprint: &str,
    ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
        Ok(None)
    }

    fn agent_mutation_document_ids(
        &self,
        _undo_token: &str,
    ) -> Result<Vec<String>, DocumentMutationStoreError> {
        Err(DocumentMutationStoreError::UndoNotFound)
    }

    fn undo_agent_mutation(
        &self,
        _undo_token: &str,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        Err(DocumentMutationStoreError::UndoNotFound)
    }

    fn save_pending_agent_mutation(
        &self,
        _pending: PendingDocumentMutation,
    ) -> Result<(), DocumentMutationStoreError> {
        Err(DocumentMutationStoreError::Transaction(
            "repository does not support pending agent mutations".into(),
        ))
    }

    fn take_pending_agent_mutation(
        &self,
        _confirmation_id: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        Err(DocumentMutationStoreError::Transaction(
            "repository does not support pending agent mutations".into(),
        ))
    }

    fn find_pending_agent_mutation(
        &self,
        _mutation_id: &str,
        _fingerprint: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationWrite {
    pub record: DocumentRecord,
    pub expected_revision: Option<u64>,
    /// SHA-256 of the source Markdown read before preparing this write.
    /// Repositories compare this value while holding their commit lock.
    #[serde(default)]
    pub source_markdown_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationBatch {
    pub mutation_id: String,
    pub fingerprint: String,
    pub undo_token: String,
    pub tool_name: String,
    pub provenance: AgentMutationProvenance,
    pub writes: Vec<DocumentMutationWrite>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDocumentMutation {
    pub confirmation_id: String,
    pub expires_at_ms: u64,
    pub batch: DocumentMutationBatch,
    pub uncovered_segment_ids: Vec<String>,
    #[serde(default)]
    pub ranges: Vec<contracts::AgentRangePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationRevision {
    pub document_id: String,
    pub previous_revision: Option<u64>,
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationCommit {
    pub mutation_id: String,
    pub undo_token: String,
    pub records: Vec<DocumentRecord>,
    pub revisions: Vec<DocumentMutationRevision>,
    pub provenance: AgentMutationProvenance,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentMutationStoreError {
    #[error("document was not found: {document_id}")]
    NotFound { document_id: String },
    #[error("document already exists: {document_id}")]
    AlreadyExists { document_id: String },
    #[error("document revision conflict for {document_id}: expected {expected}, found {actual}")]
    RevisionConflict {
        document_id: String,
        expected: u64,
        actual: u64,
    },
    #[error("document content conflict for {document_id}: expected {expected}, found {actual}")]
    ContentConflict {
        document_id: String,
        expected: String,
        actual: String,
    },
    #[error("idempotency key was reused with different arguments")]
    IdempotencyConflict,
    #[error("undo token was not found")]
    UndoNotFound,
    #[error("document transaction failed: {0}")]
    Transaction(String),
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

    pub fn scan_metadata(
        &self,
        visitor: &mut dyn FnMut(DocumentMetadata) -> Result<bool, String>,
    ) -> Result<(), String> {
        self.repository.scan_metadata(visitor)
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

    pub fn commit_agent_mutation(
        &self,
        batch: DocumentMutationBatch,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        if let Some(commit) = self
            .repository
            .find_agent_mutation(&batch.mutation_id, &batch.fingerprint)?
        {
            return Ok(commit);
        }
        self.validate_agent_mutation_sources(&batch)?;
        self.repository.commit_agent_mutation(batch)
    }

    pub fn validate_agent_mutation_sources(
        &self,
        batch: &DocumentMutationBatch,
    ) -> Result<(), DocumentMutationStoreError> {
        for write in &batch.writes {
            let Some(expected) = write.source_markdown_sha256.as_deref() else {
                continue;
            };
            let document_id = write.record.state.id();
            let current = self
                .repository
                .read(document_id)
                .map_err(DocumentMutationStoreError::Transaction)?
                .ok_or_else(|| DocumentMutationStoreError::NotFound {
                    document_id: document_id.as_str().into(),
                })?;
            let actual = markdown_sha256(&current.markdown);
            if actual != expected {
                return Err(DocumentMutationStoreError::ContentConflict {
                    document_id: document_id.as_str().into(),
                    expected: expected.into(),
                    actual,
                });
            }
        }
        Ok(())
    }

    pub fn find_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
        self.repository
            .find_agent_mutation(mutation_id, fingerprint)
    }

    pub fn undo_agent_mutation(
        &self,
        undo_token: &str,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        self.repository.undo_agent_mutation(undo_token)
    }

    pub fn agent_mutation_document_ids(
        &self,
        undo_token: &str,
    ) -> Result<Vec<String>, DocumentMutationStoreError> {
        self.repository.agent_mutation_document_ids(undo_token)
    }

    pub fn save_pending_agent_mutation(
        &self,
        pending: PendingDocumentMutation,
    ) -> Result<(), DocumentMutationStoreError> {
        self.repository.save_pending_agent_mutation(pending)
    }

    pub fn take_pending_agent_mutation(
        &self,
        confirmation_id: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        self.repository.take_pending_agent_mutation(confirmation_id)
    }

    pub fn find_pending_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        self.repository
            .find_pending_agent_mutation(mutation_id, fingerprint)
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

fn markdown_sha256(markdown: &str) -> String {
    Sha256::digest(markdown.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    pub windowing: Option<AudioWindowing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioWindowing {
    pub sample_rate_hz: u32,
    pub core_samples: u64,
    pub context_samples: u64,
    pub window_count: u64,
    pub source_samples: u64,
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
        windowing: None,
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
