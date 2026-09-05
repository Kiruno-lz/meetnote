use application::{asr_descriptor, run_pipeline, AsrBackend, BackendFuture, DiarizationBackend};
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, ComputeDevice, DiarizationRequest,
    ModelRef,
};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use runtime::{JobContext, JobRuntime};
use std::sync::Arc;

struct EmptyAsr;

impl AsrBackend for EmptyAsr {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor("test-asr", &["fixture"])
    }

    fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
        Ok(())
    }

    fn transcribe(&self, request: AsrRequest, _: JobContext) -> BackendFuture<AsrResult> {
        Box::pin(async move {
            Ok(AsrResult {
                backend: asr_descriptor("test-asr", &["fixture"]),
                model: request.model,
                language: None,
                segments: Vec::new(),
            })
        })
    }
}

#[test]
#[ignore = "requires MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR and MEETNOTE_DIARIZATION_AUDIO"]
fn sherpa_embeddings_are_matched_by_the_shared_identity_service() {
    let model_dir = std::env::var("MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR")
        .expect("MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR is required");
    let audio_path = std::env::var("MEETNOTE_DIARIZATION_AUDIO")
        .expect("MEETNOTE_DIARIZATION_AUDIO is required");
    let backend = SherpaOnnxDiarizationBackend::default();
    let model = ModelRef {
        backend_id: "sherpa-onnx".into(),
        model_id: "sherpa-onnx-speaker-diarization-bundle".into(),
        revision: None,
        artifact_path: model_dir,
        artifacts: Vec::new(),
    };
    let runtime = JobRuntime::default();
    let (probe_context, _) = runtime.create_job();
    let baseline = futures::executor::block_on(backend.diarize(
        DiarizationRequest {
            audio_path: audio_path.clone(),
            model: model.clone(),
            expected_speakers: Some(2),
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            stage_configuration: Default::default(),
        },
        probe_context,
    ))
    .expect("sherpa-onnx diarization should succeed");
    assert!(!baseline.embedding_observations.is_empty());

    let library_path = std::env::temp_dir().join(format!(
        "meetnote-sherpa-identity-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let library = application::SpeakerLibraryService::new(Arc::new(
        application::JsonSpeakerProfileStore::new(&library_path),
    ));
    library
        .enroll(
            "Real audio speaker",
            &baseline.embedding_observations[0],
            contracts::SpeakerSampleSource {
                project_id: Some("real-audio-test".into()),
                source_segment_id: Some(baseline.embedding_observations[0].id.clone()),
                start_ms: baseline.embedding_observations[0].start_ms,
                end_ms: baseline.embedding_observations[0].end_ms,
                audio_path: Some(audio_path.clone()),
                audio_sha256: None,
                backup_path: None,
            },
            0.7,
        )
        .expect("speaker enrollment should succeed");

    let (asr_context, _) = runtime.create_job();
    let (diarization_context, _) = runtime.create_job();
    let result = futures::executor::block_on(run_pipeline(
        &EmptyAsr,
        &backend,
        AsrRequest {
            audio_path: audio_path.clone(),
            model: ModelRef {
                backend_id: "test-asr".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "fixture".into(),
                artifacts: Vec::new(),
            },
            language: None,
            device: ComputeDevice::Cpu,
        },
        DiarizationRequest {
            audio_path,
            model,
            expected_speakers: Some(2),
            device: ComputeDevice::Cpu,
            speaker_library_path: Some(library_path.display().to_string()),
            stage_configuration: Default::default(),
        },
        asr_context,
        diarization_context,
    ))
    .expect("shared identity pipeline should succeed");

    assert!(result
        .diarization
        .segments
        .iter()
        .any(|segment| segment.speaker_label.as_deref() == Some("Real audio speaker")));
    let _ = std::fs::remove_file(library_path);
}
