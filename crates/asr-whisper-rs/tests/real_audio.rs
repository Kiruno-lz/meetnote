use application::AsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AsrRequest, ComputeDevice, ModelRef};
use runtime::JobRuntime;
use std::path::Path;
use std::sync::Arc;

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

fn check_digital_silence(windowed: bool) {
    let model_path =
        std::env::var("MEETNOTE_WHISPER_MODEL").expect("MEETNOTE_WHISPER_MODEL is required");
    let audio_path = std::env::var("MEETNOTE_WHISPER_SILENCE_AUDIO")
        .expect("MEETNOTE_WHISPER_SILENCE_AUDIO is required");
    let source = audio::read_audio_mono_16k(&audio_path).expect("explicit source must decode");
    assert!(!source.samples.is_empty());
    assert!(source.samples.iter().all(|sample| *sample == 0.0));
    let request = AsrRequest {
        audio_path,
        model: ModelRef {
            backend_id: "whisper-rs".into(),
            model_id: "whisper-real-test".into(),
            revision: None,
            artifact_path: model_path,
            artifacts: Vec::new(),
        },
        language: Some("en".into()),
        device: ComputeDevice::Cpu,
    };
    let backend = WhisperRsBackend::default();
    let (context, _) = JobRuntime::default().create_job();
    let result = if windowed {
        let mut session = backend.create_window_session(request).unwrap();
        let end = source.samples.len() as u64;
        let window = Arc::new(audio::AudioWindow {
            sequence: 0,
            sample_rate: source.sample_rate,
            samples: source.samples.into(),
            global_start_sample: 0,
            global_end_sample: end,
            valid_core_start_sample: 0,
            valid_core_end_sample: end,
            source_total_samples: Some(end),
            is_final: true,
        });
        let (cancelled, _) = JobRuntime::default().create_job();
        cancelled.cancel();
        assert_eq!(
            session
                .process_window(window.clone(), &cancelled)
                .unwrap_err()
                .code,
            contracts::BackendErrorCode::Cancelled
        );
        session.process_window(window, &context).unwrap();
        session.finish().unwrap()
    } else {
        futures::executor::block_on(backend.transcribe(request, context)).unwrap()
    };
    assert!(
        result.segments.is_empty(),
        "digital silence must not invent text (windowed={windowed}): {:?}",
        result.segments
    );
}

#[test]
#[ignore = "requires MEETNOTE_WHISPER_MODEL and MEETNOTE_WHISPER_SILENCE_AUDIO"]
fn whole_file_digital_silence_has_no_invented_text() {
    check_digital_silence(false);
}

#[test]
#[ignore = "requires MEETNOTE_WHISPER_MODEL and MEETNOTE_WHISPER_SILENCE_AUDIO"]
fn windowed_digital_silence_has_no_invented_text() {
    check_digital_silence(true);
}
