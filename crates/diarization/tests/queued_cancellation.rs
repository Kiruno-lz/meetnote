use application::DiarizationBackend;
use contracts::{BackendErrorCode, ComputeDevice, DiarizationRequest, ModelRef};
use diarization_native::NativeDiarizationBackend;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use runtime::JobRuntime;

#[test]
fn queued_native_cancellation_wakes_before_admission() {
    check_queued_cancellation(
        Box::new(NativeDiarizationBackend::default()),
        "diarization-native",
        "speakrs-community-1-cpu",
    );
}

#[test]
fn queued_sherpa_cancellation_wakes_before_admission() {
    check_queued_cancellation(
        Box::new(SherpaOnnxDiarizationBackend::default()),
        "sherpa-onnx",
        "sherpa-onnx-speaker-diarization-bundle",
    );
}

fn check_queued_cancellation(
    backend: Box<dyn DiarizationBackend>,
    backend_id: &str,
    model_id: &str,
) {
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
    let mut work = backend.diarize(
        DiarizationRequest {
            audio_path: "/missing/queued-cancellation.wav".into(),
            model: ModelRef {
                backend_id: backend_id.into(),
                model_id: model_id.into(),
                revision: None,
                artifact_path: "/missing/queued-cancellation-model".into(),
                artifacts: vec![],
            },
            expected_speakers: None,
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            stage_configuration: Default::default(),
        },
        context.clone(),
    );
    let (polled_tx, polled_rx) = std::sync::mpsc::channel();
    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let driver = std::thread::spawn(move || {
        let mut first = true;
        let result = futures::executor::block_on(futures::future::poll_fn(move |cx| {
            let result = work.as_mut().poll(cx);
            if first {
                first = false;
                let _ = polled_tx.send(result.is_pending());
            }
            result
        }));
        let _ = completed_tx.send(result);
    });
    let queued = polled_rx.recv_timeout(std::time::Duration::from_secs(2));
    context.cancel();
    let early = completed_rx.recv_timeout(std::time::Duration::from_secs(2));
    let returned_before_release = early.is_ok();
    // Release and join both real drivers before asserting, including against the old backend.
    drop(release_tx);
    blocking.join().unwrap();
    let result = early.unwrap_or_else(|_| {
        completed_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
    });
    driver.join().unwrap();
    assert!(queued.unwrap(), "backend never entered its admission queue");
    assert!(
        returned_before_release,
        "cancelled {backend_id} waited for another recording to finish"
    );
    assert_eq!(result.unwrap_err().code, BackendErrorCode::Cancelled);
}
