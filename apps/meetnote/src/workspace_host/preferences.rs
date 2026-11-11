use std::path::{Path, PathBuf};
use std::time::Duration;

use contracts::{ComputeDevice, DiarizationStageConfiguration};
use rune::{TextEdit, Tx};
use settings::{AppSettings, AppearanceSettings};

use super::diarization_stages::StageDraft;
use super::processing::ProcessingConfig;
use super::{Task, WorkspaceHost};
use crate::MeetNote;

#[derive(Clone, PartialEq)]
struct Values {
    appearance: AppearanceSettings,
    locale: String,
    language: Option<String>,
    compute_device: ComputeDevice,
    expected_speakers: Option<u32>,
    diarization_stages: DiarizationStageConfiguration,
}

impl From<&AppSettings> for Values {
    fn from(settings: &AppSettings) -> Self {
        Self {
            appearance: settings.appearance,
            locale: settings.ui_locale.clone(),
            language: settings.language.clone(),
            compute_device: settings.compute_device,
            expected_speakers: settings.expected_speakers,
            diarization_stages: settings.diarization_stages.clone(),
        }
    }
}

pub(crate) struct PreferenceDraft {
    saved: Values,
    pub appearance: AppearanceSettings,
    pub locale: String,
    pub language: TextEdit,
    pub compute_device: ComputeDevice,
    pub expected_speakers: TextEdit,
    pub speaker_library: String,
    pub stages: StageDraft,
}

impl PreferenceDraft {
    pub(super) fn new(config: &ProcessingConfig, settings: &AppSettings) -> Self {
        let saved = Values::from(settings);
        Self {
            appearance: saved.appearance,
            locale: saved.locale.clone(),
            language: TextEdit::new(saved.language.as_deref().unwrap_or_default()),
            compute_device: saved.compute_device,
            expected_speakers: TextEdit::new(
                saved
                    .expected_speakers
                    .map(|count| count.to_string())
                    .unwrap_or_default(),
            ),
            speaker_library: settings.speaker_library_path.clone(),
            stages: StageDraft::load(config, settings),
            saved,
        }
    }

    pub(super) fn has_changes(&self) -> bool {
        !self.values().is_ok_and(|values| values == self.saved)
    }

    pub(crate) fn saved_locale(&self) -> ui_text::Locale {
        ui_text::Locale::from_code(&self.saved.locale)
    }

    pub(super) fn replace_stages(&mut self, stages: StageDraft) -> bool {
        let edited = self.stages.selection != self.saved.diarization_stages;
        if edited
            && self.stages.parent == stages.parent
            && self.saved.diarization_stages == stages.selection
        {
            return false;
        }
        self.saved.diarization_stages = stages.selection.clone();
        self.stages = stages;
        edited
    }

    fn values(&self) -> Result<Values, String> {
        let count = self.expected_speakers.text.trim();
        let expected_speakers = if count.is_empty() {
            None
        } else {
            Some(
                count
                    .parse::<u32>()
                    .ok()
                    .filter(|&count| count > 0)
                    .ok_or_else(|| "Enter a positive speaker count or leave it empty".to_owned())?,
            )
        };
        let language = self.language.text.trim();
        Ok(Values {
            appearance: self.appearance,
            locale: self.locale.clone(),
            language: (!language.is_empty()).then(|| language.to_owned()),
            compute_device: self.compute_device,
            expected_speakers,
            diarization_stages: self.stages.selection.clone(),
        })
    }
}

impl Values {
    fn merge(self, saved: Self, current: &mut AppSettings) -> Result<(), String> {
        merge(
            &mut current.appearance.theme,
            saved.appearance.theme,
            self.appearance.theme,
            "theme",
        )?;
        merge(
            &mut current.appearance.material,
            saved.appearance.material,
            self.appearance.material,
            "window material",
        )?;
        merge(
            &mut current.ui_locale,
            saved.locale,
            self.locale,
            "UI language",
        )?;
        merge(
            &mut current.language,
            saved.language,
            self.language,
            "audio language",
        )?;
        merge(
            &mut current.compute_device,
            saved.compute_device,
            self.compute_device,
            "compute device",
        )?;
        merge(
            &mut current.expected_speakers,
            saved.expected_speakers,
            self.expected_speakers,
            "speaker count",
        )?;
        merge(
            &mut current.diarization_stages,
            saved.diarization_stages,
            self.diarization_stages,
            "diarization stages",
        )
    }
}

fn merge<T: PartialEq>(current: &mut T, saved: T, edited: T, label: &str) -> Result<(), String> {
    if edited != saved {
        if *current != saved && *current != edited {
            return Err(format!(
                "The {label} setting changed externally. Reload settings before editing it."
            ));
        }
        *current = edited;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct PreferenceHost {
    pub draft: Option<PreferenceDraft>,
    task: Option<Task<(PreferenceDraft, &'static str)>>,
}

impl PreferenceHost {
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }
}

impl MeetNote {
    pub(crate) fn save_live_preferences(&mut self, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        if host.busy() {
            return;
        }
        let Some(draft) = &host.preferences.draft else {
            return;
        };
        let edited = match draft.values() {
            Ok(edited) => edited,
            Err(error) => {
                self.notify(error, tx);
                return;
            }
        };
        let previous = draft.saved.clone();
        let stage_parent = draft.stages.parent.clone();
        let config = host.processing_config.clone();
        let root = host.live.as_ref().map(|live| live.root().to_owned());
        host.preferences.task = Some(Box::pin(runtime::spawn_blocking(move || {
            let service = config.settings_service()?;
            let current = service.snapshot().map_err(|error| error.to_string())?;
            if edited.diarization_stages != previous.diarization_stages
                && current.diarization_model != stage_parent
            {
                return Err(
                    "The diarization model changed. Reload settings before editing its stages."
                        .into(),
                );
            }
            let mut updated = current.clone();
            edited.merge(previous, &mut updated)?;
            if let Some(root) = root {
                application::validate_documents_root(
                    &root,
                    Some(Path::new(&updated.speaker_library_path)),
                    config.model_root(),
                )
                .map_err(|error| error.to_string())?;
            }
            let saved = service
                .update_if_equal(&current, updated)
                .map_err(|error| error.to_string())?;
            Ok((PreferenceDraft::new(&config, &saved), "Settings saved"))
        })));
        tx.wake_after(Duration::ZERO);
    }

    pub(crate) fn reload_live_preferences(&mut self, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        if host.busy() {
            return;
        }
        let config = host.processing_config.clone();
        host.preferences.task = Some(Box::pin(runtime::spawn_blocking(move || {
            Ok((
                PreferenceDraft::new(&config, &config.settings()?),
                "Settings reloaded",
            ))
        })));
        tx.wake_after(Duration::ZERO);
    }

    pub(super) fn poll_preferences(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let Some(result) = host
            .preferences
            .task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, host.waker.as_ref(), tx))
        else {
            return false;
        };
        host.preferences.task = None;
        match result {
            Ok((draft, message)) => {
                self.settings.apply_appearance(draft.appearance);
                self.workspace_host.preferences.draft = Some(draft);
                self.notify(message, tx);
            }
            Err(error) => self.notify(error, tx),
        }
        true
    }

    pub(crate) fn open_saved_workspace(&mut self) {
        let config = self.workspace_host.processing_config.clone();
        self.workspace_host.start(move || {
            let settings = config.settings()?;
            let root = PathBuf::from(&settings.documents_root);
            super::load_workspace(&config, root, settings, None)
        });
    }
}
