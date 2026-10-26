use application::AsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AsrRequest, ComputeDevice, ModelRef};
use runtime::JobRuntime;
use std::path::Path;

#[test]
#[ignore = "requires MEETNOTE_WHISPER_MODEL and MEETNOTE_WHISPER_AUDIO"]
fn whisper_transcribes_real_audio_with_timestamps() {
    let model_path =
        std::env::var("MEETNOTE_WHISPER_MODEL").expect("MEETNOTE_WHISPER_MODEL is required");
    let audio_path =
        std::env::var("MEETNOTE_WHISPER_AUDIO").expect("MEETNOTE_WHISPER_AUDIO is required");
    let source = audio::read_audio_mono_16k(&audio_path).expect("source audio should decode");
    let maximum_ms = source.samples.len() as u64 * 1000 / 16000;
    let model = ModelRef {
        backend_id: "whisper-rs".into(),
        model_id: "whisper-real-test".into(),
        revision: None,
        artifact_path: model_path,
        artifacts: Vec::new(),
    };
    let backend = WhisperRsBackend::default();
    backend
        .probe(&model)
        .expect("Whisper model probe should succeed");
    let (context, _) = JobRuntime::default().create_job();

    let result = futures::executor::block_on(backend.transcribe(
        AsrRequest {
            audio_path,
            model,
            language: Some("en".into()),
            device: ComputeDevice::Auto,
        },
        context,
    ))
    .expect("Whisper transcription should succeed");

    assert!(!result.segments.is_empty());
    assert!(result
        .segments
        .iter()
        .all(|segment| segment.end_ms > segment.start_ms && !segment.text.is_empty()));
    assert!(
        result
            .segments
            .iter()
            .all(|segment| segment.end_ms <= maximum_ms),
        "Whisper timestamps must fit {maximum_ms} ms of decoded source audio: {:?}",
        result.segments
    );
    assert!(Path::new(&result.model.artifact_path).is_file());
}
