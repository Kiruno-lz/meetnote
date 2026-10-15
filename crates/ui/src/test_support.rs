//! Entry points for the GPUI tests under `crates/ui/tests`: they set the app
//! up and open the desktop window exactly as `run` does, then switch routes.
//! `run` never calls them; the module is hidden from the docs and is not a
//! stable API.

use crate::assistant_ui::{
    AssistantDocumentChange, AssistantFieldDiff, AssistantMutationSummary, AssistantPendingReview,
    AssistantRangePreview, AssistantRunDiagnostics, AssistantRunResult, AssistantTreeChange,
    AssistantUiError, AssistantUsage,
};
use crate::controller::{self, DocumentConflict, RootView, SpeakerLibraryPrompt};
use crate::state::{ContentMode, StageFeedback};
use application::{BackendInventory, DocumentIssue, DocumentIssueKind};
use gpui::{AnyWindowHandle, App, Entity, WindowOptions};
use std::path::Path;

pub use crate::route::Route;

/// An overlay the tests open over a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// On Meetings: a loaded workspace with no documents.
    EmptyWorkspace,
    /// On Assistant: the first run needs an active model connection.
    AssistantSetup,
    /// On Assistant: the connection is configured and the workspace is empty.
    AssistantImport,
    /// On Assistant: a selected imported recording, before audio processing.
    AssistantImported,
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
    /// On Meetings: the Original view of the selected document, its
    /// speakers above the transcript.
    OriginalSpeakers,
    /// On Meetings: fixture-only processing of the selected document, with
    /// transcription completed and diarization at 1/4 reported units.
    ProcessingFeedback,
    /// On Settings: the form scrolled to the Speaker Library section.
    SpeakerLibrary,
    /// On Meetings: the document with this id selected, with Detail open;
    /// the fixture gives it no audio file, so its audio is missing.
    MissingAudio(&'static str),
    /// On Settings: the form scrolled to the Workspace section, under the
    /// reason a documents-root switch was refused while two jobs ran.
    WorkspaceBusy,
    /// On Assistant: a run that wrote twice before its provider timed out,
    /// with its warnings and both receipts (the first undone), paused at the
    /// first of three reviews; the results scrolled to the receipts.
    AssistantReview,
    /// On Meetings: keyboard focus on the first available document action,
    /// reached with Tab from the project tree.
    FocusedButton,
    /// On Settings: keyboard focus on the theme select, reached with Tab
    /// from the Settings entry.
    FocusedSelect,
    /// The toast of a notice that fades: an import that succeeded.
    SuccessToast,
    /// The toast of a notice that stays until closed: an import that
    /// failed.
    ErrorToast,
}

/// A receipt of the fixture run: `document` moved from `revision` to the
/// next one.
fn fixture_receipt(
    id: &str,
    document: &str,
    revision: u64,
    gaps: &[&str],
) -> AssistantMutationSummary {
    AssistantMutationSummary {
        mutation_id: id.into(),
        affected_documents: vec![AssistantDocumentChange {
            document_id: document.into(),
            previous_revision: Some(revision),
            revision: Some(revision + 1),
            deleted: false,
        }],
        undo_token: format!("undo-{id}"),
        uncovered_segment_ids: gaps.iter().map(|gap| gap.to_string()).collect(),
    }
}

/// A review of the fixture run. Every review expires at 2100-01-01 00:00
/// UTC, so the page shows the same instant on every run.
fn fixture_review(id: &str) -> AssistantPendingReview {
    AssistantPendingReview {
        confirmation_id: id.into(),
        expires_at_ms: 4_102_444_800_000,
        affected_document_ids: vec!["doc-1-weekly-sync-decisions".into()],
        field_diffs: vec![AssistantFieldDiff {
            document_id: "doc-1-weekly-sync-decisions".into(),
            field: "summary".into(),
            before: None,
            after: Some("确认 10 月 8 日发布，由王敏负责回归".into()),
        }],
        tree_changes: vec![AssistantTreeChange {
            document_id: "doc-1-weekly-sync-decisions".into(),
            previous_parent_id: None,
            new_parent_id: Some("doc-1-weekly-sync".into()),
            previous_sibling_order: Some(1),
            new_sibling_order: Some(0),
        }],
        ranges: vec![AssistantRangePreview {
            document_id: Some("doc-1-weekly-sync-decisions".into()),
            start_segment_id: "seg-014".into(),
            end_segment_id: "seg-020".into(),
            evidence_segment_ids: vec!["seg-014".into(), "seg-018".into()],
        }],
        uncovered_segment_ids: vec!["seg-021".into()],
    }
}

/// The result the fixture run returns: an answer, two receipts, the
/// timeout it stopped with and a provider notice, and three reviews.
fn fixture_review_result() -> AssistantRunResult {
    let timeout = AssistantUiError {
        code: "PROVIDER_TIMEOUT".into(),
        message: "第三次写入前模型响应超时，已写入的两项保留".into(),
        retryable: true,
    };
    AssistantRunResult {
        answer: Some("已更新本周同步的标题与摘要，决策记录等待确认。".into()),
        warning: Some(timeout.clone()),
        warnings: vec![
            timeout,
            AssistantUiError {
                code: "PROVIDER_NOTICE".into(),
                message: "上下文按窗口截断到 128000 token".into(),
                retryable: false,
            },
        ],
        evidence_segment_ids: vec!["seg-001".into(), "seg-014".into()],
        usage: Some(AssistantUsage {
            input_tokens: 1_840,
            output_tokens: 260,
            total_tokens: 2_100,
            cost_microusd: None,
        }),
        requested_model: Some("assistant-default".into()),
        resolved_model: Some("assistant-default".into()),
        mutation: None,
        mutations: vec![
            fixture_receipt("mutation-weekly-title", "doc-1-weekly-sync", 3, &[]),
            fixture_receipt(
                "mutation-weekly-summary",
                "doc-1-weekly-sync",
                4,
                &["seg-021"],
            ),
        ],
        pending: None,
        pending_reviews: vec![
            fixture_review("review-decisions"),
            fixture_review("review-actions"),
            fixture_review("review-risks"),
        ],
        diagnostics: Some(AssistantRunDiagnostics {
            path: "short_document".into(),
            tool_rounds: 3,
            validation_retries: 0,
            input_segments: 24,
            input_bytes: 9_612,
            elapsed_ms: 4_210,
        }),
    }
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

    /// The id of the document the Meetings route shows.
    pub fn selected_document(&self, cx: &App) -> Option<String> {
        let root = self.view.read(cx);
        root.ui
            .selected_project
            .as_ref()
            .map(|id| id.as_str().to_string())
    }

    /// Opens `overlay`. The theme select opens below where its trigger
    /// painted last, so the Settings route must have drawn once, as must a
    /// route whose control a focus overlay tabs to.
    pub fn open_overlay(&self, overlay: Overlay, cx: &mut App) {
        let tab_from = match overlay {
            Overlay::FocusedButton => Some((self.view.read(cx).tree_focus.clone(), 1)),
            Overlay::FocusedSelect => {
                let settings = crate::pages::chrome::ENTRIES
                    .iter()
                    .position(|(route, _)| *route == Route::Settings)
                    .expect("Settings has an entry");
                // Import, the section navigation, then the theme select.
                Some((
                    self.view.read(cx).nav_focus[settings].clone(),
                    2 + crate::pages::settings::SECTIONS.len(),
                ))
            }
            _ => None,
        };
        if let Some((start, tabs)) = tab_from {
            self.window
                .update(cx, |_, window, cx| {
                    start.focus(window, cx);
                    for _ in 0..tabs {
                        window.dispatch_keystroke(gpui::Keystroke::parse("tab").unwrap(), cx);
                    }
                })
                .expect("the window is open");
            return;
        }
        self.view.update(cx, |root, cx| match overlay {
            Overlay::AssistantSetup => {
                root.set_fixture_assistant_connection(None);
                cx.notify();
            }
            Overlay::AssistantImport => {
                root.projects.clear();
                root.ui.selected_project = None;
                cx.notify();
            }
            Overlay::AssistantImported => {
                // UI projection only: keep the fixture document's identity and
                // audio, with its processing facts cleared in memory.
                if let Some(document) = root
                    .projects
                    .iter_mut()
                    .find(|document| Some(document.id()) == root.ui.selected_project.as_ref())
                {
                    document.processing = Default::default();
                    document.results = Default::default();
                }
                cx.notify();
            }
            Overlay::EmptyWorkspace => {
                root.projects.clear();
                root.ui.selected_project = None;
                root.ui.detail_open = false;
                root.editor.discard_all();
                cx.notify();
            }
            Overlay::FocusedButton | Overlay::FocusedSelect => {}
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
            Overlay::OriginalSpeakers => root.set_content_mode(ContentMode::Original, cx),
            Overlay::ProcessingFeedback => {
                let Some(owner) = root.ui.selected_project.clone() else {
                    return;
                };
                // Fixed visual state only: no backend or runtime job is started.
                root.ui.begin_processing(owner);
                root.ui.processing_feedback.transcription = StageFeedback::Completed;
                root.ui.processing_feedback.diarization =
                    StageFeedback::Running(Some(runtime::Progress {
                        completed: 1,
                        total: Some(4),
                    }));
                cx.notify();
            }
            Overlay::SpeakerLibrary => {
                root.scroll
                    .settings
                    .scroll_to_top_of_item(crate::pages::settings::SPEAKER_LIBRARY_SECTION);
                cx.notify();
            }
            Overlay::MissingAudio(id) => {
                root.select_project(domain::ProjectId::new(id), cx);
                root.ui.detail_open = true;
            }
            Overlay::AssistantReview => {
                let documents = root.projects.clone();
                root.assistant.authorize_documents(&documents);
                let settings = root.saved_assistant().clone();
                let (request, _) = root
                    .assistant
                    .begin("整理本周同步并拆出决策".into(), &settings)
                    .expect("the fixture has an active connection and documents");
                root.assistant
                    .complete(&request.request_id, fixture_review_result());
                let (_, undone) = root
                    .assistant
                    .begin_mutation_undo("mutation-weekly-title")
                    .expect("the fixture run wrote the title");
                root.assistant.mutation_undo_complete(
                    &request.request_id,
                    &undone,
                    fixture_receipt("undo-mutation-weekly-title", "doc-1-weekly-sync", 4, &[]),
                );
                root.scroll
                    .assistant_results
                    .scroll_to_top_of_item(crate::pages::assistant::RECEIPTS_ITEM);
                cx.notify();
            }
            Overlay::SuccessToast => {
                root.ui.notice = crate::i18n::message(root.ui.locale, "imported", "产品周会");
                cx.notify();
            }
            Overlay::ErrorToast => {
                root.ui.notice = crate::i18n::message(
                    root.ui.locale,
                    "import_failed",
                    "无法解码 weekly-sync.m4a：不支持的编码",
                );
                cx.notify();
            }
            Overlay::WorkspaceBusy => {
                let busy = application::DocumentsRootError::WorkspaceBusy {
                    job_ids: vec![runtime::JobId(1), runtime::JobId(2)],
                };
                root.documents_root_switch.status = Some((
                    crate::i18n::documents_root_error(root.ui.locale, &busy),
                    true,
                ));
                root.scroll
                    .settings
                    .scroll_to_top_of_item(crate::pages::settings::WORKSPACE_SECTION);
                cx.notify();
            }
        });
    }

    /// Closes `overlay` again.
    pub fn close_overlay(&self, overlay: Overlay, cx: &mut App) {
        if matches!(overlay, Overlay::FocusedButton | Overlay::FocusedSelect) {
            self.window
                .update(cx, |_, window, _| window.blur())
                .expect("the window is open");
            return;
        }
        self.view.update(cx, |root, cx| match overlay {
            Overlay::AssistantSetup => {
                root.set_fixture_assistant_connection(
                    root.settings_draft.assistant.active_connection_id.clone(),
                );
                cx.notify();
            }
            Overlay::EmptyWorkspace | Overlay::AssistantImport | Overlay::AssistantImported => {
                root.reload_projects(cx)
            }
            Overlay::FocusedButton | Overlay::FocusedSelect => {}
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
            Overlay::OriginalSpeakers => root.set_content_mode(ContentMode::Summary, cx),
            Overlay::ProcessingFeedback => {
                root.ui.finish_processing();
                cx.notify();
            }
            Overlay::SpeakerLibrary => {
                root.scroll.settings.set_offset(gpui::Point::default());
                cx.notify();
            }
            Overlay::MissingAudio(_) => {
                root.ui.detail_open = false;
                cx.notify();
            }
            Overlay::SuccessToast | Overlay::ErrorToast => {
                root.ui.notice.clear();
                cx.notify();
            }
            Overlay::WorkspaceBusy => {
                root.documents_root_switch.status = None;
                root.scroll.settings.set_offset(gpui::Point::default());
                cx.notify();
            }
            Overlay::AssistantReview => {
                root.assistant = Default::default();
                root.scroll
                    .assistant_results
                    .set_offset(gpui::Point::default());
                cx.notify();
            }
        });
    }

    /// Whether the window root holds keyboard focus, as it does whenever no
    /// control has it.
    pub fn root_holds_focus(&self, window: &gpui::Window, cx: &App) -> bool {
        self.view.read(cx).window_focus.is_focused(window)
    }

    /// Whether the startup installation probe, project list, Speaker
    /// Library and the selected document's audio check, which run on worker
    /// threads, have reached the view.
    pub fn startup_loaded(&self, cx: &App) -> bool {
        let root = self.view.read(cx);
        root.projects_loaded
            && !root.installation.catalog.is_empty()
            && root.speaker_profiles.loaded
            && !root.audio.pending()
    }
}
