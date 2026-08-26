use crate::download::{
    download_bundle_with_progress, download_file_controlled, BundleArtifact, DownloadControl,
    DownloadError,
};
use crate::{ModelCatalog, ModelCatalogEntry};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct DownloadId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub id: DownloadId,
    pub model_id: String,
    pub state: DownloadState,
    pub completed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub artifact_index: usize,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    Updated(DownloadProgress),
}

pub struct DownloadTask {
    pub id: DownloadId,
    pub events: Receiver<DownloadEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelManagerError {
    #[error("model not found in catalog: {0}")]
    ModelNotFound(String),
    #[error("model requires authentication: {0}")]
    AuthenticationRequired(String),
    #[error("model is already installed: {0}")]
    AlreadyInstalled(String),
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
    Download(#[from] DownloadError),
    #[error("download task lock poisoned")]
    LockPoisoned,
}

struct TaskRecord {
    control: DownloadControl,
    progress: Mutex<DownloadProgress>,
    subscribers: Mutex<Vec<Sender<DownloadEvent>>>,
}

#[derive(Clone)]
pub struct ModelManager {
    catalog: Arc<ModelCatalog>,
    model_root: PathBuf,
    next_id: Arc<AtomicU64>,
    tasks: Arc<Mutex<BTreeMap<DownloadId, Arc<TaskRecord>>>>,
}

impl ModelManager {
    pub fn new(catalog: ModelCatalog, model_root: impl AsRef<Path>) -> Self {
        Self {
            catalog: Arc::new(catalog),
            model_root: model_root.as_ref().to_path_buf(),
            next_id: Arc::new(AtomicU64::new(0)),
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
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
        if model.availability(&self.model_root).install_state == crate::ModelInstallState::Installed
        {
            return Err(ModelManagerError::AlreadyInstalled(model.id));
        }
        if self.has_active_download(&model.id)? {
            return Err(ModelManagerError::AlreadyActive(model.id));
        }

        let id = DownloadId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let artifact_count = model.artifact_filenames().len();
        let record = Arc::new(TaskRecord {
            control: DownloadControl::default(),
            progress: Mutex::new(DownloadProgress {
                id,
                model_id: model.id.clone(),
                state: DownloadState::Queued,
                completed_bytes: 0,
                total_bytes: model.size_bytes,
                artifact_index: 0,
                artifact_count,
            }),
            subscribers: Mutex::new(Vec::new()),
        });
        self.tasks
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?
            .insert(id, record.clone());
        let receiver = self.subscribe(id)?;
        let manager = self.clone();
        std::thread::Builder::new()
            .name(format!("meetnote-download-{}", id.0))
            .spawn(move || manager.run(id, model, record))
            .map_err(|error| ModelManagerError::Download(DownloadError::Io(error)))?;
        Ok(DownloadTask {
            id,
            events: receiver,
        })
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
        if !model.candidate_backends.iter().any(|id| id == backend_id) {
            return Err(ModelManagerError::BackendNotSupported {
                model_id: model_id.into(),
                backend_id: backend_id.into(),
            });
        }
        if model.availability(&self.model_root).install_state != crate::ModelInstallState::Installed
        {
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
            revision: None,
            artifact_path: artifact_path.display().to_string(),
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
        record.control.resume();
        self.update(id, |progress| progress.state = DownloadState::Downloading)
    }

    pub fn cancel(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        let record = self.record(id)?;
        self.ensure_active(&record, id)?;
        record.control.cancel();
        self.update(id, |progress| progress.state = DownloadState::Cancelled)
    }

    fn run(&self, id: DownloadId, model: ModelCatalogEntry, record: Arc<TaskRecord>) {
        let result = self.download_model(id, &model, &record);
        match result {
            Ok(()) if record.control.is_cancelled() => {
                let _ = self.update(id, |progress| progress.state = DownloadState::Cancelled);
            }
            Ok(()) => {
                let _ = self.update(id, |progress| {
                    progress.state = DownloadState::Completed;
                    progress.artifact_index = progress.artifact_count;
                });
            }
            Err(DownloadError::Cancelled) => {
                let _ = self.update(id, |progress| progress.state = DownloadState::Cancelled);
            }
            Err(error) => {
                let _ = self.update(id, |progress| {
                    progress.state = DownloadState::Failed(error.to_string())
                });
            }
        }
    }

    fn download_model(
        &self,
        id: DownloadId,
        model: &ModelCatalogEntry,
        record: &TaskRecord,
    ) -> Result<(), DownloadError> {
        self.update(id, |progress| progress.state = DownloadState::Downloading)
            .map_err(|_| DownloadError::Io(std::io::Error::other("download task lock poisoned")))?;
        let paths = model.artifact_paths(&self.model_root);
        if model.artifacts.is_empty() {
            download_file_controlled(
                &model.source,
                &paths[0],
                model.sha256.as_deref(),
                &record.control,
                |completed, total| {
                    let _ = self.update(id, |progress| {
                        progress.completed_bytes = completed;
                        progress.total_bytes = total.or(model.size_bytes);
                        progress.artifact_index = 1;
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
                |index, count, completed, total| {
                    let _ = self.update(id, |progress| {
                        progress.completed_bytes = completed;
                        progress.total_bytes = total;
                        progress.artifact_index = index.max(1);
                        progress.artifact_count = count;
                    });
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
        let progress = {
            let mut progress = record
                .progress
                .lock()
                .map_err(|_| ModelManagerError::LockPoisoned)?;
            change(&mut progress);
            progress.clone()
        };
        let event = DownloadEvent::Updated(progress);
        let mut subscribers = record
            .subscribers
            .lock()
            .map_err(|_| ModelManagerError::LockPoisoned)?;
        subscribers.retain(|sender| sender.send(event.clone()).is_ok());
        Ok(())
    }

    fn has_active_download(&self, model_id: &str) -> Result<bool, ModelManagerError> {
        Ok(self.list()?.into_iter().any(|progress| {
            progress.model_id == model_id
                && matches!(
                    progress.state,
                    DownloadState::Queued | DownloadState::Downloading | DownloadState::Paused
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
            DownloadState::Queued | DownloadState::Downloading | DownloadState::Paused
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
                source_format: "bin".into(),
                source: "local/fixture.bin".into(),
                size_bytes: Some(4),
                sha256: None,
                license: "test".into(),
                requires_auth: false,
                status: "test".into(),
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

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
