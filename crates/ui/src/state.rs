use contracts::{DiarizationStageOption, ModelRef};
use domain::{DocumentState, ProjectId};
use models::ModelAvailability;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    Summary,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    ProjectDetail,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub route: super::route::Route,
    pub locale: super::i18n::Locale,
    pub selected_project: Option<ProjectId>,
    pub detail_overlay: Option<Overlay>,
    pub content_mode: ContentMode,
    pub expanded_projects: BTreeSet<ProjectId>,
    pub expanded_model_families: BTreeSet<String>,
    pub notice: String,
    pub processing: bool,
}

impl UiState {
    pub fn from_projects(projects: &[DocumentState], locale: super::i18n::Locale) -> Self {
        Self {
            route: super::route::Route::Assistant,
            locale,
            selected_project: projects.first().map(|project| project.id().clone()),
            detail_overlay: None,
            content_mode: ContentMode::Summary,
            expanded_projects: projects
                .iter()
                .filter(|project| project.hierarchy.parent_id.is_none())
                .map(|project| project.id().clone())
                .collect(),
            expanded_model_families: BTreeSet::new(),
            notice: super::i18n::text(locale, "ask_notice").into(),
            processing: false,
        }
    }
}

/// Installation facts the pages show. The controller probes them off the
/// main thread when downloads, settings or the document folder change;
/// rendering only reads this snapshot.
#[derive(Debug, Clone, Default)]
pub struct InstallationState {
    pub catalog: Vec<ModelAvailability>,
    pub default_asr_ready: bool,
    pub default_diarization_ready: bool,
    /// The diarization model directory `stage_files` were probed in.
    pub stage_root: Option<String>,
    /// Diarization stage artifact files present in `stage_root`.
    pub stage_files: BTreeSet<String>,
}

impl InstallationState {
    /// The catalog entries for `task` that support `backend_id`, as
    /// `ModelManager::browse` filters them.
    pub fn models<'a>(
        &'a self,
        task: &'a str,
        backend_id: Option<&'a str>,
    ) -> impl Iterator<Item = &'a ModelAvailability> + 'a {
        self.catalog.iter().filter(move |entry| {
            entry.model.task == task
                && backend_id.is_none_or(|backend| entry.model.supports_backend(backend))
        })
    }

    pub fn stage_installed(&self, model: &ModelRef, option: &DiarizationStageOption) -> bool {
        self.stage_root.as_deref() == Some(model.artifact_path.as_str())
            && option
                .artifact_files
                .iter()
                .all(|file| self.stage_files.contains(file))
    }

    pub fn model_status(&self, locale: super::i18n::Locale) -> String {
        let label = |ready| super::i18n::text(locale, if ready { "ready" } else { "missing" });
        format!(
            "ASR {} / DIA {}",
            label(self.default_asr_ready),
            label(self.default_diarization_ready)
        )
    }
}

/// Equal when every fact the pages show is equal.
impl PartialEq for InstallationState {
    fn eq(&self, other: &Self) -> bool {
        self.default_asr_ready == other.default_asr_ready
            && self.default_diarization_ready == other.default_diarization_ready
            && self.stage_root == other.stage_root
            && self.stage_files == other.stage_files
            && self.catalog.len() == other.catalog.len()
            && self
                .catalog
                .iter()
                .zip(&other.catalog)
                .all(|(left, right)| {
                    left.model.id == right.model.id && left.install_state == right.install_state
                })
    }
}
