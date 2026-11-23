use std::path::{Path, PathBuf};
use std::time::Duration;

use contracts::{ComputeDevice, DiarizationStageConfiguration};
use rune::{TextEdit, Tx};
use settings::{AppSettings, AppearanceSettings};
use ui_text::Locale;

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

    /// The form's values, or the ui-text key saying why they are invalid.
    fn values(&self) -> Result<Values, &'static str> {
        let count = self.expected_speakers.text.trim();
        let expected_speakers = if count.is_empty() {
            None
        } else {
            Some(
                count
                    .parse::<u32>()
                    .ok()
                    .filter(|&count| count > 0)
                    .ok_or("speaker_count_invalid")?,
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
    fn merge(self, saved: Self, current: &mut AppSettings, locale: Locale) -> Result<(), String> {
        merge(
            &mut current.appearance.theme,
            saved.appearance.theme,
            self.appearance.theme,
            ("theme", locale),
        )?;
        merge(
            &mut current.appearance.material,
            saved.appearance.material,
            self.appearance.material,
            ("material_glass", locale),
        )?;
        merge(
            &mut current.ui_locale,
            saved.locale,
            self.locale,
            ("language", locale),
        )?;
        merge(
            &mut current.language,
            saved.language,
            self.language,
            ("transcription_language", locale),
        )?;
        merge(
            &mut current.compute_device,
            saved.compute_device,
            self.compute_device,
            ("compute_device", locale),
        )?;
        merge(
            &mut current.expected_speakers,
            saved.expected_speakers,
            self.expected_speakers,
            ("expected_speakers", locale),
        )?;
        merge(
            &mut current.diarization_stages,
            saved.diarization_stages,
            self.diarization_stages,
            ("diarization_stages", locale),
        )
    }
}

/// Applies one edited field unless the store changed it to something else;
/// `label` is the field's ui-text key.
fn merge<T: PartialEq>(
    current: &mut T,
    saved: T,
    edited: T,
    (label, locale): (&str, Locale),
) -> Result<(), String> {
    if edited != saved {
        if *current != saved && *current != edited {
            return Err(ui_text::message(
                locale,
                "setting_changed_externally",
                ui_text::text(locale, label),
            ));
        }
        *current = edited;
    }
    Ok(())
}

/// Seconds a settings form waits after its last edit before saving itself.
const AUTOSAVE_DELAY: f64 = 0.7;

/// What the actions bar says about a settings form's autosave.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum SaveStatus {
    #[default]
    Idle,
    Saving,
    Saved,
    Failed(String),
}

/// What an autosave tick asks its form to do.
#[derive(Debug, PartialEq)]
pub(crate) enum Step<V> {
    Idle,
    /// Tick again at this clock time.
    WaitUntil(f64),
    Save(V),
    /// Report why the form cannot be saved.
    Invalid(String),
}

/// How a finished save treats the open form.
#[derive(Debug, PartialEq)]
pub(crate) enum Finish {
    /// The store holds what was sent: keep the form and rebase it, so edits
    /// made during the save stay a diff.
    Keep,
    /// The store merged other values in: replace the form.
    Replace,
    Failed(String),
}

/// Debounced autosave for a settings form. Each edit is acted on once:
/// saved after `AUTOSAVE_DELAY`, or reported once when invalid. A failed save
/// waits for the next edit instead of retrying.
pub(crate) struct Autosave<V> {
    observed: Option<Result<V, String>>,
    edited_at: f64,
    handled: bool,
    sent: Option<V>,
    pub status: SaveStatus,
}

impl<V> Default for Autosave<V> {
    fn default() -> Self {
        Self {
            observed: None,
            edited_at: 0.0,
            handled: false,
            sent: None,
            status: SaveStatus::Idle,
        }
    }
}

impl<V: Clone + PartialEq> Autosave<V> {
    /// `form` is the form's values or why they are invalid; `blocked` holds a
    /// due save while another task runs.
    pub fn tick(&mut self, form: Result<V, String>, saved: &V, now: f64, blocked: bool) -> Step<V> {
        if self.observed.as_ref() != Some(&form) {
            self.observed = Some(form.clone());
            self.edited_at = now;
            self.handled = false;
        }
        if matches!(&form, Ok(values) if values == saved) {
            if matches!(self.status, SaveStatus::Failed(_)) {
                self.status = SaveStatus::Idle;
            }
            return Step::Idle;
        }
        if self.handled {
            return Step::Idle;
        }
        let due = self.edited_at + AUTOSAVE_DELAY;
        if now < due {
            return Step::WaitUntil(due);
        }
        if blocked {
            return Step::Idle;
        }
        self.handled = true;
        match form {
            Ok(values) => {
                self.sent = Some(values.clone());
                self.status = SaveStatus::Saving;
                Step::Save(values)
            }
            Err(error) => {
                self.status = SaveStatus::Failed(error.clone());
                Step::Invalid(error)
            }
        }
    }

    pub fn saving(&self) -> bool {
        self.sent.is_some()
    }

    /// Records the stored values, or why the save failed.
    pub fn finish(&mut self, stored: Result<&V, String>) -> Finish {
        let sent = self.sent.take();
        match stored {
            Ok(stored) => {
                self.status = SaveStatus::Saved;
                if sent.as_ref() == Some(stored) {
                    Finish::Keep
                } else {
                    Finish::Replace
                }
            }
            Err(error) => {
                self.status = SaveStatus::Failed(error.clone());
                Finish::Failed(error)
            }
        }
    }

    /// Forgets the form after it was reloaded from the store.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

enum Done {
    Saved(PreferenceDraft),
    Reloaded(PreferenceDraft),
}

#[derive(Default)]
pub(crate) struct PreferenceHost {
    pub draft: Option<PreferenceDraft>,
    autosave: Autosave<Values>,
    task: Option<Task<Done>>,
}

impl PreferenceHost {
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    pub fn saving(&self) -> bool {
        self.autosave.saving()
    }

    /// A reload replaces the form, so it stays inert meanwhile; an autosave
    /// keeps it editable.
    pub fn reloading(&self) -> bool {
        self.busy() && !self.saving()
    }

    pub fn status(&self) -> &SaveStatus {
        &self.autosave.status
    }
}

impl MeetNote {
    /// Saves the live preference form once its edits settle.
    pub(super) fn autosave_preferences(&mut self, tx: &mut Tx) -> bool {
        let locale = self.locale();
        let host = &mut self.workspace_host;
        let blocked = host.busy();
        let Some(draft) = &host.preferences.draft else {
            return false;
        };
        let form = draft
            .values()
            .map_err(|key| ui_text::text(locale, key).to_owned());
        let step = host
            .preferences
            .autosave
            .tick(form, &draft.saved, tx.clock(), blocked);
        match step {
            Step::Idle => false,
            Step::WaitUntil(due) => {
                tx.wake_at(due);
                false
            }
            Step::Invalid(error) => {
                self.notify(error, tx);
                true
            }
            Step::Save(edited) => {
                self.save_live_preferences(edited, locale, tx);
                true
            }
        }
    }

    fn save_live_preferences(&mut self, edited: Values, locale: Locale, tx: &mut Tx) {
        let host = &mut self.workspace_host;
        let Some(draft) = &host.preferences.draft else {
            return;
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
                return Err(ui_text::text(locale, "diarization_model_changed").into());
            }
            let mut updated = current.clone();
            edited.merge(previous, &mut updated, locale)?;
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
            Ok(Done::Saved(PreferenceDraft::new(&config, &saved)))
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
            Ok(Done::Reloaded(PreferenceDraft::new(
                &config,
                &config.settings()?,
            )))
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
        let preferences = &mut host.preferences;
        preferences.task = None;
        let saving = preferences.saving();
        match result {
            Ok(Done::Reloaded(fresh)) => {
                preferences.autosave.reset();
                self.settings.apply_appearance(fresh.saved.appearance);
                self.workspace_host.preferences.draft = Some(fresh);
                self.notify(self.tr("settings_reloaded"), tx);
            }
            Ok(Done::Saved(fresh)) => {
                let finish = preferences.autosave.finish(Ok(&fresh.saved));
                let appearance = fresh.saved.appearance;
                match &mut preferences.draft {
                    Some(draft)
                        if finish == Finish::Keep && draft.stages.parent == fresh.stages.parent =>
                    {
                        draft.saved = fresh.saved;
                        draft.speaker_library = fresh.speaker_library;
                    }
                    draft => *draft = Some(fresh),
                }
                self.settings.apply_appearance(appearance);
            }
            Err(error) => {
                if saving {
                    preferences.autosave.finish(Err(error.clone()));
                }
                self.notify(error, tx);
            }
        }
        // Edits made during the save are due now.
        tx.wake_after(Duration::ZERO);
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

#[cfg(test)]
mod tests {
    use super::{AUTOSAVE_DELAY, Autosave, Finish, SaveStatus, Step};

    fn edit(autosave: &mut Autosave<u32>, form: Result<u32, &str>, now: f64) -> Step<u32> {
        autosave.tick(form.map_err(str::to_owned), &1, now, false)
    }

    #[test]
    fn saves_once_the_last_edit_settles() {
        let mut autosave = Autosave::default();
        assert_eq!(edit(&mut autosave, Ok(1), 0.0), Step::Idle);
        assert_eq!(
            edit(&mut autosave, Ok(2), 1.0),
            Step::WaitUntil(1.0 + AUTOSAVE_DELAY)
        );
        assert_eq!(
            edit(&mut autosave, Ok(3), 1.5),
            Step::WaitUntil(1.5 + AUTOSAVE_DELAY)
        );
        assert_eq!(
            edit(&mut autosave, Ok(3), 1.5 + AUTOSAVE_DELAY / 2.0),
            Step::WaitUntil(1.5 + AUTOSAVE_DELAY)
        );
        assert_eq!(edit(&mut autosave, Ok(3), 2.5), Step::Save(3));
        assert_eq!(autosave.status, SaveStatus::Saving);
        assert!(autosave.saving());
        assert_eq!(edit(&mut autosave, Ok(3), 3.0), Step::Idle);
    }

    #[test]
    fn holds_a_due_save_while_blocked() {
        let mut autosave = Autosave::default();
        assert_eq!(
            autosave.tick(Ok(2), &1, 0.0, true),
            Step::WaitUntil(AUTOSAVE_DELAY)
        );
        assert_eq!(autosave.tick(Ok(2), &1, 1.0, true), Step::Idle);
        assert_eq!(autosave.tick(Ok(2), &1, 2.0, false), Step::Save(2));
    }

    #[test]
    fn reports_invalid_values_once_and_never_saves_them() {
        let mut autosave = Autosave::default();
        edit(&mut autosave, Err("invalid"), 0.0);
        assert_eq!(
            edit(&mut autosave, Err("invalid"), 1.0),
            Step::Invalid("invalid".into())
        );
        assert_eq!(autosave.status, SaveStatus::Failed("invalid".into()));
        assert_eq!(edit(&mut autosave, Err("invalid"), 2.0), Step::Idle);
        assert_eq!(edit(&mut autosave, Err("invalid"), 9.0), Step::Idle);
        edit(&mut autosave, Ok(1), 10.0);
        assert_eq!(autosave.status, SaveStatus::Idle);
    }

    #[test]
    fn retries_a_failed_save_only_after_another_edit() {
        let mut autosave = Autosave::default();
        edit(&mut autosave, Ok(2), 0.0);
        assert_eq!(edit(&mut autosave, Ok(2), 1.0), Step::Save(2));
        assert_eq!(
            autosave.finish(Err("conflict".into())),
            Finish::Failed("conflict".into())
        );
        assert!(!autosave.saving());
        assert_eq!(autosave.status, SaveStatus::Failed("conflict".into()));
        for now in [2.0, 5.0, 60.0] {
            assert_eq!(edit(&mut autosave, Ok(2), now), Step::Idle);
        }
        edit(&mut autosave, Ok(4), 61.0);
        assert_eq!(edit(&mut autosave, Ok(4), 62.0), Step::Save(4));
    }

    #[test]
    fn keeps_the_form_when_the_store_holds_what_was_sent() {
        let mut autosave = Autosave::default();
        edit(&mut autosave, Ok(2), 0.0);
        edit(&mut autosave, Ok(2), 1.0);
        assert_eq!(autosave.finish(Ok(&2)), Finish::Keep);
        assert_eq!(autosave.status, SaveStatus::Saved);
        assert_eq!(autosave.tick(Ok(2), &2, 1.5, false), Step::Idle);
        // An edit made during the save stays a diff against the rebased value.
        assert_eq!(
            autosave.tick(Ok(5), &2, 1.6, false),
            Step::WaitUntil(1.6 + AUTOSAVE_DELAY)
        );
        assert_eq!(autosave.tick(Ok(5), &2, 3.0, false), Step::Save(5));
    }

    #[test]
    fn replaces_the_form_when_the_store_merged_other_values() {
        let mut autosave = Autosave::default();
        edit(&mut autosave, Ok(2), 0.0);
        edit(&mut autosave, Ok(2), 1.0);
        assert_eq!(autosave.finish(Ok(&7)), Finish::Replace);
        assert_eq!(autosave.status, SaveStatus::Saved);
    }
}
