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
    let decoded = audio::read_audio_mono_16k(&audio_path).expect("decode reference audio");
    let audio_end_ms = decoded.samples.len() as u64 * 1_000 / 16_000;
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
        .all(|segment| segment.end_ms > segment.start_ms && segment.end_ms <= audio_end_ms));
    assert!(output
        .observations
        .iter()
        .all(|observation| observation.vector.len() == 256
            && observation.end_ms > observation.start_ms
            && observation.end_ms <= audio_end_ms));
}

#[test]
#[ignore = "requires explicit MEETNOTE_DIARIZATION_MODEL_DIR and MEETNOTE_DIARIZATION_SILENCE_AUDIO"]
fn real_silent_model_returns_no_speakers_without_panicking() {
    let model_dir = std::env::var("MEETNOTE_DIARIZATION_MODEL_DIR").expect("explicit model path");
    let audio_path =
        std::env::var("MEETNOTE_DIARIZATION_SILENCE_AUDIO").expect("explicit silence path");
    let decoded = audio::read_audio_mono_16k(&audio_path).unwrap();
    assert!(!decoded.samples.is_empty());
    assert!(decoded.samples.iter().all(|sample| *sample == 0.0));
    let backend = NativeDiarizationBackend::default();
    let (context, _) = JobRuntime::default().create_job();
    let request = DiarizationRequest {
        audio_path,
        model: ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "speakrs-community-1-cpu".into(),
            revision: Some(diarization_native::SPEAKRS_MODEL_REVISION.into()),
            artifact_path: model_dir,
            artifacts: Vec::new(),
        },
        expected_speakers: None,
        device: ComputeDevice::Cpu,
        speaker_library_path: None,
        stage_configuration: Default::default(),
    };
    let result = futures::executor::block_on(backend.analyze(request.clone(), context.clone()))
        .expect("a completed no-speaker inference must succeed");
    assert!(result.result.segments.is_empty());
    assert!(result.observations.is_empty());
    assert_eq!(result.result.speaker_count, Some(0));
    let mut session = backend.create_window_session(request).unwrap();
    let end = decoded.samples.len() as u64;
    session
        .process_window(
            std::sync::Arc::new(audio::AudioWindow {
                sequence: 0,
                sample_rate: 16_000,
                samples: decoded.samples.into(),
                global_start_sample: 0,
                global_end_sample: end,
                valid_core_start_sample: 0,
                valid_core_end_sample: end,
                source_total_samples: Some(end),
                is_final: true,
            }),
            &context,
        )
        .unwrap();
    let windowed = session
        .finish()
        .expect("silent windows must preserve the no-speaker state");
    assert!(windowed.segments.is_empty());
    assert!(windowed.embedding_observations.is_empty());
    assert_eq!(windowed.speaker_count, Some(0));
}
