use application::DiarizationBackend;
use contracts::{ComputeDevice, DiarizationRequest, ModelRef};
use diarization_native::NativeDiarizationBackend;
use runtime::JobRuntime;

#[test]
#[ignore = "requires MEETNOTE_DIARIZATION_MODEL_DIR and MEETNOTE_DIARIZATION_AUDIO"]
fn real_model_produces_valid_segments_and_observations() {
    let model_dir = std::env::var("MEETNOTE_DIARIZATION_MODEL_DIR")
        .expect("MEETNOTE_DIARIZATION_MODEL_DIR is required");
    let audio_path = std::env::var("MEETNOTE_DIARIZATION_AUDIO")
        .expect("MEETNOTE_DIARIZATION_AUDIO is required");
    let backend = NativeDiarizationBackend::default();
    let model = ModelRef {
        backend_id: "diarization-native".into(),
        model_id: "speakrs-community-1-cpu".into(),
        revision: Some("5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f".into()),
        artifact_path: model_dir,
    };
    backend.probe(&model).expect("model probe should succeed");
    let (context, _) = JobRuntime::default().create_job();

    let output = futures::executor::block_on(backend.analyze(
        DiarizationRequest {
            audio_path,
            model,
            expected_speakers: Some(2),
            device: ComputeDevice::Cpu,
        },
        context,
    ))
    .expect("real diarization should succeed");

    eprintln!(
        "native acceptance: segments={}, speakers={:?}, observations={}, matches={}",
        output.result.segments.len(),
        output.result.speaker_count,
        output.observations.len(),
        output.identity_matches.len()
    );

    assert!(!output.result.segments.is_empty());
    assert!(!output.observations.is_empty());
    assert!(output
        .result
        .segments
        .iter()
        .all(|segment| segment.end_ms > segment.start_ms));
    assert!(output
        .observations
        .iter()
        .all(|observation| observation.vector.len() == 256));
}
