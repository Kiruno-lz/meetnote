use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use application::{AudioStatus, WorkspaceService};
use contracts::{BackendErrorCode, DiarizationStageOption, DocumentRecord, ModelRef};
use domain::ProjectId;
use futures::Stream;
use futures::channel::{mpsc, oneshot};
use meetnote::audio_services::{AudioBackends, model_catalog, processing_options};
use models::{ModelInstallState, ModelManager};
use rune::Tx;
use runtime::{CancellationToken, JobEvent, JobRuntime, Progress};
use settings::{AppSettings, JsonSettingsRepository, SettingsRepository, SettingsService};

use super::{LiveWorkspace, Task, WorkspaceHost};
use crate::MeetNote;

const CANCELLED: &str = "Audio processing cancelled";

#[derive(Clone)]
pub(super) struct ProcessingConfig {
    settings: PathBuf,
    models: PathBuf,
    jobs: Arc<JobRuntime>,
    audio: Arc<OnceLock<AudioServices>>,
    pub(super) persist_workspace: bool,
}

pub(super) struct AudioServices {
    backends: AudioBackends,
    pub manager: ModelManager,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            settings: "data/settings/settings.json".into(),
            models: "models".into(),
            jobs: Arc::new(JobRuntime::default()),
            audio: Arc::default(),
            persist_workspace: false,
        }
    }
}

impl ProcessingConfig {
    pub(super) fn configure(&mut self, settings: Option<PathBuf>, models: Option<PathBuf>) {
        if settings.is_some() || models.is_some() {
            self.audio = Arc::default();
        }
        if let Some(settings) = settings {
            self.settings = settings;
            self.persist_workspace = true;
        }
        if let Some(models) = models {
            self.models = models;
        }
    }

    pub(super) fn model_root(&self) -> &Path {
        &self.models
    }

    pub(super) fn jobs(&self) -> Arc<JobRuntime> {
        self.jobs.clone()
    }

    pub(super) fn settings_service(&self) -> Result<SettingsService, String> {
        SettingsService::open(Arc::new(JsonSettingsRepository::open(&self.settings)))
            .map_err(|error| error.to_string())
    }

    pub(super) fn settings(&self) -> Result<AppSettings, String> {
        Ok(self.saved_settings()?.unwrap_or_default())
    }

    pub(super) fn saved_settings(&self) -> Result<Option<AppSettings>, String> {
        if !self
            .settings
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Ok(None);
        }
        JsonSettingsRepository::open(&self.settings)
            .load()
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub(super) fn audio(&self, settings: &AppSettings) -> Result<&AudioServices, String> {
        if let Some(audio) = self.audio.get() {
            return Ok(audio);
        }
        let catalog = model_catalog().map_err(|error| error.to_string())?;
        Ok(self.audio.get_or_init(|| {
            let backends = AudioBackends::registered(Path::new(&settings.speaker_library_path));
            let manager = backends.models(catalog, &self.models);
            AudioServices { backends, manager }
        }))
    }

    pub(super) fn configurable_stages(
        &self,
        settings: &AppSettings,
        model: &ModelRef,
    ) -> Option<Vec<DiarizationStageOption>> {
        let audio = self.audio(settings).ok()?;
        let entry = audio.manager.catalog().find(&model.model_id)?;
        if entry.fixed_diarization_stages || !entry.supports_backend(&model.backend_id) {
            return None;
        }
        let descriptor = audio
            .backends
            .inventory()
            .diarization
            .into_iter()
            .find(|descriptor| descriptor.id == model.backend_id)?;
        Some(
            descriptor
                .diarization_stage_options
                .into_iter()
                .filter(|option| {
                    descriptor
                        .configurable_diarization_stages
                        .contains(&option.stage)
                })
                .collect(),
        )
    }
}

struct Stage {
    updates: mpsc::UnboundedReceiver<JobEvent>,
    progress: Option<Progress>,
}

impl Stage {
    fn poll(&mut self, cx: &mut Context<'_>, tx: &mut Tx) -> bool {
        let mut changed = false;
        for _ in 0..32 {
            match Pin::new(&mut self.updates).poll_next(cx) {
                Poll::Ready(Some(JobEvent::Progress { progress, .. })) => {
                    changed |= self.progress != Some(progress);
                    self.progress = Some(progress);
                }
                Poll::Ready(Some(_)) => {}
                Poll::Ready(None) | Poll::Pending => return changed,
            }
        }
        tx.wake_after(Duration::ZERO);
        changed
    }
}

struct StageEvents {
    asr: Stage,
    diarization: Stage,
}

enum ProcessingProgress {
    Preparing(oneshot::Receiver<StageEvents>),
    Running(StageEvents),
    Finishing,
}

pub(super) struct ProcessingRun {
    id: ProjectId,
    progress: ProcessingProgress,
    cancellation: CancellationToken,
    completion: Task<DocumentRecord>,
}

impl Drop for ProcessingRun {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl ProcessingRun {
    fn start(config: ProcessingConfig, live: LiveWorkspace, id: ProjectId, revision: u64) -> Self {
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let (started, progress) = oneshot::channel();
        let document = id.clone();
        Self {
            id,
            progress: ProcessingProgress::Preparing(progress),
            cancellation,
            completion: Box::pin(runtime::spawn_blocking(move || {
                process(config, live, document, revision, started, &token)
            })),
        }
    }

    fn poll_progress(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut cx = Context::from_waker(waker.unwrap_or_else(|| Waker::noop()));
        if let ProcessingProgress::Preparing(started) = &mut self.progress {
            match Pin::new(started).poll(&mut cx) {
                Poll::Ready(Ok(stages)) => {
                    self.progress = ProcessingProgress::Running(stages);
                    return true;
                }
                Poll::Ready(Err(_)) => self.progress = ProcessingProgress::Finishing,
                Poll::Pending => {}
            }
        }
        if let ProcessingProgress::Running(stages) = &mut self.progress {
            return stages.asr.poll(&mut cx, tx) | stages.diarization.poll(&mut cx, tx);
        }
        false
    }
}

fn installed_model(
    models: &ModelManager,
    task: &str,
    label: &str,
    backend: &str,
    preferred: Option<&ModelRef>,
) -> Result<ModelRef, String> {
    let model = models
        .configured_model_ref(task, backend, preferred)
        .map_err(|error| error.to_string())?;
    if !models
        .browse(Some(task), Some(backend))
        .iter()
        .any(|entry| {
            entry.model.id == model.model_id && entry.install_state == ModelInstallState::Installed
        })
    {
        return Err(format!("Install {label} for {backend} before processing"));
    }
    Ok(model)
}

fn process(
    config: ProcessingConfig,
    live: LiveWorkspace,
    id: ProjectId,
    revision: u64,
    started: oneshot::Sender<StageEvents>,
    cancellation: &CancellationToken,
) -> Result<DocumentRecord, String> {
    if cancellation.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let settings = config.settings()?;
    let audio = config.audio(&settings)?;
    let models = &audio.manager;
    let asr = installed_model(
        models,
        "asr",
        "an ASR model",
        settings.asr_backend_id.as_deref().unwrap_or("whisper-rs"),
        settings.asr_model.as_ref(),
    )?;
    let diarization = installed_model(
        models,
        "diarization",
        "a diarization model",
        settings
            .diarization_backend_id
            .as_deref()
            .unwrap_or("diarization-native"),
        settings.diarization_model.as_ref(),
    )?;
    let document = live.read(&id)?;
    if document.state.revision() != revision {
        return Err("The document changed before audio processing started".into());
    }
    if cancellation.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let workspace = WorkspaceService::with_options(
        audio.backends.processing(config.jobs.clone()),
        live.documents.clone(),
        processing_options(&settings),
    );
    let job = workspace
        .start_project_processing(id.clone(), asr, diarization)
        .map_err(|error| error.message)?;
    let job_ids = job.job_ids();
    cancellation.link(&job.asr_context.cancellation_token());
    cancellation.link(&job.diarization_context.cancellation_token());
    let _ = started.send(StageEvents {
        asr: Stage {
            updates: job.asr_events,
            progress: None,
        },
        diarization: Stage {
            updates: job.diarization_events,
            progress: None,
        },
    });
    let result = futures::executor::block_on(job.future);
    workspace.discard_processing_job_ids(job_ids);
    result.map_err(|error| match error.code {
        BackendErrorCode::Cancelled => CANCELLED.into(),
        _ => error.message,
    })?;
    live.read(&id)
}

impl MeetNote {
    pub fn process_document(&mut self, tx: &mut Tx) {
        let Some(state) = self.documents.selected_state() else {
            return;
        };
        if let Some(run) = &self.workspace_host.processing {
            if &run.id == state.id() {
                run.cancellation.cancel();
            }
            return;
        }
        if !self.can_process_audio() {
            return;
        }
        if self.documents.has_unsaved_edits() {
            self.notify("Save document edits before processing audio", tx);
            return;
        }
        if let Some(live) = self.workspace_host.live.clone() {
            self.workspace_host.processing = Some(ProcessingRun::start(
                self.workspace_host.processing_config.clone(),
                live,
                state.id().clone(),
                state.revision(),
            ));
            tx.wake_after(Duration::ZERO);
        }
    }

    pub fn processing_here(&self) -> bool {
        self.workspace_host.processing.as_ref().is_some_and(|run| {
            self.documents
                .selected_state()
                .is_some_and(|state| &run.id == state.id())
        })
    }

    pub fn can_process_audio(&self) -> bool {
        !self.workspace_host.busy()
            && matches!(
                self.audio_status(),
                Some(Ok(
                    AudioStatus::Present { .. } | AudioStatus::Unverifiable { .. }
                ))
            )
    }

    pub fn processing_progress(&self) -> Option<(&str, [Option<f32>; 2])> {
        let run = self
            .workspace_host
            .processing
            .as_ref()
            .filter(|_| self.processing_here())?;
        if run.cancellation.is_cancelled() {
            return Some(("Cancelling audio processing…", [None, None]));
        }
        match &run.progress {
            ProcessingProgress::Preparing(_) => Some(("Preparing audio models…", [None, None])),
            ProcessingProgress::Running(stages) => Some((
                "Processing audio",
                [
                    progress_fraction(stages.asr.progress),
                    progress_fraction(stages.diarization.progress),
                ],
            )),
            ProcessingProgress::Finishing => Some(("Finishing audio processing…", [None, None])),
        }
    }

    pub(super) fn poll_processing(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let Some(run) = &mut host.processing else {
            return false;
        };
        let progressed = run.poll_progress(host.waker.as_ref(), tx);
        let Some(result) = WorkspaceHost::poll(&mut run.completion, host.waker.as_ref(), tx) else {
            return progressed;
        };
        host.processing = None;
        match result {
            Ok(record) => {
                self.documents.observe_record(record);
                self.refresh_workspace_search();
                self.notify("Audio processing finished", tx);
            }
            Err(error) => self.notify(error, tx),
        }
        true
    }
}

fn progress_fraction(progress: Option<Progress>) -> Option<f32> {
    let progress = progress?;
    let total = progress.total.filter(|total| *total > 0)?;
    Some((progress.completed as f32 / total as f32).clamp(0.0, 1.0))
}
