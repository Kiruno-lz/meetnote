use contracts::{
    AsrRequest, AsrResult, BackendError, BackendTask, DiarizationRequest, DiarizationResult,
};
use domain::{align_transcript_and_speakers, SpeakerTaggedSegment};
use runtime::JobContext;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

mod speaker_identity;
mod workspace;

pub use contracts::BackendDescriptor;
pub use speaker_identity::{
    apply_identity_matches, cosine_similarity, ClusterIdentityResolver, IdentityResolverConfig,
    InMemorySpeakerProfileStore, JsonSpeakerProfileStore, SpeakerLibraryError,
    SpeakerLibraryService, SpeakerProfileStore,
};
pub use workspace::{
    render_markdown, ProcessingContexts, ProcessingRequest, ProcessingService,
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

pub trait ProjectRepository: Send + Sync {
    fn save(&self, project: domain::MeetingProject) -> Result<(), String>;
    fn get(&self, project_id: &domain::ProjectId)
        -> Result<Option<domain::MeetingProject>, String>;
    fn list(&self) -> Result<Vec<domain::MeetingProject>, String>;
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
    }
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
