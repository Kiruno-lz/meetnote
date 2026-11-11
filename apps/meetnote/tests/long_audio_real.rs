use application::{LongAudioConfig, run_windowed_pipeline};
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use audio::{AudioWindowPlan, TARGET_SAMPLE_RATE_HZ};
use contracts::{AsrRequest, ComputeDevice, DiarizationRequest, ModelRef};
use diarization_native::NativeDiarizationBackend;
use runtime::JobRuntime;
use std::collections::BTreeSet;
use std::sync::Arc;

#[test]
#[ignore = "requires long-audio model and audio environment variables"]
fn real_windowed_pipeline_keeps_asr_diarization_and_global_speakers() {
    let audio_path =
        std::env::var("MEETNOTE_LONG_AUDIO_AUDIO").expect("MEETNOTE_LONG_AUDIO_AUDIO is required");
    let asr_model_dir = std::env::var("MEETNOTE_LONG_AUDIO_ASR_MODEL_DIR")
        .expect("MEETNOTE_LONG_AUDIO_ASR_MODEL_DIR is required");
    let diarization_model_dir = std::env::var("MEETNOTE_LONG_AUDIO_DIARIZATION_MODEL_DIR")
        .expect("MEETNOTE_LONG_AUDIO_DIARIZATION_MODEL_DIR is required");
    let asr_model = ModelRef {
        backend_id: "sherpa-onnx".into(),
        model_id: "paraformer-zh".into(),
        revision: None,
        artifact_path: asr_model_dir,
        artifacts: Vec::new(),
    };
    let diarization_model = ModelRef {
        backend_id: "diarization-native".into(),
        model_id: "speakrs-community-1-cpu".into(),
        revision: Some("5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f".into()),
        artifact_path: diarization_model_dir,
        artifacts: Vec::new(),
    };
    let runtime = JobRuntime::default();
    let (asr_context, _) = runtime.create_job();
    let (diarization_context, _) = runtime.create_job();
    let second = u64::from(TARGET_SAMPLE_RATE_HZ);

    let result = futures::executor::block_on(run_windowed_pipeline(
        Arc::new(SherpaOnnxAsrBackend::default()),
        Arc::new(NativeDiarizationBackend::default()),
        AsrRequest {
            audio_path: audio_path.clone(),
            model: asr_model,
            language: Some("zh".into()),
            device: ComputeDevice::Cpu,
        },
        DiarizationRequest {
            audio_path,
            model: diarization_model,
            expected_speakers: None,
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            stage_configuration: Default::default(),
        },
        asr_context,
        diarization_context,
        LongAudioConfig {
            activation_samples: 0,
            decode_chunk_samples: TARGET_SAMPLE_RATE_HZ as usize,
            window_plan: AudioWindowPlan::new(second * 60, second * 15).unwrap(),
        },
    ))
    .expect("real bounded-window pipeline should succeed");

    assert!(!result.asr.segments.is_empty());
    assert!(!result.diarization.segments.is_empty());
    assert!(!result.diarization.embedding_observations.is_empty());
    assert!(!result.aligned.is_empty());
    let windowing = result
        .windowing
        .expect("window provenance should be present");
    assert_eq!(windowing.window_count, 4);
    assert!(windowing.source_samples > second * 180);
    assert!(
        result
            .asr
            .segments
            .windows(2)
            .all(|pair| pair[0].start_ms <= pair[1].start_ms)
    );
    assert!(
        result
            .diarization
            .segments
            .windows(2)
            .all(|pair| pair[0].start_ms <= pair[1].start_ms)
    );
    let clusters = result
        .diarization
        .embedding_observations
        .iter()
        .map(|observation| observation.cluster_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        result.diarization.speaker_count,
        Some(clusters.len() as u32)
    );
    eprintln!(
        "windowed acceptance: transcript_segments={}, transcript_chars={}, diarization_segments={}, observations={}, speakers={}",
        result.asr.segments.len(),
        result
            .asr
            .segments
            .iter()
            .map(|segment| segment.text.chars().count())
            .sum::<usize>(),
        result.diarization.segments.len(),
        result.diarization.embedding_observations.len(),
        clusters.len()
    );
}
