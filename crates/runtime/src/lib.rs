use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

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
    }

    pub fn report(&self, progress: Progress) {
        let _ = self.events.send(JobEvent::Progress {
            job_id: self.job_id,
            progress,
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
}
