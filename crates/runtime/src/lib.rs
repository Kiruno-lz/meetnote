use async_lock::Semaphore;
use event_listener::Event;
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

struct CancellationState {
    cancelled: AtomicBool,
    changed: Event,
    linked: Mutex<Vec<std::sync::Weak<CancellationState>>>,
    callbacks: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for CancellationState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CancellationState")
            .field("cancelled", &self.cancelled.load(Ordering::Acquire))
            .field(
                "linked",
                &self.linked.lock().map(|items| items.len()).unwrap_or(0),
            )
            .field(
                "callbacks",
                &self.callbacks.lock().map(|items| items.len()).unwrap_or(0),
            )
            .finish()
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn invoke_callback(callback: Arc<dyn Fn() + Send + Sync>) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                changed: Event::new(),
                linked: Mutex::new(Vec::new()),
                callbacks: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::Release) {
            return;
        }
        self.state.changed.notify(usize::MAX);
        let callbacks = std::mem::take(&mut *lock_unpoisoned(&self.state.callbacks));
        for callback in callbacks {
            invoke_callback(callback);
        }
        let linked = lock_unpoisoned(&self.state.linked).clone();
        for token in linked.into_iter().filter_map(|item| item.upgrade()) {
            CancellationToken { state: token }.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    async fn wait_cancelled(&self) {
        // Register before checking the flag so cancellation cannot be lost
        // between the check and awaiting notification. Dropping the future
        // removes its listener without retaining a callback on the token.
        let listener = self.state.changed.listen();
        if !self.is_cancelled() {
            listener.await;
        }
    }

    pub fn link(&self, other: &Self) {
        if Arc::ptr_eq(&self.state, &other.state) {
            return;
        }
        lock_unpoisoned(&self.state.linked).push(Arc::downgrade(&other.state));
        lock_unpoisoned(&other.state.linked).push(Arc::downgrade(&self.state));
        if self.is_cancelled() || other.is_cancelled() {
            self.cancel();
            other.cancel();
        }
    }

    pub fn on_cancel(&self, callback: impl Fn() + Send + Sync + 'static) {
        let callback: Arc<dyn Fn() + Send + Sync> = Arc::new(callback);
        if self.is_cancelled() {
            invoke_callback(callback);
            return;
        }
        let mut callbacks = lock_unpoisoned(&self.state.callbacks);
        if self.is_cancelled() {
            drop(callbacks);
            invoke_callback(callback);
        } else {
            callbacks.push(callback);
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

static BLOCKING_RUNTIME: std::sync::OnceLock<JobRuntime> = std::sync::OnceLock::new();

pub fn spawn_blocking<T, F>(work: F) -> impl Future<Output = T> + Send
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let runtime = BLOCKING_RUNTIME.get_or_init(JobRuntime::default);
    let (context, _) = runtime.create_job();
    // Anonymous work exposes no job ID or event stream. Its context owns the
    // record and shared gate; the private registry must not retain UI reads
    // forever, including futures dropped unpolled or workers that panic.
    runtime
        .remove(context.job_id())
        .expect("anonymous worker context must be registered");
    context.started();
    async move {
        let value = context.spawn_blocking_uncancellable(work).await;
        context.completed();
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub completed: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub job_id: JobId,
    pub state: JobState,
    pub progress: Option<Progress>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobEvent {
    Started { job_id: JobId },
    Progress { job_id: JobId, progress: Progress },
    Completed { job_id: JobId },
    Failed { job_id: JobId, message: String },
    Cancelled { job_id: JobId },
}

#[derive(Debug, Clone)]
pub struct JobContext {
    job_id: JobId,
    record: Arc<JobRecord>,
}

#[derive(Debug)]
struct JobRecord {
    snapshot: Mutex<JobSnapshot>,
    cancelled: CancellationToken,
    events: UnboundedSender<JobEvent>,
    worker_gate: Arc<Semaphore>,
}

impl JobRecord {
    fn mark_cancelled(&self) -> bool {
        let changed = {
            let mut snapshot = lock_unpoisoned(&self.snapshot);
            if matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                false
            } else {
                snapshot.state = JobState::Cancelled;
                snapshot.error = None;
                true
            }
        };
        if changed {
            let job_id = lock_unpoisoned(&self.snapshot).job_id;
            let _ = self.events.unbounded_send(JobEvent::Cancelled { job_id });
        }
        changed
    }

    fn cancel(&self) {
        self.mark_cancelled();
        self.cancelled.cancel();
    }
}

impl JobContext {
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    pub fn is_cancelled(&self) -> bool {
        self.record.cancelled.is_cancelled()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.record.cancelled.clone()
    }

    pub fn cancel(&self) {
        self.record.cancel();
    }

    pub fn report(&self, progress: Progress) {
        let mut snapshot = lock_unpoisoned(&self.record.snapshot);
        if self.record.cancelled.is_cancelled()
            || matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        {
            return;
        }
        snapshot.progress = Some(progress);
        drop(snapshot);
        let _ = self.record.events.unbounded_send(JobEvent::Progress {
            job_id: self.job_id,
            progress,
        });
    }

    pub fn started(&self) {
        let mut snapshot = lock_unpoisoned(&self.record.snapshot);
        if self.record.cancelled.is_cancelled() || snapshot.state != JobState::Queued {
            return;
        }
        snapshot.state = JobState::Running;
        snapshot.error = None;
        drop(snapshot);
        let _ = self.record.events.unbounded_send(JobEvent::Started {
            job_id: self.job_id,
        });
    }

    pub fn completed(&self) {
        let mut snapshot = lock_unpoisoned(&self.record.snapshot);
        if self.record.cancelled.is_cancelled() {
            drop(snapshot);
            self.record.mark_cancelled();
            return;
        }
        if matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return;
        }
        snapshot.state = JobState::Completed;
        snapshot.error = None;
        drop(snapshot);
        let _ = self.record.events.unbounded_send(JobEvent::Completed {
            job_id: self.job_id,
        });
    }

    pub fn failed(&self, message: impl Into<String>) {
        let message = message.into();
        let mut snapshot = lock_unpoisoned(&self.record.snapshot);
        if self.record.cancelled.is_cancelled() {
            drop(snapshot);
            self.record.mark_cancelled();
            return;
        }
        if matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return;
        }
        snapshot.state = JobState::Failed;
        snapshot.error = Some(message.clone());
        drop(snapshot);
        let _ = self.record.events.unbounded_send(JobEvent::Failed {
            job_id: self.job_id,
            message,
        });
    }

    pub fn spawn_blocking<T, F>(&self, work: F) -> impl Future<Output = T> + Send
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let context = self.clone();
        async move { context.spawn_blocking_uncancellable(work).await }
    }

    fn spawn_blocking_uncancellable<T, F>(&self, work: F) -> impl Future<Output = T> + Send
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let record = self.record.clone();
        let job_id = self.job_id;
        async move {
            let permit = record.worker_gate.acquire_arc().await;
            let (sender, receiver) = futures::channel::oneshot::channel();
            std::thread::Builder::new()
                .name(format!("meetnote-job-{}", job_id.0))
                .spawn(move || {
                    // The real worker owns admission even if its awaiting
                    // future is dropped while non-cooperative work continues.
                    let _permit = permit;
                    let _ = sender.send(work());
                })
                .expect("failed to spawn job worker");
            receiver
                .await
                .expect("job worker dropped before returning a result")
        }
    }

    pub fn spawn_blocking_cancellable<T, F>(
        &self,
        work: F,
    ) -> impl Future<Output = Result<T, JobCancelled>> + Send
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let record = self.record.clone();
        let job_id = self.job_id;
        async move {
            let permit = {
                let cancelled = record.cancelled.wait_cancelled();
                let admission = record.worker_gate.acquire_arc();
                futures::pin_mut!(cancelled, admission);
                match futures::future::select(cancelled, admission).await {
                    futures::future::Either::Left(_) => return Err(JobCancelled),
                    futures::future::Either::Right((permit, _)) => permit,
                }
            };
            if record.cancelled.is_cancelled() {
                return Err(JobCancelled);
            }
            let worker_cancellation = record.cancelled.clone();
            let (sender, receiver) = futures::channel::oneshot::channel();
            std::thread::Builder::new()
                .name(format!("meetnote-job-{}", job_id.0))
                .spawn(move || {
                    let _permit = permit;
                    if worker_cancellation.is_cancelled() {
                        let _ = sender.send(Err(JobCancelled));
                        return;
                    }
                    let value = work();
                    let _ = sender.send(if worker_cancellation.is_cancelled() {
                        Err(JobCancelled)
                    } else {
                        Ok(value)
                    });
                })
                .expect("failed to spawn job worker");
            match receiver.await.unwrap_or(Err(JobCancelled)) {
                Ok(value) if !record.cancelled.is_cancelled() => Ok(value),
                _ => Err(JobCancelled),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("job was cancelled before the worker started")]
pub struct JobCancelled;

#[derive(Debug, thiserror::Error)]
pub enum JobRuntimeError {
    #[error("job not found: {0:?}")]
    NotFound(JobId),
    #[error("job runtime lock poisoned")]
    LockPoisoned,
}

#[derive(Clone)]
pub struct JobRuntime {
    next_id: Arc<AtomicU64>,
    jobs: Arc<Mutex<BTreeMap<JobId, Arc<JobRecord>>>>,
    worker_gate: Arc<Semaphore>,
}

impl JobRuntime {
    pub fn with_max_concurrency(max_concurrency: usize) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            worker_gate: Arc::new(Semaphore::new(max_concurrency.max(1))),
        }
    }

    pub fn create_job(&self) -> (JobContext, UnboundedReceiver<JobEvent>) {
        let job_id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let (events, receiver) = mpsc::unbounded();
        let record = Arc::new(JobRecord {
            snapshot: Mutex::new(JobSnapshot {
                job_id,
                state: JobState::Queued,
                progress: None,
                error: None,
            }),
            cancelled: CancellationToken::new(),
            events,
            worker_gate: self.worker_gate.clone(),
        });
        self.jobs
            .lock()
            .expect("job runtime lock poisoned")
            .insert(job_id, record.clone());
        let context = JobContext { job_id, record };
        (context, receiver)
    }

    pub fn snapshot(&self, job_id: JobId) -> Result<JobSnapshot, JobRuntimeError> {
        let record = self
            .jobs
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)?
            .get(&job_id)
            .cloned()
            .ok_or(JobRuntimeError::NotFound(job_id))?;
        record
            .snapshot
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)
            .map(|snapshot| snapshot.clone())
    }

    pub fn list(&self) -> Result<Vec<JobSnapshot>, JobRuntimeError> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)?;
        jobs.values()
            .map(|record| {
                record
                    .snapshot
                    .lock()
                    .map_err(|_| JobRuntimeError::LockPoisoned)
                    .map(|snapshot| snapshot.clone())
            })
            .collect()
    }

    pub fn cancel(&self, job_id: JobId) -> Result<(), JobRuntimeError> {
        let record = self
            .jobs
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)?
            .get(&job_id)
            .cloned()
            .ok_or(JobRuntimeError::NotFound(job_id))?;
        JobContext { job_id, record }.cancel();
        Ok(())
    }

    pub fn remove(&self, job_id: JobId) -> Result<JobSnapshot, JobRuntimeError> {
        let record = self
            .jobs
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)?
            .remove(&job_id)
            .ok_or(JobRuntimeError::NotFound(job_id))?;
        record
            .snapshot
            .lock()
            .map_err(|_| JobRuntimeError::LockPoisoned)
            .map(|snapshot| snapshot.clone())
    }
}

impl Default for JobRuntime {
    fn default() -> Self {
        let max_concurrency = std::thread::available_parallelism()
            .map(|value| value.get().min(4))
            .unwrap_or(2);
        Self::with_max_concurrency(max_concurrency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    /// Hold the only worker permit until the test exits, including on assertion failure.
    struct OccupiedRuntime {
        runtime: JobRuntime,
        release: Option<std::sync::mpsc::Sender<()>>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl OccupiedRuntime {
        fn new() -> Self {
            let runtime = JobRuntime::with_max_concurrency(1);
            let (context, _) = runtime.create_job();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let worker = std::thread::spawn(move || {
                futures::executor::block_on(context.spawn_blocking(move || {
                    started_tx.send(()).unwrap();
                    let _ = release_rx.recv();
                }));
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Self {
                runtime,
                release: Some(release_tx),
                worker: Some(worker),
            }
        }
    }

    impl Drop for OccupiedRuntime {
        fn drop(&mut self) {
            drop(self.release.take());
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    #[test]
    fn dropping_unpolled_cancellable_work_releases_queued_resources() {
        assert_dropped_queued_resources(false);
    }

    #[test]
    fn dropping_polled_cancellable_work_releases_queued_resources() {
        assert_dropped_queued_resources(true);
    }

    fn assert_dropped_queued_resources(poll: bool) {
        let occupied = OccupiedRuntime::new();
        let (context, _) = occupied.runtime.create_job();
        let retained = Arc::new(());
        let captured = retained.clone();
        let called = Arc::new(AtomicBool::new(false));
        let worker_called = called.clone();
        let mut future = Box::pin(context.spawn_blocking_cancellable(move || {
            worker_called.store(true, Ordering::Release);
            drop(captured);
        }));
        if poll {
            let mut task = std::task::Context::from_waker(futures::task::noop_waker_ref());
            assert!(future.as_mut().poll(&mut task).is_pending());
        }
        drop(future);
        assert!(!called.load(Ordering::Acquire));
        assert_eq!(Arc::strong_count(&retained), 1,
            "discarded queued work must release captured audio/models before the occupied worker exits");
    }

    #[test]
    fn dropping_queued_uncancellable_work_releases_resources() {
        let occupied = OccupiedRuntime::new();
        let (context, _) = occupied.runtime.create_job();
        let retained = Arc::new(());
        let captured = retained.clone();
        let mut future = Box::pin(context.spawn_blocking(move || drop(captured)));
        let mut task = std::task::Context::from_waker(futures::task::noop_waker_ref());
        assert!(future.as_mut().poll(&mut task).is_pending());
        drop(future);
        assert_eq!(Arc::strong_count(&retained), 1);
    }

    #[test]
    fn linked_cancellation_wakes_all_queued_waiters_without_permit_release() {
        use futures::FutureExt;
        let occupied = OccupiedRuntime::new();
        let (context, _) = occupied.runtime.create_job();
        let linked = CancellationToken::new();
        context.cancellation_token().link(&linked);
        let called = Arc::new(AtomicBool::new(false));
        struct WakeFlag(AtomicBool);
        impl futures::task::ArcWake for WakeFlag {
            fn wake_by_ref(wake: &Arc<Self>) {
                wake.0.store(true, Ordering::Release);
            }
        }
        let mut pending = Vec::new();
        let mut wakes = Vec::new();
        for _ in 0..32 {
            let wake = Arc::new(WakeFlag(AtomicBool::new(false)));
            let waker = futures::task::waker(wake.clone());
            let mut task = std::task::Context::from_waker(&waker);
            let called = called.clone();
            let mut future = Box::pin(context.spawn_blocking_cancellable(move || {
                called.store(true, Ordering::Release);
            }));
            assert!(future.as_mut().poll(&mut task).is_pending());
            pending.push(future);
            wakes.push(wake);
        }
        linked.cancel();
        assert!(
            wakes.iter().all(|wake| wake.0.load(Ordering::Acquire)),
            "each queued task must be woken without a worker permit becoming free"
        );
        for future in pending {
            assert_eq!(future.now_or_never(), Some(Err(JobCancelled)));
        }
        // Cancellation before the first poll must also skip work.
        assert_eq!(
            context.spawn_blocking_cancellable(|| 42).now_or_never(),
            Some(Err(JobCancelled))
        );
        assert!(!called.load(Ordering::Acquire));
    }

    #[test]
    fn dropping_running_future_keeps_admission_until_the_worker_exits() {
        use futures::FutureExt;
        for cancellable in [false, true] {
            let runtime = JobRuntime::with_max_concurrency(1);
            let (first, _) = runtime.create_job();
            let (second, _) = runtime.create_job();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let work = move || {
                started_tx.send(()).unwrap();
                let _ = release_rx.recv();
            };
            let mut future: futures::future::BoxFuture<'_, Result<(), JobCancelled>> =
                if cancellable {
                    Box::pin(first.spawn_blocking_cancellable(work))
                } else {
                    Box::pin(first.spawn_blocking(work).map(Ok))
                };
            let mut task = std::task::Context::from_waker(futures::task::noop_waker_ref());
            assert!(future.as_mut().poll(&mut task).is_pending());
            started_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            first.cancel(); // Running native work is cooperative, not forcibly stopped.
            drop(future);
            let (second_tx, second_rx) = std::sync::mpsc::channel();
            let mut second_future = Box::pin(second.spawn_blocking(move || {
                second_tx.send(()).unwrap();
                42
            }));
            assert!(second_future.as_mut().poll(&mut task).is_pending());
            let escaped = second_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_ok();
            drop(release_tx);
            assert_eq!(futures::executor::block_on(second_future), 42);
            assert!(
                !escaped,
                "a dropped or cancelled receiver released a running worker permit"
            );
        }
    }

    #[test]
    fn worker_panic_releases_admission_for_the_next_job() {
        let runtime = JobRuntime::with_max_concurrency(1);
        let (context, _) = runtime.create_job();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            futures::executor::block_on(context.spawn_blocking(|| panic!("injected")))
        }));
        assert!(result.is_err());
        assert_eq!(
            futures::executor::block_on(context.spawn_blocking(|| 42)),
            42
        );
        assert_eq!(
            futures::executor::block_on(context.spawn_blocking_cancellable(|| panic!("injected"))),
            Err(JobCancelled)
        );
        assert!(
            !context.is_cancelled(),
            "worker exit must not impersonate a user cancellation token"
        );
        assert_eq!(
            futures::executor::block_on(context.spawn_blocking_cancellable(|| 43)),
            Ok(43)
        );
    }

    #[test]
    fn anonymous_blocking_work_does_not_accumulate_registered_jobs() {
        let count = || BLOCKING_RUNTIME.get().unwrap().list().unwrap().len();
        let unpolled = spawn_blocking(|| 1);
        drop(unpolled);
        let after_unpolled = count();

        assert_eq!(futures::executor::block_on(spawn_blocking(|| 42)), 42);
        let after_completed = count();

        let panic = std::panic::catch_unwind(|| {
            futures::executor::block_on(spawn_blocking(|| panic!("injected worker panic")));
        });
        assert!(
            panic.is_err(),
            "anonymous workers must retain panic propagation"
        );
        let after_panicked = count();
        assert_eq!(
            [after_unpolled, after_completed, after_panicked],
            [0, 0, 0],
            "anonymous UI reads have no public job handle or retained job history"
        );
    }

    #[test]
    fn creates_monotonic_job_ids_and_reports_progress() {
        let runtime = JobRuntime::default();
        let (first, receiver) = runtime.create_job();
        let (second, _) = runtime.create_job();

        first.report(Progress {
            completed: 1,
            total: Some(2),
        });

        assert_eq!(first.job_id(), JobId(1));
        assert_eq!(second.job_id(), JobId(2));
        assert_eq!(
            futures::executor::block_on(receiver.into_future())
                .0
                .unwrap(),
            JobEvent::Progress {
                job_id: JobId(1),
                progress: Progress {
                    completed: 1,
                    total: Some(2),
                },
            }
        );
    }

    #[test]
    fn cancellation_is_observable_and_sets_the_cancelled_state() {
        let runtime = JobRuntime::default();
        let (context, receiver) = runtime.create_job();

        context.cancel();

        assert!(context.is_cancelled());
        assert_eq!(
            futures::executor::block_on(receiver.into_future())
                .0
                .unwrap(),
            JobEvent::Cancelled { job_id: JobId(1) }
        );
    }

    #[test]
    fn tracks_job_lifecycle_and_supports_runtime_cancellation() {
        let runtime = JobRuntime::default();
        let (context, _receiver) = runtime.create_job();
        let job_id = context.job_id();

        assert_eq!(runtime.snapshot(job_id).unwrap().state, JobState::Queued);
        context.started();
        context.report(Progress {
            completed: 2,
            total: Some(4),
        });
        assert_eq!(runtime.snapshot(job_id).unwrap().state, JobState::Running);
        assert_eq!(
            runtime.snapshot(job_id).unwrap().progress,
            Some(Progress {
                completed: 2,
                total: Some(4),
            })
        );

        runtime.cancel(job_id).unwrap();
        assert_eq!(runtime.snapshot(job_id).unwrap().state, JobState::Cancelled);
        let removed = runtime.remove(job_id).unwrap();
        assert_eq!(removed.state, JobState::Cancelled);
        assert!(matches!(
            runtime.snapshot(job_id),
            Err(JobRuntimeError::NotFound(found)) if found == job_id
        ));
    }

    #[test]
    fn terminal_state_is_monotonic_and_emits_only_one_terminal_event() {
        let runtime = JobRuntime::default();
        let (context, mut receiver) = runtime.create_job();
        let job_id = context.job_id();

        context.started();
        context.cancel();
        context.completed();
        context.failed("late failure");
        context.report(Progress {
            completed: 1,
            total: Some(1),
        });

        assert_eq!(runtime.snapshot(job_id).unwrap().state, JobState::Cancelled);
        assert_eq!(
            futures::executor::block_on(receiver.next()),
            Some(JobEvent::Started { job_id })
        );
        assert_eq!(
            futures::executor::block_on(receiver.next()),
            Some(JobEvent::Cancelled { job_id })
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn limits_blocking_workers_with_a_shared_gate() {
        let runtime = JobRuntime::with_max_concurrency(1);
        let (first, _) = runtime.create_job();
        let (second, _) = runtime.create_job();
        let (first_started_tx, first_started_rx) = std::sync::mpsc::channel();
        let (second_started_tx, second_started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let first_release = release.clone();
        let first_thread = std::thread::spawn(move || {
            futures::executor::block_on(first.spawn_blocking(move || {
                first_started_tx.send(()).unwrap();
                while !first_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }));
        });
        first_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let second_thread = std::thread::spawn(move || {
            futures::executor::block_on(second.spawn_blocking(move || {
                second_started_tx.send(()).unwrap();
            }));
        });
        assert!(second_started_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());
        release.store(true, Ordering::Release);
        first_thread.join().unwrap();
        second_thread.join().unwrap();
        assert!(second_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok());
    }
}
