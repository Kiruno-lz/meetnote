use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use application::{JsonSpeakerProfileStore, SpeakerLibraryService};
use contracts::SpeakerProfile;
use rune::{FileDialogOptions, TextEdit, Tx};

use super::{Task, WorkspaceHost};
use crate::{Confirm, MeetNote};

pub(crate) struct ProfileRow {
    pub id: String,
    pub display_name: String,
    pub sample_count: usize,
    pub name: TextEdit,
}

pub(crate) enum LibraryChange {
    Rename { id: String, name: String },
    Merge { source: String, target: String },
    Delete(String),
}

pub(crate) struct LibraryPathChange {
    source: PathBuf,
    destination: PathBuf,
}

enum PathReply {
    Confirm(LibraryPathChange),
    Saved(String),
}

#[derive(Default)]
pub(crate) struct LibraryHost {
    path: PathBuf,
    task: Option<Task<Vec<SpeakerProfile>>>,
    path_task: Option<Task<PathReply>>,
    notice: Option<&'static str>,
    pub rows: Vec<ProfileRow>,
    pub error: Option<String>,
}

impl LibraryHost {
    pub fn busy(&self) -> bool {
        self.task.is_some() || self.path_task.is_some()
    }

    pub(super) fn writing(&self) -> bool {
        self.notice.is_some() || self.path_task.is_some()
    }

    pub fn name_draft(&mut self, id: &str) -> &mut TextEdit {
        &mut self.rows.iter_mut().find(|row| row.id == id).unwrap().name
    }

    fn start(&mut self, change: Option<LibraryChange>, tx: &mut Tx) {
        let path = self.path.clone();
        self.notice = match &change {
            Some(LibraryChange::Rename { .. }) => Some("speaker_library_renamed"),
            Some(LibraryChange::Merge { .. }) => Some("speaker_library_merged"),
            Some(LibraryChange::Delete(_)) => Some("speaker_library_deleted"),
            None => None,
        };
        self.error = None;
        self.task = Some(Box::pin(runtime::spawn_blocking(move || {
            let service = SpeakerLibraryService::new(Arc::new(JsonSpeakerProfileStore::new(&path)));
            let result = match change {
                Some(LibraryChange::Rename { id, name }) => service.rename(&id, name).map(drop),
                Some(LibraryChange::Merge { source, target }) => {
                    service.merge_profiles(&target, &source).map(drop)
                }
                Some(LibraryChange::Delete(id)) => service.delete(&id).map(drop),
                None => Ok(()),
            };
            result
                .and_then(|()| service.profiles())
                .map_err(|error| format!("{}: {error}", path.display()))
        })));
        tx.wake_after(Duration::ZERO);
    }

    fn replace_rows(&mut self, profiles: Vec<SpeakerProfile>) {
        let old = std::mem::take(&mut self.rows);
        self.rows = profiles
            .into_iter()
            .map(|profile| {
                let name = old
                    .iter()
                    .find(|row| row.id == profile.id && row.display_name == profile.display_name)
                    .map_or_else(
                        || TextEdit::new(&profile.display_name),
                        |row| row.name.clone(),
                    );
                ProfileRow {
                    id: profile.id,
                    display_name: profile.display_name,
                    sample_count: profile.samples.len(),
                    name,
                }
            })
            .collect();
    }
}

impl MeetNote {
    pub(crate) fn choose_speaker_library(&mut self, tx: &mut Tx) {
        let host = &self.workspace_host;
        if host.busy() {
            return;
        }
        let Some(preferences) = &host.preferences.draft else {
            return;
        };
        let source = PathBuf::from(&preferences.speaker_library);
        tx.open_folder(
            FileDialogOptions {
                title: self.tr("choose_directory").into(),
                directory: source.parent().map(Path::to_owned),
                ..Default::default()
            },
            move |s: &mut MeetNote, result, tx| match result {
                Ok(Some(folder)) => {
                    let destination = folder.join("library.json");
                    if source != destination {
                        s.switch_speaker_library(
                            LibraryPathChange {
                                source,
                                destination,
                            },
                            None,
                            tx,
                        );
                    }
                }
                Err(error) => s.notify(error.to_string(), tx),
                Ok(None) => {}
            },
        );
    }

    pub(crate) fn switch_speaker_library(
        &mut self,
        paths: LibraryPathChange,
        copy: Option<bool>,
        tx: &mut Tx,
    ) {
        let host = &mut self.workspace_host;
        if host.busy() {
            return;
        }
        let config = host.processing_config.clone();
        let root = host.live.as_ref().map(|live| live.root().to_owned());
        host.speaker_library.path_task = Some(Box::pin(runtime::spawn_blocking(move || {
            let service = config.settings_service()?;
            let current = service.snapshot().map_err(|error| error.to_string())?;
            if Path::new(&current.speaker_library_path) != paths.source {
                return Err(
                    "The Speaker Library location changed. Reload settings and choose again."
                        .into(),
                );
            }
            if let Some(root) = root {
                application::validate_documents_root(
                    &root,
                    Some(&paths.destination),
                    config.model_root(),
                )
                .map_err(|error| error.to_string())?;
            }
            let presence = application::speaker_library_presence(&paths.source, &paths.destination);
            if copy.is_none() && presence.source_exists && !presence.destination_exists {
                return Ok(PathReply::Confirm(paths));
            }
            let mut updated = current.clone();
            updated.speaker_library_path = paths
                .destination
                .to_str()
                .ok_or("The Speaker Library path is not valid Unicode")?
                .to_owned();
            if copy == Some(true) {
                application::migrate_file(&paths.source, &paths.destination)?;
            }
            let saved = service
                .update_if_equal(&current, updated)
                .map_err(|error| error.to_string())?;
            Ok(PathReply::Saved(saved.speaker_library_path))
        })));
        tx.wake_after(Duration::ZERO);
    }

    pub(crate) fn reload_speaker_profiles(&mut self, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        if !host.busy()
            && !host.speaker_library.busy()
            && !host.speaker_library.path.as_os_str().is_empty()
        {
            host.speaker_library.start(None, tx);
        }
    }

    pub(crate) fn change_speaker_profiles(&mut self, change: LibraryChange, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        if !host.busy()
            && !host.speaker_library.busy()
            && !host.speaker_library.path.as_os_str().is_empty()
        {
            host.speaker_library.start(Some(change), tx);
        }
    }

    pub(super) fn poll_speaker_library(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.poll_speaker_library_path(tx);
        let host = &mut self.workspace_host;
        let Some(preferences) = &host.preferences.draft else {
            return changed;
        };
        let library = &mut host.speaker_library;
        let path = Path::new(&preferences.speaker_library);
        if !library.writing() && library.path != path {
            library.task = None;
            library.rows.clear();
            library.error = None;
            library.path = path.to_owned();
            if !path.as_os_str().is_empty() {
                library.start(None, tx);
            }
            changed = true;
        }
        if let Some(result) = library
            .task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, host.waker.as_ref(), tx))
        {
            library.task = None;
            let notice = library.notice.take();
            match result {
                Ok(profiles) => {
                    library.replace_rows(profiles);
                    if let Some(notice) = notice {
                        self.notify(self.tr(notice), tx);
                    }
                }
                Err(error) => library.error = Some(error),
            }
            changed = true;
        }
        changed
    }

    fn poll_speaker_library_path(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let Some(result) = host
            .speaker_library
            .path_task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, host.waker.as_ref(), tx))
        else {
            return false;
        };
        host.speaker_library.path_task = None;
        match result {
            Ok(PathReply::Confirm(paths)) => {
                self.confirm = Some(Confirm::MoveSpeakerLibrary(paths))
            }
            Ok(PathReply::Saved(path)) => {
                if let Some(draft) = &mut host.preferences.draft {
                    draft.speaker_library = path;
                }
                self.notify(self.tr("speaker_library_location_saved"), tx);
            }
            Err(error) => self.notify(error, tx),
        }
        true
    }
}
