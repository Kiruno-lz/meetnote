use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

struct CancellationState {
    cancelled: AtomicBool,
    changed: std::sync::Condvar,
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

fn wait_unpoisoned<'a, T>(condition: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condition
        .wait(guard)
        .unwrap_or_else(|error| error.into_inner())
}

fn wait_timeout_unpoisoned<'a, T>(
    condition: &Condvar,
    guard: MutexGuard<'a, T>,
    timeout: Duration,
) -> MutexGuard<'a, T> {
    condition
        .wait_timeout(guard, timeout)
        .map(|(guard, _)| guard)
        .unwrap_or_else(|error| error.into_inner().0)
}

fn invoke_callback(callback: Arc<dyn Fn() + Send + Sync>) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                changed: std::sync::Condvar::new(),
                linked: Mutex::new(Vec::new()),
                callbacks: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::Release) {
            return;
        }
        self.state.changed.notify_all();
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

pub fn spawn_blocking<T, F>(work: F) -> impl Future<Output = T> + Send
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    static RUNTIME: std::sync::OnceLock<JobRuntime> = std::sync::OnceLock::new();
    let runtime = RUNTIME.get_or_init(JobRuntime::default).clone();
    let (context, _) = runtime.create_job();
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
    worker_gate: Arc<WorkerGate>,
}

#[derive(Debug)]
struct WorkerGate {
    active: Mutex<usize>,
    limit: usize,
    changed: std::sync::Condvar,
}

struct WorkerPermit {
    gate: Arc<WorkerGate>,
}

impl WorkerGate {
    fn acquire(
        self: &Arc<Self>,
        cancellation: &CancellationToken,
    ) -> Result<WorkerPermit, JobCancelled> {
        let gate = Arc::downgrade(self);
        cancellation.on_cancel(move || {
            if let Some(gate) = gate.upgrade() {
                gate.changed.notify_all();
            }
        });
        let mut active = lock_unpoisoned(&self.active);
        while *active >= self.limit {
            if cancellation.is_cancelled() {
                return Err(JobCancelled);
            }
            active = wait_timeout_unpoisoned(&self.changed, active, Duration::from_millis(10));
        }
        if cancellation.is_cancelled() {
            return Err(JobCancelled);
        }
        *active += 1;
        Ok(WorkerPermit { gate: self.clone() })
    }

    fn acquire_uncancellable(self: &Arc<Self>) -> WorkerPermit {
        let mut active = lock_unpoisoned(&self.active);
        while *active >= self.limit {
            active = wait_unpoisoned(&self.changed, active);
        }
        *active += 1;
        WorkerPermit { gate: self.clone() }
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        let mut active = lock_unpoisoned(&self.gate.active);
        *active = active.saturating_sub(1);
        self.gate.changed.notify_one();
    }
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
            self.worker_gate.changed.notify_all();
            let job_id = lock_unpoisoned(&self.snapshot).job_id;
            let _ = self.events.unbounded_send(JobEvent::Cancelled { job_id });
        }
        changed
    }

    fn cancel(&self) {
        self.mark_cancelled();
        self.cancelled.cancel();
        self.worker_gate.changed.notify_all();
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
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name(format!("meetnote-job-{}", self.job_id.0))
            .spawn(move || {
                let _permit = record.worker_gate.acquire_uncancellable();
                let _ = sender.send(work());
            })
            .expect("failed to spawn job worker");
        async move {
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
        let cancellation = record.cancelled.clone();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name(format!("meetnote-job-{}", self.job_id.0))
            .spawn(move || {
                let Ok(_permit) = record.worker_gate.acquire(&worker_cancellation) else {
                    let _ = sender.send(Err(JobCancelled));
                    return;
                };
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

        async move {
            match receiver.await.unwrap_or(Err(JobCancelled)) {
                Ok(value) if !cancellation.is_cancelled() => Ok(value),
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
    worker_gate: Arc<WorkerGate>,
}

impl JobRuntime {
    pub fn with_max_concurrency(max_concurrency: usize) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            worker_gate: Arc::new(WorkerGate {
                active: Mutex::new(0),
                limit: max_concurrency.max(1),
                changed: std::sync::Condvar::new(),
            }),
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
