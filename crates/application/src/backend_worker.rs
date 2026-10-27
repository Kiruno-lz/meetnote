//! Audio backend admission and error normalization; native work stays in its backend.
use crate::BackendFuture;
use contracts::{BackendError, BackendErrorCode};
use runtime::JobContext;

/// Wait for the owning runtime before starting native work. Cancellation while
/// queued never loads a model; once running, the real worker keeps its permit
/// until cooperative work has exited, even if the awaiting future is dropped.
pub fn run_backend_worker<T, F>(context: JobContext, work: F) -> BackendFuture<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, BackendError> + Send + 'static,
{
    Box::pin(async move {
        match context.spawn_blocking_cancellable(work).await {
            Ok(result) => result,
            Err(_) if context.is_cancelled() => Err(BackendError {
                code: BackendErrorCode::Cancelled,
                message: "audio backend work was cancelled".into(),
            }),
            Err(_) => Err(BackendError {
                code: BackendErrorCode::InferenceFailed,
                message: "audio backend worker exited without a result".into(),
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use runtime::JobRuntime;

    #[test]
    fn completed_backend_errors_keep_their_code_and_message() {
        let runtime = JobRuntime::with_max_concurrency(1);
        let (context, _) = runtime.create_job();
        let error =
            futures::executor::block_on(run_backend_worker::<(), _>(context.clone(), || {
                Err(BackendError {
                    code: BackendErrorCode::InvalidAudio,
                    message: "fixture decode failure".into(),
                })
            }))
            .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidAudio);
        assert_eq!(error.message, "fixture decode failure");
        assert_eq!(
            futures::executor::block_on(run_backend_worker(context, || Ok(42))).unwrap(),
            42
        );
    }

    #[test]
    fn cancellation_before_polling_does_not_run_or_retain_native_setup() {
        let (context, _) = JobRuntime::with_max_concurrency(1).create_job();
        let resources = std::sync::Arc::new(());
        let captured = resources.clone();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_called = called.clone();
        let work = run_backend_worker(context.clone(), move || {
            drop(captured);
            worker_called.store(true, std::sync::atomic::Ordering::Release);
            Ok(42)
        });
        context.cancel();
        assert_eq!(
            futures::executor::block_on(work).unwrap_err().code,
            BackendErrorCode::Cancelled
        );
        assert!(!called.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(std::sync::Arc::strong_count(&resources), 1);
    }

    #[test]
    fn an_unexpected_worker_exit_is_not_user_cancellation() {
        let (context, _) = JobRuntime::with_max_concurrency(1).create_job();
        let error =
            futures::executor::block_on(run_backend_worker::<(), _>(context.clone(), || {
                panic!("injected backend panic")
            }))
            .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InferenceFailed);
        assert!(!context.is_cancelled());
        assert_eq!(
            futures::executor::block_on(run_backend_worker(context, || Ok(43))).unwrap(),
            43
        );
    }

    #[test]
    fn cancellation_does_not_publish_a_result_before_running_work_exits() {
        let (context, _) = JobRuntime::with_max_concurrency(1).create_job();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let mut work = run_backend_worker(context.clone(), move || {
            started_tx.send(()).unwrap();
            let _ = release_rx.recv();
            Ok(42)
        });
        assert!(work.as_mut().now_or_never().is_none());
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        context.cancel();
        let early = work.as_mut().now_or_never();
        let returned_early = early.is_some();
        drop(release_tx);
        let result = early.unwrap_or_else(|| futures::executor::block_on(work));
        assert!(
            !returned_early,
            "cancellation must wait for admitted native work to leave"
        );
        assert_eq!(result.unwrap_err().code, BackendErrorCode::Cancelled);
    }
}
