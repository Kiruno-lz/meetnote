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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationTarget {
    route: super::route::Route,
    project: Option<ProjectId>,
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
