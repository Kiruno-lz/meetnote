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
    pub notice: String,
    pub processing: bool,
    /// The foreground processing job's stages, while `processing`.
    pub(crate) progress: Option<ProcessingProgress>,
    /// Whether the transcript's speakers show their rename and merge rows.
    pub(crate) speakers_expanded: bool,
    /// Whether the scanner issues list each path and reason.
    pub(crate) issues_expanded: bool,
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
            notice: super::i18n::text(locale, "ask_notice").into(),
            processing: false,
            progress: None,
            speakers_expanded: false,
            issues_expanded: false,
        }
    }

    /// Marks `document` as the one foreground processing job, before any
    /// of its stages has reported.
    pub(crate) fn begin_processing(&mut self, document: ProjectId) {
        self.processing = true;
        self.progress = Some(ProcessingProgress::new(document));
    }

    pub(crate) fn end_processing(&mut self) {
        self.processing = false;
        self.progress = None;
    }

    /// The document the foreground job processes.
    pub(crate) fn processing_document(&self) -> Option<&ProjectId> {
        self.progress
            .as_ref()
            .filter(|_| self.processing)
            .map(|progress| &progress.document)
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
}

/// A job of the foreground processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Transcription,
    Diarization,
}

/// A stage as its job's events left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StageProgress {
    /// No event yet.
    #[default]
    Waiting,
    /// Started; `completed` of `total` units once it has reported progress.
    Running {
        completed: u64,
        total: Option<u64>,
    },
    Completed,
    Failed,
    Cancelled,
}

impl StageProgress {
    fn apply(&mut self, event: &runtime::JobEvent) {
        *self = match event {
            runtime::JobEvent::Started { .. } => Self::Running {
                completed: 0,
                total: None,
            },
            runtime::JobEvent::Progress { progress, .. } => Self::Running {
                completed: progress.completed,
                total: progress.total,
            },
            runtime::JobEvent::Completed { .. } => Self::Completed,
            runtime::JobEvent::Failed { .. } => Self::Failed,
            runtime::JobEvent::Cancelled { .. } => Self::Cancelled,
        };
    }

    /// Whether its work is done: completed, or every counted unit reported.
    fn done(self) -> bool {
        match self {
            Self::Completed => true,
            Self::Running {
                completed,
                total: Some(total),
            } => completed >= total,
            _ => false,
        }
    }

    /// The counted units as `(completed, total)` while some remain.
    fn count(self) -> Option<(u64, u64)> {
        match self {
            Self::Running {
                completed,
                total: Some(total),
            } if completed < total => Some((completed, total)),
            _ => None,
        }
    }

    /// The share of its work done, once it is known.
    fn fraction(self) -> Option<f64> {
        if self.done() {
            return Some(1.);
        }
        self.count()
            .map(|(completed, total)| completed as f64 / total as f64)
    }

    /// Whether it has reported any progress of its own.
    fn reported(self) -> bool {
        matches!(self, Self::Running { completed, total } if completed > 0 || total.is_some())
            || self.done()
    }
}

/// The foreground processing of `document`: its transcription and
/// diarization jobs. Diarization of short audio runs once transcription is
/// done; long audio interleaves both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessingProgress {
    pub(crate) document: ProjectId,
    pub(crate) transcription: StageProgress,
    pub(crate) diarization: StageProgress,
}

impl ProcessingProgress {
    pub(crate) fn new(document: ProjectId) -> Self {
        Self {
            document,
            transcription: StageProgress::Waiting,
            diarization: StageProgress::Waiting,
        }
    }

    /// Applies an event of `stage`'s job.
    pub(crate) fn apply(&mut self, stage: Stage, event: &runtime::JobEvent) {
        match stage {
            Stage::Transcription => self.transcription.apply(event),
            Stage::Diarization => self.diarization.apply(event),
        }
    }

    /// The share done in percent, each stage half of it; `None` while
    /// neither stage knows its extent.
    pub(crate) fn percent(&self) -> Option<f32> {
        let (transcription, diarization) =
            (self.transcription.fraction(), self.diarization.fraction());
        if transcription.is_none() && diarization.is_none() {
            return None;
        }
        Some(((transcription.unwrap_or(0.) + diarization.unwrap_or(0.)) * 50.) as f32)
    }

    /// One line on where processing is, such as `正在转录 3/10 ·
    /// 等待说话人分离`.
    pub(crate) fn phase(&self, locale: super::i18n::Locale) -> String {
        use super::i18n::{message, text};
        let (transcription, diarization) = (self.transcription, self.diarization);
        if transcription == StageProgress::Waiting && diarization == StageProgress::Waiting {
            return text(locale, "processing_preparing").into();
        }
        let count = |key: &str, (completed, total): (u64, u64)| {
            message(locale, key, &format!("{completed}/{total}"))
        };
        let transcribing = match transcription {
            StageProgress::Failed => text(locale, "transcription_failed").into(),
            StageProgress::Cancelled => text(locale, "transcription_cancelled").into(),
            stage if stage.done() => text(locale, "transcription_done").into(),
            stage => match stage.count() {
                Some(units) => count("transcribing_count", units),
                None => text(locale, "transcribing").into(),
            },
        };
        let separating = match diarization {
            StageProgress::Failed => text(locale, "diarization_failed").into(),
            StageProgress::Cancelled => text(locale, "diarization_cancelled").into(),
            stage if stage.done() => text(locale, "diarization_done").into(),
            stage => match stage.count() {
                Some(units) => count("diarizing_count", units),
                None if stage.reported() || transcription.done() => {
                    text(locale, "diarizing").into()
                }
                None => text(locale, "diarization_waiting").into(),
            },
        };
        format!("{transcribing} · {separating}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationTarget {
    route: super::route::Route,
    project: Option<ProjectId>,
}

/// Which models processing would run with are not installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MissingModels {
    pub(crate) transcription: bool,
    pub(crate) diarization: bool,
}

impl MissingModels {
    pub(crate) fn any(self) -> bool {
        self.transcription || self.diarization
    }

    /// The sentence that names what to install.
    pub(crate) fn hint_key(self) -> Option<&'static str> {
        match (self.transcription, self.diarization) {
            (true, true) => Some("models_missing_both"),
            (true, false) => Some("models_missing_asr"),
            (false, true) => Some("models_missing_diarization"),
            (false, false) => None,
        }
    }
}

/// The models processing would run with that are not installed, as the
/// installation service last found them: the model `settings` choose for a
/// stage, else its default. A backend chosen without a model cannot run; a
/// chosen model outside the catalog is taken as installed. Nothing is
/// missing before the first probe lands.
pub(crate) fn missing_models(
    installation: &application::InstallationState,
    settings: &settings::AppSettings,
    defaults: (&contracts::ModelRef, &contracts::ModelRef),
) -> MissingModels {
    if installation.catalog.is_empty() {
        return MissingModels::default();
    }
    let missing = |backend: Option<&String>,
                   chosen: Option<&contracts::ModelRef>,
                   default: &contracts::ModelRef,
                   default_ready: bool| match (backend, chosen) {
        (Some(_), None) => true,
        (_, Some(model)) if model != default => installation
            .catalog
            .iter()
            .find(|entry| entry.model.id == model.model_id)
            .is_some_and(|entry| entry.install_state != models::ModelInstallState::Installed),
        _ => !default_ready,
    };
    MissingModels {
        transcription: missing(
            settings.asr_backend_id.as_ref(),
            settings.asr_model.as_ref(),
            defaults.0,
            installation.default_asr_ready,
        ),
        diarization: missing(
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
    use crate::i18n::Locale;
    use runtime::{JobEvent, JobId, Progress};

    fn progress(completed: u64, total: u64) -> JobEvent {
        JobEvent::Progress {
            job_id: JobId(1),
            progress: Progress {
                completed,
                total: Some(total),
            },
        }
    }

    fn started() -> JobEvent {
        JobEvent::Started { job_id: JobId(1) }
    }

    #[test]
    fn the_phase_names_each_stage_and_its_count() {
        let mut job = ProcessingProgress::new(ProjectId::new("weekly"));
        let phase = |job: &ProcessingProgress| job.phase(Locale::ZhCn);
        assert_eq!(phase(&job), "正在准备……");
        job.apply(Stage::Transcription, &started());
        job.apply(Stage::Diarization, &started());
        assert_eq!(phase(&job), "正在转录 · 等待说话人分离");
        job.apply(Stage::Transcription, &progress(3, 10));
        assert_eq!(phase(&job), "正在转录 3/10 · 等待说话人分离");
        job.apply(Stage::Transcription, &progress(10, 10));
        assert_eq!(phase(&job), "转录完成 · 正在分离说话人");
        job.apply(Stage::Diarization, &progress(2, 5));
        assert_eq!(phase(&job), "转录完成 · 正在分离说话人 2/5");
        assert_eq!(
            job.phase(Locale::EnUs),
            "Transcribed · Identifying speakers 2/5"
        );
        for stage in [Stage::Transcription, Stage::Diarization] {
            job.apply(stage, &JobEvent::Completed { job_id: JobId(1) });
        }
        assert_eq!(phase(&job), "转录完成 · 说话人分离完成");

        let mut failed = ProcessingProgress::new(ProjectId::new("weekly"));
        failed.apply(
            Stage::Transcription,
            &JobEvent::Failed {
                job_id: JobId(1),
                message: "model missing".into(),
            },
        );
        failed.apply(
            Stage::Diarization,
            &JobEvent::Cancelled { job_id: JobId(2) },
        );
        assert_eq!(phase(&failed), "转录失败 · 说话人分离已取消");
    }

    #[test]
    fn each_stage_is_half_of_the_percentage_once_its_extent_is_known() {
        let mut job = ProcessingProgress::new(ProjectId::new("weekly"));
        assert_eq!(job.percent(), None, "nothing reported: indeterminate");
        job.apply(Stage::Transcription, &started());
        job.apply(Stage::Diarization, &started());
        assert_eq!(job.percent(), None);
        job.apply(Stage::Transcription, &progress(3, 10));
        assert_eq!(job.percent(), Some(15.));
        job.apply(Stage::Transcription, &progress(10, 10));
        job.apply(Stage::Diarization, &progress(2, 5));
        assert_eq!(job.percent(), Some(70.));
        job.apply(
            Stage::Diarization,
            &JobEvent::Completed { job_id: JobId(2) },
        );
        assert_eq!(job.percent(), Some(100.));
    }

    fn installation(installed: &[&str]) -> application::InstallationState {
        let catalog = models::ModelCatalog::from_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../resources/models/catalog.json"),
        )
        .unwrap();
        application::InstallationState {
            catalog: catalog
                .models
                .into_iter()
                .map(|model| models::ModelAvailability {
                    install_state: if installed.contains(&model.id.as_str()) {
                        models::ModelInstallState::Installed
                    } else {
                        models::ModelInstallState::Missing
                    },
                    model,
                    artifact_paths: Vec::new(),
                    probe: None,
                    failure: None,
                    record_error: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn model(id: &str) -> contracts::ModelRef {
        contracts::ModelRef {
            backend_id: "backend".into(),
            model_id: id.into(),
            revision: None,
            artifact_path: format!("models/{id}"),
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn processing_needs_the_chosen_or_default_models_installed() {
        let catalog = installation(&[]).catalog;
        let task_model = |task: &str| {
            catalog
                .iter()
                .find(|entry| entry.model.task == task)
                .map(|entry| entry.model.id.clone())
                .unwrap()
        };
        let (asr, diarization) = (task_model("asr"), task_model("diarization"));
        let defaults = (model("default-asr"), model("default-diarization"));
        let defaults = (&defaults.0, &defaults.1);
        let missing = |transcription, diarization| MissingModels {
            transcription,
            diarization,
        };
        // No backend chosen: processing runs the defaults.
        let settings = settings::AppSettings {
            asr_backend_id: None,
            diarization_backend_id: None,
            ..Default::default()
        };

        assert_eq!(
            missing_models(&Default::default(), &settings, defaults),
            missing(false, false),
            "nothing is known before the first probe"
        );
        let mut probed = installation(&[]);
        assert_eq!(
            missing_models(&probed, &settings, defaults),
            missing(true, true)
        );
        probed.default_asr_ready = true;
        probed.default_diarization_ready = true;
        assert_eq!(
            missing_models(&probed, &settings, defaults),
            missing(false, false)
        );

        // Chosen catalog models count as they are installed.
        let mut chosen = settings.clone();
        chosen.asr_backend_id = Some("backend".into());
        chosen.asr_model = Some(model(&asr));
        chosen.diarization_backend_id = Some("backend".into());
        chosen.diarization_model = Some(model(&diarization));
        let installed_asr = installation(&[asr.as_str()]);
        assert_eq!(
            missing_models(&installed_asr, &chosen, defaults),
            missing(false, true)
        );
        // A chosen backend without a model, and a model outside the catalog.
        chosen.asr_model = None;
        chosen.diarization_model = Some(model("custom"));
        assert_eq!(
            missing_models(&installed_asr, &chosen, defaults),
            missing(true, false)
        );
        assert_eq!(missing(true, true).hint_key(), Some("models_missing_both"));
        assert_eq!(
            missing(false, true).hint_key(),
            Some("models_missing_diarization")
        );
        assert_eq!(missing(false, false).hint_key(), None);
    }

    #[test]
    fn processing_names_its_document_until_it_ends() {
        let mut ui = UiState::new(Locale::ZhCn);
        assert_eq!(ui.processing_document(), None);
        ui.begin_processing(ProjectId::new("weekly"));
        assert!(ui.processing);
        assert_eq!(ui.processing_document(), Some(&ProjectId::new("weekly")));
        ui.end_processing();
        assert!(!ui.processing);
        assert_eq!(ui.progress, None);
    }
}
