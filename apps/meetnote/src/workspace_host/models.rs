use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use contracts::ModelRef;
use futures::Stream;
use futures::channel::mpsc::UnboundedReceiver;
use models::manager::{DownloadEvent, DownloadId, DownloadProgress};
use models::{ModelAvailability, ModelManager};
use rune::Tx;
use settings::AppSettings;

use super::{Task, WorkspaceHost, diarization_stages::StageDraft, processing::ProcessingConfig};
use crate::{MeetNote, Route};

#[derive(Default)]
pub(crate) struct ModelHost {
    session: Option<Session>,
    task: Option<Task<Reply>>,
    refresh: bool,
    pub error: Option<String>,
    pub expanded: BTreeSet<String>,
}

struct Session {
    manager: ModelManager,
    events: UnboundedReceiver<DownloadEvent>,
    catalog: Catalog,
    downloads: BTreeMap<String, DownloadProgress>,
}

pub(crate) struct Catalog {
    pub entries: Vec<ModelAvailability>,
    selected: [Option<ModelRef>; 2],
}

impl Catalog {
    fn load(manager: &ModelManager, settings: AppSettings) -> Self {
        Self {
            entries: manager.browse(None, None),
            selected: [settings.asr_model, settings.diarization_model],
        }
    }

    pub fn selected(&self, entry: &ModelAvailability) -> Option<&str> {
        let model = &entry.model;
        self.selected.iter().flatten().find_map(|selected| {
            (model.id == selected.model_id
                && model.supports_backend(&selected.backend_id)
                && if model.artifacts.is_empty() {
                    entry
                        .artifact_paths
                        .iter()
                        .any(|path| path == std::path::Path::new(&selected.artifact_path))
                } else {
                    entry
                        .artifact_paths
                        .iter()
                        .zip(&model.artifacts)
                        .all(|(path, artifact)| {
                            *path
                                == std::path::Path::new(&selected.artifact_path)
                                    .join(&artifact.filename)
                        })
                })
            .then_some(selected.backend_id.as_str())
        })
    }
}

enum Reply {
    Opened(Session),
    Refreshed(Catalog),
    Changed(Result<Change, String>),
}

enum Change {
    Notice(&'static str),
    Selection(Option<Box<StageDraft>>),
}

#[derive(Clone)]
pub(crate) enum ModelAction {
    Download(String),
    Pause(DownloadId),
    Resume(DownloadId),
    Cancel(DownloadId),
    Probe(String),
    Select { id: String, backend: String },
}

impl ModelAction {
    fn run(self, manager: &ModelManager, config: &ProcessingConfig) -> Result<Change, String> {
        match self {
            Self::Download(id) => manager.start(&id).map(|_| "Model download started"),
            Self::Pause(id) => manager.pause(id).map(|_| "Model download paused"),
            Self::Resume(id) => manager.resume(id).map(|_| "Model download resumed"),
            Self::Cancel(id) => manager.cancel(id).map(|_| "Model cancellation requested"),
            Self::Probe(id) => {
                let task = manager.reprobe(&id).map_err(|error| error.to_string())?;
                let record = futures::executor::block_on(task.finished)
                    .map_err(|_| "The model probe stopped before reporting a result".to_owned())?
                    .map_err(|error| error.to_string())?;
                return Ok(Change::Notice(if record.passed() {
                    "Model probe passed"
                } else {
                    "Model probe failed"
                }));
            }
            Self::Select { id, backend } => {
                let model = manager
                    .model_ref(&id, &backend)
                    .map_err(|error| error.to_string())?;
                let entry = manager
                    .catalog()
                    .find(&id)
                    .ok_or_else(|| format!("Unknown model {id}"))?;
                let settings = config.settings_service()?;
                let previous = settings.snapshot().map_err(|error| error.to_string())?;
                let mut updated = previous.clone();
                match entry.task.as_str() {
                    "asr" => {
                        updated.asr_backend_id = Some(backend);
                        updated.asr_model = Some(model);
                    }
                    "diarization" => {
                        if updated.diarization_model.as_ref() != Some(&model) {
                            updated.diarization_stages = Default::default();
                        }
                        updated.diarization_backend_id = Some(backend);
                        updated.diarization_model = Some(model);
                    }
                    task => return Err(format!("Model task {task} has no audio setting")),
                }
                let saved = settings
                    .update_if_equal(&previous, updated)
                    .map_err(|error| error.to_string())?;
                return Ok(Change::Selection(
                    (entry.task == "diarization")
                        .then(|| Box::new(StageDraft::load(config, &saved))),
                ));
            }
        }
        .map(Change::Notice)
        .map_err(|error| error.to_string())
    }
}

impl ModelHost {
    pub fn catalog(&self) -> Option<&Catalog> {
        self.session.as_ref().map(|session| &session.catalog)
    }

    pub fn progress(&self, id: &str) -> Option<&DownloadProgress> {
        self.session.as_ref()?.downloads.get(id)
    }

    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    fn start(
        &mut self,
        tx: &mut Tx,
        work: impl FnOnce() -> Result<Reply, String> + Send + 'static,
    ) {
        self.task = Some(Box::pin(runtime::spawn_blocking(work)));
        tx.wake_after(Duration::ZERO);
    }

    fn poll_downloads(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        let mut cx = Context::from_waker(waker.unwrap_or_else(|| Waker::noop()));
        let mut changed = false;
        for _ in 0..32 {
            match Pin::new(&mut session.events).poll_next(&mut cx) {
                Poll::Ready(Some(DownloadEvent::Updated(progress))) => {
                    self.refresh |= progress.state.is_terminal();
                    if session
                        .downloads
                        .get(&progress.model_id)
                        .is_none_or(|last| last.id <= progress.id)
                    {
                        session
                            .downloads
                            .insert(progress.model_id.clone(), progress);
                        changed = true;
                    }
                }
                Poll::Pending | Poll::Ready(None) => return changed,
            }
        }
        tx.wake_after(Duration::ZERO);
        changed
    }
}

impl MeetNote {
    pub(crate) fn change_model(&mut self, action: ModelAction, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        if host.models.busy() {
            return;
        }
        if let Some(session) = &host.models.session {
            let manager = session.manager.clone();
            let config = host.processing_config.clone();
            host.models.start(tx, move || {
                Ok(Reply::Changed(action.run(&manager, &config)))
            });
        }
    }

    pub(crate) fn refresh_models(&mut self, tx: &mut Tx) {
        self.workspace_host.models.error = None;
        self.workspace_host.models.refresh = true;
        tx.wake_after(Duration::ZERO);
    }

    pub(super) fn poll_models(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let mut changed = host.models.poll_downloads(host.waker.as_ref(), tx);
        if let Some(result) = host
            .models
            .task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, host.waker.as_ref(), tx))
        {
            host.models.task = None;
            changed = true;
            match result {
                Ok(Reply::Opened(session)) => host.models.session = Some(session),
                Ok(Reply::Refreshed(catalog)) => {
                    if let Some(session) = &mut host.models.session {
                        session.catalog = catalog;
                    }
                }
                Ok(Reply::Changed(result)) => {
                    host.models.refresh = true;
                    match result {
                        Ok(Change::Notice(message)) => self.notify(message, tx),
                        Ok(Change::Selection(stages)) => {
                            let replaced = match (stages, &mut host.preferences.draft) {
                                (Some(stages), Some(draft)) => draft.replace_stages(*stages),
                                _ => false,
                            };
                            self.notify(
                                if replaced {
                                    "Model selection saved. Stage edits were cleared."
                                } else {
                                    "Model selection saved"
                                },
                                tx,
                            );
                        }
                        Err(error) => self.notify(error, tx),
                    }
                }
                Err(error) => host.models.error = Some(error),
            }
        }
        let host = &mut self.workspace_host;
        if host.models.busy() || host.models.error.is_some() || host.live.is_none() {
            return changed;
        }
        let config = host.processing_config.clone();
        if let Some(session) = &host.models.session {
            if std::mem::take(&mut host.models.refresh) {
                let manager = session.manager.clone();
                host.models.start(tx, move || {
                    Ok(Reply::Refreshed(Catalog::load(
                        &manager,
                        config.settings()?,
                    )))
                });
            }
        } else if self.route == Route::Models {
            host.models.refresh = false;
            host.models.start(tx, move || {
                let settings = config.settings()?;
                let manager = config.audio(&settings)?.manager.clone();
                let events = manager.subscribe_all_async();
                let downloads = manager
                    .list()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|progress| (progress.model_id.clone(), progress))
                    .collect();
                let catalog = Catalog::load(&manager, settings);
                Ok(Reply::Opened(Session {
                    manager,
                    events,
                    catalog,
                    downloads,
                }))
            });
        }
        changed
    }
}
