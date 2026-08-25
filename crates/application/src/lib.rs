use contracts::{
    AsrRequest, AsrResult, BackendError, BackendTask, DiarizationRequest, DiarizationResult,
};
use domain::{align_transcript_and_speakers, SpeakerTaggedSegment};
use runtime::JobContext;
use std::future::Future;
use std::pin::Pin;

mod workspace;

pub use contracts::BackendDescriptor;
pub use workspace::{render_markdown, ProcessingRequest, ProcessingService, WorkspaceService};

pub type BackendFuture<T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send>>;

pub trait AsrBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &contracts::ModelRef) -> Result<(), BackendError>;
    fn transcribe(&self, request: AsrRequest, context: JobContext) -> BackendFuture<AsrResult>;
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
    let asr_future = asr_backend.transcribe(asr_request, asr_context);
    let diarization_future = diarization_backend.diarize(diarization_request, diarization_context);
    let (asr, diarization) = futures::join!(asr_future, diarization_future);
    let asr = asr?;
    let diarization = diarization?;
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
                        start_ms: 0,
                        end_ms: 1_000,
                        confidence: Some(1.0),
                    }],
                    speaker_count: Some(1),
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
            },
            asr_context,
            diarization_context,
        ))
        .unwrap();

        assert_eq!(result.aligned[0].speaker_id, "speaker-1");
    }
}
