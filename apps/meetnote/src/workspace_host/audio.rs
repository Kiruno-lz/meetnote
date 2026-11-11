use std::path::Path;
use std::time::Duration;

use application::AudioStatus;
use domain::{DocumentState, ProjectId};
use rune::{FileDialogOptions, Tx};

use super::{Answer, Task, WorkspaceHost};
use crate::{MeetNote, Route};

#[derive(Default)]
pub(super) enum AudioCheck {
    #[default]
    Idle,
    Checking(Source, Task<AudioStatus>),
    Ready(Source, Result<AudioStatus, String>),
}

pub(super) struct Source {
    workspace: u64,
    id: ProjectId,
    revision: u64,
    path: String,
}

impl Source {
    fn from_state(workspace: u64, state: &DocumentState) -> Self {
        Self {
            workspace,
            id: state.id().clone(),
            revision: state.revision(),
            path: state.audio_path().into(),
        }
    }

    fn matches(&self, workspace: u64, state: &DocumentState) -> bool {
        self.workspace == workspace
            && &self.id == state.id()
            && self.revision == state.revision()
            && self.path == state.audio_path()
    }
}

impl MeetNote {
    pub fn audio_status(&self) -> Option<&Result<AudioStatus, String>> {
        let AudioCheck::Ready(source, result) = &self.workspace_host.audio else {
            return None;
        };
        let state = self.documents.selected_state()?;
        source
            .matches(self.workspace_host.generation, state)
            .then_some(result)
    }

    pub fn recheck_audio(&mut self, tx: &mut Tx) {
        if matches!(self.workspace_host.audio, AudioCheck::Ready(..)) {
            self.workspace_host.audio = AudioCheck::Idle;
            tx.wake_after(Duration::ZERO);
        }
    }

    pub fn relink_audio(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        if self.documents.has_unsaved_edits() {
            self.notify("Save document edits before reconnecting the recording", tx);
            return;
        }
        let (Some(live), Some(state)) = (
            self.workspace_host.live.clone(),
            self.documents.selected_state(),
        ) else {
            return;
        };
        let source = Source::from_state(self.workspace_host.generation, state);
        tx.open_file(
            FileDialogOptions {
                title: "Reconnect source audio".into(),
                directory: Path::new(&source.path).parent().map(Path::to_path_buf),
                ..Default::default()
            },
            move |s: &mut MeetNote, answer, tx| match answer {
                Ok(Some(path))
                    if !s.workspace_host.busy()
                        && !s.documents.has_unsaved_edits()
                        && s.documents.selected_state().is_some_and(|state| {
                            source.matches(s.workspace_host.generation, state)
                        }) =>
                {
                    s.workspace_host.start(move || {
                        let path = path
                            .to_str()
                            .ok_or_else(|| "The audio path is not valid Unicode".to_owned())?;
                        let result = live
                            .documents
                            .relink_audio(&source.id, source.revision, path)
                            .map(|_| ())
                            .map_err(|error| error.to_string());
                        Ok(Answer::Relinked {
                            result,
                            snapshot: live.documents.snapshot(),
                        })
                    });
                    tx.wake_after(Duration::ZERO);
                }
                Ok(Some(_)) => {
                    s.notify("The selected document changed or the workspace is busy", tx)
                }
                Ok(None) => {}
                Err(error) => s.notify(error.to_string(), tx),
            },
        );
    }

    pub(super) fn poll_audio(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let state = self.documents.selected_state();
        if let AudioCheck::Checking(source, task) = &mut host.audio {
            let Some(result) = WorkspaceHost::poll(task, host.waker.as_ref(), tx) else {
                return false;
            };
            if let Some(state) = state.filter(|state| source.matches(host.generation, state)) {
                let source = Source::from_state(host.generation, state);
                host.audio = AudioCheck::Ready(source, result);
                return true;
            }
            host.audio = AudioCheck::Idle;
        }
        let (Some(live), Some(state), Route::Documents) = (host.live.as_ref(), state, self.route)
        else {
            return false;
        };
        if matches!(&host.audio, AudioCheck::Ready(source, _) if source.matches(host.generation, state))
        {
            return false;
        }
        let source = Source::from_state(host.generation, state);
        let id = source.id.clone();
        let documents = live.documents.clone();
        host.audio = AudioCheck::Checking(
            source,
            Box::pin(runtime::spawn_blocking(move || {
                documents
                    .audio_status(&id)
                    .map_err(|error| error.to_string())
            })),
        );
        tx.wake_after(Duration::ZERO);
        false
    }
}
