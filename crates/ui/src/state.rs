use domain::{MeetingProject, ProjectId};
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
    pub assistant_query: String,
    pub notice: String,
    pub processing: bool,
    pub downloading: bool,
}

impl UiState {
    pub fn from_projects(projects: &[MeetingProject], locale: super::i18n::Locale) -> Self {
        Self {
            route: super::route::Route::Assistant,
            locale,
            selected_project: projects.first().map(|project| project.id.clone()),
            detail_overlay: None,
            content_mode: ContentMode::Summary,
            expanded_projects: projects
                .iter()
                .filter(|project| project.parent_id.is_none())
                .map(|project| project.id.clone())
                .collect(),
            assistant_query: String::new(),
            notice: "Ask your voice library, or import an audio project.".into(),
            processing: false,
            downloading: false,
        }
    }
}
