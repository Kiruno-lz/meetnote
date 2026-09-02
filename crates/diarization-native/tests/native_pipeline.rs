use application::DiarizationBackend;
use contracts::{
    AlgorithmRef, ComputeDevice, DiarizationRequest, DiarizationStageConfiguration, ModelRef,
};
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
        artifact_path: model_dir.clone(),
        artifacts: Vec::new(),
    };
    let stage_configuration = DiarizationStageConfiguration {
        segmentation_model: Some(ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "pyannote-segmentation-3.0".into(),
            revision: Some("5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f".into()),
            artifact_path: std::path::Path::new(&model_dir)
                .join("segmentation-3.0.onnx")
                .display()
                .to_string(),
            artifacts: Vec::new(),
        }),
        embedding_model: Some(ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "wespeaker-voxceleb-resnet34".into(),
            revision: Some("5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f".into()),
            artifact_path: std::path::Path::new(&model_dir)
                .join("wespeaker-voxceleb-resnet34.onnx")
                .display()
                .to_string(),
            artifacts: Vec::new(),
        }),
        clustering_algorithm: Some(AlgorithmRef {
            id: "speakrs-ahc-plda-vbx".into(),
            revision: Some("speakrs-community-1-cpu".into()),
            parameters: Some("community-1-default".into()),
        }),
    };
    backend.probe(&model).expect("model probe should succeed");
    let (context, _) = JobRuntime::default().create_job();

    let output = futures::executor::block_on(backend.analyze(
        DiarizationRequest {
            audio_path,
            model,
            expected_speakers: Some(2),
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            stage_configuration,
        },
        context,
    ))
    .expect("real diarization should succeed");

    eprintln!(
        "native acceptance: segments={}, speakers={:?}, observations={}, matches={}",
        output.result.segments.len(),
        output.result.speaker_count,
        output.observations.len(),
        output.anonymous_clusters.len()
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
