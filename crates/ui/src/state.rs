use domain::{DocumentState, ProjectId};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    Summary,
    Original,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub route: super::route::Route,
    pub locale: super::i18n::Locale,
    pub selected_project: Option<ProjectId>,
    /// Whether the Detail panel beside the Meetings document is open.
    pub detail_open: bool,
    pub content_mode: ContentMode,
    pub expanded_projects: BTreeSet<ProjectId>,
    pub expanded_model_families: BTreeSet<String>,
    pub notice: String,
    pub processing: bool,
    /// A left press in a titlebar drag strip that has not yet moved the
    /// window (`pages::chrome`).
    pub titlebar_drag_armed: bool,
}

impl UiState {
    /// The state before the project list has loaded.
    pub fn new(locale: super::i18n::Locale) -> Self {
        Self {
            route: super::route::Route::Assistant,
            locale,
            selected_project: None,
            detail_open: false,
            content_mode: ContentMode::Summary,
            expanded_projects: BTreeSet::new(),
            expanded_model_families: BTreeSet::new(),
            notice: super::i18n::text(locale, "ask_notice").into(),
            processing: false,
            titlebar_drag_armed: false,
        }
    }

    /// Selects the first project unless one is selected, and expands every
    /// root: the tree as it opens once the project list first loads.
    pub fn open_loaded_projects(&mut self, projects: &[DocumentState]) {
        if self.selected_project.is_none() {
            self.selected_project = projects.first().map(|project| project.id().clone());
        }
        self.expanded_projects.extend(
            projects
                .iter()
                .filter(|project| project.hierarchy.parent_id.is_none())
                .map(|project| project.id().clone()),
        );
    }
}

/// The default models' readiness for the Meetings status line.
pub fn model_status(
    installation: &application::InstallationState,
    locale: super::i18n::Locale,
) -> String {
    let label = |ready| super::i18n::text(locale, if ready { "ready" } else { "missing" });
    format!(
        "ASR {} / DIA {}",
        label(installation.default_asr_ready),
        label(installation.default_diarization_ready)
    )
}
