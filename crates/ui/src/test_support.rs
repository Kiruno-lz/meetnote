//! Entry points for the GPUI tests under `crates/ui/tests`: they set the app
//! up and open the desktop window exactly as `run` does, then switch routes.
//! `run` never calls them; the module is hidden from the docs and is not a
//! stable API.

use crate::controller::{self, DocumentConflict, RootView, SpeakerLibraryPrompt};
use crate::state::ContentMode;
use application::{BackendInventory, DocumentIssue, DocumentIssueKind};
use gpui::{AnyWindowHandle, App, Entity, WindowOptions};
use std::path::Path;

pub use crate::route::Route;

/// An overlay the tests open over a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The Settings theme select with its menu open.
    ThemeSelect,
    /// The Speaker Library migration question.
    SpeakerLibraryDialog,
    /// On Meetings: two scanner issues above the selected document, with
    /// Detail open beside it.
    IssuesAndDetail,
    /// On Meetings: the editor on the selected document's draft, under the
    /// notice of a save another writer's change refused.
    EditorConflict,
}

/// The issues the scanner reports for an unreadable package and a stray
/// file in a documents root at `data/documents`.
fn fixture_issues() -> Vec<DocumentIssue> {
    let root = Path::new("data/documents");
    [
        DocumentIssueKind::InvalidPackage {
            path: root.join("broken-package"),
            message: "cannot parse data/documents/broken-package/state.json: \
                      expected value at line 1 column 1"
                .into(),
        },
        DocumentIssueKind::UnexpectedFile {
            path: root.join("notes.txt"),
        },
    ]
    .iter()
    .map(|kind| DocumentIssue::from_kind(root, kind))
    .collect()
}

/// The app-wide setup `run` performs before it opens the window, with every
/// accessibility display flag off, so no test depends on the machine's.
pub fn init(cx: &mut App) {
    controller::init_app(crate::theme::Accessibility::default(), cx);
}

/// The desktop window as `run` opens it.
pub struct MainWindow {
    window: AnyWindowHandle,
    view: Entity<RootView>,
}

impl MainWindow {
    pub fn open(
        inventory: BackendInventory,
        services: crate::DesktopServices,
        options: WindowOptions,
        cx: &mut App,
    ) -> gpui::Result<Self> {
        let (window, view) = controller::open_main_window(inventory, services, options, cx)?;
        Ok(Self { window, view })
    }

    pub fn window(&self) -> AnyWindowHandle {
        self.window
    }

    pub fn route(&self, cx: &App) -> Route {
        self.view.read(cx).ui.route
    }

    pub fn navigate(&self, route: Route, cx: &mut App) {
        self.view.update(cx, |root, cx| root.navigate(route, cx));
    }

    /// Opens `overlay`. The theme select opens below where its trigger
    /// painted last, so the Settings route must have drawn once.
    pub fn open_overlay(&self, overlay: Overlay, cx: &mut App) {
        self.view.update(cx, |root, cx| match overlay {
            Overlay::ThemeSelect => {
                let select = root
                    .settings_selectors
                    .as_ref()
                    .map(|selectors| selectors.theme.clone())
                    .expect("the settings selects are built on render");
                select.update(cx, |select, cx| select.open_menu(cx));
            }
            Overlay::SpeakerLibraryDialog => {
                root.speaker_library_prompt = Some(SpeakerLibraryPrompt {
                    source: "data/speakers/library.json".into(),
                    destination: "data/speakers/moved/library.json".into(),
                });
                cx.notify();
            }
            Overlay::IssuesAndDetail => {
                root.document_issues = fixture_issues();
                root.ui.detail_open = true;
                cx.notify();
            }
            Overlay::EditorConflict => {
                let selected = root.ui.selected_project.clone();
                let Some(document) = root
                    .projects
                    .iter()
                    .find(|project| Some(project.id()) == selected.as_ref())
                    .cloned()
                else {
                    return;
                };
                // The draft as a read of the fixture document returns it.
                let record = contracts::DocumentRecord {
                    markdown: format!(
                        "# {}\n\n{}\n",
                        document.title(),
                        document.content.summary.clone().unwrap_or_default()
                    ),
                    state: document.clone(),
                };
                root.show_loaded_draft(&record, cx);
                root.ui.content_mode = ContentMode::Edit;
                root.document_conflict = Some(DocumentConflict {
                    document_id: document.id().clone(),
                    title: document.title().into(),
                    expected: document.revision(),
                    actual: document.revision() + 1,
                });
                cx.notify();
            }
        });
    }

    /// Closes `overlay` again.
    pub fn close_overlay(&self, overlay: Overlay, cx: &mut App) {
        self.view.update(cx, |root, cx| match overlay {
            Overlay::ThemeSelect => {
                if let Some(selectors) = &root.settings_selectors {
                    selectors
                        .theme
                        .update(cx, |select, cx| select.close_menu(cx));
                }
            }
            Overlay::SpeakerLibraryDialog => {
                root.speaker_library_prompt = None;
                cx.notify();
            }
            Overlay::IssuesAndDetail => {
                root.document_issues.clear();
                root.ui.detail_open = false;
                cx.notify();
            }
            Overlay::EditorConflict => {
                root.ui.content_mode = ContentMode::Summary;
                root.document_conflict = None;
                cx.notify();
            }
        });
    }

    /// Whether the startup installation probe and project list, which load
    /// on worker threads, have reached the view.
    pub fn startup_loaded(&self, cx: &App) -> bool {
        let root = self.view.read(cx);
        root.projects_loaded && !root.installation.catalog.is_empty()
    }
}
