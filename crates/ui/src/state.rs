use domain::{DocumentState, ProjectId};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    Summary,
    Original,
    /// The editor of the document's semantic fields.
    Edit,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub route: super::route::Route,
    pub(crate) sidebar_open: bool,
    pub(crate) settings_section: usize,
    history: Vec<NavigationTarget>,
    history_cursor: usize,
    pub locale: super::i18n::Locale,
    pub selected_project: Option<ProjectId>,
    /// Whether the Detail panel beside the Meetings document is open.
    pub detail_open: bool,
    pub content_mode: ContentMode,
    pub expanded_projects: BTreeSet<ProjectId>,
    pub expanded_model_families: BTreeSet<String>,
    /// Models whose card shows its checksums and probe results.
    pub expanded_model_details: BTreeSet<String>,
    /// The latest message for the user; empty when there is none. The toast
    /// shows it (`take_up_notice`).
    pub notice: String,
    pub(crate) toast: ToastState,
    pub processing: bool,
    /// The document owned by the one active processing task.
    pub(crate) processing_project: Option<ProjectId>,
}

impl UiState {
    /// The state before the project list has loaded.
    pub fn new(locale: super::i18n::Locale) -> Self {
        Self {
            route: super::route::Route::Assistant,
            sidebar_open: true,
            settings_section: 0,
            history: vec![NavigationTarget {
                route: super::route::Route::Assistant,
                project: None,
            }],
            history_cursor: 0,
            locale,
            selected_project: None,
            detail_open: false,
            content_mode: ContentMode::Summary,
            expanded_projects: BTreeSet::new(),
            expanded_model_families: BTreeSet::new(),
            expanded_model_details: BTreeSet::new(),
            notice: String::new(),
            toast: ToastState::default(),
            processing: false,
            processing_project: None,
        }
    }

    pub(crate) fn begin_processing(&mut self, document_id: ProjectId) {
        self.processing = true;
        self.processing_project = Some(document_id);
    }

    pub(crate) fn finish_processing(&mut self) {
        self.processing = false;
        self.processing_project = None;
    }

    pub(crate) fn is_processing_document(&self, document_id: &ProjectId) -> bool {
        self.processing && self.processing_project.as_ref() == Some(document_id)
    }

    pub(crate) fn processing_status(&self, document: &DocumentState) -> domain::ProjectStatus {
        if self.is_processing_document(document.id()) {
            domain::ProjectStatus::Processing
        } else {
            document.processing_status()
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.history_cursor > 0
    }
    pub(crate) fn can_go_forward(&self) -> bool {
        self.history_cursor + 1 < self.history.len()
    }

    pub(crate) fn remember_navigation(&mut self) {
        let target = NavigationTarget {
            route: self.route,
            project: self.selected_project.clone(),
        };
        if self.history[self.history_cursor] == target {
            return;
        }
        self.history.truncate(self.history_cursor + 1);
        self.history.push(target);
        self.history_cursor += 1;
    }

    pub(crate) fn travel(&mut self, forward: bool) -> bool {
        if if forward {
            !self.can_go_forward()
        } else {
            !self.can_go_back()
        } {
            return false;
        }
        self.history_cursor = if forward {
            self.history_cursor + 1
        } else {
            self.history_cursor - 1
        };
        let target = &self.history[self.history_cursor];
        self.route = target.route;
        self.selected_project = target.project.clone();
        self.detail_open = false;
        true
    }

    /// Selects the first project unless one is selected, and expands every
    /// root: the tree as it opens once the project list first loads.
    pub fn open_loaded_projects(&mut self, projects: &[DocumentState]) {
        if self.selected_project.is_none() {
            self.selected_project = projects.first().map(|project| project.id().clone());
        }
        self.history[self.history_cursor].project = self.selected_project.clone();
        self.expanded_projects.extend(
            projects
                .iter()
                .filter(|project| project.hierarchy.parent_id.is_none())
                .map(|project| project.id().clone()),
        );
    }

    /// Sets a notice whose text does not tell its tone, such as an error
    /// worded by a service.
    pub(crate) fn set_notice(&mut self, tone: NoticeTone, notice: String) {
        self.toast.given = Some((tone, notice.clone()));
        self.notice = notice;
    }

    /// The toast on screen, if any.
    pub(crate) fn toast(&self) -> Option<&Toast> {
        self.toast.shown.as_ref()
    }

    /// Takes up the notice set since the last call: a new notice opens a
    /// toast in place of the one shown, a cleared notice closes it, and
    /// progress its page already shows leaves it as it is.
    pub(crate) fn take_up_notice(&mut self) -> Option<NoticeChange> {
        if self.notice == self.toast.seen {
            return None;
        }
        self.toast.seen = self.notice.clone();
        if self.notice.is_empty() {
            return self
                .toast
                .shown
                .as_ref()
                .filter(|toast| !toast.leaving)
                .map(|toast| NoticeChange::Cleared(toast.generation));
        }
        let tone = match &self.toast.given {
            Some((tone, notice)) if *notice == self.notice => Some(*tone),
            _ => notice_tone(&self.notice),
        }?;
        self.toast.generation += 1;
        self.toast.shown = Some(Toast {
            generation: self.toast.generation,
            tone,
            notice: self.notice.clone(),
            leaving: false,
        });
        Some(NoticeChange::Opened {
            generation: self.toast.generation,
            tone,
        })
    }

    /// Starts the toast of `generation` fading out. False once another
    /// notice has replaced it, or when it is already leaving.
    pub(crate) fn toast_leave(&mut self, generation: u64) -> bool {
        match &mut self.toast.shown {
            Some(toast) if toast.generation == generation && !toast.leaving => {
                toast.leaving = true;
                true
            }
            _ => false,
        }
    }

    /// Removes the toast of `generation` and clears the notice it showed,
    /// so the same notice set again opens a toast again.
    pub(crate) fn toast_remove(&mut self, generation: u64) {
        let Some(toast) = self
            .toast
            .shown
            .take_if(|toast| toast.generation == generation)
        else {
            return;
        };
        if self.notice == toast.notice {
            self.notice.clear();
            self.toast.seen.clear();
        }
    }
}

/// How a notice reads: the toast's icon, and whether it closes on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeTone {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeTone {
    /// Information and success fade on their own; a warning or an error
    /// stays until it is closed or replaced.
    pub(crate) fn fades(self) -> bool {
        matches!(self, Self::Info | Self::Success)
    }
}

/// How long a fading toast stays before it fades out.
pub(crate) const TOAST_LIFETIME: std::time::Duration = std::time::Duration::from_secs(4);

/// The toast's bookkeeping.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToastState {
    /// The notice last taken up; any other is new.
    seen: String,
    /// The tone `set_notice` gave the notice it set.
    given: Option<(NoticeTone, String)>,
    generation: u64,
    shown: Option<Toast>,
}

/// The toast of one notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    /// Counts the toasts opened; a fade or a close ends only its own.
    pub(crate) generation: u64,
    pub(crate) tone: NoticeTone,
    pub(crate) notice: String,
    /// Fading out after it timed out or was closed.
    pub(crate) leaving: bool,
}

/// What taking up a notice changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeChange {
    /// A toast of `tone` opened.
    Opened { generation: u64, tone: NoticeTone },
    /// The notice of the toast of this generation was cleared.
    Cleared(u64),
}

/// A localized notice of a known kind.
#[derive(Debug, Clone, Copy)]
enum Template {
    Text(&'static str),
    Message(&'static str),
    Pair(&'static str),
}

use Template::{Message, Pair, Text};

/// The notices whose text tells their tone, by tone. `None` is feedback the
/// page already shows, which opens no toast: progress, and the save of a
/// setting that applies at once.
const TEMPLATES: [(Option<NoticeTone>, &[Template]); 4] = [
    (
        Some(NoticeTone::Error),
        &[
            Message("documents_load_failed"),
            Message("import_failed"),
            Message("processing_failed"),
            Message("job_failed"),
            Message("settings_failed"),
            Message("speaker_library_migration_failed"),
            Message("speaker_library_failed"),
            Message("speaker_library_read_failed"),
            Message("speaker_edit_failed"),
            Message("document_read_failed"),
            Message("document_save_failed"),
            Message("move_failed"),
            Message("reorder_failed"),
            Message("documents_root_not_saved"),
            Message("documents_root_restore_failed"),
            Message("download_failed"),
            Message("reprobe_failed"),
            Message("model_not_ready"),
            Message("assistant_probe_failed"),
        ],
    ),
    (
        Some(NoticeTone::Warning),
        &[
            Text("select_project_first"),
            Text("project_missing"),
            Text("audio_missing_cannot_process"),
            Text("title_required"),
            Text("settings_conflict"),
            Text("assistant_credential_target_conflict"),
            Text("assistant_connection_required"),
            Text("assistant_expired"),
            Message("document_conflict"),
            Message("asr_model_required"),
            Message("diarization_model_required"),
        ],
    ),
    (
        Some(NoticeTone::Success),
        &[
            Text("assistant_settings_saved"),
            Text("hierarchy_updated"),
            Text("order_updated"),
            Text("assistant_completed"),
            Message("imported"),
            Message("finished"),
            Message("installed"),
            Message("reprobed"),
            Message("model_selected"),
            Message("backend_selected"),
            Message("document_saved"),
            Message("document_reloaded"),
            Message("audio_relinked"),
            Message("documents_root_switched"),
            Message("speaker_library_updated"),
            Message("speaker_renamed"),
            Message("speaker_profile_renamed"),
            Message("speaker_profile_deleted"),
            Pair("speakers_merged"),
            Pair("speaker_profiles_merged"),
        ],
    ),
    (
        None,
        &[
            Text("settings_saved"),
            Text("language_changed"),
            Message("job_started"),
            Message("job_progress"),
            Message("job_progress_total"),
            Message("job_completed"),
        ],
    ),
];

const LOCALES: [super::i18n::Locale; 2] = [super::i18n::Locale::ZhCn, super::i18n::Locale::EnUs];

/// Stands for a value filled into a template.
const HOLE: char = '\u{1}';

/// The texts `template` renders in `locale`, with holes for its values:
/// one value, or two or three joined by `|` as the job messages take them.
fn template_texts(template: Template, locale: super::i18n::Locale) -> Vec<String> {
    let hole = HOLE.to_string();
    match template {
        Text(key) => vec![super::i18n::text(locale, key).to_string()],
        Message(key) => [1, 2, 3]
            .map(|holes| super::i18n::message(locale, key, &vec![hole.as_str(); holes].join("|")))
            .to_vec(),
        Pair(key) => vec![super::i18n::message_pair(locale, key, &hole, &hole)],
    }
}

/// Whether `notice` reads as `text` with its holes filled. A text with no
/// words of its own, such as an unknown key's, matches nothing.
fn fills(text: &str, notice: &str) -> bool {
    if text
        .chars()
        .all(|c| c == HOLE || c == '|' || c.is_whitespace())
    {
        return false;
    }
    let mut parts = text.split(HOLE);
    let Some(mut rest) = notice.strip_prefix(parts.next().unwrap_or_default()) else {
        return false;
    };
    let parts = parts.collect::<Vec<_>>();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    rest.ends_with(last)
}

/// Whether `notice` is the code-prefixed text of an Assistant error, such
/// as `PROVIDER_TIMEOUT: …`; returns the code.
fn error_code(notice: &str) -> Option<&str> {
    let (code, _) = notice.split_once(": ")?;
    (code.contains('_')
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'))
    .then_some(code)
}

/// The tone of `notice`, read from its text in either locale; `None` for
/// no notice, and for progress its page already shows. A notice of no
/// known kind informs.
pub(crate) fn notice_tone(notice: &str) -> Option<NoticeTone> {
    if notice.is_empty() || is_progress(notice) {
        return None;
    }
    if let Some(code) = error_code(notice) {
        return Some(if code.ends_with("_CANCELLED") {
            NoticeTone::Info
        } else {
            NoticeTone::Error
        });
    }
    TEMPLATES
        .iter()
        .find(|(_, templates)| {
            templates.iter().any(|template| {
                LOCALES.iter().any(|locale| {
                    template_texts(*template, *locale)
                        .iter()
                        .any(|text| fills(text, notice))
                })
            })
        })
        .map_or(Some(NoticeTone::Info), |(tone, _)| *tone)
}

/// A model download's progress line, or an Assistant run's stage (the
/// controller writes `assistant <stage>` untranslated).
fn is_progress(notice: &str) -> bool {
    notice.starts_with("assistant ")
        || LOCALES.iter().any(|locale| {
            // The words between the model id and the first number.
            let line = super::i18n::download_progress(*locale, &HOLE.to_string(), 1, 1, 0, None);
            let after_id = &line[HOLE.len_utf8()..];
            let marker = &after_id[..after_id
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(after_id.len())];
            notice.find(marker).is_some_and(|at| at > 0)
        })
}

/// What the toast of `notice` reads: an Assistant error without its code.
pub(crate) fn toast_text(notice: &str) -> &str {
    match error_code(notice) {
        Some(code) => notice[code.len() + 2..].trim_start(),
        None => notice,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationTarget {
    route: super::route::Route,
    project: Option<ProjectId>,
}

/// Processing prerequisites the installation snapshot can identify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MissingModels {
    pub(crate) transcription: bool,
    pub(crate) diarization: bool,
}

impl MissingModels {
    pub(crate) fn any(self) -> bool {
        self.transcription || self.diarization
    }

    pub(crate) fn hint_key(self) -> Option<&'static str> {
        match (self.transcription, self.diarization) {
            (true, true) => Some("models_missing_both"),
            (true, false) => Some("models_missing_asr"),
            (false, true) => Some("models_missing_diarization"),
            (false, false) => None,
        }
    }
}

/// Match the references processing uses to published installation facts.
/// References outside the catalog's artifact paths remain the backend
/// preflight's responsibility; a matching id alone does not prove readiness.
pub(crate) fn missing_models(
    installation: &application::InstallationState,
    settings: &settings::AppSettings,
    defaults: (&contracts::ModelRef, &contracts::ModelRef),
) -> MissingModels {
    let missing = |task: &str,
                   backend: Option<&String>,
                   chosen: Option<&contracts::ModelRef>,
                   default: &contracts::ModelRef,
                   default_ready: bool| {
        if backend.is_some() && chosen.is_none() {
            return true;
        }
        let model = chosen.unwrap_or(default);
        if let Some(entry) = installation
            .catalog
            .iter()
            .find(|entry| entry.model.id == model.model_id)
        {
            if entry.model.task != task || !entry.model.supports_backend(&model.backend_id) {
                return true;
            }
            let path = std::path::Path::new(&model.artifact_path);
            let same_artifacts = if entry.model.artifacts.is_empty() {
                entry
                    .artifact_paths
                    .first()
                    .is_some_and(|artifact| artifact == path)
            } else {
                !entry.artifact_paths.is_empty()
                    && entry
                        .artifact_paths
                        .iter()
                        .all(|artifact| artifact.parent() == Some(path))
            };
            if same_artifacts {
                return entry.install_state != models::ModelInstallState::Installed;
            }
        }
        // An empty catalog is the state before the initial probe lands.
        model == default && !installation.catalog.is_empty() && !default_ready
    };
    MissingModels {
        transcription: missing(
            "asr",
            settings.asr_backend_id.as_ref(),
            settings.asr_model.as_ref(),
            defaults.0,
            installation.default_asr_ready,
        ),
        diarization: missing(
            "diarization",
            settings.diarization_backend_id.as_ref(),
            settings.diarization_model.as_ref(),
            defaults.1,
            installation.default_diarization_ready,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Locale};

    #[test]
    fn model_guidance_uses_the_chosen_reference_and_leaves_external_paths_to_preflight() {
        let catalog =
            models::ModelCatalog::parse(include_str!("../../../resources/models/catalog.json"))
                .unwrap();
        let model = catalog.find("paraformer-zh").unwrap().clone();
        let asr = contracts::ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: model.id.clone(),
            revision: model.revision.clone(),
            artifact_path: model.install_directory("models").display().to_string(),
            artifacts: vec![],
        };
        let diarization = contracts::ModelRef {
            backend_id: "external".into(),
            model_id: "external".into(),
            revision: None,
            artifact_path: "external-diarization".into(),
            artifacts: vec![],
        };
        let mut installation = application::InstallationState {
            catalog: vec![models::ModelAvailability {
                artifact_paths: model.artifact_paths("models"),
                model,
                install_state: models::ModelInstallState::Installed,
                probe: None,
                failure: None,
                record_error: None,
            }],
            default_asr_ready: true,
            default_diarization_ready: true,
            ..Default::default()
        };
        let defaults = (&asr, &diarization);
        let mut settings = settings::AppSettings {
            asr_backend_id: None,
            diarization_backend_id: None,
            ..Default::default()
        };
        assert!(
            !missing_models(&installation, &settings, defaults).any(),
            "installed defaults do not block processing"
        );
        settings.asr_backend_id = Some(asr.backend_id.clone());
        assert!(
            missing_models(&installation, &settings, defaults).transcription,
            "a chosen backend still needs its chosen model"
        );
        settings.asr_model = Some(asr.clone());
        assert!(!missing_models(&installation, &settings, defaults).any());
        installation.catalog[0].install_state = models::ModelInstallState::Partial;
        assert!(
            missing_models(&installation, &settings, defaults).transcription,
            "the selected bundle must be complete"
        );
        settings.asr_model.as_mut().unwrap().artifact_path = "external-copy".into();
        assert!(
            !missing_models(&installation, &settings, defaults).transcription,
            "a matching catalog id cannot diagnose a different artifact path"
        );
        settings.asr_model = Some(asr.clone());
        installation.catalog[0].install_state = models::ModelInstallState::Installed;
        settings.asr_backend_id = Some("whisper-rs".into());
        settings.asr_model.as_mut().unwrap().backend_id = "whisper-rs".into();
        assert!(
            missing_models(&installation, &settings, defaults).transcription,
            "installed files do not make an incompatible backend usable"
        );
        settings.asr_backend_id = Some(asr.backend_id.clone());
        settings.asr_model = Some(asr.clone());
        installation.catalog[0].model.task = "diarization".into();
        assert!(
            missing_models(&installation, &settings, defaults).transcription,
            "the selected model must serve the processing stage"
        );
        settings.asr_backend_id = None;
        settings.asr_model = None;
        assert!(
            !missing_models(&Default::default(), &settings, defaults).any(),
            "the initial snapshot has no installation diagnosis yet"
        );
    }

    #[test]
    fn every_template_has_words_of_its_own_in_both_locales() {
        for (_, templates) in TEMPLATES {
            for template in templates {
                for locale in LOCALES {
                    let texts = template_texts(*template, locale);
                    assert!(
                        texts.iter().any(|text| {
                            !text
                                .chars()
                                .all(|c| c == HOLE || c == '|' || c.is_whitespace())
                                && !text.contains(" → ")
                        }),
                        "{template:?} has no text in {locale:?}: {texts:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_notice_reads_its_tone_in_either_locale() {
        for locale in LOCALES {
            for (notice, tone) in [
                (
                    i18n::message(locale, "import_failed", "bad header"),
                    NoticeTone::Error,
                ),
                (
                    i18n::message(locale, "job_failed", "ASR|out of memory"),
                    NoticeTone::Error,
                ),
                (
                    i18n::message(locale, "documents_root_restore_failed", "a: b"),
                    NoticeTone::Error,
                ),
                (
                    i18n::text(locale, "select_project_first").into(),
                    NoticeTone::Warning,
                ),
                (
                    i18n::message(locale, "document_conflict", "周会"),
                    NoticeTone::Warning,
                ),
                (
                    i18n::message(locale, "finished", "周会"),
                    NoticeTone::Success,
                ),
                (
                    i18n::message_pair(locale, "speakers_merged", "A", "B"),
                    NoticeTone::Success,
                ),
                (
                    i18n::text(locale, "assistant_settings_saved").into(),
                    NoticeTone::Success,
                ),
                (
                    i18n::message(locale, "processing", "周会"),
                    NoticeTone::Info,
                ),
                (
                    i18n::text(locale, "cancellation_requested").into(),
                    NoticeTone::Info,
                ),
            ] {
                assert_eq!(notice_tone(&notice), Some(tone), "{notice}");
            }
        }
        assert_eq!(
            notice_tone("a sentence of no known kind"),
            Some(NoticeTone::Info)
        );
    }

    #[test]
    fn progress_instant_saves_and_no_notice_open_no_toast() {
        assert_eq!(notice_tone(""), None);
        for locale in LOCALES {
            for notice in [
                i18n::text(locale, "settings_saved").into(),
                i18n::text(locale, "language_changed").into(),
                i18n::message(locale, "job_started", "ASR"),
                i18n::message(locale, "job_progress", "DIA|12"),
                i18n::message(locale, "job_progress_total", "ASR|3|10"),
                i18n::message(locale, "job_completed", "ASR"),
                i18n::download_progress(locale, "whisper-small", 2, 3, 41_000_000, Some(3_000_000)),
                i18n::download_progress(locale, "whisper-small", 1, 3, 0, None),
            ] {
                assert_eq!(notice_tone(&notice), None, "{notice}");
            }
        }
        assert_eq!(notice_tone("assistant planning · read_document"), None);
    }

    #[test]
    fn an_assistant_error_is_an_error_read_without_its_code() {
        let notice = "PROVIDER_TIMEOUT: 模型响应超时";
        assert_eq!(notice_tone(notice), Some(NoticeTone::Error));
        assert_eq!(toast_text(notice), "模型响应超时");
        assert_eq!(
            notice_tone("TASK_CANCELLED: cancelled by the user"),
            Some(NoticeTone::Info),
            "a cancellation is no error"
        );
        for plain in [
            "Model selected: whisper.",
            "ASR: ready",
            "已选择模型：whisper。",
        ] {
            assert_eq!(toast_text(plain), plain, "no code to drop");
        }
    }

    fn ui() -> UiState {
        UiState::new(Locale::ZhCn)
    }

    #[test]
    fn the_initial_state_shows_no_toast() {
        let mut ui = ui();
        assert_eq!(ui.notice, "");
        assert_eq!(ui.take_up_notice(), None);
        assert_eq!(ui.toast(), None);
    }

    #[test]
    fn a_toast_opens_per_new_notice_and_ends_only_its_own_generation() {
        let mut ui = ui();
        ui.notice = i18n::message(Locale::ZhCn, "document_saved", "周会");
        let Some(NoticeChange::Opened { generation, tone }) = ui.take_up_notice() else {
            panic!("a new notice opens a toast");
        };
        assert_eq!(tone, NoticeTone::Success);
        assert_eq!(ui.take_up_notice(), None, "taken up once");

        ui.notice = i18n::text(Locale::ZhCn, "select_project_first").into();
        let Some(NoticeChange::Opened {
            generation: next, ..
        }) = ui.take_up_notice()
        else {
            panic!("a newer notice replaces the toast");
        };
        assert!(next > generation);
        assert!(!ui.toast_leave(generation), "the old fade ends nothing");
        ui.toast_remove(generation);
        assert_eq!(ui.toast().map(|toast| toast.generation), Some(next));

        assert!(ui.toast_leave(next));
        assert!(!ui.toast_leave(next), "already leaving");
        ui.toast_remove(next);
        assert_eq!(ui.toast(), None);
        assert_eq!(ui.notice, "", "the removed toast's notice is cleared");

        ui.notice = i18n::text(Locale::ZhCn, "select_project_first").into();
        assert!(
            matches!(ui.take_up_notice(), Some(NoticeChange::Opened { .. })),
            "the same notice again opens a toast again"
        );
    }

    #[test]
    fn progress_keeps_the_toast_and_a_cleared_notice_closes_it() {
        let mut ui = ui();
        ui.notice = i18n::message(Locale::ZhCn, "processing", "周会");
        let Some(NoticeChange::Opened { generation, .. }) = ui.take_up_notice() else {
            panic!("processing opens a toast");
        };
        ui.notice = i18n::message(Locale::ZhCn, "job_progress_total", "ASR|1|4");
        assert_eq!(ui.take_up_notice(), None);
        assert_eq!(ui.toast().map(|toast| toast.generation), Some(generation));
        // The progress line is not the toast's notice; removing the toast
        // keeps it.
        assert!(ui.toast_leave(generation));
        ui.toast_remove(generation);
        assert_eq!(
            ui.notice,
            i18n::message(Locale::ZhCn, "job_progress_total", "ASR|1|4")
        );

        ui.notice = i18n::message(Locale::ZhCn, "import_failed", "x");
        let Some(NoticeChange::Opened { generation, .. }) = ui.take_up_notice() else {
            panic!("the error opens a toast");
        };
        ui.notice.clear();
        assert_eq!(ui.take_up_notice(), Some(NoticeChange::Cleared(generation)));
    }

    #[test]
    fn a_notice_set_with_a_tone_keeps_it() {
        let mut ui = ui();
        ui.set_notice(NoticeTone::Error, "所选音频文件不存在：/a.wav".into());
        assert!(matches!(
            ui.take_up_notice(),
            Some(NoticeChange::Opened {
                tone: NoticeTone::Error,
                ..
            })
        ));
        ui.notice = "another sentence".into();
        assert!(matches!(
            ui.take_up_notice(),
            Some(NoticeChange::Opened {
                tone: NoticeTone::Info,
                ..
            })
        ));
    }
}
