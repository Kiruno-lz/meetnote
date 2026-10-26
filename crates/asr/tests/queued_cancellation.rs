use application::AsrBackend;
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AsrRequest, BackendErrorCode, ComputeDevice, ModelRef};
use futures::FutureExt;
use runtime::JobRuntime;

#[test]
fn queued_whisper_cancellation_finishes_before_admission() {
    check_queued_cancellation(
        Box::new(WhisperRsBackend::default()),
        "whisper-rs",
        "whisper-base-en-ggml",
    );
}

#[test]
fn queued_sherpa_cancellation_finishes_before_admission() {
    check_queued_cancellation(
        Box::new(SherpaOnnxAsrBackend::default()),
        "sherpa-onnx",
        "paraformer-zh",
    );
}

fn check_queued_cancellation(backend: Box<dyn AsrBackend>, backend_id: &str, model_id: &str) {
    let runtime = JobRuntime::with_max_concurrency(1);
    let (blocker, _) = runtime.create_job();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let blocking = std::thread::spawn(move || {
        futures::executor::block_on(blocker.spawn_blocking(move || {
            started_tx.send(()).unwrap();
            let _ = release_rx.recv();
        }));
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let (context, _) = runtime.create_job();
    let mut work = backend.transcribe(
        AsrRequest {
            audio_path: "/missing/queued-cancellation.wav".into(),
            model: ModelRef {
                backend_id: backend_id.into(),
                model_id: model_id.into(),
                revision: None,
                artifact_path: "/missing/queued-cancellation-model".into(),
                artifacts: vec![],
            },
            language: Some("en".into()),
            device: ComputeDevice::Cpu,
        },
        context.clone(),
    );
    assert!(work.as_mut().now_or_never().is_none());
    context.cancel();
    let early = work.as_mut().now_or_never();
    let returned_before_release = early.is_some();
    // Always release and join before asserting, including against the old implementation.
    drop(release_tx);
    blocking.join().unwrap();
    let result = early.unwrap_or_else(|| futures::executor::block_on(work));
    assert!(
        returned_before_release,
        "cancelled {backend_id} waited for another recording to finish"
    );
    assert_eq!(result.unwrap_err().code, BackendErrorCode::Cancelled);
}
