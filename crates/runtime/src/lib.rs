use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub completed: u64,
    pub total: Option<u64>,
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
    cancelled: Arc<AtomicBool>,
    events: Sender<JobEvent>,
}

impl JobContext {
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = self.events.send(JobEvent::Cancelled {
            job_id: self.job_id,
        });
    }

    pub fn report(&self, progress: Progress) {
        let _ = self.events.send(JobEvent::Progress {
            job_id: self.job_id,
            progress,
        });
    }

    pub fn started(&self) {
        let _ = self.events.send(JobEvent::Started {
            job_id: self.job_id,
        });
    }

    pub fn completed(&self) {
        let _ = self.events.send(JobEvent::Completed {
            job_id: self.job_id,
        });
    }

    pub fn failed(&self, message: impl Into<String>) {
        let _ = self.events.send(JobEvent::Failed {
            job_id: self.job_id,
            message: message.into(),
        });
    }
}

#[derive(Debug, Default)]
pub struct JobRuntime {
    next_id: AtomicU64,
}

impl JobRuntime {
    pub fn create_job(&self) -> (JobContext, Receiver<JobEvent>) {
        let job_id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let (events, receiver) = mpsc::channel();
        let context = JobContext {
            job_id,
            cancelled: Arc::new(AtomicBool::new(false)),
            events,
        };
        (context, receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            receiver.recv().unwrap(),
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
            receiver.recv().unwrap(),
            JobEvent::Cancelled { job_id: JobId(1) }
        );
    }
}
