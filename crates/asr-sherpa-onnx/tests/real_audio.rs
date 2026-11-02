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

#[test]
#[ignore = "requires MEETNOTE_SHERPA_ONNX_MODEL_DIR and MEETNOTE_SHERPA_ONNX_SILENCE_AUDIO"]
fn paraformer_rejects_silence_without_hallucinated_transcripts() {
    let model_dir = std::env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR").unwrap();
    let audio_path = std::env::var("MEETNOTE_SHERPA_ONNX_SILENCE_AUDIO").unwrap();
    let backend = SherpaOnnxAsrBackend::default();
    let request = AsrRequest {
        audio_path: audio_path.clone(),
        model: ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "paraformer-zh".into(),
            revision: Some("2024-03-09".into()),
            artifact_path: model_dir,
            artifacts: Vec::new(),
        },
        language: Some("zh".into()),
        device: ComputeDevice::Cpu,
    };
    let audio = audio::read_audio_mono_16k(&audio_path).unwrap();
    assert!(audio.samples.len() > 30 * 16000);
    assert!(audio.samples.iter().all(|sample| *sample == 0.));
    let (context, _) = JobRuntime::default().create_job();
    let error =
        futures::executor::block_on(backend.transcribe(request.clone(), context)).unwrap_err();
    assert_eq!(
        error.code,
        contracts::BackendErrorCode::OutputValidationFailed
    );
    let mut session = backend.create_window_session(request).unwrap();
    let total = audio.samples.len() as u64;
    let (context, _) = JobRuntime::default().create_job();
    session
        .process_window(
            Arc::new(audio::AudioWindow {
                sequence: 0,
                sample_rate: 16000,
                samples: audio.samples.into(),
                global_start_sample: 0,
                global_end_sample: total,
                valid_core_start_sample: 0,
                valid_core_end_sample: total,
                source_total_samples: Some(total),
                is_final: true,
            }),
            &context,
        )
        .unwrap();
    assert_eq!(
        session.finish().unwrap_err().code,
        contracts::BackendErrorCode::OutputValidationFailed
    );
}

#[test]
#[ignore = "requires MEETNOTE_QWEN3_ONNX_MODEL_DIR and MEETNOTE_QWEN3_ONNX_AUDIO (short mono 16k WAV)"]
fn qwen_official_bundle_transcribes_without_a_tokens_file() {
    let root = std::env::var("MEETNOTE_QWEN3_ONNX_MODEL_DIR").unwrap();
    let audio_path = std::env::var("MEETNOTE_QWEN3_ONNX_AUDIO").unwrap();
    assert!(!Path::new(&root).join("tokens.txt").exists());
    let audio = audio::read_audio_mono_16k(&audio_path).unwrap();
    assert!(!audio.samples.is_empty() && audio.samples.len() <= 30 * 16000);
    let request = AsrRequest {
        audio_path,
        model: ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "qwen3-asr-0.6b".into(),
            revision: Some("2026-03-25".into()),
            artifact_path: root,
            artifacts: Vec::new(),
        },
        language: std::env::var("MEETNOTE_QWEN3_ONNX_LANGUAGE").ok(),
        device: ComputeDevice::Cpu,
    };
    let backend = SherpaOnnxAsrBackend::default();
    let (context, _) = JobRuntime::default().create_job();
    let whole = futures::executor::block_on(backend.transcribe(request.clone(), context)).unwrap();
    let mut session = backend.create_window_session(request).unwrap();
    let total = audio.samples.len() as u64;
    let (context, _) = JobRuntime::default().create_job();
    session
        .process_window(
            Arc::new(audio::AudioWindow {
                sequence: 0,
                sample_rate: 16000,
                samples: audio.samples.into(),
                global_start_sample: 0,
                global_end_sample: total,
                valid_core_start_sample: 0,
                valid_core_end_sample: total,
                source_total_samples: Some(total),
                is_final: true,
            }),
            &context,
        )
        .unwrap();
    let window = session.finish().unwrap();
    let text = |result: &contracts::AsrResult| {
        result
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>()
    };
    assert!(!text(&whole).trim().is_empty());
    assert_eq!(text(&whole), text(&window));
    for segment in whole.segments.iter().chain(&window.segments) {
        assert!(segment.start_ms < segment.end_ms && segment.end_ms <= total * 1000 / 16000);
    }
}
