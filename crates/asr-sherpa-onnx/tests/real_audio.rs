use application::{AsrBackend, LongAudioConfig};
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use audio::{AudioChunkReader, ProcessingWindowPlanner};
use contracts::{AsrRequest, ComputeDevice, ModelRef};
use runtime::JobRuntime;
use std::path::Path;
use std::sync::Arc;

#[test]
#[ignore = "requires MEETNOTE_SHERPA_ONNX_MODEL_DIR and MEETNOTE_SHERPA_ONNX_AUDIO"]
fn paraformer_transcribes_real_audio() {
    let model_dir = std::env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR")
        .expect("MEETNOTE_SHERPA_ONNX_MODEL_DIR is required");
    let audio_path = std::env::var("MEETNOTE_SHERPA_ONNX_AUDIO")
        .expect("MEETNOTE_SHERPA_ONNX_AUDIO is required");
    let model = ModelRef {
        backend_id: "sherpa-onnx".into(),
        model_id: "paraformer-zh".into(),
        revision: None,
        artifact_path: model_dir,
        artifacts: Vec::new(),
    };
    let backend = SherpaOnnxAsrBackend::default();
    backend
        .probe(&model)
        .expect("sherpa-onnx ASR model probe should succeed");
    let (context, _) = JobRuntime::default().create_job();

    let result = futures::executor::block_on(backend.transcribe(
        AsrRequest {
            audio_path,
            model,
            language: Some("zh".into()),
            device: ComputeDevice::Cpu,
        },
        context,
    ))
    .expect("sherpa-onnx ASR transcription should succeed");

    eprintln!(
        "paraformer acceptance: segments={}, chars={}",
        result.segments.len(),
        result
            .segments
            .iter()
            .map(|segment| segment.text.chars().count())
            .sum::<usize>()
    );

    assert!(!result.segments.is_empty());
    assert!(!result.segments[0].text.is_empty());
    assert!(Path::new(&result.model.artifact_path).is_dir());
}

#[test]
#[ignore = "requires MEETNOTE_SHERPA_ONNX_MODEL_DIR and MEETNOTE_SHERPA_ONNX_AUDIO"]
fn paraformer_transcribes_real_audio_through_bounded_windows() {
    let model_dir = std::env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR")
        .expect("MEETNOTE_SHERPA_ONNX_MODEL_DIR is required");
    let audio_path = std::env::var("MEETNOTE_SHERPA_ONNX_AUDIO")
        .expect("MEETNOTE_SHERPA_ONNX_AUDIO is required");
    let request = AsrRequest {
        audio_path: audio_path.clone(),
        model: ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "paraformer-zh".into(),
            revision: None,
            artifact_path: model_dir,
            artifacts: Vec::new(),
        },
        language: Some("zh".into()),
        device: ComputeDevice::Cpu,
    };
    let backend = SherpaOnnxAsrBackend::default();
    let mut session = backend.create_window_session(request).unwrap();
    let config = LongAudioConfig::default();
    let mut reader = AudioChunkReader::open(audio_path, config.decode_chunk_samples).unwrap();
    let mut planner = ProcessingWindowPlanner::new(config.window_plan);
    let (context, _) = JobRuntime::default().create_job();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        for window in planner.push(chunk).unwrap() {
            session.process_window(Arc::new(window), &context).unwrap();
        }
    }
    planner.finish().unwrap();
    let result = session.finish().unwrap();
    eprintln!(
        "windowed paraformer acceptance: segments={}, chars={}",
        result.segments.len(),
        result
            .segments
            .iter()
            .map(|segment| segment.text.chars().count())
            .sum::<usize>()
    );
    assert!(!result.segments.is_empty());
    assert!(result
        .segments
        .windows(2)
        .all(|pair| pair[0].end_ms <= pair[1].start_ms));
}
