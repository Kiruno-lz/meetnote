use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub fn spawn_blocking<T, F>(work: F) -> impl Future<Output = T> + Send
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("meetnote-inference".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .expect("failed to spawn inference worker");

    async move {
        receiver
            .await
            .expect("inference worker dropped before returning a result")
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
    cancelled: Arc<AtomicBool>,
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
    fn acquire(self: &Arc<Self>) -> WorkerPermit {
        let mut active = self.active.lock().expect("worker gate lock poisoned");
        while *active >= self.limit {
            active = self
                .changed
                .wait(active)
                .expect("worker gate lock poisoned");
        }
        *active += 1;
        WorkerPermit { gate: self.clone() }
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            *active = active.saturating_sub(1);
            self.gate.changed.notify_one();
        }
    }
}

impl JobContext {
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    pub fn is_cancelled(&self) -> bool {
        self.record.cancelled.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.record.cancelled.store(true, Ordering::Relaxed);
        if let Ok(mut snapshot) = self.record.snapshot.lock() {
            snapshot.state = JobState::Cancelled;
            snapshot.error = None;
        }
        let _ = self.record.events.unbounded_send(JobEvent::Cancelled {
            job_id: self.job_id,
        });
    }

    pub fn report(&self, progress: Progress) {
        if let Ok(mut snapshot) = self.record.snapshot.lock() {
            snapshot.progress = Some(progress);
        }
        let _ = self.record.events.unbounded_send(JobEvent::Progress {
            job_id: self.job_id,
            progress,
        });
    }

    pub fn started(&self) {
        if let Ok(mut snapshot) = self.record.snapshot.lock() {
            snapshot.state = JobState::Running;
            snapshot.error = None;
        }
        let _ = self.record.events.unbounded_send(JobEvent::Started {
            job_id: self.job_id,
        });
    }

    pub fn completed(&self) {
        if let Ok(mut snapshot) = self.record.snapshot.lock() {
            snapshot.state = JobState::Completed;
            snapshot.error = None;
        }
        let _ = self.record.events.unbounded_send(JobEvent::Completed {
            job_id: self.job_id,
        });
    }

    pub fn failed(&self, message: impl Into<String>) {
        let message = message.into();
        if let Ok(mut snapshot) = self.record.snapshot.lock() {
            snapshot.state = JobState::Failed;
            snapshot.error = Some(message.clone());
        }
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
        let record = self.record.clone();
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name(format!("meetnote-job-{}", self.job_id.0))
            .spawn(move || {
                let _permit = record.worker_gate.acquire();
                let _ = sender.send(work());
            })
            .expect("failed to spawn job worker");

        async move {
            receiver
                .await
                .expect("job worker dropped before returning a result")
        }
    }
}

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
            cancelled: Arc::new(AtomicBool::new(false)),
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
