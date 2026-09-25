use crate::download::{
    download_bundle_with_progress, download_file_controlled_with_size, BundleArtifact,
    DownloadControl, DownloadError,
};
use crate::probe::{
    self, ArtifactChecksum, BackendCheck, BackendProbeResult, ChecksumResult, ModelFailure,
    ModelFailureRecord, ModelOperation, ModelRecord, ProbeOutcome, ProbeRecord,
};
use crate::{ModelCatalog, ModelCatalogEntry};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DownloadId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Verifying,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub id: DownloadId,
    pub model_id: String,
    pub state: DownloadState,
    pub completed_bytes: u64,
    pub speed_bytes_per_second: Option<u64>,
    pub total_bytes: Option<u64>,
    pub artifact_index: usize,
    pub artifact_count: usize,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    #[serde(default)]
    pub retry_count: u32,
}

impl DownloadState {
    /// Completed, cancelled and failed tasks never change state again.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    Updated(DownloadProgress),
}

pub struct DownloadTask {
    pub id: DownloadId,
    pub events: Receiver<DownloadEvent>,
    /// The same events as `events`, pushed by the download thread; the stream
    /// ends after the terminal state.
    pub updates: UnboundedReceiver<DownloadEvent>,
}

/// A running `ModelManager::reprobe`.
pub struct ReprobeTask {
    pub model_id: String,
    /// Resolves once the probe ran and its record was written; `Err` only
    /// when the record could not be written.
    pub finished: oneshot::Receiver<Result<ProbeRecord, ModelManagerError>>,
}

/// Published by `ModelManager::subscribe_reprobes_async` when a re-probe
/// ends, after its record was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReprobeFinished {
    pub model_id: String,
    pub record: ProbeRecord,
}

pub trait BackendProbe: Send + Sync {
    fn probe(&self, task: &str, model: &contracts::ModelRef) -> Result<(), String>;
}

#[derive(Debug, thiserror::Error)]
pub enum ModelManagerError {
    #[error("model not found in catalog: {0}")]
    ModelNotFound(String),
    #[error("model requires authentication: {0}")]
    AuthenticationRequired(String),
    #[error("model is already installed: {0}")]
    AlreadyInstalled(String),
    #[error("model is not ready for download: {0}")]
    ModelNotReady(String),
    #[error("model is not installed: {0}")]
    ModelNotInstalled(String),
    #[error("model {model_id} does not support backend {backend_id}")]
    BackendNotSupported {
        model_id: String,
        backend_id: String,
    },
    #[error("model download is already active: {0}")]
    AlreadyActive(String),
    #[error("download task not found: {0:?}")]
    TaskNotFound(DownloadId),
    #[error("download task is not active: {0:?}")]
    TaskNotActive(DownloadId),
    #[error("model download failed: {0}")]
    Download(Box<DownloadError>),
    #[error("backend probe failed: {0}")]
    BackendProbeFailed(String),
    #[error("failed to write model probe record {path}: {source}")]
    Record {
        path: String,
        source: std::io::Error,
    },
    #[error("download task lock poisoned")]
    LockPoisoned,
}

impl From<DownloadError> for ModelManagerError {
    fn from(error: DownloadError) -> Self {
        Self::Download(Box::new(error))
    }
}

struct TaskRecord {
    control: DownloadControl,
    progress: Mutex<DownloadProgress>,
    subscribers: Mutex<Vec<Sender<DownloadEvent>>>,
    /// Pushed under the `progress` lock, so they see every change in order;
    /// dropped once the task reaches a terminal state.
    async_subscribers: Mutex<Vec<UnboundedSender<DownloadEvent>>>,
    worker_started: AtomicBool,
}

impl TaskRecord {
    fn new(progress: DownloadProgress) -> Self {
        Self {
            control: DownloadControl::default(),
            progress: Mutex::new(progress),
            subscribers: Mutex::new(Vec::new()),
            async_subscribers: Mutex::new(Vec::new()),
            worker_started: AtomicBool::new(false),
        }
    }
}

#[derive(Clone)]
pub struct ModelManager {
    catalog: Arc<ModelCatalog>,
    model_root: PathBuf,
    task_state_path: PathBuf,
    next_id: Arc<AtomicU64>,
    tasks: Arc<Mutex<BTreeMap<DownloadId, Arc<TaskRecord>>>>,
    /// Receive every task's updates; see `subscribe_all_async`.
    all_subscribers: Arc<Mutex<Vec<UnboundedSender<DownloadEvent>>>>,
    backend_probe: Option<Arc<dyn BackendProbe>>,
    /// Models with a re-probe running.
    reprobes: Arc<Mutex<BTreeSet<String>>>,
    reprobe_subscribers: Arc<Mutex<Vec<UnboundedSender<ReprobeFinished>>>>,
}

impl ModelManager {
    pub fn new(catalog: ModelCatalog, model_root: impl AsRef<Path>) -> Self {
        Self::with_backend_probe(catalog, model_root, None)
    }

    pub fn with_backend_probe(
        catalog: ModelCatalog,
        model_root: impl AsRef<Path>,
        backend_probe: Option<Arc<dyn BackendProbe>>,
    ) -> Self {
        let model_root = model_root.as_ref().to_path_buf();
        let task_state_path = model_root.join(".download-tasks.json");
        let persisted = load_task_progress(&task_state_path);
        let next_id = persisted
            .iter()
            .map(|progress| progress.id.0)
            .max()
            .unwrap_or(0);
        let tasks = persisted
            .into_iter()
            .map(|mut progress| {
                if matches!(
                    progress.state,
                    DownloadState::Queued | DownloadState::Downloading | DownloadState::Verifying
                ) {
                    progress.state = DownloadState::Paused;
                }
                (progress.id, Arc::new(TaskRecord::new(progress)))
            })
            .collect();
        Self {
            catalog: Arc::new(catalog),
            model_root,
            task_state_path,
            next_id: Arc::new(AtomicU64::new(next_id)),
            tasks: Arc::new(Mutex::new(tasks)),
            all_subscribers: Arc::new(Mutex::new(Vec::new())),
            backend_probe,
            reprobes: Arc::new(Mutex::new(BTreeSet::new())),
            reprobe_subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    pub fn browse(
        &self,
        task: Option<&str>,
        backend_id: Option<&str>,
    ) -> Vec<crate::ModelAvailability> {
        self.catalog.browse(&self.model_root, task, backend_id)
    }

    pub fn start(&self, model_id: &str) -> Result<DownloadTask, ModelManagerError> {
        let model = self
            .catalog
            .find(model_id)
            .cloned()
            .ok_or_else(|| ModelManagerError::ModelNotFound(model_id.into()))?;
        if model.requires_auth {
            return Err(ModelManagerError::AuthenticationRequired(model.id));
        }
        if model.status != "ready" {
            return Err(ModelManagerError::ModelNotReady(model.id));
        }
        if model.install_state(&self.model_root) == crate::ModelInstallState::Installed {
            return Err(ModelManagerError::AlreadyInstalled(model.id));
        }
        if self.has_active_download(&model.id)?
            || lock_unpoisoned(&self.reprobes).contains(&model.id)
        {
            return Err(ModelManagerError::AlreadyActive(model.id));
        }

        let id = DownloadId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let artifact_count = model.artifact_filenames().len();
        let record = Arc::new(TaskRecord::new(DownloadProgress {
            id,
            model_id: model.id.clone(),
            state: DownloadState::Queued,
            completed_bytes: 0,
            speed_bytes_per_second: None,
            total_bytes: model.size_bytes,
            artifact_index: 0,
            artifact_count,
            eta_seconds: None,
            retry_count: 0,
        }));
        self.tasks
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .insert(id, record.clone());
        self.persist_tasks()?;
        let receiver = self.subscribe(id)?;
        let updates = self.subscribe_async(id)?;
        self.spawn_worker(id, model, record)?;
        Ok(DownloadTask {
            id,
            events: receiver,
            updates,
        })
    }

    /// The task's current progress followed by every later update, pushed by
    /// the download thread. The stream ends after the terminal state.
    pub fn subscribe_async(
        &self,
        id: DownloadId,
    ) -> Result<UnboundedReceiver<DownloadEvent>, ModelManagerError> {
        let record = self.record(id)?;
        let (sender, receiver) = unbounded();
        let progress = record
            .progress
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        let terminal = progress.state.is_terminal();
        let _ = sender.unbounded_send(DownloadEvent::Updated(progress.clone()));
        if !terminal {
            record
                .async_subscribers
                .lock()
                .map_err(|_| ModelManagerError::LockPoisoned)?
                .push(sender);
        }
        Ok(receiver)
    }

    /// Every task's updates from now on, including tasks started later.
    pub fn subscribe_all_async(&self) -> UnboundedReceiver<DownloadEvent> {
        let (sender, receiver) = unbounded();
        lock_unpoisoned(&self.all_subscribers).push(sender);
        receiver
    }

    pub fn subscribe(&self, id: DownloadId) -> Result<Receiver<DownloadEvent>, ModelManagerError> {
        let record = self.record(id)?;
        let (sender, receiver) = mpsc::channel();
        let current = record
            .progress
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .clone();
        sender
            .send(DownloadEvent::Updated(current))
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        record
            .subscribers
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .push(sender);
        Ok(receiver)
    }

    pub fn status(&self, id: DownloadId) -> Result<DownloadProgress, ModelManagerError> {
        self.record(id)?
            .progress
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)
            .map(|progress| progress.clone())
    }

    pub fn list(&self) -> Result<Vec<DownloadProgress>, ModelManagerError> {
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        tasks
            .values()
            .map(|record| {
                record
                    .progress
                    .lock()
                    .map_err(|_| ModelManagerError::LockPoisoned)
                    .map(|progress| progress.clone())
            })
            .collect()
    }

    pub fn model_ref(
        &self,
        model_id: &str,
        backend_id: &str,
    ) -> Result<contracts::ModelRef, ModelManagerError> {
        let model = self
            .catalog
            .find(model_id)
            .ok_or_else(|| ModelManagerError::ModelNotFound(model_id.into()))?;
        if !model.supports_backend(backend_id) {
            return Err(ModelManagerError::BackendNotSupported {
                model_id: model_id.into(),
                backend_id: backend_id.into(),
            });
        }
        if model.install_state(&self.model_root) != crate::ModelInstallState::Installed {
            return Err(ModelManagerError::ModelNotInstalled(model_id.into()));
        }
        let artifact_path = if model.artifacts.is_empty() {
            model.artifact_paths(&self.model_root)[0].clone()
        } else {
            model.install_directory(&self.model_root)
        };
        Ok(contracts::ModelRef {
            backend_id: backend_id.into(),
            model_id: model.id.clone(),
            revision: model.revision.clone(),
            artifact_path: artifact_path.display().to_string(),
            artifacts: model_artifact_refs(model),
        })
    }

    pub fn configured_model_ref(
        &self,
        task: &str,
        backend_id: &str,
        preferred: Option<&contracts::ModelRef>,
    ) -> Result<contracts::ModelRef, ModelManagerError> {
        if let Some(preferred) = preferred {
            if preferred.backend_id == backend_id
                && self
                    .catalog
                    .find(&preferred.model_id)
                    .is_some_and(|model| model.task == task && model.supports_backend(backend_id))
            {
                return self.model_ref_from_catalog(&preferred.model_id, backend_id);
            }
        }
        let ready = || {
            self.catalog.models.iter().filter(|model| {
                model.task == task && model.supports_backend(backend_id) && model.status == "ready"
            })
        };
        let model = ready()
            .find(|model| {
                model.install_state(&self.model_root) == crate::ModelInstallState::Installed
            })
            .or_else(|| ready().next())
            .ok_or_else(|| ModelManagerError::BackendNotSupported {
                model_id: task.into(),
                backend_id: backend_id.into(),
            })?;
        self.model_ref_from_catalog(&model.id, backend_id)
    }

    fn model_ref_from_catalog(
        &self,
        model_id: &str,
        backend_id: &str,
    ) -> Result<contracts::ModelRef, ModelManagerError> {
        let model = self
            .catalog
            .find(model_id)
            .ok_or_else(|| ModelManagerError::ModelNotFound(model_id.into()))?;
        if !model.supports_backend(backend_id) {
            return Err(ModelManagerError::BackendNotSupported {
                model_id: model_id.into(),
                backend_id: backend_id.into(),
            });
        }
        let artifact_path = if model.artifacts.is_empty() {
            model.artifact_paths(&self.model_root)[0].clone()
        } else {
            model.install_directory(&self.model_root)
        };
        Ok(contracts::ModelRef {
            backend_id: backend_id.into(),
            model_id: model.id.clone(),
            revision: model.revision.clone(),
            artifact_path: artifact_path.display().to_string(),
            artifacts: model_artifact_refs(model),
        })
    }

    pub fn pause(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        let record = self.record(id)?;
        self.ensure_active(&record, id)?;
        record.control.pause();
        self.update(id, |progress| progress.state = DownloadState::Paused)
    }

    pub fn resume(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        let record = self.record(id)?;
        self.ensure_active(&record, id)?;
        let model_id = record
            .progress
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .model_id
            .clone();
        record.control.resume();
        self.update(id, |progress| progress.state = DownloadState::Downloading)?;
        if !record.worker_started.swap(true, Ordering::AcqRel) {
            let model = self
                .catalog
                .find(&model_id)
                .cloned()
                .ok_or(ModelManagerError::ModelNotFound(model_id))?;
            self.spawn_worker(id, model, record)?;
        }
        Ok(())
    }

    /// Probes an installed model again on its own thread: hashes its files
    /// against the catalog, runs every candidate backend probe, and replaces
    /// the probe record. A failed re-probe keeps the files and does not make
    /// the model unusable here: `model_ref` still resolves, and processing
    /// runs its own preflight. The failure stays in the record until a later
    /// download or re-probe succeeds.
    pub fn reprobe(&self, model_id: &str) -> Result<ReprobeTask, ModelManagerError> {
        let model = self
            .catalog
            .find(model_id)
            .cloned()
            .ok_or_else(|| ModelManagerError::ModelNotFound(model_id.into()))?;
        if model.install_state(&self.model_root) != crate::ModelInstallState::Installed {
            return Err(ModelManagerError::ModelNotInstalled(model.id));
        }
        if !lock_unpoisoned(&self.reprobes).insert(model.id.clone()) {
            return Err(ModelManagerError::AlreadyActive(model.id));
        }
        let (sender, finished) = oneshot::channel();
        let manager = self.clone();
        let worker_model = model.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("meetnote-reprobe-{}", model.id))
            .spawn(move || {
                let record = manager.probe_model(&worker_model, ModelOperation::Reprobe);
                let failure = match &record.outcome {
                    ProbeOutcome::Passed => None,
                    ProbeOutcome::Failed { reason } => Some(ModelFailureRecord {
                        operation: ModelOperation::Reprobe,
                        failed_at_ms: record.probed_at_ms,
                        revision: record.revision.clone(),
                        reason: reason.clone(),
                    }),
                };
                let written = manager.write_record(
                    &worker_model,
                    ModelRecord {
                        probe: Some(record.clone()),
                        failure,
                    },
                );
                lock_unpoisoned(&manager.reprobes).remove(&worker_model.id);
                let finished = ReprobeFinished {
                    model_id: worker_model.id.clone(),
                    record: record.clone(),
                };
                lock_unpoisoned(&manager.reprobe_subscribers)
                    .retain(|subscriber| subscriber.unbounded_send(finished.clone()).is_ok());
                let _ = sender.send(written.map(|()| record));
            });
        if let Err(error) = spawned {
            lock_unpoisoned(&self.reprobes).remove(&model.id);
            return Err(ModelManagerError::Download(Box::new(DownloadError::Io(
                error,
            ))));
        }
        Ok(ReprobeTask {
            model_id: model.id,
            finished,
        })
    }

    fn write_record(
        &self,
        model: &ModelCatalogEntry,
        record: ModelRecord,
    ) -> Result<(), ModelManagerError> {
        let path = model.probe_record_path(&self.model_root);
        probe::write_record(&path, &model.id, &record).map_err(|source| ModelManagerError::Record {
            path: path.display().to_string(),
            source,
        })
    }

    /// Records `reason` as the model's last failure and ends the task with
    /// it. The files are left as they are.
    fn fail_download(&self, id: DownloadId, model: &ModelCatalogEntry, reason: ModelFailure) {
        let message = reason.to_string();
        let _ = self.write_record(
            model,
            ModelRecord {
                probe: None,
                failure: Some(ModelFailureRecord {
                    operation: ModelOperation::Download,
                    failed_at_ms: probe::now_ms(),
                    revision: model.revision.clone(),
                    reason,
                }),
            },
        );
        let _ = self.update(id, |progress| {
            progress.state = DownloadState::Failed(message)
        });
    }

    /// Every re-probe ending from now on.
    pub fn subscribe_reprobes_async(&self) -> UnboundedReceiver<ReprobeFinished> {
        let (sender, receiver) = unbounded();
        lock_unpoisoned(&self.reprobe_subscribers).push(sender);
        receiver
    }

    /// The artifact probe: checksums, then every candidate backend. After a
    /// download the checksums were verified while downloading; a re-probe
    /// hashes the installed files again.
    fn probe_model(&self, model: &ModelCatalogEntry, kind: ModelOperation) -> ProbeRecord {
        let probed_at_ms = probe::now_ms();
        let directory = model.install_directory(&self.model_root);
        let mut failure = None;
        let checksums = model_artifact_refs(model)
            .into_iter()
            .map(|artifact| {
                let path = directory.join(&artifact.filename);
                let result = checksum_result(&artifact, &path, kind);
                if failure.is_none() {
                    failure = match &result {
                        ChecksumResult::Mismatched { actual_sha256 } => {
                            Some(ModelFailure::ChecksumMismatch {
                                path: path.display().to_string(),
                                expected: artifact.expected_sha256.clone().unwrap_or_default(),
                                actual: actual_sha256.clone(),
                            })
                        }
                        ChecksumResult::Unreadable { message } => Some(ModelFailure::Io {
                            message: message.clone(),
                        }),
                        _ => None,
                    };
                }
                ArtifactChecksum {
                    filename: artifact.filename,
                    expected_sha256: artifact.expected_sha256,
                    result,
                }
            })
            .collect();
        let backends = model
            .candidate_backends
            .iter()
            .map(|backend_id| {
                let result = self.probe_backend(model, backend_id);
                if let (None, BackendCheck::Failed { message }) = (&failure, &result) {
                    failure = Some(ModelFailure::BackendProbe {
                        backend_id: backend_id.clone(),
                        message: message.clone(),
                    });
                }
                BackendProbeResult {
                    backend_id: backend_id.clone(),
                    result,
                }
            })
            .collect();
        ProbeRecord {
            kind,
            probed_at_ms,
            revision: model.revision.clone(),
            checksums,
            backends,
            outcome: failure.map_or(ProbeOutcome::Passed, |reason| ProbeOutcome::Failed {
                reason,
            }),
        }
    }

    fn probe_backend(&self, model: &ModelCatalogEntry, backend_id: &str) -> BackendCheck {
        let Some(probe) = &self.backend_probe else {
            return BackendCheck::NotRegistered;
        };
        match self
            .model_ref_from_catalog(&model.id, backend_id)
            .map_err(|error| error.to_string())
            .and_then(|model_ref| probe.probe(&model.task, &model_ref))
        {
            Ok(()) => BackendCheck::Passed,
            Err(message) => BackendCheck::Failed { message },
        }
    }

    pub fn cancel(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        let record = self.record(id)?;
        self.ensure_active(&record, id)?;
        record.control.cancel();
        self.update(id, |progress| progress.state = DownloadState::Cancelled)
    }

    fn run(&self, id: DownloadId, model: ModelCatalogEntry, record: Arc<TaskRecord>) {
        let mut retries = 0;
        let result = loop {
            match self.download_model(id, &model, &record) {
                Ok(()) => break Ok(()),
                Err(error) if is_retryable(&error) && retries < 3 => {
                    retries += 1;
                    let _ = self.update(id, |progress| {
                        progress.retry_count = retries;
                        progress.state = DownloadState::Queued;
                    });
                    std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(retries)));
                    if record.control.checkpoint().is_err() {
                        break Err(DownloadError::Cancelled);
                    }
                }
                Err(error) => break Err(error),
            }
        };
        match result {
            Ok(()) if record.control.is_cancelled() => {
                let _ = self.update(id, |progress| progress.state = DownloadState::Cancelled);
            }
            Ok(()) => {
                let _ = self.update(id, |progress| progress.state = DownloadState::Verifying);
                let probed = self.probe_model(&model, ModelOperation::Download);
                if let ProbeOutcome::Failed { reason } = probed.outcome {
                    let _ = fs::remove_dir_all(model.install_directory(&self.model_root));
                    self.fail_download(id, &model, reason);
                    return;
                }
                // Written even when cancelled now: the probed files stay
                // installed.
                let written = self.write_record(
                    &model,
                    ModelRecord {
                        probe: Some(probed),
                        failure: None,
                    },
                );
                match written {
                    Err(error) => self.fail_download(
                        id,
                        &model,
                        ModelFailure::Io {
                            message: error.to_string(),
                        },
                    ),
                    Ok(()) if record.control.is_cancelled() => {
                        let _ =
                            self.update(id, |progress| progress.state = DownloadState::Cancelled);
                    }
                    Ok(()) => {
                        let _ = self.update(id, |progress| {
                            progress.state = DownloadState::Completed;
                            progress.artifact_index = progress.artifact_count;
                            progress.eta_seconds = Some(0);
                        });
                    }
                }
            }
            Err(_error) if record.control.is_cancelled() => {
                let _ = self.update(id, |progress| progress.state = DownloadState::Cancelled);
            }
            Err(error) => match ModelFailure::from_download(&error) {
                Some(reason) => self.fail_download(id, &model, reason),
                None => {
                    let _ = self.update(id, |progress| progress.state = DownloadState::Cancelled);
                }
            },
        }
    }

    fn spawn_worker(
        &self,
        id: DownloadId,
        model: ModelCatalogEntry,
        record: Arc<TaskRecord>,
    ) -> Result<(), ModelManagerError> {
        record.worker_started.store(true, Ordering::Release);
        let manager = self.clone();
        std::thread::Builder::new()
            .name(format!("meetnote-download-{}", id.0))
            .spawn(move || manager.run(id, model, record))
            .map_err(|error| ModelManagerError::Download(Box::new(DownloadError::Io(error))))?;
        Ok(())
    }

    fn download_model(
        &self,
        id: DownloadId,
        model: &ModelCatalogEntry,
        record: &TaskRecord,
    ) -> Result<(), DownloadError> {
        record.control.checkpoint()?;
        self.update(id, |progress| progress.state = DownloadState::Downloading)
            .map_err(|_| DownloadError::Io(std::io::Error::other("download task lock poisoned")))?;
        let paths = model.artifact_paths(&self.model_root);
        if model.artifacts.is_empty() {
            let rate = TransferRate::default();
            download_file_controlled_with_size(
                &model.source,
                &paths[0],
                model.sha256.as_deref(),
                model.size_bytes,
                &record.control,
                |completed, total| {
                    let speed = rate.sample(completed);
                    let _ = self.update(id, |progress| {
                        progress.completed_bytes = completed;
                        progress.speed_bytes_per_second = speed;
                        progress.total_bytes = total.or(model.size_bytes);
                        progress.artifact_index = 1;
                        progress.eta_seconds =
                            eta_seconds(completed, total.or(model.size_bytes), speed);
                    });
                },
            )?;
        } else {
            let artifacts: Vec<_> = model
                .artifacts
                .iter()
                .map(|artifact| BundleArtifact {
                    filename: artifact.filename.clone(),
                    source: artifact.source.clone(),
                    expected_sha256: artifact.sha256.clone(),
                    archive_member: artifact.archive_member.clone(),
                })
                .collect();
            download_bundle_with_progress(
                &artifacts,
                model.install_directory(&self.model_root),
                &record.control,
                {
                    let mut rate = TransferRate::default();
                    let mut rate_artifact = usize::MAX;
                    move |index, count, completed, total| {
                        if index != rate_artifact {
                            rate = TransferRate::default();
                            rate_artifact = index;
                        }
                        let speed = rate.sample(completed);
                        let _ = self.update(id, |progress| {
                            progress.completed_bytes = completed;
                            progress.speed_bytes_per_second = speed;
                            progress.total_bytes = total;
                            progress.artifact_index = index.max(1);
                            progress.artifact_count = count;
                            progress.eta_seconds = eta_seconds(completed, total, speed);
                        });
                    }
                },
            )?;
        }
        Ok(())
    }

    fn update<F>(&self, id: DownloadId, change: F) -> Result<(), ModelManagerError>
    where
        F: FnOnce(&mut DownloadProgress),
    {
        let record = self.record(id)?;
        let event = {
            let mut progress = record
                .progress
                .lock()
                .map_err(|_| ModelManagerError::LockPoisoned)?;
            change(&mut progress);
            let event = DownloadEvent::Updated(progress.clone());
            let mut async_subscribers = record
                .async_subscribers
                .lock()
                .map_err(|_| ModelManagerError::LockPoisoned)?;
            async_subscribers.retain(|sender| sender.unbounded_send(event.clone()).is_ok());
            if progress.state.is_terminal() {
                async_subscribers.clear();
            }
            drop(async_subscribers);
            lock_unpoisoned(&self.all_subscribers)
                .retain(|sender| sender.unbounded_send(event.clone()).is_ok());
            event
        };
        let mut subscribers = record
            .subscribers
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        subscribers.retain(|sender| sender.send(event.clone()).is_ok());
        drop(subscribers);
        self.persist_tasks()
    }

    fn has_active_download(&self, model_id: &str) -> Result<bool, ModelManagerError> {
        Ok(self.list()?.into_iter().any(|progress| {
            progress.model_id == model_id
                && matches!(
                    progress.state,
                    DownloadState::Queued
                        | DownloadState::Downloading
                        | DownloadState::Paused
                        | DownloadState::Verifying
                )
        }))
    }

    fn ensure_active(&self, record: &TaskRecord, id: DownloadId) -> Result<(), ModelManagerError> {
        let progress = record
            .progress
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        if matches!(
            progress.state,
            DownloadState::Queued
                | DownloadState::Downloading
                | DownloadState::Paused
                | DownloadState::Verifying
        ) {
            Ok(())
        } else {
            Err(ModelManagerError::TaskNotActive(id))
        }
    }

    fn record(&self, id: DownloadId) -> Result<Arc<TaskRecord>, ModelManagerError> {
        self.tasks
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .get(&id)
            .cloned()
            .ok_or(ModelManagerError::TaskNotFound(id))
    }

    fn persist_tasks(&self) -> Result<(), ModelManagerError> {
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        let progress = tasks
            .values()
            .map(|record| {
                record
                    .progress
                    .lock()
                    .map_err(|_| ModelManagerError::LockPoisoned)
                    .map(|progress| progress.clone())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(parent) = self.task_state_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| ModelManagerError::Download(Box::new(DownloadError::Io(error))))?;
        }
        let content = serde_json::to_vec_pretty(&progress).map_err(|error| {
            ModelManagerError::Download(Box::new(DownloadError::Io(std::io::Error::other(error))))
        })?;
        let temporary = self.task_state_path.with_extension("json.tmp");
        fs::write(&temporary, content)
            .map_err(|error| ModelManagerError::Download(Box::new(DownloadError::Io(error))))?;
        fs::rename(&temporary, &self.task_state_path)
            .map_err(|error| ModelManagerError::Download(Box::new(DownloadError::Io(error))))
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn eta_seconds(completed: u64, total: Option<u64>, speed: Option<u64>) -> Option<u64> {
    let remaining = total?.saturating_sub(completed);
    let speed = speed.filter(|speed| *speed > 0)?;
    Some(remaining.div_ceil(speed))
}

fn is_retryable(error: &DownloadError) -> bool {
    matches!(error, DownloadError::Request(_) | DownloadError::Io(_))
}

fn load_task_progress(path: &Path) -> Vec<DownloadProgress> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn checksum_result(
    artifact: &contracts::ModelArtifactRef,
    path: &Path,
    kind: ModelOperation,
) -> ChecksumResult {
    let Some(expected) = &artifact.expected_sha256 else {
        return ChecksumResult::NotDeclared;
    };
    match (kind, artifact.archive_member.is_some()) {
        (ModelOperation::Download, true) => ChecksumResult::ArchiveMatched,
        (ModelOperation::Download, false) => ChecksumResult::Matched,
        (ModelOperation::Reprobe, true) => ChecksumResult::ArchiveNotRechecked,
        (ModelOperation::Reprobe, false) => match probe::file_sha256(path) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => ChecksumResult::Matched,
            Ok(actual) => ChecksumResult::Mismatched {
                actual_sha256: actual,
            },
            Err(error) => ChecksumResult::Unreadable {
                message: format!("{}: {error}", path.display()),
            },
        },
    }
}

fn model_artifact_refs(model: &ModelCatalogEntry) -> Vec<contracts::ModelArtifactRef> {
    if !model.artifacts.is_empty() {
        return model
            .artifacts
            .iter()
            .map(|artifact| contracts::ModelArtifactRef {
                filename: artifact.filename.clone(),
                expected_sha256: artifact.sha256.clone(),
                archive_member: artifact.archive_member.clone(),
            })
            .collect();
    }
    vec![contracts::ModelArtifactRef {
        filename: model
            .artifact_filenames()
            .into_iter()
            .next()
            .unwrap_or_else(|| "model.bin".into()),
        expected_sha256: model.sha256.clone(),
        archive_member: None,
    }]
}

#[derive(Debug)]
struct TransferRate {
    started_at: Instant,
}

impl Default for TransferRate {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl TransferRate {
    fn sample(&self, completed_bytes: u64) -> Option<u64> {
        let elapsed = self.started_at.elapsed().as_secs_f64();
        (elapsed > 0.0).then_some((completed_bytes as f64 / elapsed) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelCatalog, ModelCatalogEntry};

    fn fixture_catalog() -> ModelCatalog {
        ModelCatalog {
            schema_version: "meetnote.model-catalog.v1".into(),
            models: vec![ModelCatalogEntry {
                id: "fixture".into(),
                task: "asr".into(),
                model_family: "fixture".into(),
                candidate_backends: vec!["fake".into()],
                revision: Some("fixture-v1".into()),
                source_format: "bin".into(),
                source: "local/fixture.bin".into(),
                size_bytes: Some(4),
                estimated_vram_bytes: None,
                sha256: Some("fixture-sha256".into()),
                license: "test".into(),
                requires_auth: false,
                status: "ready".into(),
                artifact_files: vec![],
                artifacts: vec![],
            }],
        }
    }

    #[test]
    fn browsing_filters_by_task_and_backend_and_reports_missing() {
        let root = std::env::temp_dir().join(format!("meetnote-model-manager-{}", unique_suffix()));
        let manager = ModelManager::new(fixture_catalog(), &root);
        let models = manager.browse(Some("asr"), Some("fake"));

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].install_state, crate::ModelInstallState::Missing);
        assert!(manager.browse(Some("diarization"), None).is_empty());
    }

    #[test]
    fn cancellation_is_exposed_as_a_terminal_download_state() {
        let root = std::env::temp_dir().join(format!("meetnote-model-manager-{}", unique_suffix()));
        let manager = ModelManager::new(fixture_catalog(), &root);
        let task = manager.start("fixture").unwrap();
        manager.cancel(task.id).unwrap();

        assert_eq!(
            manager.status(task.id).unwrap().state,
            DownloadState::Cancelled
        );
    }

    #[test]
    fn restores_interrupted_downloads_as_resumable_paused_tasks() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-model-manager-restore-{}",
            unique_suffix()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let progress = vec![DownloadProgress {
            id: DownloadId(9),
            model_id: "fixture".into(),
            state: DownloadState::Downloading,
            completed_bytes: 12,
            speed_bytes_per_second: Some(3),
            total_bytes: Some(100),
            artifact_index: 1,
            artifact_count: 1,
            eta_seconds: None,
            retry_count: 0,
        }];
        std::fs::write(
            root.join(".download-tasks.json"),
            serde_json::to_vec(&progress).unwrap(),
        )
        .unwrap();

        let manager = ModelManager::new(fixture_catalog(), &root);

        let restored = manager.status(DownloadId(9)).unwrap();
        assert_eq!(restored.state, DownloadState::Paused);
        assert_eq!(restored.completed_bytes, 12);
        assert!(matches!(
            manager.start("fixture"),
            Err(ModelManagerError::AlreadyActive(model_id)) if model_id == "fixture"
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_a_model_ref_from_catalog_without_hardcoded_artifact_paths() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-model-manager-resolve-{}",
            unique_suffix()
        ));
        let manager = ModelManager::new(fixture_catalog(), &root);

        let model = manager.configured_model_ref("asr", "fake", None).unwrap();

        assert_eq!(model.model_id, "fixture");
        assert_eq!(model.revision.as_deref(), Some("fixture-v1"));
        assert_eq!(
            model.artifacts[0].expected_sha256.as_deref(),
            Some("fixture-sha256")
        );
        assert_eq!(
            model.artifact_path,
            root.join("asr/fixture/fixture.bin").display().to_string()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn transfer_rate_reports_a_byte_rate_for_download_progress() {
        // Started in the past: an immediate sample can see no elapsed time.
        let rate = TransferRate {
            started_at: Instant::now()
                .checked_sub(std::time::Duration::from_millis(500))
                .unwrap(),
        };

        assert!(rate.sample(1_000_000).is_some());
        assert_eq!(eta_seconds(50, Some(100), Some(25)), Some(2));
    }

    struct RecordingProbe {
        calls: Mutex<Vec<String>>,
    }

    impl BackendProbe for RecordingProbe {
        fn probe(&self, task: &str, model: &contracts::ModelRef) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{task}:{}", model.model_id));
            Ok(())
        }
    }

    #[test]
    fn probes_each_registered_backend_after_artifacts_are_installed() {
        let root =
            std::env::temp_dir().join(format!("meetnote-model-manager-probe-{}", unique_suffix()));
        let manager = ModelManager::with_backend_probe(
            fixture_catalog(),
            &root,
            Some(Arc::new(RecordingProbe {
                calls: Mutex::new(Vec::new()),
            })),
        );
        let model = manager.catalog.find("fixture").unwrap();
        let path = model.artifact_paths(&root)[0].clone();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture").unwrap();

        let record = manager.probe_model(model, ModelOperation::Download);

        assert!(record.passed(), "{record:?}");
        assert_eq!(
            record.backends,
            vec![BackendProbeResult {
                backend_id: "fake".into(),
                result: BackendCheck::Passed,
            }]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completes_download_only_after_backend_probe() {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request);
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nfixture",
            )
            .unwrap();
        });

        let mut catalog = fixture_catalog();
        catalog.models[0].source = format!("http://{address}/fixture.bin");
        catalog.models[0].size_bytes = Some(7);
        catalog.models[0].sha256 = None;
        let root = std::env::temp_dir().join(format!(
            "meetnote-model-manager-download-probe-{}",
            unique_suffix()
        ));
        let probe = Arc::new(RecordingProbe {
            calls: Mutex::new(Vec::new()),
        });
        let manager = ModelManager::with_backend_probe(catalog, &root, Some(probe.clone()));
        let task = manager.start("fixture").unwrap();
        let mut terminal = None;
        while let Ok(event) = task.events.recv_timeout(std::time::Duration::from_secs(2)) {
            let DownloadEvent::Updated(progress) = event;
            if matches!(
                progress.state,
                DownloadState::Completed | DownloadState::Cancelled | DownloadState::Failed(_)
            ) {
                terminal = Some(progress.state);
                break;
            }
        }

        assert_eq!(terminal, Some(DownloadState::Completed));
        assert_eq!(
            probe.calls.lock().unwrap().first().map(String::as_str),
            Some("asr:fixture")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn download_control_unblocks_a_paused_worker_and_supports_cancellation() {
        let control = DownloadControl::default();
        control.pause();
        let worker_control = control.clone();
        let worker = std::thread::spawn(move || worker_control.checkpoint());
        control.cancel();

        assert!(matches!(
            worker.join().unwrap(),
            Err(DownloadError::Cancelled)
        ));
    }

    /// Serves `body` to a single request and returns its URL.
    fn serve_once(body: &'static [u8]) -> String {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request);
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
            std::io::Write::write_all(&mut stream, body).unwrap();
        });
        format!("http://{address}/fixture.bin")
    }

    /// Drains `stream` on its own thread; panics unless it ends within
    /// `timeout`.
    fn collect_within(
        stream: UnboundedReceiver<DownloadEvent>,
        timeout: std::time::Duration,
    ) -> Vec<DownloadProgress> {
        use futures::StreamExt;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(futures::executor::block_on(stream.collect::<Vec<_>>()));
        });
        receiver
            .recv_timeout(timeout)
            .expect("download stream did not end")
            .into_iter()
            .map(|DownloadEvent::Updated(progress)| progress)
            .collect()
    }

    #[test]
    fn terminal_states_are_completed_cancelled_and_failed() {
        assert!(DownloadState::Completed.is_terminal());
        assert!(DownloadState::Cancelled.is_terminal());
        assert!(DownloadState::Failed("error".into()).is_terminal());
        for state in [
            DownloadState::Queued,
            DownloadState::Downloading,
            DownloadState::Paused,
            DownloadState::Verifying,
        ] {
            assert!(!state.is_terminal(), "{state:?}");
        }
    }

    #[test]
    fn async_download_events_arrive_in_order_and_end_at_the_terminal_state() {
        let mut catalog = fixture_catalog();
        catalog.models[0].source = serve_once(b"fixture");
        catalog.models[0].size_bytes = Some(7);
        catalog.models[0].sha256 = None;
        let root =
            std::env::temp_dir().join(format!("meetnote-model-manager-async-{}", unique_suffix()));
        let manager = ModelManager::new(catalog, &root);
        let task = manager.start("fixture").unwrap();

        let progress = collect_within(task.updates, std::time::Duration::from_secs(5));

        let mut states: Vec<_> = progress
            .iter()
            .map(|progress| progress.state.clone())
            .collect();
        states.dedup();
        assert_eq!(
            states,
            vec![
                DownloadState::Queued,
                DownloadState::Downloading,
                DownloadState::Verifying,
                DownloadState::Completed,
            ]
        );
        assert!(progress
            .windows(2)
            .all(|pair| pair[0].completed_bytes <= pair[1].completed_bytes));
        assert_eq!(progress.last().unwrap().completed_bytes, 7);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_async_subscription_to_an_ended_task_yields_its_terminal_state_and_ends() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-model-manager-async-cancel-{}",
            unique_suffix()
        ));
        let manager = ModelManager::new(fixture_catalog(), &root);
        let task = manager.start("fixture").unwrap();
        manager.cancel(task.id).unwrap();

        let live = collect_within(task.updates, std::time::Duration::from_secs(5));
        let late = collect_within(
            manager.subscribe_async(task.id).unwrap(),
            std::time::Duration::from_secs(5),
        );

        assert_eq!(live.first().unwrap().state, DownloadState::Queued);
        assert_eq!(live.last().unwrap().state, DownloadState::Cancelled);
        assert_eq!(late.last().unwrap().state, DownloadState::Cancelled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_manager_wide_async_subscription_sees_tasks_started_later() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-model-manager-async-all-{}",
            unique_suffix()
        ));
        let manager = ModelManager::new(fixture_catalog(), &root);
        let mut all = manager.subscribe_all_async();
        let task = manager.start("fixture").unwrap();
        manager.cancel(task.id).unwrap();

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match all.try_recv() {
                Ok(DownloadEvent::Updated(progress))
                    if progress.id == task.id && progress.state == DownloadState::Cancelled =>
                {
                    break
                }
                Ok(_) => {}
                Err(futures::channel::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(error) => panic!("no cancelled update for the task: {error:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("meetnote-model-{name}-{}", unique_suffix()))
    }

    /// Serves each body to one request, in order, and returns the URL.
    fn serve_sequence(bodies: Vec<&'static [u8]>) -> String {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request);
                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
                std::io::Write::write_all(&mut stream, body).unwrap();
            }
        });
        format!("http://{address}/fixture.bin")
    }

    /// A catalog whose single artifact is `body`, served once per entry of
    /// `served`.
    fn served_catalog(body: &[u8], served: Vec<&'static [u8]>) -> ModelCatalog {
        let mut catalog = fixture_catalog();
        catalog.models[0].source = serve_sequence(served);
        catalog.models[0].size_bytes = Some(body.len() as u64);
        catalog.models[0].sha256 = Some(sha256_hex(body));
        catalog
    }

    /// A backend probe that passes unless told to fail, and can hold the
    /// next probe until released.
    #[derive(Default)]
    struct SwitchProbe {
        failure: Mutex<Option<String>>,
        hold: Mutex<Option<Receiver<()>>>,
        threads: Mutex<Vec<std::thread::ThreadId>>,
    }

    impl SwitchProbe {
        fn failing(message: &str) -> Self {
            let probe = Self::default();
            probe.fail_with(Some(message));
            probe
        }

        fn fail_with(&self, message: Option<&str>) {
            *self.failure.lock().unwrap() = message.map(Into::into);
        }

        /// Holds the next probe until the returned sender sends or drops.
        fn hold_next(&self) -> Sender<()> {
            let (release, held) = mpsc::channel();
            *self.hold.lock().unwrap() = Some(held);
            release
        }
    }

    impl BackendProbe for SwitchProbe {
        fn probe(&self, _task: &str, _model: &contracts::ModelRef) -> Result<(), String> {
            self.threads
                .lock()
                .unwrap()
                .push(std::thread::current().id());
            let held = self.hold.lock().unwrap().take();
            if let Some(held) = held {
                let _ = held.recv_timeout(std::time::Duration::from_secs(5));
            }
            self.failure.lock().unwrap().clone().map_or(Ok(()), Err)
        }
    }

    fn terminal_state(task: &DownloadTask) -> DownloadState {
        loop {
            let DownloadEvent::Updated(progress) = task
                .events
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("the download did not end");
            if progress.state.is_terminal() {
                return progress.state;
            }
        }
    }

    fn finished_within(task: ReprobeTask) -> Result<ProbeRecord, ModelManagerError> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(futures::executor::block_on(task.finished));
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the re-probe did not finish")
            .expect("the re-probe worker dropped its result")
    }

    /// Writes the fixture model's single artifact as if it were installed.
    fn install(manager: &ModelManager, bytes: &[u8]) -> PathBuf {
        let model = manager.catalog.find("fixture").unwrap();
        let path = model.artifact_paths(&manager.model_root)[0].clone();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn availability(manager: &ModelManager) -> crate::ModelAvailability {
        manager.browse(None, None).remove(0)
    }

    fn directory_entries(path: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_passing_download_probe_is_recorded_beside_the_install_and_survives_a_restart() {
        let catalog = served_catalog(b"fixture", vec![b"fixture"]);
        let root = temp_root("record-pass");
        let manager = ModelManager::with_backend_probe(
            catalog.clone(),
            &root,
            Some(Arc::new(SwitchProbe::default())),
        );
        let before = probe::now_ms();

        let task = manager.start("fixture").unwrap();
        assert_eq!(terminal_state(&task), DownloadState::Completed);
        let after = probe::now_ms();

        let restarted = availability(&ModelManager::new(catalog, &root));
        let record = restarted.probe.expect("no probe record after a restart");
        assert_eq!(record.kind, ModelOperation::Download);
        assert!(record.passed(), "{record:?}");
        assert!((before..=after).contains(&record.probed_at_ms));
        assert_eq!(record.revision.as_deref(), Some("fixture-v1"));
        assert_eq!(
            record.checksums,
            vec![ArtifactChecksum {
                filename: "fixture.bin".into(),
                expected_sha256: Some(sha256_hex(b"fixture")),
                result: ChecksumResult::Matched,
            }]
        );
        assert_eq!(
            record.backends,
            vec![BackendProbeResult {
                backend_id: "fake".into(),
                result: BackendCheck::Passed,
            }]
        );
        assert_eq!(restarted.failure, None);
        assert_eq!(restarted.record_error, None);
        // Provenance hashes the whole install directory, so the record stays
        // beside it.
        assert!(root.join("asr/fixture.probe.json").is_file());
        assert_eq!(
            directory_entries(&root.join("asr/fixture")),
            ["fixture.bin"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failing_download_probe_records_its_reason_and_removes_the_install() {
        let catalog = served_catalog(b"fixture", vec![b"fixture"]);
        let root = temp_root("record-probe-fail");
        let manager = ModelManager::with_backend_probe(
            catalog.clone(),
            &root,
            Some(Arc::new(SwitchProbe::failing("unsupported layout"))),
        );
        let before = probe::now_ms();

        let task = manager.start("fixture").unwrap();
        let state = terminal_state(&task);

        assert_eq!(
            state,
            DownloadState::Failed("backend probe failed for fake: unsupported layout".into())
        );
        assert!(!root.join("asr/fixture").exists());
        let restarted = availability(&ModelManager::new(catalog, &root));
        assert_eq!(restarted.install_state, crate::ModelInstallState::Missing);
        assert_eq!(restarted.probe, None);
        let failure = restarted.failure.expect("no failure after the task ended");
        assert_eq!(failure.operation, ModelOperation::Download);
        assert_eq!(failure.revision.as_deref(), Some("fixture-v1"));
        assert!(failure.failed_at_ms >= before);
        assert_eq!(
            failure.reason,
            ModelFailure::BackendProbe {
                backend_id: "fake".into(),
                message: "unsupported layout".into(),
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_later_successful_download_clears_the_recorded_failure() {
        let catalog = served_catalog(b"fixture", vec![b"broken!", b"fixture"]);
        let root = temp_root("record-retry");
        let manager = ModelManager::with_backend_probe(
            catalog,
            &root,
            Some(Arc::new(SwitchProbe::default())),
        );

        let first = manager.start("fixture").unwrap();
        assert!(matches!(terminal_state(&first), DownloadState::Failed(_)));
        let failure = availability(&manager).failure.expect("no recorded failure");
        assert!(matches!(
            &failure.reason,
            ModelFailure::ChecksumMismatch { path, expected, actual }
                if path.ends_with("asr/fixture/fixture.part")
                    && *expected == sha256_hex(b"fixture")
                    && *actual == sha256_hex(b"broken!")
        ));

        let second = manager.start("fixture").unwrap();
        assert_eq!(terminal_state(&second), DownloadState::Completed);
        let recovered = availability(&manager);
        assert_eq!(recovered.failure, None);
        assert!(recovered.probe.is_some_and(|record| record.passed()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reprobe_rehashes_and_probes_an_installed_model_off_the_calling_thread() {
        let mut catalog = fixture_catalog();
        catalog.models[0].sha256 = Some(sha256_hex(b"fixture"));
        let root = temp_root("reprobe-pass");
        let probe = Arc::new(SwitchProbe::default());
        let manager = ModelManager::with_backend_probe(catalog, &root, Some(probe.clone()));
        install(&manager, b"fixture");
        assert_eq!(
            availability(&manager).probe,
            None,
            "installed without a record"
        );
        let mut finished = manager.subscribe_reprobes_async();

        let task = manager.reprobe("fixture").unwrap();
        let record = finished_within(task).unwrap();

        assert_eq!(record.kind, ModelOperation::Reprobe);
        assert!(record.passed(), "{record:?}");
        assert_eq!(record.checksums[0].result, ChecksumResult::Matched);
        assert_eq!(record.backends[0].result, BackendCheck::Passed);
        assert_ne!(
            probe.threads.lock().unwrap().as_slice(),
            [std::thread::current().id()]
        );
        assert_eq!(availability(&manager).probe, Some(record.clone()));
        assert_eq!(
            finished.try_recv().unwrap(),
            ReprobeFinished {
                model_id: "fixture".into(),
                record,
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failing_reprobe_reports_its_reason_and_keeps_the_install_usable() {
        let mut catalog = fixture_catalog();
        catalog.models[0].sha256 = Some(sha256_hex(b"fixture"));
        let root = temp_root("reprobe-fail");
        let probe = Arc::new(SwitchProbe::default());
        let manager = ModelManager::with_backend_probe(catalog, &root, Some(probe.clone()));
        let artifact = install(&manager, b"fixture");
        probe.fail_with(Some("session rejected"));

        let failed = finished_within(manager.reprobe("fixture").unwrap()).unwrap();

        let reason = ModelFailure::BackendProbe {
            backend_id: "fake".into(),
            message: "session rejected".into(),
        };
        assert_eq!(
            failed.outcome,
            ProbeOutcome::Failed {
                reason: reason.clone()
            }
        );
        assert!(artifact.is_file());
        let entry = availability(&manager);
        assert_eq!(entry.install_state, crate::ModelInstallState::Installed);
        assert_eq!(entry.probe, Some(failed));
        let failure = entry.failure.expect("no recorded failure");
        assert_eq!(failure.operation, ModelOperation::Reprobe);
        assert_eq!(failure.reason, reason);
        // The processing preflight decides whether processing can run.
        assert!(manager.model_ref("fixture", "fake").is_ok());

        probe.fail_with(None);
        let passed = finished_within(manager.reprobe("fixture").unwrap()).unwrap();
        assert!(passed.passed());
        assert_eq!(availability(&manager).failure, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_reprobe_reports_an_artifact_that_no_longer_matches_its_checksum() {
        let mut catalog = fixture_catalog();
        catalog.models[0].sha256 = Some(sha256_hex(b"fixture"));
        let root = temp_root("reprobe-checksum");
        let manager = ModelManager::with_backend_probe(
            catalog,
            &root,
            Some(Arc::new(SwitchProbe::default())),
        );
        let artifact = install(&manager, b"tampered");

        let record = finished_within(manager.reprobe("fixture").unwrap()).unwrap();

        assert_eq!(
            record.checksums[0].result,
            ChecksumResult::Mismatched {
                actual_sha256: sha256_hex(b"tampered"),
            }
        );
        assert_eq!(
            record.outcome,
            ProbeOutcome::Failed {
                reason: ModelFailure::ChecksumMismatch {
                    path: artifact.display().to_string(),
                    expected: sha256_hex(b"fixture"),
                    actual: sha256_hex(b"tampered"),
                },
            }
        );
        assert!(artifact.is_file(), "a re-probe never deletes files");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_members_are_checked_through_their_archive_only_while_downloading() {
        let mut catalog = fixture_catalog();
        catalog.models[0].artifacts = vec![
            crate::CatalogArtifact {
                filename: "model.onnx".into(),
                source: "local/archive.tar.bz2".into(),
                sha256: Some(sha256_hex(b"archive")),
                archive_member: Some("root/model.onnx".into()),
            },
            crate::CatalogArtifact {
                filename: "tokens.txt".into(),
                source: "local/tokens.txt".into(),
                sha256: None,
                archive_member: None,
            },
        ];
        let root = temp_root("reprobe-archive");
        let manager = ModelManager::new(catalog, &root);
        let directory = root.join("asr/fixture");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("model.onnx"), b"member").unwrap();
        std::fs::write(directory.join("tokens.txt"), b"tokens").unwrap();
        let results = |record: &ProbeRecord| {
            record
                .checksums
                .iter()
                .map(|checksum| checksum.result.clone())
                .collect::<Vec<_>>()
        };

        let model = manager.catalog.find("fixture").unwrap();
        let downloaded = manager.probe_model(model, ModelOperation::Download);
        let reprobed = finished_within(manager.reprobe("fixture").unwrap()).unwrap();

        assert_eq!(
            results(&downloaded),
            [ChecksumResult::ArchiveMatched, ChecksumResult::NotDeclared]
        );
        assert_eq!(
            results(&reprobed),
            [
                ChecksumResult::ArchiveNotRechecked,
                ChecksumResult::NotDeclared
            ]
        );
        assert_eq!(
            reprobed.backends,
            vec![BackendProbeResult {
                backend_id: "fake".into(),
                result: BackendCheck::NotRegistered,
            }]
        );
        assert!(reprobed.passed());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reprobe_rejects_unknown_missing_and_already_probing_models() {
        let root = temp_root("reprobe-reject");
        let probe = Arc::new(SwitchProbe::default());
        let manager =
            ModelManager::with_backend_probe(fixture_catalog(), &root, Some(probe.clone()));

        assert!(matches!(
            manager.reprobe("unknown"),
            Err(ModelManagerError::ModelNotFound(id)) if id == "unknown"
        ));
        assert!(matches!(
            manager.reprobe("fixture"),
            Err(ModelManagerError::ModelNotInstalled(id)) if id == "fixture"
        ));

        install(&manager, b"fixture");
        let release = probe.hold_next();
        let running = manager.reprobe("fixture").unwrap();
        assert!(matches!(
            manager.reprobe("fixture"),
            Err(ModelManagerError::AlreadyActive(id)) if id == "fixture"
        ));
        release.send(()).unwrap();
        finished_within(running).unwrap();
        finished_within(manager.reprobe("fixture").unwrap()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn browse_exposes_catalog_provenance_and_reports_an_unreadable_record() {
        let root = temp_root("record-unreadable");
        let manager = ModelManager::new(fixture_catalog(), &root);
        let record_path = manager
            .catalog
            .find("fixture")
            .unwrap()
            .probe_record_path(&root);
        std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
        std::fs::write(&record_path, b"{not json").unwrap();

        let entry = availability(&manager);

        assert_eq!(entry.model.license, "test");
        assert_eq!(entry.model.source, "local/fixture.bin");
        assert_eq!(entry.model.sha256.as_deref(), Some("fixture-sha256"));
        assert_eq!(entry.probe, None);
        assert_eq!(
            entry
                .record_error
                .expect("the invalid record was not reported")
                .path,
            record_path
        );

        install(&manager, b"fixture");
        let record = finished_within(manager.reprobe("fixture").unwrap()).unwrap();
        let replaced = availability(&manager);
        assert_eq!(replaced.record_error, None);
        assert_eq!(replaced.probe, Some(record));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
