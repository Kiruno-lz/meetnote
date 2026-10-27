use application::DiarizationBackend;
use contracts::{ComputeDevice, DiarizationRequest, ModelRef};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;

#[test]
#[ignore = "requires MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR and MEETNOTE_DIARIZATION_AUDIO"]
fn real_sherpa_timeline_and_embeddings_stay_within_source() {
    let audio_path = std::env::var("MEETNOTE_DIARIZATION_AUDIO")
        .expect("MEETNOTE_DIARIZATION_AUDIO is required");
    let samples = audio::read_audio_mono_16k(&audio_path)
        .expect("explicit audio must decode")
        .samples;
    let audio_end_ms = samples.len() as u64 / 16;
    let runtime = runtime::JobRuntime::default();
    let (context, _) = runtime.create_job();
    let result = futures::executor::block_on(
        SherpaOnnxDiarizationBackend::default().diarize(
            DiarizationRequest {
                audio_path,
                model: ModelRef {
                    backend_id: "sherpa-onnx".into(),
                    model_id: "sherpa-onnx-speaker-diarization-bundle".into(),
                    revision: None,
                    artifact_path: std::env::var("MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR")
                        .expect("MEETNOTE_SHERPA_DIARIZATION_MODEL_DIR is required"),
                    artifacts: Vec::new(),
                },
                expected_speakers: None,
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
                stage_configuration: Default::default(),
            },
            context,
        ),
    )
    .expect("sherpa diarization must succeed");
    assert!(
        !result.segments.is_empty(),
        "use an explicit speech fixture"
    );
    assert!(!result.embedding_observations.is_empty());
    assert_eq!(
        result.speaker_count,
        Some(
            result
                .segments
                .iter()
                .map(|segment| &segment.speaker_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len() as u32
        )
    );
    let invalid = result
        .segments
        .iter()
        .map(|s| ("timeline", s.start_ms, s.end_ms))
        .chain(
            result
                .embedding_observations
                .iter()
                .map(|s| ("embedding", s.start_ms, s.end_ms)),
        )
        .filter(|(_, start, end)| start >= end || *end > audio_end_ms)
        .collect::<Vec<_>>();
    assert!(
        invalid.is_empty(),
        "ranges outside decoded source ending at {audio_end_ms}ms: {invalid:?}"
    );
}
