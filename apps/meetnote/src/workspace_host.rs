use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use application::{
    DocumentChanges, DocumentCommandError, DocumentCommandService, DocumentSaveBase,
    DocumentSearchResults, DocumentService, DocumentSnapshot,
};
use contracts::{DocumentPatch, DocumentRecord};
use domain::{ProjectId, SpeakerCorrection};
use futures::Stream;
use persistence::FileDocumentRepository;
use rune::{FileDialogOptions, Tx};
use runtime::CancellationToken;

use crate::{Confirm, MeetNote, Route, documents::TreeMove};

pub(crate) mod assistant;
pub(crate) mod assistant_connection;
mod audio;
pub(crate) mod diarization_stages;
pub(crate) mod models;
pub(crate) mod preferences;
mod processing;
pub(crate) mod speaker_library;

type Task<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

#[derive(Default)]
pub struct WorkspaceHost {
    pub live: Option<LiveWorkspace>,
    task: Option<Task<Answer>>,
    search: Option<Search>,
    watch: Option<Watch>,
    waker: Option<Waker>,
    generation: u64,
    audio: audio::AudioCheck,
    processing_config: processing::ProcessingConfig,
    processing: Option<processing::ProcessingRun>,
    pub models: models::ModelHost,
    pub preferences: preferences::PreferenceHost,
    pub assistant: assistant::AssistantHost,
    pub speaker_library: speaker_library::LibraryHost,
}

struct Search {
    task: Task<DocumentSearchResults>,
    cancellation: CancellationToken,
}

struct Watch {
    changes: DocumentChanges,
    refresh_due: Option<f64>,
}

impl Watch {
    fn poll(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut context = Context::from_waker(waker.unwrap_or_else(|| Waker::noop()));
        for _ in 0..32 {
            match Pin::new(&mut self.changes).poll_next(&mut context) {
                Poll::Ready(Some(_)) => {
                    self.refresh_due.get_or_insert_with(|| tx.clock() + 0.12);
                }
                Poll::Ready(None) => return false,
                Poll::Pending => return true,
            }
        }
        tx.wake_after(std::time::Duration::ZERO);
        true
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Clone)]
pub struct LiveWorkspace {
    root: PathBuf,
    documents: DocumentService,
}

impl LiveWorkspace {
    fn open(root: PathBuf) -> Result<Self, String> {
        let documents = DocumentService::new(Arc::new(FileDocumentRepository::open(&root)?));
        Ok(Self { root, documents })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn read(&self, id: &ProjectId) -> Result<DocumentRecord, String> {
        self.documents
            .read(id)?
            .ok_or_else(|| format!("Document {id:?} is no longer available"))
    }

    fn import(&self, path: PathBuf) -> Result<DocumentRecord, String> {
        let path = path
            .to_str()
            .ok_or_else(|| "The audio path is not valid Unicode".to_owned())?;
        let imported = self.documents.import_audio(path)?;
        self.read(imported.id())
    }

    fn save(
        &self,
        id: &ProjectId,
        base: DocumentSaveBase<'_>,
        patch: DocumentPatch,
    ) -> Result<DocumentRecord, DocumentCommandError> {
        DocumentCommandService::new(self.documents.clone()).update_document_checked(id, base, patch)
    }

    fn move_document(
        &self,
        id: &ProjectId,
        revision: u64,
        movement: TreeMove,
    ) -> Result<(), String> {
        let commands = DocumentCommandService::new(self.documents.clone());
        match movement {
            TreeMove::Reorder { target, before } => {
                commands.reorder_document(id, revision, &ProjectId::new(target), before)
            }
            TreeMove::Reparent(parent) => {
                let parent = parent.map(ProjectId::new);
                let order = self
                    .documents
                    .list()?
                    .iter()
                    .filter(|state| state.hierarchy.parent_id == parent)
                    .map(|state| state.hierarchy.sibling_order)
                    .max()
                    .map_or(0, |last| last.saturating_add(1));
                commands.reparent_document(id, revision, parent.as_ref(), order)
            }
        }
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}

enum Answer {
    Opened(
        LiveWorkspace,
        DocumentSnapshot,
        Result<DocumentChanges, String>,
        Box<preferences::PreferenceDraft>,
        ::settings::AssistantSettings,
    ),
    Imported(DocumentRecord),
    Read(ProjectId, Result<DocumentRecord, String>),
    Saved {
        id: ProjectId,
        patch: DocumentPatch,
        result: Result<DocumentRecord, DocumentCommandError>,
    },
    Moved {
        id: ProjectId,
        result: Result<(), String>,
        snapshot: Result<DocumentSnapshot, String>,
    },
    Refreshed(DocumentSnapshot, Option<Result<DocumentRecord, String>>),
    Relinked {
        result: Result<(), String>,
        snapshot: Result<DocumentSnapshot, String>,
    },
    SpeakersCorrected {
        id: ProjectId,
        correction: SpeakerCorrection,
        result: Result<DocumentRecord, DocumentCommandError>,
    },
}

struct WindowWake(rune::Waker);

impl Wake for WindowWake {
    fn wake(self: Arc<Self>) {
        (self.0)();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        (self.0)();
    }
}

impl WorkspaceHost {
    pub fn busy(&self) -> bool {
        self.task.is_some()
            || self.processing.is_some()
            || self.preferences.busy()
            || self.assistant.busy()
            || self.speaker_library.writing()
    }

    pub fn configure_processing(&mut self, settings: Option<PathBuf>, models: Option<PathBuf>) {
        self.processing_config.configure(settings, models);
    }

    fn start(&mut self, work: impl FnOnce() -> Result<Answer, String> + Send + 'static) {
        self.task = Some(Box::pin(runtime::spawn_blocking(work)));
    }

    fn poll<T>(
        task: &mut Task<T>,
        waker: Option<&Waker>,
        tx: &mut Tx,
    ) -> Option<Result<T, String>> {
        let has_waker = waker.is_some();
        let waker = waker.unwrap_or_else(|| Waker::noop());
        match task.as_mut().poll(&mut Context::from_waker(waker)) {
            Poll::Pending => {
                if !has_waker {
                    tx.wake_after(std::time::Duration::from_millis(16));
                }
                None
            }
            Poll::Ready(answer) => Some(answer),
        }
    }

    pub fn searching(&self) -> bool {
        self.search.is_some()
    }

    pub fn cancel_search(&mut self) {
        self.search = None;
    }

    pub fn search(&mut self, query: String, limit: usize) {
        if let Some(live) = self.live.clone() {
            let cancellation = CancellationToken::new();
            let token = cancellation.clone();
            let task = Box::pin(runtime::spawn_blocking(move || {
                live.documents.search(&query, limit, &token)
            }));
            self.search = Some(Search { task, cancellation });
        }
    }
}

fn prepare_workspace(
    config: &processing::ProcessingConfig,
    root: PathBuf,
    persist: bool,
) -> Result<Answer, String> {
    let service = persist.then(|| config.settings_service()).transpose()?;
    let settings = if let Some(service) = &service {
        service.snapshot().map_err(|error| error.to_string())?
    } else {
        config.settings()?
    };
    load_workspace(config, root, settings, service)
}

fn load_workspace(
    config: &processing::ProcessingConfig,
    root: PathBuf,
    settings: ::settings::AppSettings,
    service: Option<::settings::SettingsService>,
) -> Result<Answer, String> {
    application::validate_documents_root(
        &root,
        Some(Path::new(&settings.speaker_library_path)),
        config.model_root(),
    )
    .map_err(|error| error.to_string())?;
    let live = LiveWorkspace::open(root)?;
    let changes = live.documents.subscribe_changes();
    let snapshot = live.documents.snapshot()?;
    if let Some(service) = service {
        let path = live
            .root
            .to_str()
            .ok_or("The workspace path is not valid Unicode")?;
        if settings.documents_root != path {
            let mut updated = settings.clone();
            updated.documents_root = path.to_owned();
            service
                .update_if_equal(&settings, updated)
                .map_err(|error| error.to_string())?;
        }
    }
    let assistant = assistant_connection::public_assistant(settings.assistant.clone());
    Ok(Answer::Opened(
        live,
        snapshot,
        changes,
        Box::new(preferences::PreferenceDraft::new(config, &settings)),
        assistant,
    ))
}

impl MeetNote {
    pub fn setup(&mut self, wake: rune::Waker) {
        #[cfg(target_os = "macos")]
        {
            self.reduce_transparency = objc2_app_kit::NSWorkspace::sharedWorkspace()
                .accessibilityDisplayShouldReduceTransparency();
        }
        self.workspace_host.waker = Some(Waker::from(Arc::new(WindowWake(wake))));
        self.workspace_host.configure_processing(
            std::env::var_os("MEETNOTE_SETTINGS_PATH").map(PathBuf::from),
            std::env::var_os("MEETNOTE_MODEL_ROOT").map(PathBuf::from),
        );
        self.workspace_host.processing_config.persist_workspace = true;
        if let Some(root) = std::env::var_os("MEETNOTE_DOCUMENTS_ROOT") {
            self.open_workspace(PathBuf::from(root), false);
        } else if !self.sample_mode {
            self.open_saved_workspace();
        }
    }

    pub fn choose_workspace(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        if self.documents.has_unsaved_edits() {
            self.notify("Save your document edits before switching workspaces", tx);
            return;
        }
        tx.open_folder(
            FileDialogOptions {
                title: self.tr("documents_root").into(),
                directory: self
                    .workspace_host
                    .live
                    .as_ref()
                    .map(|live| live.root.clone()),
                ..Default::default()
            },
            |s: &mut MeetNote, result, tx| match result {
                Ok(Some(path)) if !s.workspace_host.busy() && !s.documents.has_unsaved_edits() => {
                    s.open_workspace(path, s.workspace_host.processing_config.persist_workspace);
                    tx.wake_after(std::time::Duration::ZERO);
                }
                Ok(Some(_)) => s.notify("The workspace is busy or has unsaved edits", tx),
                Ok(_) => {}
                Err(error) => s.notify(error.to_string(), tx),
            },
        );
    }

    fn open_workspace(&mut self, path: PathBuf, persist: bool) {
        let config = self.workspace_host.processing_config.clone();
        self.workspace_host
            .start(move || prepare_workspace(&config, path, persist));
    }

    pub fn import_audio(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            if self.sample_mode {
                self.confirm = Some(Confirm::Import);
            } else {
                self.choose_workspace(tx);
            }
            return;
        };
        tx.open_file(
            FileDialogOptions {
                title: self.tr("import_audio").into(),
                ..Default::default()
            },
            move |s: &mut MeetNote, result, tx| match result {
                Ok(Some(path)) if !s.workspace_host.busy() => {
                    s.workspace_host
                        .start(move || live.import(path).map(Answer::Imported));
                }
                Ok(_) => {}
                Err(error) => s.notify(error.to_string(), tx),
            },
        );
    }

    pub fn load_document(&mut self, id: ProjectId) {
        if let Some(live) = self
            .workspace_host
            .live
            .clone()
            .filter(|_| self.workspace_host.task.is_none())
        {
            self.workspace_host.start(move || {
                let result = live.read(&id);
                Ok(Answer::Read(id, result))
            });
        }
    }

    pub fn save_workspace_document(
        &mut self,
        id: ProjectId,
        revision: u64,
        markdown: Option<String>,
        patch: DocumentPatch,
    ) {
        if let Some(live) = self
            .workspace_host
            .live
            .clone()
            .filter(|_| !self.workspace_host.busy())
        {
            self.workspace_host.start(move || {
                let result = live.save(
                    &id,
                    DocumentSaveBase {
                        revision,
                        markdown: markdown.as_deref(),
                    },
                    patch.clone(),
                );
                Ok(Answer::Saved { id, patch, result })
            });
        }
    }

    pub fn move_workspace_document(&mut self, id: ProjectId, revision: u64, movement: TreeMove) {
        if let Some(live) = self
            .workspace_host
            .live
            .clone()
            .filter(|_| !self.workspace_host.busy())
        {
            self.workspace_host.start(move || {
                let result = live.move_document(&id, revision, movement);
                // Even a failed multi-document operation may have written some siblings.
                Ok(Answer::Moved {
                    id,
                    result,
                    snapshot: live.documents.snapshot(),
                })
            });
        }
    }

    pub fn correct_workspace_speakers(
        &mut self,
        id: ProjectId,
        revision: u64,
        correction: SpeakerCorrection,
        tx: &mut Tx,
    ) {
        if self.workspace_host.busy() {
            self.notify("The workspace is busy", tx);
            return;
        }
        if let Some(live) = self.workspace_host.live.clone() {
            self.workspace_host.start(move || {
                let commands = DocumentCommandService::new(live.documents);
                let result = match &correction {
                    SpeakerCorrection::Label { speaker_id, label } => {
                        commands.set_speaker_label(&id, revision, speaker_id, label.clone())
                    }
                    SpeakerCorrection::Merge { from, into } => {
                        commands.merge_speaker(&id, revision, from, into)
                    }
                };
                Ok(Answer::SpeakersCorrected {
                    id,
                    correction,
                    result,
                })
            });
            tx.wake_after(std::time::Duration::ZERO);
        }
    }

    pub fn poll_workspace(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let answer = host
            .task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, host.waker.as_ref(), tx));
        let mut changed = answer.is_some();
        if changed {
            host.task = None;
        }
        match answer {
            Some(Ok(Answer::Opened(live, snapshot, changes, preferences, assistant))) => {
                self.sample_mode = false;
                self.workspace_host.live = Some(live);
                self.workspace_host.assistant.reset_workspace(assistant);
                self.settings.apply_appearance(preferences.appearance);
                if !self
                    .workspace_host
                    .preferences
                    .draft
                    .as_ref()
                    .is_some_and(preferences::PreferenceDraft::has_changes)
                {
                    self.workspace_host.preferences.draft = Some(*preferences);
                } else if let Some(draft) = &mut self.workspace_host.preferences.draft {
                    draft.speaker_library = preferences.speaker_library;
                }
                self.workspace_host.generation += 1;
                self.workspace_host.cancel_search();
                self.workspace_host.watch = match changes {
                    Ok(changes) => Some(Watch {
                        changes,
                        refresh_due: None,
                    }),
                    Err(error) => {
                        self.notify(format!("Workspace monitoring could not start: {error}"), tx);
                        None
                    }
                };
                self.documents.open_snapshot(snapshot);
                self.assistant = Default::default();
                self.select = None;
                self.confirm = None;
                self.reset_workspace_history();
                self.navigate(Route::Documents);
            }
            Some(Ok(Answer::Imported(record))) => {
                let id = record.state.id().as_str().to_owned();
                self.workspace_host.cancel_search();
                self.documents.insert_imported(record);
                self.select_document(&id);
                self.notify("Recording imported", tx);
            }
            Some(Ok(Answer::Read(id, result))) => {
                if let Err(error) = &result {
                    self.notify(error.clone(), tx);
                }
                self.documents.finish_read(&id, result);
            }
            Some(Ok(Answer::Saved { id, patch, result })) => match result {
                Ok(record) => {
                    self.documents.finish_save(record, &patch);
                    self.refresh_workspace_search();
                    self.notify("Document saved", tx);
                }
                Err(error) => {
                    if matches!(
                        error,
                        DocumentCommandError::RevisionConflict { .. }
                            | DocumentCommandError::MarkdownConflict(_)
                            | DocumentCommandError::NotFound(_)
                    ) {
                        self.documents.mark_save_conflict(&id);
                    }
                    self.notify(error.to_string(), tx);
                }
            },
            Some(Ok(Answer::Moved {
                id,
                result,
                snapshot,
            })) => {
                match snapshot {
                    Ok(snapshot) => {
                        self.documents
                            .reconcile_snapshot(snapshot, result.is_ok().then_some(id.as_str()));
                        self.refresh_workspace_search();
                    }
                    Err(error) => self.notify(error, tx),
                }
                match result {
                    Ok(()) => self.documents.reveal_tree_document(id.as_str(), tx),
                    Err(error) => self.notify(error, tx),
                }
            }
            Some(Ok(Answer::Refreshed(snapshot, record))) => {
                self.documents.reconcile_snapshot(snapshot, None);
                if let Some(record) = record {
                    match record {
                        Ok(record) => self.documents.observe_record(record),
                        Err(error) => self.notify(error, tx),
                    }
                }
                self.refresh_workspace_search();
            }
            Some(Ok(Answer::Relinked { result, snapshot })) => {
                match snapshot {
                    Ok(snapshot) => self.documents.reconcile_snapshot(snapshot, None),
                    Err(error) => self.notify(error, tx),
                }
                match result {
                    Ok(()) => {
                        self.recheck_audio(tx);
                        self.notify("Recording reconnected", tx);
                    }
                    Err(error) => self.notify(error, tx),
                }
            }
            Some(Ok(Answer::SpeakersCorrected {
                id,
                correction,
                result,
            })) => match result {
                Ok(record) => {
                    self.documents
                        .finish_speaker_correction(record, &correction);
                    self.refresh_workspace_search();
                    match correction {
                        SpeakerCorrection::Label { .. } => self.notify("Speaker name saved", tx),
                        SpeakerCorrection::Merge { into, .. } => {
                            if self.documents.selection().0 == id.as_str() {
                                tx.focus(format!("speaker-name-{into}"), tx.focus_visible());
                            }
                            self.notify("Speakers merged", tx);
                        }
                    }
                }
                Err(error) => self.notify(error.to_string(), tx),
            },
            Some(Err(error)) => self.notify(error, tx),
            None => {}
        }
        let host = &mut self.workspace_host;
        if let Some(result) = host
            .search
            .as_mut()
            .and_then(|search| WorkspaceHost::poll(&mut search.task, host.waker.as_ref(), tx))
        {
            host.cancel_search();
            self.documents.finish_search(result);
            changed = true;
        }
        if self.poll_workspace_watch(tx) {
            changed = true;
        }
        if self.workspace_host.live.is_some()
            && self.workspace_host.task.is_none()
            && let Some(id) = self.documents.pending_read()
        {
            self.load_document(id);
            // Poll the queued task on the next frame so the background executor can wake it.
            tx.wake_after(std::time::Duration::ZERO);
        }
        self.poll_processing(tx)
            | self.poll_audio(tx)
            | self.poll_models(tx)
            | self.poll_preferences(tx)
            | self.poll_speaker_library(tx)
            | self.poll_live_assistant(tx)
            | changed
    }

    fn poll_workspace_watch(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let busy = host.task.is_some();
        let Some(watch) = &mut host.watch else {
            return false;
        };
        if !watch.poll(host.waker.as_ref(), tx) {
            host.watch = None;
            self.notify(
                "Workspace monitoring stopped. Reopen the workspace to resume.",
                tx,
            );
            return true;
        }
        if let Some(due) = watch.refresh_due {
            if tx.clock() < due {
                tx.wake_at(due);
            } else if !busy && let Some(live) = host.live.clone() {
                watch.refresh_due = None;
                let selected = self.documents.observed_document();
                host.start(move || {
                    let snapshot = live.documents.snapshot()?;
                    let record = selected.map(|id| live.read(&id));
                    Ok(Answer::Refreshed(snapshot, record))
                });
                tx.wake_after(std::time::Duration::ZERO);
            }
        }
        false
    }
}
