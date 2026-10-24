//! L2: the Documents slice of the Meetings route on real document packages
//! in a temporary documents root (`FileDocumentRepository`): scanner issues,
//! the Detail facts, the revision-checked editor and tree moves, and the
//! conflict notice when another writer changed a document first.
//!
//! The desktop opens without a file watcher; a test that changes the
//! directory reports it through a change stream it feeds, as the watcher
//! would.

use crate::tests::{
    click_selector, desktop_services, field_value, focus_field, open_desktop_on,
    stored_key_settings, temp_directory, test_settings, wait_for, FakeAssistantUiService, Field,
};
use crate::{
    controller::RootView,
    controls, i18n,
    pages::{detail, meetings},
    route::Route,
};
use application::{
    DocumentChange, DocumentRepository as _, DocumentService, ProcessingService, WorkspaceService,
};
use asr_whisper_rs::WhisperRsBackend;
use contracts::DocumentRecord;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::{
    DiarizationSegment, DocumentState, ModelProvenance, ProcessingRecord, ProjectId,
    RuntimeProvenance, TranscriptSegment,
};
use gpui::{point, px, Bounds, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext};
use persistence::FileDocumentRepository;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) type View = gpui::Entity<RootView>;
type Area = gpui::Entity<controls::TextArea>;

#[gpui::test]
fn workspace_search_is_available_beside_the_project_tree(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(
        DocumentState::new(ProjectId::new("meeting"), "Review", "audio.wav"),
        "项目预算",
    );
    let (_view, cx, _changes) = open_on(cx, &documents);
    assert!(cx.debug_bounds("workspace-search-input").is_some());
    assert!(cx.debug_bounds("workspace-search-submit").is_some());
}

pub(crate) fn search(cx: &mut VisualTestContext, view: &View, query: &str) {
    let input = view.read_with(cx, |root, _| root.search.input.clone());
    focus_field(cx, &input);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(query);
    cx.simulate_keystrokes("enter");
    wait_for(cx, view, |root| root.search.result.is_some());
}

#[gpui::test]
fn workspace_search_shortcut_reopens_closed_and_folding_sidebars(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("meeting", "Review"), "Notes");
    let (view, cx, _changes) = open_on(cx, &documents);
    let width = rendered(cx, "project-sidebar").size.width;
    for elapsed in [
        std::time::Duration::from_secs(1),
        std::time::Duration::from_millis(40),
    ] {
        cx.simulate_keystrokes("ctrl-cmd-s");
        cx.executor().advance_clock(elapsed);
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-shift-f");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(
            rendered(cx, "project-sidebar").size.width,
            width,
            "the shortcut must reopen the real sidebar layout"
        );
        let input = view.read_with(cx, |root, _| root.search.input.clone());
        assert!(cx.update(
            |window, cx| gpui::Focusable::focus_handle(input.read(cx), cx).is_focused(window)
        ));
    }
}

#[gpui::test]
fn workspace_search_reveals_keyboard_results_in_a_minimum_window(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    for index in 0..40 {
        let id = format!("doc-{index:02}");
        documents.put(
            DocumentState::new(
                ProjectId::new(&id),
                format!("Budget review {index}"),
                "audio.wav",
            ),
            "Notes",
        );
    }
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    search(cx, &view, "budget");
    for _ in 0..33 {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
    }
    let viewport = rendered(cx, "workspace-search-scrollbar-viewport");
    let button = rendered(cx, "search-open-doc-30");
    let ring = rendered(cx, "focus-ring");
    assert!(
        within(button, viewport),
        "button={button:?}, viewport={viewport:?}, ring={ring:?}"
    );
    assert!(within(rendered(cx, "focus-ring"), viewport));
}

#[gpui::test]
fn workspace_search_opens_hidden_child_and_retains_results(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("parent", "Weekly review"), "Notes");
    let mut child = processed("child", "Decision");
    child.hierarchy.parent_id = Some(ProjectId::new("parent"));
    child.hierarchy.depth = 1;
    child.results.transcript[0].text = "项目预算已经确认".into();
    child.results.transcript[0].start_ms = 62_000;
    child.results.transcript[0].end_ms = 65_000;
    child.results.speaker_segments[0].text = "项目预算已经确认".into();
    child.results.speaker_segments[0].start_ms = 62_000;
    child.results.speaker_segments[0].end_ms = 65_000;
    documents.put(child, "Notes");
    let before = documents.package_bytes("child");
    let (view, cx, _changes) = open_on(cx, &documents);
    view.update(cx, |root, _| root.ui.expanded_projects.clear());
    cx.simulate_keystrokes("cmd-shift-f");
    let input = view.read_with(cx, |root, _| root.search.input.clone());
    assert!(cx
        .update(|window, cx| gpui::Focusable::focus_handle(input.read(cx), cx).is_focused(window)));
    search(cx, &view, "项目预算");
    assert!(cx.debug_bounds("search-open-child").is_some());
    assert!(cx.debug_bounds("search-open-parent").is_none());
    click_selector(cx, "search-open-child");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("child"))
    );
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Original
    );
    assert!(cx.debug_bounds("workspace-search-results").is_some());
    let result = view.read_with(cx, |root, _| {
        root.search
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
    });
    assert_eq!(result.hits[0].start_ms, Some(62_000));
    click_selector(cx, "workspace-search-clear");
    assert!(cx.debug_bounds("project-row-child").is_some());
    assert_eq!(
        documents.package_bytes("child"),
        before,
        "search and opening results never save documents"
    );
}

#[gpui::test]
fn workspace_search_refreshes_external_markdown_and_handles_empty_results(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("meeting", "Review"), "项目预算");
    let (view, cx, changes) = open_on(cx, &documents);
    search(cx, &view, "项目预算");
    click_selector(cx, "search-open-meeting");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("meeting"))
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Edit
    );
    std::fs::write(documents.path().join("meeting/document.md"), "新的决定").unwrap();
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.search
            .result
            .as_ref()
            .is_some_and(|result| result.as_ref().is_ok_and(|result| result.hits.is_empty()))
    });
    assert!(cx.debug_bounds("workspace-search-empty").is_some());
    search(cx, &view, "新的决定");
    assert!(cx.debug_bounds("search-open-meeting").is_some());
    search(cx, &view, "不存在的内容");
    assert!(cx.debug_bounds("workspace-search-empty").is_some());
}

/// A documents root of its own, removed when the test ends.
pub(crate) struct DocumentsRoot(PathBuf);

impl DocumentsRoot {
    pub(crate) fn new() -> Self {
        let path = temp_directory();
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn repository(&self) -> FileDocumentRepository {
        FileDocumentRepository::open(&self.0).unwrap()
    }

    /// Stores `document` with its revision as set, and `markdown`.
    pub(crate) fn put(&self, document: DocumentState, markdown: &str) {
        self.repository()
            .save(document, Some(markdown.into()))
            .unwrap();
    }

    /// Another writer of the directory, such as an external editor or Agent:
    /// each save is the stored revision plus one.
    pub(crate) fn external_save(&self, document: DocumentState) {
        DocumentService::new(Arc::new(self.repository()))
            .save(document)
            .unwrap();
    }

    pub(crate) fn read(&self, id: &str) -> DocumentRecord {
        self.repository()
            .read(&ProjectId::new(id))
            .unwrap()
            .unwrap_or_else(|| panic!("`{id}` is on disk"))
    }

    pub(crate) fn revision(&self, id: &str) -> u64 {
        self.read(id).state.revision()
    }

    /// The bytes of the package's `state.json` and `document.md`.
    pub(crate) fn package_bytes(&self, id: &str) -> [Vec<u8>; 2] {
        ["state.json", "document.md"].map(|file| std::fs::read(self.0.join(id).join(file)).unwrap())
    }
}

impl Drop for DocumentsRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Makes a real repository unreadable without altering its document packages.
pub(crate) struct UnreadableDocumentsRoot {
    path: PathBuf,
    moved: PathBuf,
}

impl UnreadableDocumentsRoot {
    pub(crate) fn new(documents: &DocumentsRoot) -> Self {
        let path = documents.path().to_path_buf();
        let moved = temp_directory();
        std::fs::rename(&path, &moved).unwrap();
        std::fs::write(&path, b"root temporarily occupied").unwrap();
        Self { path, moved }
    }
}

impl Drop for UnreadableDocumentsRoot {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).unwrap();
        std::fs::rename(&self.moved, &self.path).unwrap();
    }
}

/// The desktop on `root`, on the Meetings route once the list has loaded,
/// and the change stream that reloads it.
pub(crate) fn open_on<'a>(
    cx: &'a mut TestAppContext,
    root: &DocumentsRoot,
) -> (
    View,
    &'a mut VisualTestContext,
    futures::channel::mpsc::UnboundedSender<DocumentChange>,
) {
    let mut services =
        desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(root.repository())),
    );
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
    let (changes, receiver) = futures::channel::mpsc::unbounded();
    view.update(cx, |root, cx| {
        root.follow_document_changes(receiver, cx);
        root.navigate(Route::Meetings, cx);
    });
    wait_for(cx, &view, |root| !root.audio.pending());
    (view, cx, changes)
}

pub(crate) fn leak(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

#[track_caller]
pub(crate) fn rendered(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("`{selector}` is not rendered"))
}

pub(crate) fn within(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left()
        && inner.top() >= outer.top()
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

pub(crate) fn select_row(cx: &mut VisualTestContext, view: &View, id: &str) {
    click_selector(cx, leak(format!("project-row-{id}")));
    let id = ProjectId::new(id);
    wait_for(cx, view, |root| {
        root.ui.selected_project.as_ref() == Some(&id) && !root.audio.pending()
    });
}

/// Opens the editor of the selected document once its draft has loaded.
fn open_editor(cx: &mut VisualTestContext, view: &View, id: &str) {
    click_selector(cx, "content-mode-edit");
    let id = ProjectId::new(id);
    wait_for(cx, view, |root| root.editor.shown() == Some(&id));
    cx.run_until_parked();
}

#[gpui::test]
fn unavailable_processing_models_offer_models_without_starting_or_reporting_an_error(
    cx: &mut TestAppContext,
) {
    let root = DocumentsRoot::new();
    let audio = root.path().join("first/recording.wav");
    root.put(
        DocumentState::new(
            ProjectId::new("first"),
            "First recording",
            audio.display().to_string(),
        ),
        "",
    );
    std::fs::write(&audio, b"recording").unwrap();
    let (view, cx, _changes) = open_on(cx, &root);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    let before = view.read_with(cx, |root, _| root.ui.notice.clone());
    click_selector(cx, "process-project");
    view.update_in(cx, |root, window, cx| {
        root.process_selected(&gpui::ClickEvent::default(), window, cx);
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        before,
        "missing setup is explained in place before a job starts"
    );
    assert!(!view.read_with(cx, |root, _| root.ui.processing));
    assert!(view.read_with(cx, |root, _| root.ui.processing_project.is_none()));
    rendered(cx, "models-required");
    click_selector(cx, "go-to-models");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Models);

    // A model disappearing must never disable cancellation of a running job.
    view.update(cx, |root, cx| {
        root.navigate(Route::Meetings, cx);
        root.ui.begin_processing(ProjectId::new("first"));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("models-required").is_none());
    click_selector(cx, "process-project");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        i18n::text(i18n::Locale::ZhCn, "cancellation_requested")
    );
}

#[gpui::test]
fn model_setup_actions_fit_beside_detail_at_the_minimum_window_size(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    let audio = documents.path().join("first/recording.wav");
    documents.put(
        DocumentState::new(
            ProjectId::new("first"),
            "First",
            audio.display().to_string(),
        ),
        "",
    );
    std::fs::write(&audio, b"recording").unwrap();
    let mut ready = processed("second", "Second");
    ready.source.audio_path = audio.display().to_string();
    ready
        .processing
        .latest
        .as_mut()
        .unwrap()
        .input_audio_checksum = None;
    ready.hierarchy.sibling_order = 1;
    documents.put(ready, "# Second\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    assert!(
        view.read_with(cx, |root, _| root.document_issues.is_empty()),
        "the source audio is a valid document attachment"
    );
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    cx.run_until_parked();
    click_selector(cx, "open-project-detail");
    for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
        view.update(cx, |root, cx| {
            root.ui.locale = locale;
            cx.notify();
        });
        cx.run_until_parked();
        let body = rendered(cx, "meeting-content");
        let guidance = rendered(cx, "models-required");
        let page = rendered(cx, "meetings-page");
        let title = rendered(cx, "document-title");
        let path = rendered(cx, "document-audio-path");
        let actions = rendered(cx, "document-actions");
        let modes = rendered(cx, "content-mode-group");
        let issues = cx.debug_bounds("document-issues");
        assert!(
            body.size.width >= px(192.),
            "Detail leaves a usable document column: {body:?}"
        );
        assert!(within(rendered(cx, "process-project"), page));
        assert!(within(rendered(cx, "open-project-detail"), page));
        assert!(
            within(guidance, body),
            "the unprocessed document explains its next step in the body: {guidance:?} in {body:?} ({locale:?}); page={page:?}, title={title:?}, path={path:?}, actions={actions:?}, modes={modes:?}, issues={issues:?}"
        );
        let action = rendered(cx, "go-to-models");
        assert!(
            within(action, body),
            "the setup action fits the narrow body: {action:?} in {body:?}"
        );
        assert!(action.size.width >= px(48.) && action.size.height >= px(24.));
    }

    select_row(cx, &view, "second");
    wait_for(cx, &view, |root| !root.audio.pending());
    for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
        view.update(cx, |root, cx| {
            root.ui.locale = locale;
            cx.notify();
        });
        cx.run_until_parked();
        let page = rendered(cx, "meetings-page");
        let guidance = rendered(cx, "models-required");
        let action = rendered(cx, "go-to-models");
        let body = rendered(cx, "meeting-content");
        assert!(within(guidance, page) && within(action, page), "the ready document keeps its compact setup guidance inside the content plane: {guidance:?}, {action:?}, {page:?}");
        assert!(action.size.width >= px(48.) && action.size.height >= px(24.));
        assert!(
            body.size.height >= px(96.),
            "the ready document keeps room for readable content: {body:?} ({locale:?})"
        );
    }
    click_selector(cx, "go-to-models");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Models);
}

fn editor_fields(cx: &mut VisualTestContext, view: &View) -> (Field, Field, Area, Area) {
    view.read_with(cx, |root, _| {
        (
            root.editor.title.clone(),
            root.editor.tags.clone(),
            root.editor.summary.clone(),
            root.editor.body.clone(),
        )
    })
}

#[gpui::test]
fn the_detail_trigger_closes_and_reopens_without_interrupting_the_draft(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "Weekly"), "# Original document\n");
    let before = documents.package_bytes("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    open_editor(cx, &view, "weekly");
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "A draft to keep");
    let focus = cx.update(|window, cx| window.focused(cx));
    click_selector(cx, "open-project-detail");
    assert!(cx.debug_bounds("detail-panel").is_some());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    click_selector(cx, "open-project-detail");
    assert!(
        cx.debug_bounds("detail-panel").is_none(),
        "the same detail trigger must close the panel it opened"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    cx.simulate_input("!");
    assert_eq!(field_value(cx, &title), "A draft to keep!");
    // Keyboard users can operate the same stable tab stop in both states.
    view.update_in(cx, |root, window, cx| root.detail_trigger.focus(window, cx));
    cx.run_until_parked();
    for (key, open) in [("enter", true), ("space", false)] {
        let keystroke = gpui::Keystroke::parse(key).unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("detail-panel").is_some(), open);
    }
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        Some(view.read_with(cx, |root, _| root.detail_trigger.clone()))
    );
    click_selector(cx, "open-project-detail");
    let close = rendered(cx, "close-detail");
    assert!(close.size.width > px(0.));
    click_selector(cx, "close-detail");
    assert!(cx.debug_bounds("detail-panel").is_none());
    assert_eq!(field_value(cx, &title), "A draft to keep!");
    assert_eq!(documents.package_bytes("weekly"), before);
}

#[gpui::test]
fn the_editor_status_fits_beside_detail_and_remains_keyboard_operable(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    let audio = documents.path().join("weekly/recording.wav");
    let mut document = processed("weekly", "产品周会");
    document.source.audio_path = audio.display().to_string();
    document
        .processing
        .latest
        .as_mut()
        .unwrap()
        .input_audio_checksum = None;
    documents.put(document, "# 产品周会\n");
    std::fs::write(audio, b"fixture recording").unwrap();
    let before = documents.package_bytes("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    open_editor(cx, &view, "weekly");
    click_selector(cx, "open-project-detail");
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    for (locale, expected) in [
        (i18n::Locale::ZhCn, "final"),
        (i18n::Locale::EnUs, "archived"),
    ] {
        view.update(cx, |root, cx| {
            root.ui.locale = locale;
            root.editor.follow_locale(locale, &root.projects, cx);
            cx.notify();
        });
        cx.run_until_parked();
        let body = rendered(cx, "meeting-content");
        let status = rendered(cx, "select-document-status");
        assert!(status.left() >= body.left() && status.right() <= body.right(),
            "the entire status control fits its reading column: {status:?} in {body:?} ({locale:?})");
        let select = view.read_with(cx, |root, _| root.editor.status.clone());
        let tags = view.read_with(cx, |root, _| root.editor.tags.clone());
        focus_field(cx, &tags);
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert!(
            cx.update(
                |window, cx| gpui::Focusable::focus_handle(select.read(cx), cx).is_focused(window)
            ),
            "Tab from tags reaches the status select"
        );
        let focused_status = rendered(cx, "select-document-status");
        let focused_body = rendered(cx, "meeting-content");
        assert!(within(focused_status, focused_body),
            "keyboard focus reveals the whole status trigger: {focused_status:?} in {focused_body:?} ({locale:?})");
        cx.simulate_keystrokes("enter");
        let viewport = Bounds::from_corners(point(px(0.), px(0.)), point(px(900.), px(600.)));
        assert!(
            within(rendered(cx, "select-menu"), viewport),
            "the keyboard menu remains within the window"
        );
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            select.read_with(cx, |select, _| select.selected_value().map(str::to_string)),
            Some(expected.into())
        );
        assert_eq!(
            documents.package_bytes("weekly"),
            before,
            "choosing a status only edits the draft"
        );
    }
}

#[gpui::test]
fn a_long_status_menu_opens_with_the_current_choice_visible(cx: &mut TestAppContext) {
    long_status_menu(cx, crate::pages::chrome::MIN_WINDOW_SIZE, true);
}

#[gpui::test]
fn a_long_status_menu_opened_by_pointer_keeps_manual_scrolling(cx: &mut TestAppContext) {
    long_status_menu(cx, gpui::size(px(1180.), px(760.)), false);
}

fn long_status_menu(cx: &mut TestAppContext, size: gpui::Size<Pixels>, keyboard: bool) {
    let documents = DocumentsRoot::new();
    let mut active = processed("active", "Active document");
    active.document_status.value = "stage-19".into();
    documents.put(active, "# Active document\n");
    for index in 0..19 {
        let mut document = processed(&format!("status-{index:02}"), &format!("Status {index:02}"));
        document.document_status.value = format!("stage-{index:02}");
        documents.put(document, "");
    }
    let before = documents.package_bytes("active");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "active");
    open_editor(cx, &view, "active");
    cx.simulate_resize(size);
    let select = view.read_with(cx, |root, _| root.editor.status.clone());
    let index = view.read_with(cx, |root, _| {
        controls::SelectOption::position(
            &crate::editor::status_options(&root.projects, "stage-19", root.ui.locale),
            Some("stage-19"),
        )
        .unwrap()
    });
    cx.update(|window, cx| gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx));
    cx.run_until_parked();
    if keyboard {
        cx.simulate_keystrokes("enter");
    } else {
        click_selector(cx, "select-document-status");
    }
    cx.run_until_parked();
    let menu = rendered(cx, "select-menu");
    let current = rendered(cx, leak(format!("select-option-{index}")));
    assert!(within(current, menu), "the current status must be visible when the long menu first opens: {current:?} in {menu:?}");
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("stage-19".into())
    );
    // Opening reveals once; later redraws must leave manual browsing alone.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let before_scroll = rendered(cx, leak(format!("select-option-{index}")));
    let position = rendered(cx, "select-menu").center();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(100.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let scrolled = rendered(cx, leak(format!("select-option-{index}")));
    assert!(
        scrolled.top() > before_scroll.top(),
        "the menu can be manually scrolled"
    );
    let scrolled_position = scrolled.top() - rendered(cx, "select-menu").top();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        rendered(cx, leak(format!("select-option-{index}"))).top()
            - rendered(cx, "select-menu").top(),
        scrolled_position,
        "a redraw must not snap back to the selected status"
    );
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert!(
        within(
            rendered(cx, leak(format!("select-option-{}", index - 1))),
            rendered(cx, "select-menu")
        ),
        "keyboard navigation reveals its next choice after manual scrolling"
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        select.read_with(cx, |select, _| select.selected_value().map(str::to_owned)),
        Some("stage-18".into())
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        within(
            rendered(cx, leak(format!("select-option-{}", index - 1))),
            rendered(cx, "select-menu")
        ),
        "reopening keeps the new draft choice visible"
    );
    // Choose a visible item in the middle of the list, away from either
    // end. Reopening must not move that already visible choice to the top.
    let position = rendered(cx, "select-menu").center();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(300.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let middle = leak(format!("select-option-{}", index - 12));
    let item = rendered(cx, middle);
    let menu = rendered(cx, "select-menu");
    assert!(
        within(item, menu),
        "the middle choice is fully visible before selecting"
    );
    let previous_position = item.top() - menu.top();
    click_selector(cx, middle);
    assert_eq!(
        select.read_with(cx, |select, _| select.selected_value().map(str::to_owned)),
        Some("stage-07".into())
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        rendered(cx, middle).top() - rendered(cx, "select-menu").top(),
        previous_position,
        "reopening must preserve the position of an already visible choice"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        documents.package_bytes("active"),
        before,
        "menu navigation only edits the draft"
    );
}

/// Replaces a field's text as typing does.
fn retype(cx: &mut VisualTestContext, field: &Field, text: &str) {
    focus_field(cx, field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(text);
    assert_eq!(field_value(cx, field), text);
}

fn area_value(cx: &mut VisualTestContext, area: &Area) -> String {
    area.read_with(cx, |area, cx| area.value(cx))
}

fn retype_area(cx: &mut VisualTestContext, area: &Area, text: &str) {
    cx.update(|window, cx| gpui::Focusable::focus_handle(area.read(cx), cx).focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(text);
    assert_eq!(area_value(cx, area), text);
}

/// Drags the handle of row `id` and drops it at `target`.
fn drag_row(cx: &mut VisualTestContext, id: &str, target: gpui::Point<Pixels>) {
    let start = rendered(cx, leak(format!("drag-{id}"))).center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        start + point(px(6.), px(6.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

/// A processed root document at revision 3 in the `review` status.
pub(crate) fn processed(id: &str, title: &str) -> DocumentState {
    let mut state = DocumentState::new(ProjectId::new(id), title, format!("{id}.wav"));
    state.schema.revision = 3;
    state.document_status.value = "review".into();
    state.content.tags = vec!["周会".into()];
    state.content.summary = Some("旧的总结".into());
    state.results.transcript = vec![TranscriptSegment {
        id: "t-1".into(),
        start_ms: 0,
        end_ms: 4_000,
        text: "先过一下页面".into(),
    }];
    state.results.diarization = vec![DiarizationSegment {
        id: "d-1".into(),
        speaker_id: "speaker-0".into(),
        speaker_profile_id: None,
        speaker_label: None,
        start_ms: 0,
        end_ms: 4_000,
        confidence: None,
    }];
    state.results.speaker_segments = domain::align_transcript_and_speakers(
        &state.results.transcript,
        &state.results.diarization,
    );
    state.record_processing(ProcessingRecord {
        processing_revision: 1,
        completed_at_ms: 1_788_000_000_000,
        input_audio_checksum: Some("cd".repeat(32)),
        asr: ModelProvenance {
            backend_id: "whisper-rs".into(),
            model_id: "large-v3-turbo".into(),
            revision: Some("r1".into()),
            artifact_sha256: Some("ab".repeat(32)),
            artifact_path: "models/large-v3-turbo".into(),
            artifacts: Vec::new(),
        },
        diarization: Default::default(),
        alignment: Default::default(),
        runtime: RuntimeProvenance {
            device: "Metal".into(),
            language: Some("zh".into()),
            ..Default::default()
        },
    });
    state
}

#[gpui::test]
fn scanner_issues_list_path_and_reason_until_the_files_are_fixed(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(
        DocumentState::new(ProjectId::new("valid"), "Valid", "valid.wav"),
        "# Valid\n",
    );
    let broken = root.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("state.json"), "{ not json").unwrap();
    let stray = root.path().join("notes.txt");
    std::fs::write(&stray, "stray").unwrap();
    let (view, cx, changes) = open_on(cx, &root);
    click_selector(cx, "document-issues-toggle");

    let issues = view.read_with(cx, |root, _| root.document_issues.clone());
    assert_eq!(issues.len(), 2, "{issues:#?}");
    let issue = |path: &Path| {
        issues
            .iter()
            .find(|issue| issue.path == path.display().to_string())
            .unwrap_or_else(|| panic!("no issue names {}: {issues:#?}", path.display()))
    };
    assert!(
        issue(&broken)
            .message
            .starts_with("The document package cannot be read"),
        "{issues:#?}"
    );
    assert!(
        issue(&stray).message.contains("not a document package"),
        "{issues:#?}"
    );
    // The panel heads the Meetings content plane, above the document.
    let page = rendered(cx, "meetings-page");
    let panel = rendered(cx, "document-issues");
    assert!(within(panel, page), "{panel:?} in {page:?}");
    assert!(panel.bottom() <= rendered(cx, "meeting-content").top());
    for index in 0..2 {
        let entry = rendered(cx, leak(format!("document-issue-{index}")));
        assert!(
            entry.left() >= panel.left() && entry.right() <= panel.right(),
            "issue {index} lies in the panel: {entry:?} in {panel:?}"
        );
    }
    assert!(cx.debug_bounds("project-row-valid").is_some());

    // An external editor explicitly repairs the invalid package. Managed
    // repository saves must refuse to overwrite unreadable existing assets.
    std::fs::remove_file(&stray).unwrap();
    persistence::DocumentPackage::new(
        &broken,
        DocumentState::new(ProjectId::new("broken"), "Repaired", "broken.wav"),
        "# Repaired\n",
    )
    .unwrap()
    .save()
    .unwrap();
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.document_issues.is_empty() && root.projects.len() == 2
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("document-issues").is_none(),
        "no issues, no panel"
    );
    assert!(cx.debug_bounds("project-row-broken").is_some());
}

#[gpui::test]
fn scanner_issues_read_in_the_ui_locale_and_keep_their_kind_after_a_move(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    for (index, id) in ["a", "b"].into_iter().enumerate() {
        let mut document = DocumentState::new(ProjectId::new(id), id.to_uppercase(), "a.wav");
        document.hierarchy.sibling_order = index as u32;
        root.put(document, "# Document\n");
    }
    let broken = root.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("state.json"), "{ not json").unwrap();
    std::fs::write(root.path().join("notes.txt"), "stray").unwrap();
    let (view, cx, _changes) = open_on(cx, &root);
    click_selector(cx, "document-issues-toggle");
    for index in 0..2 {
        rendered(cx, leak(format!("document-issue-{index}")));
    }

    // Under zh-CN the panel reads each issue in Chinese, not the scanner's
    // English sentence.
    let sentences = |cx: &mut VisualTestContext, locale| {
        view.read_with(cx, |root, _| {
            root.document_issues
                .iter()
                .map(|issue| {
                    (
                        issue.kind.clone(),
                        issue.message.clone(),
                        meetings::issue_sentence(issue, locale),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    let chinese = sentences(cx, i18n::Locale::ZhCn);
    assert_eq!(chinese.len(), 2, "{chinese:#?}");
    for (kind, message, sentence) in &chinese {
        let kind = kind.as_ref().expect("the scanner names the issue's kind");
        assert_eq!(sentence, &i18n::document_issue(i18n::Locale::ZhCn, kind));
        assert_ne!(sentence, message, "not the English fallback");
        let expected = match kind {
            application::DocumentIssueKind::InvalidPackage { .. } => "无法读取",
            application::DocumentIssueKind::UnexpectedFile { .. } => "不是文档包",
            other => panic!("unexpected issue {other:?}"),
        };
        assert!(sentence.contains(expected), "{sentence}");
    }
    for (_, message, sentence) in sentences(cx, i18n::Locale::EnUs) {
        assert_eq!(sentence, message);
    }

    // A tree move returns the issues with the listing; they keep their kind.
    let before_a = rendered(cx, "before-a").center();
    drag_row(cx, "b", before_a);
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "b" && project.hierarchy.sibling_order == 0)
    });
    let moved = sentences(cx, i18n::Locale::ZhCn);
    assert_eq!(
        moved, chinese,
        "the moved listing keeps the localized issues"
    );
}

#[gpui::test]
fn detail_shows_revision_status_provenance_and_statistics(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(processed("weekly", "产品周会"), "# 产品周会\n");
    let mut unprocessed = DocumentState::new(ProjectId::new("new"), "New", "new.wav");
    unprocessed.hierarchy.sibling_order = 1;
    root.put(unprocessed, "# New\n");
    let (view, cx, _changes) = open_on(cx, &root);

    select_row(cx, &view, "weekly");
    click_selector(cx, "open-project-detail");
    let panel = rendered(cx, "detail-panel");
    for row in [
        "detail-title",
        "detail-id",
        "detail-audio-path",
        "detail-parent",
        "detail-document-status",
        "detail-revision",
        "detail-asr",
        "detail-diarization",
        "detail-device",
        "detail-language",
        "detail-input-checksum",
        "detail-transcript-segments",
        "detail-speakers",
        "detail-speech-duration",
        "detail-characters",
        "detail-provenance-gaps",
    ] {
        let bounds = rendered(cx, row);
        assert!(
            bounds.left() >= panel.left() && bounds.right() <= panel.right(),
            "`{row}` lies in Detail: {bounds:?} in {panel:?}"
        );
    }
    assert!(cx.debug_bounds("detail-processing-record").is_none());
    // The rows are the facts of the document as it was read from disk.
    let sections = view.read_with(cx, |root, _| {
        let weekly = root
            .projects
            .iter()
            .find(|project| project.id().as_str() == "weekly")
            .unwrap();
        detail::detail_sections(weekly, &root.projects, root.ui.locale, None)
    });
    let value = |key: &str| {
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.key == key)
            .map(|row| row.value.clone())
    };
    assert_eq!(value("detail_revision").as_deref(), Some("3"));
    assert_eq!(value("detail_document_status").as_deref(), Some("待复核"));
    assert_eq!(
        value("detail_asr").as_deref(),
        Some("whisper-rs · large-v3-turbo · r1")
    );
    assert_eq!(value("detail_speakers").as_deref(), Some("1"));

    // Detail stays open and follows the selection.
    select_row(cx, &view, "new");
    rendered(cx, "detail-processing-record");
    assert!(cx.debug_bounds("detail-asr").is_none());
    assert!(cx.debug_bounds("detail-revision").is_some());
}

/// Wheel over the reading area's padding so nested editor inputs do not
/// consume a gesture meant for the document itself.
pub(crate) fn scroll_document(cx: &mut VisualTestContext, view: &View) -> gpui::Point<Pixels> {
    let area = rendered(cx, "meeting-content");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(area.left() + px(4.), area.center().y),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-240.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let offset = view.read_with(cx, |root, _| root.scroll.meeting_content.offset());
    assert!(offset.y < px(0.), "the document really scrolls: {offset:?}");
    offset
}

#[gpui::test]
fn a_new_document_starts_at_the_top_without_resetting_the_current_read(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    for (index, id) in ["first", "second"].into_iter().enumerate() {
        let mut document = processed(id, id);
        document.hierarchy.sibling_order = index as u32;
        document.content.summary = Some(format!("Reading {id} document.\n").repeat(120));
        documents.put(document, "# Original document\n");
    }
    let (view, cx, changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    select_row(cx, &view, "first");
    let offset = scroll_document(cx, &view);
    select_row(cx, &view, "first");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        offset,
        "selecting the current document keeps its reading position"
    );
    let mut external = documents.read("first").state;
    external.content.title = "Refreshed externally".into();
    documents.external_save(external);
    let first_bytes = documents.package_bytes("first");
    let second_bytes = documents.package_bytes("second");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.title() == "Refreshed externally")
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        offset,
        "a background refresh of the same document preserves the current read"
    );
    select_row(cx, &view, "second");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default(),
        "a newly selected document must not inherit the previous document's middle"
    );
    let offset = scroll_document(cx, &view);
    click_selector(cx, "nav-Settings");
    click_selector(cx, "nav-Meetings");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        offset,
        "visiting another route keeps the same document's reading position"
    );
    assert_eq!(documents.package_bytes("first"), first_bytes);
    assert_eq!(documents.package_bytes("second"), second_bytes);
}

#[gpui::test]
fn switching_document_views_starts_at_the_top_and_keeps_the_edit_draft(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    let mut document = processed("weekly", "Weekly");
    document.content.summary = Some("A long summary paragraph.\n".repeat(120));
    document.results.transcript = (0..100)
        .map(|index| TranscriptSegment {
            id: format!("t-{index}"),
            start_ms: index * 4000,
            end_ms: (index + 1) * 4000,
            text: format!("Transcript paragraph {index}."),
        })
        .collect();
    document.results.diarization[0].end_ms = 400000;
    document.results.speaker_segments = domain::align_transcript_and_speakers(
        &document.results.transcript,
        &document.results.diarization,
    );
    documents.put(document, "# Original body\n");
    let before = documents.package_bytes("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    view.update(cx, |root, cx| {
        root.ui.locale = i18n::Locale::EnUs;
        cx.notify();
    });
    select_row(cx, &view, "weekly");
    scroll_document(cx, &view);
    click_selector(cx, "content-mode-original");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default(),
        "Original starts at its first speaker and sentence"
    );
    let offset = scroll_document(cx, &view);
    click_selector(cx, "content-mode-original");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        offset
    );
    open_editor(cx, &view, "weekly");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default()
    );
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "Keep my draft across views");
    scroll_document(cx, &view);
    click_selector(cx, "content-mode-summary");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default()
    );
    open_editor(cx, &view, "weekly");
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default()
    );
    assert_eq!(field_value(cx, &title), "Keep my draft across views");
    assert_eq!(documents.package_bytes("weekly"), before);
}

#[gpui::test]
fn an_unprocessed_selection_has_a_selected_view_and_keeps_its_edit_draft(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("ready", "Ready"), "# Processed body\n");
    let audio = documents.path().join("new/recording.wav");
    let mut imported = DocumentState::new(
        ProjectId::new("new"),
        "New recording",
        audio.display().to_string(),
    );
    imported.hierarchy.sibling_order = 1;
    documents.put(imported, "# New recording\n");
    std::fs::write(audio, b"fixture recording").unwrap();
    let ready_bytes = documents.package_bytes("ready");
    let new_bytes = documents.package_bytes("new");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    select_row(cx, &view, "ready");
    click_selector(cx, "content-mode-original");
    select_row(cx, &view, "new");
    wait_for(cx, &view, |root| !root.audio.pending());
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Summary,
        "the hidden Original option must not leave every visible choice unselected"
    );
    assert!(cx.debug_bounds("content-mode-original").is_none());
    assert!(within(
        rendered(cx, "models-required"),
        rendered(cx, "meeting-content")
    ));
    click_selector(cx, "go-to-models");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Models);
    click_selector(cx, "nav-Meetings");
    open_editor(cx, &view, "new");
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "Keep the unprocessed draft");
    select_row(cx, &view, "ready");
    select_row(cx, &view, "new");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("new"))
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Edit
    );
    assert_eq!(field_value(cx, &title), "Keep the unprocessed draft");
    assert_eq!(documents.package_bytes("ready"), ready_bytes);
    assert_eq!(documents.package_bytes("new"), new_bytes);
}

#[gpui::test]
fn removing_the_stored_processing_record_reselects_an_available_view(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "Weekly"), "# Original body\n");
    let (view, cx, changes) = open_on(cx, &documents);
    click_selector(cx, "content-mode-original");
    let mut external = documents.read("weekly").state;
    external.processing = Default::default();
    external.results = Default::default();
    documents.external_save(external);
    let before = documents.package_bytes("weekly");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.id().as_str() == "weekly" && p.processing.latest.is_none())
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Summary,
        "an external refresh must not leave the disappeared Original option selected"
    );
    assert!(cx.debug_bounds("content-mode-original").is_none());
    open_editor(cx, &view, "weekly");
    let (title, ..) = editor_fields(cx, &view);
    assert_eq!(field_value(cx, &title), "Weekly");
    assert_eq!(documents.package_bytes("weekly"), before);
}

#[gpui::test]
fn a_failed_later_attempt_keeps_saved_results_readable_and_correctable(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "Weekly"), "# Original body\n");
    let (view, cx, changes) = open_on(cx, &documents);
    click_selector(cx, "content-mode-original");
    let mut external = documents.read("weekly").state;
    external.set_processing_error("A later processing attempt failed");
    documents.external_save(external);
    let failed = documents.read("weekly");
    let before = documents.package_bytes("weekly");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects.iter().any(|p| {
            p.processing.last_error.as_deref() == Some("A later processing attempt failed")
        })
    });
    assert!(
        cx.debug_bounds("content-mode-original").is_some(),
        "a failed later attempt must not hide the last successful transcript"
    );
    rendered(cx, "document-status-Failed");
    click_selector(cx, "document-speakers-toggle");
    rendered(cx, "document-speakers");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Original
    );
    click_selector(cx, "content-mode-summary");
    rendered(cx, "document-summary");
    assert_eq!(
        documents.package_bytes("weekly"),
        before,
        "reading writes nothing"
    );
    click_selector(cx, "content-mode-original");
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    retype(cx, &name, "Host");
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none()
            && documents.revision("weekly") == failed.state.revision() + 1
    });
    let after = documents.read("weekly");
    assert_eq!(after.state.processing, failed.state.processing);
    assert_eq!(after.markdown, failed.markdown);
    assert_eq!(
        after.state.results.transcript,
        failed.state.results.transcript
    );
    assert!(after
        .state
        .results
        .speaker_segments
        .iter()
        .all(|s| s.speaker_label.as_deref() == Some("Host")));
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
    rendered(cx, "document-status-Failed");
}

#[gpui::test]
fn the_editor_saves_every_semantic_field_with_the_loaded_revision(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(processed("weekly", "产品周会"), "# 产品周会\n\n旧正文\n");
    let before = root.read("weekly");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    open_editor(cx, &view, "weekly");

    let (title, tags, summary, body) = editor_fields(cx, &view);
    assert_eq!(field_value(cx, &title), "产品周会");
    assert_eq!(field_value(cx, &tags), "周会");
    assert_eq!(area_value(cx, &summary), "旧的总结");
    assert_eq!(area_value(cx, &body), "# 产品周会\n\n旧正文\n");
    retype(cx, &title, "产品周会（修订）");
    retype(cx, &tags, "周会, 决策");
    retype_area(cx, &summary, "新的总结");
    retype_area(cx, &body, "# 产品周会\n\n修订后的正文");
    // The status select: `review` is selected; Down moves to `final`.
    let status = view.read_with(cx, |root, _| root.editor.status.clone());
    cx.update(|window, cx| gpui::Focusable::focus_handle(status.read(cx), cx).focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        status.read_with(cx, |select, _| select.selected_value().map(str::to_string)),
        Some("final".into())
    );
    assert_eq!(
        root.revision("weekly"),
        before.state.revision(),
        "edits stay a draft until saved"
    );

    click_selector(cx, "save-document");
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    let after = root.read("weekly");
    assert_eq!(after.state.revision(), before.state.revision() + 1);
    assert_eq!(after.state.title(), "产品周会（修订）");
    assert_eq!(after.state.content.tags, ["周会", "决策"]);
    assert_eq!(after.state.content.summary.as_deref(), Some("新的总结"));
    assert_eq!(after.state.document_status.value, "final");
    assert_eq!(after.markdown, "# 产品周会\n\n修订后的正文");
    assert_eq!(
        after.state.processing, before.state.processing,
        "processing is not editable"
    );
    assert_eq!(after.state.results, before.state.results);
    // The tree and the notice follow the saved document.
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.title() == "产品周会（修订）")
    });
    let saved = i18n::message(i18n::Locale::ZhCn, "document_saved", "产品周会（修订）");
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), saved);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn the_save_shortcut_keeps_the_editor_focus_scroll_and_insertion_point(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "Weekly"), "# Weekly\n");
    let revision = documents.revision("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    select_row(cx, &view, "weekly");
    open_editor(cx, &view, "weekly");
    let (title, _, _, body) = editor_fields(cx, &view);
    focus_field(cx, &title);
    cx.simulate_keystrokes("ctrl-tab ctrl-tab ctrl-tab ctrl-tab");
    assert!(
        cx.update(|window, cx| gpui::Focusable::focus_handle(body.read(cx), cx).is_focused(window)),
        "keyboard traversal reaches the Markdown field"
    );
    let edited = format!(
        "# Weekly\n\n{}pending",
        "A paragraph in a long document.\n\n".repeat(30)
    );
    retype_area(cx, &body, &edited);
    cx.simulate_keystrokes("left left left");
    cx.run_until_parked();
    let focus = cx.update(|window, cx| window.focused(cx));
    let scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.offset());
    cx.simulate_keystrokes("cmd-s");
    wait_for(cx, &view, |root| {
        root.editor.saving.is_none()
            && root.editor.draft_revision(&ProjectId::new("weekly")) == Some(revision + 1)
            && root
                .projects
                .iter()
                .any(|p| p.id().as_str() == "weekly" && p.revision() == revision + 1)
    });
    assert_eq!(documents.read("weekly").markdown, edited);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        scroll
    );
    cx.simulate_input("!");
    assert_eq!(
        area_value(cx, &body),
        format!("{}pend!ing", edited.strip_suffix("pending").unwrap()),
        "save completion preserves the insertion point"
    );
    assert_eq!(
        documents.read("weekly").markdown,
        edited,
        "new typing stays a draft"
    );
    let saved = documents.package_bytes("weekly");
    click_selector(cx, "content-mode-summary");
    cx.simulate_keystrokes("cmd-s");
    assert!(view.read_with(cx, |root, _| root.editor.saving.is_none()));
    assert_eq!(
        documents.package_bytes("weekly"),
        saved,
        "Summary does not save hidden fields"
    );
    cx.simulate_keystrokes("cmd-4 cmd-s");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Settings);
    assert!(view.read_with(cx, |root, _| root.editor.saving.is_none()));
    assert_eq!(
        documents.package_bytes("weekly"),
        saved,
        "Settings does not save a document draft"
    );
}

#[gpui::test]
fn the_save_shortcut_does_not_write_a_draft_hidden_by_another_documents_read_error(
    cx: &mut TestAppContext,
) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    documents.put(processed("b", "B"), "# B\n");
    let a = documents.package_bytes("a");
    let b = documents.package_bytes("b");
    let revision = documents.revision("a");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    open_editor(cx, &view, "a");
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "A draft");
    std::fs::remove_file(documents.path().join("b/state.json")).unwrap();
    select_row(cx, &view, "b");
    wait_for(cx, &view, |root| {
        root.editor.read_errors.contains_key(&ProjectId::new("b"))
    });
    std::fs::write(documents.path().join("b/state.json"), &b[0]).unwrap();
    select_row(cx, &view, "a");
    open_editor(cx, &view, "a");
    assert_eq!(field_value(cx, &title), "A draft");
    select_row(cx, &view, "b");
    rendered(cx, "document-editor-read-failed");
    assert_eq!(
        view.read_with(cx, |root, _| root.editor.shown().cloned()),
        Some(ProjectId::new("a")),
        "the retained editor belongs to A while B shows its read error"
    );
    cx.simulate_keystrokes("cmd-s");
    assert!(view.read_with(cx, |root, _| root.editor.saving.is_none()));
    assert_eq!(
        documents.package_bytes("a"),
        a,
        "an invisible draft must not be saved"
    );
    assert_eq!(documents.package_bytes("b"), b);
    select_row(cx, &view, "a");
    cx.simulate_keystrokes("cmd-s");
    wait_for(cx, &view, |root| {
        root.editor.draft_revision(&ProjectId::new("a")) == Some(revision + 1)
    });
    assert_eq!(documents.read("a").state.title(), "A draft");
}

#[gpui::test]
fn a_save_after_an_external_change_conflicts_and_leaves_the_disk_unchanged(
    cx: &mut TestAppContext,
) {
    let root = DocumentsRoot::new();
    root.put(processed("weekly", "产品周会"), "# 产品周会\n");
    let loaded = root.revision("weekly");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    open_editor(cx, &view, "weekly");
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "界面里的标题");

    // Another writer changes the document after the draft was read.
    let mut external = root.read("weekly").state;
    external.content.title = "外部修改的标题".into();
    root.external_save(external);
    let on_disk = root.package_bytes("weekly");

    cx.simulate_keystrokes("cmd-s");
    wait_for(cx, &view, |root| root.document_conflict.is_some());
    let conflict = view.read_with(cx, |root, _| root.document_conflict.clone().unwrap());
    assert_eq!(conflict.document_id.as_str(), "weekly");
    assert_eq!(conflict.title, "产品周会", "the notice names the document");
    assert_eq!((conflict.expected, conflict.actual), (loaded, loaded + 1));
    let plane = rendered(cx, "meetings-page");
    let notice = rendered(cx, "document-conflict");
    assert!(within(notice, plane));
    assert!(within(rendered(cx, "reload-conflicted-document"), notice));
    assert_eq!(
        root.package_bytes("weekly"),
        on_disk,
        "the refused save wrote nothing"
    );
    assert_eq!(
        field_value(cx, &title),
        "界面里的标题",
        "the draft stays until the user reloads"
    );

    click_selector(cx, "reload-conflicted-document");
    wait_for(cx, &view, |root| {
        root.document_conflict.is_none()
            && root
                .projects
                .iter()
                .any(|project| project.title() == "外部修改的标题")
            && root.editor.draft_revision(&ProjectId::new("weekly")) == Some(loaded + 1)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("document-conflict").is_none());
    assert_eq!(field_value(cx, &title), "外部修改的标题");

    // A save based on the reloaded revision succeeds.
    retype(cx, &title, "合并后的标题");
    click_selector(cx, "save-document");
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    let saved = root.read("weekly").state;
    assert_eq!(saved.revision(), loaded + 2);
    assert_eq!(saved.title(), "合并后的标题");
}

#[gpui::test]
fn a_long_chinese_conflict_keeps_recovery_and_the_editor_visible(cx: &mut TestAppContext) {
    long_conflict_recovery(cx, i18n::Locale::ZhCn, false);
}

#[gpui::test]
fn a_long_english_conflict_keeps_recovery_beside_detail(cx: &mut TestAppContext) {
    long_conflict_recovery(cx, i18n::Locale::EnUs, true);
}

fn long_conflict_recovery(cx: &mut TestAppContext, locale: i18n::Locale, detail: bool) {
    let documents = DocumentsRoot::new();
    let long_title = match locale {
        i18n::Locale::ZhCn => "产品规划与用户访谈记录".repeat(80),
        i18n::Locale::EnUs => "Product planning and customer interviews ".repeat(70),
    };
    documents.put(processed("weekly", &long_title), "# Original document\n");
    let revision = documents.revision("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    select_row(cx, &view, "weekly");
    open_editor(cx, &view, "weekly");
    view.update(cx, |root, cx| {
        root.ui.locale = locale;
        cx.notify();
    });
    if detail {
        click_selector(cx, "open-project-detail");
    }
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "Local draft");
    let mut external = documents.read("weekly").state;
    external.content.summary = Some("External update".into());
    documents.external_save(external);
    let external_bytes = documents.package_bytes("weekly");
    cx.simulate_keystrokes("cmd-s");
    wait_for(cx, &view, |root| {
        root.document_conflict.is_some()
            && root
                .projects
                .iter()
                .any(|p| p.id().as_str() == "weekly" && p.revision() == revision + 1)
    });
    // Acknowledge the global notification; the persistent conflict panel
    // must keep recovery available after the notification is dismissed.
    click_selector(cx, "notice-toast-close");
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    wait_for(cx, &view, |root| root.ui.toast().is_none());
    let notice = rendered(cx, "document-conflict");
    let plane = rendered(cx, "meetings-page");
    assert!(
        within(notice, plane),
        "the long conflict must stay in the content plane: {notice:?} in {plane:?}"
    );
    assert!(within(rendered(cx, "reload-conflicted-document"), notice));
    let editor = rendered(cx, "meeting-content");
    assert!(
        editor.size.height >= px(80.),
        "the conflict must leave room to keep editing: {editor:?} below {notice:?}"
    );
    let message = rendered(cx, "document-conflict-message");
    assert_eq!(
        rendered(cx, "document-conflict-scrollbar-viewport").right(),
        message.right(),
        "the recovery scrollbar stays at its reading area's right edge"
    );
    assert_eq!(
        rendered(cx, "meeting-content-scrollbar-viewport").right(),
        plane.right(),
        "compact padding must not move the document scrollbar away from its container edge"
    );
    let text = rendered(cx, "document-conflict-text");
    assert!(
        text.size.height > message.size.height,
        "the full title is kept in a bounded reading area"
    );
    focus_field(cx, &title);
    let focus = cx.update(|window, cx| window.focused(cx));
    let editor_scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.offset());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: message.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-10000.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let text = rendered(cx, "document-conflict-text");
    let message = rendered(cx, "document-conflict-message");
    assert!(
        text.top() < message.top() && text.bottom() <= message.bottom() + px(1.),
        "scrolling reaches the full title, recovery hint and revision"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        editor_scroll
    );
    cx.simulate_input("!");
    assert_eq!(
        field_value(cx, &title),
        "Local draft!",
        "reading the conflict must not interrupt editing"
    );
    assert_eq!(
        documents.package_bytes("weekly"),
        external_bytes,
        "the refused save and later typing leave the external version intact"
    );
    click_selector(cx, "reload-conflicted-document");
    wait_for(cx, &view, |root| {
        root.document_conflict.is_none()
            && root.editor.draft_revision(&ProjectId::new("weekly")) == Some(revision + 1)
    });
    assert_eq!(field_value(cx, &title), long_title);
    assert_eq!(
        documents.package_bytes("weekly"),
        external_bytes,
        "reload is read-only"
    );
    // A later conflict gets its own reading position, rather than inheriting
    // the bottom of the previous conflict's long message.
    retype(cx, &title, "Second draft");
    let mut external = documents.read("weekly").state;
    external.content.summary = Some("Another external update".into());
    documents.external_save(external);
    let external_bytes = documents.package_bytes("weekly");
    cx.simulate_keystrokes("cmd-s");
    wait_for(cx, &view, |root| {
        root.document_conflict
            .as_ref()
            .is_some_and(|c| c.expected == revision + 1 && c.actual == revision + 2)
            && root
                .projects
                .iter()
                .any(|p| p.id().as_str() == "weekly" && p.revision() == revision + 2)
    });
    assert!(
        rendered(cx, "document-conflict-text").top()
            >= rendered(cx, "document-conflict-message").top(),
        "a new conflict starts at the beginning of its message"
    );
    assert_eq!(field_value(cx, &title), "Second draft");
    assert_eq!(documents.package_bytes("weekly"), external_bytes);
}

/// `rec` with the children `a` and `b`, and a second recording `other`.
fn tree_fixture(root: &DocumentsRoot) {
    let recording = DocumentState::new(ProjectId::new("rec"), "Recording", "rec.wav");
    let mut a = recording.semantic_child(ProjectId::new("a"), "A", 0, 1_000, Vec::new());
    a.hierarchy.sibling_order = 0;
    let mut b = recording.semantic_child(ProjectId::new("b"), "B", 1_000, 2_000, Vec::new());
    b.hierarchy.sibling_order = 1;
    let mut other = DocumentState::new(ProjectId::new("other"), "Other", "other.wav");
    other.hierarchy.sibling_order = 1;
    for document in [recording, a, b, other] {
        let markdown = format!("# {}\n", document.title());
        root.put(document, &markdown);
    }
}

fn revisions(root: &DocumentsRoot) -> [u64; 4] {
    ["rec", "a", "b", "other"].map(|id| root.revision(id))
}

#[gpui::test]
fn tree_moves_are_revision_checked_commands(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    tree_fixture(&root);
    let [rec, a, b, other] = revisions(&root);
    let (view, cx, _changes) = open_on(cx, &root);

    // `b` dropped before `a`: only the reordered siblings are written.
    let before_a = rendered(cx, "before-a").center();
    drag_row(cx, "b", before_a);
    wait_for(cx, &view, |_| root.revision("b") == b + 1);
    assert_eq!(revisions(&root), [rec, a + 1, b + 1, other]);
    assert_eq!(
        [root.read("b"), root.read("a")].map(|record| record.state.hierarchy.sibling_order),
        [0, 1]
    );
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .find(|project| project.id().as_str() == "b")
            .is_some_and(|project| project.revision() == b + 1)
    });
    cx.run_until_parked();
    assert!(
        rendered(cx, "project-row-b").bottom() <= rendered(cx, "project-row-a").top(),
        "the tree shows the new order"
    );

    // `b` dropped onto `a`: only `b` is written.
    let onto_a = rendered(cx, "project-row-a").center();
    drag_row(cx, "b", onto_a);
    wait_for(cx, &view, |_| root.revision("b") == b + 2);
    assert_eq!(revisions(&root), [rec, a + 1, b + 2, other]);
    let moved = root.read("b").state.hierarchy;
    assert_eq!(
        (moved.parent_id.as_ref().map(ProjectId::as_str), moved.depth),
        (Some("a"), 2)
    );
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));

    // Another writer changes `other` after the tree was read: its drag is
    // refused, nothing is written and the tree reloads.
    let mut external = root.read("other").state;
    external.content.title = "Other (edited)".into();
    root.external_save(external);
    let written = revisions(&root);
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "a" && project.revision() == a + 1)
    });
    let before_rec = rendered(cx, "before-rec").center();
    drag_row(cx, "other", before_rec);
    wait_for(cx, &view, |root| root.document_conflict.is_some());
    let conflict = view.read_with(cx, |root, _| root.document_conflict.clone().unwrap());
    assert_eq!(conflict.document_id.as_str(), "other");
    assert_eq!(conflict.title, "Other");
    assert_eq!(revisions(&root), written, "the refused move wrote nothing");
    rendered(cx, "document-conflict");
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.title() == "Other (edited)")
    });
}

/// Presses `keys` while the tree holds focus and waits until the moved
/// document `id` reaches `revision` in the tree, then until the tree has
/// drawn it.
fn shift_row(cx: &mut VisualTestContext, view: &View, keys: &str, id: &str, revision: u64) {
    cx.simulate_keystrokes(keys);
    let id = ProjectId::new(id);
    wait_for(cx, view, |root| {
        root.projects
            .iter()
            .any(|project| project.id() == &id && project.revision() == revision)
    });
    cx.run_until_parked();
}

/// Row `id` is selected, the tree holds focus and its ring is on the row.
#[track_caller]
fn tree_focus_on(cx: &mut VisualTestContext, view: &View, id: &str) {
    let focus = view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.selected_project.as_ref().map(ProjectId::as_str),
            Some(id),
            "the moved row stays selected"
        );
        root.tree_focus.clone()
    });
    assert!(
        cx.update(|window, _| focus.is_focused(window)),
        "the tree keeps focus"
    );
    let row = rendered(cx, leak(format!("project-row-{id}")));
    assert!(
        within(rendered(cx, "focus-ring"), row),
        "the ring is on the moved row"
    );
}

fn parent_of(root: &DocumentsRoot, id: &str) -> (Option<String>, u8, u32) {
    let hierarchy = root.read(id).state.hierarchy;
    (
        hierarchy.parent_id.map(|parent| parent.to_string()),
        hierarchy.depth,
        hierarchy.sibling_order,
    )
}

#[gpui::test]
fn option_arrows_move_the_selected_row_through_the_tree_commands(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    tree_fixture(&root);
    let [rec, a, b, other] = revisions(&root);
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "b");

    // Option-Up: `b` before `a`; only the reordered siblings are written.
    shift_row(cx, &view, "alt-up", "b", b + 1);
    assert_eq!(revisions(&root), [rec, a + 1, b + 1, other]);
    assert_eq!(
        [root.read("b"), root.read("a")].map(|record| record.state.hierarchy.sibling_order),
        [0, 1]
    );
    assert!(
        rendered(cx, "project-row-b").bottom() <= rendered(cx, "project-row-a").top(),
        "the tree shows the new order"
    );
    tree_focus_on(cx, &view, "b");
    let order_updated = i18n::text(i18n::Locale::ZhCn, "order_updated");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        order_updated
    );

    // Option-Down: back after `a`.
    shift_row(cx, &view, "alt-down", "b", b + 2);
    assert_eq!(revisions(&root), [rec, a + 2, b + 2, other]);
    tree_focus_on(cx, &view, "b");

    // Option-Right: into `a`, which opens so the row stays in view; only
    // `b` is written.
    shift_row(cx, &view, "alt-right", "b", b + 3);
    assert_eq!(revisions(&root), [rec, a + 2, b + 3, other]);
    assert_eq!(parent_of(&root, "b"), (Some("a".into()), 2, 0));
    assert!(view.read_with(cx, |root, _| root
        .ui
        .expanded_projects
        .contains(&ProjectId::new("a"))));
    tree_focus_on(cx, &view, "b");
    let hierarchy_updated = i18n::text(i18n::Locale::ZhCn, "hierarchy_updated");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        hierarchy_updated
    );

    // Option-Left: out of `a`, to the end of `rec`'s children.
    shift_row(cx, &view, "alt-left", "b", b + 4);
    assert_eq!(revisions(&root), [rec, a + 2, b + 4, other]);
    assert_eq!(parent_of(&root, "b"), (Some("rec".into()), 1, 1));
    tree_focus_on(cx, &view, "b");

    // Nothing to move past: Option-Down on the last sibling writes nothing.
    cx.simulate_keystrokes("alt-down");
    cx.run_until_parked();
    assert_eq!(revisions(&root), [rec, a + 2, b + 4, other]);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn option_arrows_are_refused_past_the_third_level_and_on_a_stale_revision(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    tree_fixture(&root);
    let b = root.read("b").state;
    let b1 = b.semantic_child(ProjectId::new("b1"), "B1", 1_000, 1_500, Vec::new());
    root.put(b1, "# B1\n");
    let before = revisions(&root);
    let b1_revision = root.revision("b1");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "b");

    // Into `a` would put `b1` on a fourth level: refused like a drop.
    cx.simulate_keystrokes("alt-right");
    let refusal = i18n::message(
        i18n::Locale::ZhCn,
        "move_failed",
        &application::DocumentCommandError::Tree(domain::ProjectTreeError::MaximumDepth)
            .to_string(),
    );
    wait_for(cx, &view, |root| root.ui.notice == refusal);
    assert_eq!(revisions(&root), before, "the refused move wrote nothing");
    assert_eq!(root.revision("b1"), b1_revision);
    assert_eq!(parent_of(&root, "b"), (Some("rec".into()), 1, 1));
    cx.run_until_parked();
    tree_focus_on(cx, &view, "b");

    // Another writer changes `b` after the tree was read: the move is
    // refused, the conflict notice opens and nothing is written.
    let mut external = root.read("b").state;
    external.content.title = "B (edited)".into();
    root.external_save(external);
    let written = revisions(&root);
    cx.simulate_keystrokes("alt-up");
    wait_for(cx, &view, |root| root.document_conflict.is_some());
    let conflict = view.read_with(cx, |root, _| root.document_conflict.clone().unwrap());
    assert_eq!(conflict.document_id.as_str(), "b");
    assert_eq!(revisions(&root), written, "the refused move wrote nothing");
    rendered(cx, "document-conflict");
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.title() == "B (edited)")
    });
    cx.run_until_parked();
    tree_focus_on(cx, &view, "b");
}

#[gpui::test]
fn every_editor_control_carries_its_label_in_the_current_locale(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let (view, cx, _changes) = open_on(cx, &root);
    let labels = |cx: &mut VisualTestContext| {
        view.read_with(cx, |root, cx| {
            let editor = &root.editor;
            [
                editor.title.read(cx).accessibility_label().to_string(),
                editor.tags.read(cx).accessibility_label().to_string(),
                editor.summary.read(cx).accessibility_label().to_string(),
                editor.body.read(cx).accessibility_label().to_string(),
                editor.status.read(cx).accessibility_label().to_string(),
            ]
        })
    };
    let expected = |locale| {
        [
            "document_title",
            "document_tags",
            "document_summary",
            "document_body",
            "document_status",
        ]
        .map(|key| i18n::text(locale, key).to_string())
    };
    assert_eq!(labels(cx), expected(i18n::Locale::ZhCn));
    view.update(cx, |root, cx| root.set_locale(i18n::Locale::EnUs, cx));
    cx.run_until_parked();
    assert_eq!(
        labels(cx),
        expected(i18n::Locale::EnUs),
        "a locale change relabels the editor"
    );
}

/// Real runtime events with a gated processing outcome; no model inference.
fn pending_processing(
    cx: &mut VisualTestContext,
    view: &View,
) -> futures::channel::oneshot::Sender<Result<DocumentState, contracts::BackendError>> {
    let jobs = runtime::JobRuntime::default();
    let (asr_context, asr_events) = jobs.create_job();
    let (diarization_context, diarization_events) = jobs.create_job();
    let (outcome, result) = futures::channel::oneshot::channel();
    let job = application::ProjectProcessingJob {
        future: Box::pin(async move { result.await.unwrap() }),
        asr_context: asr_context.clone(),
        diarization_context: diarization_context.clone(),
        asr_events,
        diarization_events,
    };
    view.update(cx, |root, cx| {
        root.ui
            .begin_processing(root.ui.selected_project.clone().unwrap());
        root.cancel_contexts = Some((asr_context, diarization_context));
        cx.notify();
        cx.spawn(async move |this, cx| RootView::follow_processing(this, job, cx).await)
            .detach();
    });
    cx.run_until_parked();
    outcome
}

#[gpui::test]
fn processing_completion_keeps_another_documents_draft_focus_and_history(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let mut b = processed("b", "B");
    b.hierarchy.sibling_order = 1;
    b.content.summary = Some("继续整理会议记录。\n".repeat(120));
    documents.put(b, "# B\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(gpui::size(px(1180.), px(600.)));
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    select_row(cx, &view, "b");
    open_editor(cx, &view, "b");
    let (title, _, _, body) = editor_fields(cx, &view);
    retype(cx, &title, "B 未保存的标题");
    retype_area(cx, &body, "继续编辑 B");
    let scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.clone());
    scroll.set_offset(point(px(0.), px(-100.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let position = scroll.offset();
    assert!(position.y < px(0.));
    let focus = cx.update(|window, cx| window.focused(cx));
    let before = documents.package_bytes("b");
    let mut finished = documents.read("a").state;
    finished.content.summary = Some("A 的新结果".into());
    documents.external_save(finished);
    outcome.send(Ok(documents.read("a").state)).unwrap();
    wait_for(cx, &view, |root| {
        !root.ui.processing
            && root.projects.iter().any(|p| {
                p.id().as_str() == "a" && p.content.summary.as_deref() == Some("A 的新结果")
            })
    });
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.selected_project,
            Some(ProjectId::new("b")),
            "A must not replace B"
        );
        assert_eq!(root.editor.shown(), Some(&ProjectId::new("b")));
        assert_eq!(root.ui.content_mode, crate::state::ContentMode::Edit);
        assert_eq!(root.ui.notice, "已完成 A");
        assert!(root.ui.processing_project.is_none());
    });
    assert_eq!(scroll.offset(), position);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(field_value(cx, &title), "B 未保存的标题");
    assert_eq!(area_value(cx, &body), "继续编辑 B");
    assert_eq!(documents.package_bytes("b"), before);
    click_selector(cx, "navigate-back");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("a"))
    );
    click_selector(cx, "navigate-forward");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("b"))
    );
    assert_eq!(field_value(cx, &title), "B 未保存的标题");
}

#[gpui::test]
fn processing_completion_refreshes_the_still_selected_document(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(
        DocumentState::new(ProjectId::new("a"), "A", "a.wav"),
        "# A\n",
    );
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    assert!(cx.debug_bounds("content-mode-original").is_none());
    let mut finished = processed("a", "A");
    finished.schema.revision = documents.revision("a");
    documents.external_save(finished);
    outcome.send(Ok(documents.read("a").state)).unwrap();
    wait_for(cx, &view, |root| {
        !root.ui.processing
            && root.projects.iter().any(|p| {
                p.id().as_str() == "a" && p.processing_status() == domain::ProjectStatus::Ready
            })
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("a"))
    );
    rendered(cx, "content-mode-original");
}

#[gpui::test]
fn a_failed_document_read_waits_for_retry_and_recovers_restored_files(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let before = documents.package_bytes("a");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    // An external editor moves the package between list loading and Edit.
    std::fs::remove_file(documents.path().join("a/state.json")).unwrap();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.ui.notice.contains("state.json") && root.editor.loading.is_empty()
    });
    cx.run_until_parked();
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    cx.run_until_parked();
    assert!(within(
        rendered(cx, "retry-document-read"),
        rendered(cx, "meeting-content")
    ));
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    assert!(
        notice.contains("state.json"),
        "the failed read keeps its source path"
    );
    assert!(cx.debug_bounds("document-editor-loading").is_none());
    assert!(view.read_with(cx, |root, _| root.editor.shown().is_none()));
    std::fs::write(documents.path().join("a/state.json"), &before[0]).unwrap();
    // Redrawing after the file returns does not silently restart a read.
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(view.read_with(cx, |root, _| root.editor.shown().is_none()));
    rendered(cx, "retry-document-read");
    // The failed read's stored text remains Chinese across a locale change.
    view.update(cx, |root, cx| {
        root.ui.locale = i18n::Locale::EnUs;
        cx.notify();
    });
    let reached = (0..40).any(|_| {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        cx.debug_bounds("focus-ring")
            .is_some_and(|ring| within(ring, rendered(cx, "retry-document-read")))
    });
    assert!(reached, "Tab reaches the document read recovery action");
    let enter = gpui::Keystroke::parse("enter").unwrap();
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: enter.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke: enter });
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("a"))
    });
    cx.run_until_parked();
    let (title, _, _, body) = editor_fields(cx, &view);
    assert_eq!(field_value(cx, &title), "A");
    assert_eq!(area_value(cx, &body), "# A\n");
    assert_eq!(documents.package_bytes("a"), before);
    assert!(
        view.read_with(cx, |root, _| root.ui.notice.is_empty()),
        "the restored document must not retain its failed-read notice"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(view.read_with(cx, |root, _| root.ui.toast().is_none()));
}

fn document_retry_after_locale_change(
    cx: &mut TestAppContext,
    first: i18n::Locale,
    next: i18n::Locale,
    newer_error: bool,
) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let before = documents.package_bytes("a");
    let settings = test_settings();
    let mut initial = settings.snapshot().unwrap();
    initial.ui_locale = first.code().into();
    settings.update(initial).unwrap();
    let mut services = desktop_services(
        Arc::new(FakeAssistantUiService::default()),
        settings.clone(),
    );
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(documents.repository())),
    );
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    std::fs::remove_file(documents.path().join("a/state.json")).unwrap();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.read_errors.contains_key(&ProjectId::new("a"))
    });
    cx.run_until_parked();
    let old_toast = view.read_with(cx, |root, _| root.ui.toast().unwrap().notice.clone());
    assert!(old_toast.contains("state.json"));
    // Apply the real Settings language operation while the read error is shown.
    view.update(cx, |root, cx| {
        root.navigate(Route::Settings, cx);
        root.set_locale(next, cx);
    });
    wait_for(cx, &view, |root| {
        root.ui.notice == i18n::text(next, "language_changed")
    });
    assert_eq!(settings.snapshot().unwrap().ui_locale, next.code());
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.toast().unwrap().notice.clone()),
        old_toast
    );
    std::fs::write(documents.path().join("a/state.json"), &before[0]).unwrap();
    let expected_notice = if newer_error {
        i18n::message(next, "document_read_failed", "another/state.json")
    } else {
        i18n::text(next, "language_changed").into()
    };
    view.update(cx, |root, cx| {
        root.navigate(Route::Meetings, cx);
        if newer_error {
            root.ui.notice = expected_notice.clone();
        }
        cx.notify();
    });
    cx.run_until_parked();
    click_selector(cx, "retry-document-read");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("a"))
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert!(root.editor.read_errors.is_empty());
        assert_eq!(
            root.ui.notice, expected_notice,
            "a newer notice is retained"
        );
        if newer_error {
            assert_eq!(root.ui.toast().unwrap().notice, expected_notice);
        } else {
            assert!(
                root.ui.toast().is_none(),
                "the restored document must not retain its old-language read error toast"
            );
        }
    });
    assert_eq!(documents.package_bytes("a"), before);
}

#[gpui::test]
fn document_retry_clears_the_chinese_error_after_settings_switches_to_english(
    cx: &mut TestAppContext,
) {
    document_retry_after_locale_change(cx, i18n::Locale::ZhCn, i18n::Locale::EnUs, false);
}

#[gpui::test]
fn document_retry_clears_the_english_error_after_settings_switches_to_chinese(
    cx: &mut TestAppContext,
) {
    document_retry_after_locale_change(cx, i18n::Locale::EnUs, i18n::Locale::ZhCn, false);
}

#[gpui::test]
fn document_retry_preserves_a_newer_error_after_settings_changes_language(cx: &mut TestAppContext) {
    document_retry_after_locale_change(cx, i18n::Locale::ZhCn, i18n::Locale::EnUs, true);
}

#[gpui::test]
fn a_document_read_retry_preserves_a_newer_notice_and_moved_focus(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let before = documents.package_bytes("a");
    let read_gate = crate::tests::Gate::default();
    let mut services =
        desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(gated_read_repository(documents.repository(), &read_gate)),
    );
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    std::fs::remove_file(documents.path().join("a/state.json")).unwrap();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.read_errors.contains_key(&ProjectId::new("a"))
    });
    std::fs::write(documents.path().join("a/state.json"), &before[0]).unwrap();
    read_gate.arm();
    click_selector(cx, "retry-document-read");
    wait_for(cx, &view, |_| read_gate.counts().0 == 1);

    // The user moves to a different field; a later error has the same
    // translated prefix but belongs to another document.
    let later_notice = i18n::message(
        i18n::Locale::ZhCn,
        "document_read_failed",
        "another/state.json",
    );
    view.update(cx, |root, cx| {
        root.navigate(Route::Assistant, cx);
        // A same-root list refresh must not invalidate this editor read.
        root.reload_projects(cx);
        root.ui.notice = later_notice.clone();
        cx.notify();
    });
    cx.run_until_parked();
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "continue this question");
    let moved_focus = cx.update(|window, cx| window.focused(cx));
    read_gate.release();
    wait_for(cx, &view, |root| root.editor.loading.is_empty());
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        later_notice
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), moved_focus);
    assert_eq!(field_value(cx, &prompt), "continue this question");
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Assistant);
        assert!(root.editor.read_errors.is_empty());
        assert_eq!(root.editor.shown(), Some(&ProjectId::new("a")));
        assert_eq!(root.ui.toast().unwrap().notice, later_notice);
    });
    assert_eq!(documents.package_bytes("a"), before);
}

#[gpui::test]
fn processing_status_and_cancel_belong_to_the_running_document(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let mut b = processed("b", "B");
    b.hierarchy.sibling_order = 1;
    documents.put(b, "# B\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    select_row(cx, &view, "b");
    wait_for(cx, &view, |root| !root.audio.pending());
    cx.run_until_parked();
    rendered(cx, "project-status-a-Processing");
    rendered(cx, "project-status-b-Ready");
    rendered(cx, "document-status-Ready");
    let contexts = view.read_with(cx, |root, _| root.cancel_contexts.clone().unwrap());
    click_selector(cx, "process-project");
    assert!(
        !contexts.0.is_cancelled() && !contexts.1.is_cancelled(),
        "B must not cancel A"
    );
    select_row(cx, &view, "a");
    wait_for(cx, &view, |root| !root.audio.pending());
    cx.run_until_parked();
    rendered(cx, "document-status-Processing");
    click_selector(cx, "process-project");
    assert!(
        contexts.0.is_cancelled() && contexts.1.is_cancelled(),
        "A can still be cancelled: {:?}",
        view.read_with(cx, |root, _| (
            root.ui.selected_project.clone(),
            root.ui.processing_project.clone(),
            root.ui.notice.clone()
        ))
    );
    outcome
        .send(Err(contracts::BackendError {
            code: contracts::BackendErrorCode::Cancelled,
            message: "cancelled".into(),
        }))
        .unwrap();
    wait_for(cx, &view, |root| !root.ui.processing);
    cx.run_until_parked();
    rendered(cx, "project-status-a-Ready");
    assert!(cx.debug_bounds("project-status-a-Processing").is_none());
    assert!(view.read_with(cx, |root, _| root.ui.processing_project.is_none()));
}

#[gpui::test]
fn processing_failure_clears_the_document_identity(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    let context = view.read_with(cx, |root, _| {
        root.cancel_contexts.as_ref().unwrap().0.clone()
    });
    context.failed("fixture failure");
    cx.run_until_parked();
    rendered(cx, "processing-transcription-failed");
    assert!(view.read_with(cx, |root, _| root.ui.notice.is_empty()));
    outcome
        .send(Err(contracts::BackendError {
            code: contracts::BackendErrorCode::InferenceFailed,
            message: "fixture failure".into(),
        }))
        .unwrap();
    wait_for(cx, &view, |root| !root.ui.processing);
    cx.run_until_parked();
    rendered(cx, "project-status-a-Ready");
    assert!(view.read_with(cx, |root, _| root.ui.processing_project.is_none()));
}

#[gpui::test]
fn processing_feedback_stays_visible_in_default_and_minimum_windows(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    wait_for(cx, &view, |root| !root.audio.pending());
    let outcome = pending_processing(cx, &view);
    let contexts = view.read_with(cx, |root, _| root.cancel_contexts.clone().unwrap());
    contexts.0.started();
    contexts.1.started();
    cx.run_until_parked();
    let progress = rendered(cx, "processing-progress");
    assert!(within(rendered(cx, "processing-transcription"), progress));
    assert!(within(rendered(cx, "processing-diarization"), progress));
    rendered(cx, "processing-transcription-indeterminate");
    rendered(cx, "processing-diarization-indeterminate");
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), "");
    contexts.0.report(runtime::Progress {
        completed: 1,
        total: Some(4),
    });
    contexts.1.report(runtime::Progress {
        completed: 3,
        total: None,
    });
    cx.run_until_parked();
    rendered(cx, "processing-transcription-determinate");
    rendered(cx, "processing-diarization-indeterminate");
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    cx.run_until_parked();
    let progress = rendered(cx, "processing-progress");
    assert!(within(progress, rendered(cx, "meetings-page")));
    assert!(progress.bottom() <= rendered(cx, "content-mode-group").top());
    assert!(rendered(cx, "meeting-content").size.height > px(0.));
    contexts.0.report(runtime::Progress {
        completed: 4,
        total: Some(4),
    });
    cx.run_until_parked();
    rendered(cx, "processing-transcription-running");
    contexts.0.completed();
    cx.run_until_parked();
    rendered(cx, "processing-transcription-completed");
    outcome.send(Ok(documents.read("a").state)).unwrap();
    wait_for(cx, &view, |root| !root.ui.processing);
    cx.run_until_parked();
    assert!(cx.debug_bounds("processing-progress").is_none());
}

#[gpui::test]
fn background_processing_feedback_preserves_the_active_edit_and_notice(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let mut b = processed("b", "B");
    b.hierarchy.sibling_order = 1;
    documents.put(b, "# B\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    let contexts = view.read_with(cx, |root, _| root.cancel_contexts.clone().unwrap());
    select_row(cx, &view, "b");
    open_editor(cx, &view, "b");
    let (title, _, _, body) = editor_fields(cx, &view);
    retype(cx, &title, "B 的草稿");
    retype_area(cx, &body, "继续编辑 B");
    let focus = cx.update(|window, cx| window.focused(cx));
    view.update(cx, |root, cx| {
        root.ui.notice = i18n::message(root.ui.locale, "document_saved", "B");
        cx.notify();
    });
    cx.run_until_parked();
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let toast = view.read_with(cx, |root, _| root.ui.toast().map(|toast| toast.generation));
    contexts.0.started();
    contexts.1.started();
    contexts.0.report(runtime::Progress {
        completed: 2,
        total: Some(5),
    });
    contexts.1.report(runtime::Progress {
        completed: 1,
        total: Some(3),
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("processing-progress").is_none());
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), notice);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.toast().map(|toast| toast.generation)),
        toast
    );
    assert_eq!(field_value(cx, &title), "B 的草稿");
    assert_eq!(area_value(cx, &body), "继续编辑 B");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    outcome.send(Ok(documents.read("a").state)).unwrap();
    wait_for(cx, &view, |root| !root.ui.processing);
}

#[gpui::test]
fn completed_processing_cancellation_is_one_neutral_owner_notice(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let mut b = processed("b", "B");
    b.hierarchy.sibling_order = 1;
    documents.put(b, "# B\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let outcome = pending_processing(cx, &view);
    let contexts = view.read_with(cx, |root, _| root.cancel_contexts.clone().unwrap());
    contexts.0.cancel();
    contexts.1.cancel();
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |root, _| root.ui.notice.is_empty()),
        "stage cancellation does not replace the final outcome notice"
    );
    select_row(cx, &view, "b");
    outcome
        .send(Err(contracts::BackendError {
            code: contracts::BackendErrorCode::Cancelled,
            message: "cancelled".into(),
        }))
        .unwrap();
    wait_for(cx, &view, |root| !root.ui.processing);
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        let toast = root.ui.toast().unwrap();
        assert_eq!(toast.notice, "已取消 A");
        assert_eq!(toast.tone, crate::state::NoticeTone::Info);
        assert!(root.ui.processing_project.is_none());
    });
}

/// Gates real repository returns after package IO and its locks are finished.
struct GatedDocumentRepository {
    inner: FileDocumentRepository,
    gate: crate::tests::Gate,
    snapshot_failure_gate: crate::tests::Gate,
    read_gate: crate::tests::Gate,
}

pub(crate) fn gated_read_repository(
    inner: FileDocumentRepository,
    read_gate: &crate::tests::Gate,
) -> Arc<dyn application::DocumentRepository> {
    Arc::new(GatedDocumentRepository {
        inner,
        gate: Default::default(),
        snapshot_failure_gate: Default::default(),
        read_gate: read_gate.clone(),
    })
}

pub(crate) fn gated_save_repository(
    inner: FileDocumentRepository,
    gate: &crate::tests::Gate,
) -> Arc<dyn application::DocumentRepository> {
    gated_import_repository(inner, gate, &crate::tests::Gate::default())
}

pub(crate) fn gated_import_repository(
    inner: FileDocumentRepository,
    gate: &crate::tests::Gate,
    snapshot_failure_gate: &crate::tests::Gate,
) -> Arc<dyn application::DocumentRepository> {
    Arc::new(GatedDocumentRepository {
        inner,
        gate: gate.clone(),
        snapshot_failure_gate: snapshot_failure_gate.clone(),
        read_gate: Default::default(),
    })
}

impl application::DocumentRepository for GatedDocumentRepository {
    fn snapshot(&self) -> Result<application::DocumentSnapshot, String> {
        let snapshot = self.inner.snapshot();
        if snapshot.is_err() {
            self.snapshot_failure_gate.pass();
        }
        snapshot
    }

    fn get(&self, id: &domain::DocumentId) -> Result<Option<DocumentState>, String> {
        self.inner.get(id)
    }

    fn read(&self, id: &domain::DocumentId) -> Result<Option<DocumentRecord>, String> {
        let record = self.inner.read(id);
        self.read_gate.pass();
        record
    }

    fn watch(&self) -> Result<std::sync::mpsc::Receiver<DocumentChange>, String> {
        self.inner.watch()
    }

    fn subscribe_changes(&self) -> Result<application::DocumentChanges, String> {
        self.inner.subscribe_changes()
    }

    fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
        self.inner.save(document, markdown)?;
        self.gate.pass();
        Ok(())
    }
}

fn open_import_on<'a>(
    cx: &'a mut TestAppContext,
    documents: &DocumentsRoot,
    gate: &crate::tests::Gate,
) -> (View, &'a mut VisualTestContext) {
    open_import_with_snapshot_gate_on(cx, documents, gate, &crate::tests::Gate::default())
}

fn open_import_with_snapshot_gate_on<'a>(
    cx: &'a mut TestAppContext,
    documents: &DocumentsRoot,
    gate: &crate::tests::Gate,
    snapshot_failure_gate: &crate::tests::Gate,
) -> (View, &'a mut VisualTestContext) {
    let mut services =
        desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(GatedDocumentRepository {
            inner: documents.repository(),
            gate: gate.clone(),
            snapshot_failure_gate: snapshot_failure_gate.clone(),
            read_gate: Default::default(),
        })),
    );
    open_desktop_on(cx, services)
}

#[gpui::test]
fn background_import_preserves_the_document_chosen_while_completion_waits(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let mut b = processed("b", "B");
    b.hierarchy.sibling_order = 1;
    b.content.summary = Some("继续阅读与编辑这份记录。\n".repeat(100));
    documents.put(b, "# B\n");
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("arriving.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_import_on(cx, &documents, &gate);
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "a");
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    assert_eq!(
        documents.repository().snapshot().unwrap().documents.len(),
        3,
        "the real import committed its package before returning"
    );
    assert_eq!(view.read_with(cx, |root, _| root.projects.len()), 2);
    select_row(cx, &view, "b");
    open_editor(cx, &view, "b");
    let (title, _, _, body) = editor_fields(cx, &view);
    retype(cx, &title, "B 等待导入时的草稿");
    retype_area(cx, &body, "继续整理 B，保持这里的输入");
    let scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.clone());
    scroll.set_offset(point(px(0.), px(-100.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let position = scroll.offset();
    assert!(position.y < px(0.));
    let focus = cx.update(|window, cx| window.focused(cx));
    let before = documents.package_bytes("b");
    gate.release();
    wait_for(cx, &view, |root| root.projects.len() == 3);
    cx.run_until_parked();
    let imported = view.read_with(cx, |root, _| {
        root.projects
            .iter()
            .find(|doc| doc.title() == "arriving")
            .unwrap()
            .id()
            .clone()
    });
    let record = documents.read(imported.as_str());
    assert_eq!(record.state.audio_path(), audio.display().to_string());
    assert!(
        !record.markdown.is_empty(),
        "the WorkspaceService created a real document package"
    );
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.selected_project,
            Some(ProjectId::new("b")),
            "import must not replace the document chosen while it waited"
        );
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.content_mode, crate::state::ContentMode::Edit);
        assert_eq!(root.editor.shown(), Some(&ProjectId::new("b")));
        assert_eq!(
            root.ui.toast().unwrap().notice,
            i18n::message(root.ui.locale, "imported", "arriving")
        );
    });
    assert_eq!(field_value(cx, &title), "B 等待导入时的草稿");
    assert_eq!(area_value(cx, &body), "继续整理 B，保持这里的输入");
    assert_eq!(scroll.offset(), position);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(documents.package_bytes("b"), before);
    click_selector(cx, "navigate-back");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("a"))
    );
    click_selector(cx, "navigate-forward");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("b"))
    );
    assert_eq!(field_value(cx, &title), "B 等待导入时的草稿");
}

#[gpui::test]
fn first_import_selects_its_real_document_without_leaving_the_current_route(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("first.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_import_on(cx, &documents, &gate);
    view.read_with(cx, |root, _| {
        assert!(root.projects_loaded);
        assert!(root.projects.is_empty());
        assert!(root.ui.selected_project.is_none());
        assert_eq!(root.ui.route, Route::Assistant);
    });
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    gate.release();
    wait_for(cx, &view, |root| {
        root.projects.len() == 1 && !root.audio.pending()
    });
    cx.run_until_parked();
    let id = view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Assistant);
        assert_eq!(
            root.ui.selected_project.as_ref(),
            Some(root.projects[0].id())
        );
        root.projects[0].id().clone()
    });
    let imported = documents.read(id.as_str());
    assert_eq!(imported.state.title(), "first");
    assert_eq!(imported.state.audio_path(), audio.display().to_string());
    assert!(!imported.markdown.is_empty());
}

#[gpui::test]
fn a_stale_import_snapshot_does_not_choose_a_document(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("late.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_import_on(cx, &documents, &gate);
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    documents.put(
        DocumentState::new(ProjectId::new("external"), "External", "external.wav"),
        "# External\n",
    );
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| root.projects.len() == 2);
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "external");
    gate.release();
    let notice = i18n::message(i18n::Locale::ZhCn, "imported", "late");
    wait_for(cx, &view, |root| root.ui.notice == notice);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("external")),
        "an outdated completion cannot replace the user's newer selection"
    );
    assert_eq!(
        documents.repository().snapshot().unwrap().documents.len(),
        2
    );
}

#[gpui::test]
fn an_import_snapshot_failure_preserves_the_selection(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("saved.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_import_on(cx, &documents, &gate);
    wait_for(cx, &view, |root| !root.audio.pending());
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    // The import already wrote its package. Temporarily occupying its root
    // with a file makes the real repository's following snapshot fail.
    let repository = documents.repository();
    let moved = audio_files.path().join("temporarily-moved-workspace");
    std::fs::rename(documents.path(), &moved).unwrap();
    std::fs::write(documents.path(), b"root temporarily occupied").unwrap();
    let error = repository.snapshot().unwrap_err();
    gate.release();
    let expected = i18n::message(i18n::Locale::ZhCn, "documents_load_failed", &error);
    wait_for(cx, &view, |root| root.ui.notice == expected);
    let (selected, notice) = view.read_with(cx, |root, _| {
        assert_eq!(
            root.projects.len(),
            1,
            "a failed read retains the last list"
        );
        assert_eq!(root.projects_error.as_deref(), Some(error.as_str()));
        (root.ui.selected_project.clone(), root.ui.notice.clone())
    });
    std::fs::remove_file(documents.path()).unwrap();
    std::fs::rename(moved, documents.path()).unwrap();
    assert_eq!(
        notice,
        i18n::message(i18n::Locale::ZhCn, "documents_load_failed", &error),
        "import success must not hide the following snapshot failure"
    );
    assert_eq!(
        selected,
        Some(ProjectId::new("a")),
        "a read failure is not evidence that the user has no selection"
    );
    assert_eq!(
        documents.repository().snapshot().unwrap().documents.len(),
        2,
        "the import was really persisted"
    );
}

#[gpui::test]
fn workspace_read_failure_keeps_the_editor_and_retry_keeps_its_draft(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    let mut a = processed("a", "A");
    let audio = documents.path().join("a/recording.wav");
    a.source.audio_path = audio.display().to_string();
    a.processing.latest.as_mut().unwrap().input_audio_checksum = None;
    a.content.summary = Some("继续阅读与编辑这份记录。\n".repeat(100));
    documents.put(a, "# A\n");
    std::fs::write(audio, b"fixture audio").unwrap();
    std::fs::create_dir_all(documents.path().join("broken")).unwrap();
    std::fs::write(documents.path().join("broken/state.json"), b"{").unwrap();
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(gpui::size(px(1100.), px(700.)));
    cx.run_until_parked();
    open_editor(cx, &view, "a");
    let (title, _, _, body) = editor_fields(cx, &view);
    retype(cx, &title, "目录暂不可读时的草稿");
    retype_area(cx, &body, "仍在编辑这里的内容");
    let scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.clone());
    scroll.set_offset(point(px(0.), px(-20.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let position = scroll.offset();
    assert!(position.y < px(0.));
    let focus = cx.update(|window, cx| window.focused(cx));
    let (issues, audio) = view.read_with(cx, |root, _| {
        (root.document_issues.clone(), format!("{:?}", root.audio))
    });
    assert!(!issues.is_empty());
    let before = documents.package_bytes("a");
    let repository = documents.repository();
    let unreadable = UnreadableDocumentsRoot::new(&documents);
    let error = repository.snapshot().unwrap_err();
    let expected = i18n::message(i18n::Locale::ZhCn, "documents_load_failed", &error);
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| root.ui.notice == expected);
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.projects.len(),
            1,
            "a read error is not an empty workspace"
        );
        assert_eq!(root.document_issues, issues);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("a")));
        assert_eq!(root.editor.shown(), Some(&ProjectId::new("a")));
        assert_eq!(
            format!("{:?}", root.audio),
            audio,
            "failed reads do not recheck audio"
        );
    });
    assert_eq!(field_value(cx, &title), "目录暂不可读时的草稿");
    assert_eq!(area_value(cx, &body), "仍在编辑这里的内容");
    assert_eq!(scroll.offset(), position);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert!(cx.debug_bounds("empty-import-audio").is_none());
    let banner = rendered(cx, "workspace-read-error");
    assert!(within(rendered(cx, "retry-workspace-read"), banner));
    drop(unreadable);
    assert_eq!(documents.package_bytes("a"), before);
    // Other valid facts may change while the user's unsaved draft remains open.
    documents.put(
        DocumentState::new(ProjectId::new("b"), "B", "b.wav"),
        "# B\n",
    );
    click_selector(cx, "open-project-detail");
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    cx.run_until_parked();
    for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
        view.update(cx, |root, cx| {
            root.ui.locale = locale;
            cx.notify();
        });
        cx.run_until_parked();
        let page = rendered(cx, "meetings-page");
        let banner = rendered(cx, "workspace-read-error");
        assert!(within(banner, page), "{banner:?} in {page:?}");
        assert!(within(rendered(cx, "workspace-read-error-message"), banner));
        assert!(within(rendered(cx, "retry-workspace-read"), banner));
        let body = rendered(cx, "meeting-content");
        assert!(within(body, page), "{body:?} in {page:?}");
        assert!(
            body.size.height >= px(72.),
            "the editor remains usable beside Detail: {body:?} ({locale:?}); error={banner:?}"
        );
    }
    let later_notice = i18n::message(i18n::Locale::ZhCn, "document_saved", "另一份记录");
    view.update(cx, |root, cx| {
        root.ui.notice = later_notice.clone();
        cx.notify();
    });
    click_selector(cx, "retry-workspace-read");
    let retry_focus = cx.update(|window, cx| window.focused(cx));
    wait_for(cx, &view, |root| {
        root.projects.len() == 2 && !root.audio.pending()
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("workspace-read-error").is_none());
    assert_eq!(field_value(cx, &title), "目录暂不可读时的草稿");
    assert_eq!(area_value(cx, &body), "仍在编辑这里的内容");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("a"))
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), retry_focus);
    assert_eq!(documents.package_bytes("a"), before);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        later_notice
    );
}

#[gpui::test]
fn a_long_workspace_error_can_be_read_with_its_edge_scrollbar(cx: &mut TestAppContext) {
    long_workspace_error_reading(cx, i18n::Locale::ZhCn, false);
}

#[gpui::test]
fn a_long_workspace_error_remains_readable_beside_detail(cx: &mut TestAppContext) {
    long_workspace_error_reading(cx, i18n::Locale::EnUs, true);
}

fn long_workspace_error_reading(cx: &mut TestAppContext, locale: i18n::Locale, detail: bool) {
    let container = DocumentsRoot::new();
    let mut path = container.path().to_path_buf();
    for index in 0..7 {
        path.push(format!(
            "{} {index}",
            "planning and customer interview materials ".repeat(2)
        ));
    }
    std::fs::create_dir_all(&path).unwrap();
    let documents = DocumentsRoot(path);
    documents.put(processed("weekly", "Weekly"), "# Original document\n");
    let before = documents.package_bytes("weekly");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    open_editor(cx, &view, "weekly");
    view.update(cx, |root, cx| {
        root.ui.locale = locale;
        cx.notify();
    });
    if detail {
        click_selector(cx, "open-project-detail");
    }
    let (title, ..) = editor_fields(cx, &view);
    retype(cx, &title, "Keep this draft");
    let repository = documents.repository();
    let unreadable = UnreadableDocumentsRoot::new(&documents);
    let error = repository.snapshot().unwrap_err();
    assert!(error.contains(&documents.path().display().to_string()));
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects_error.as_deref() == Some(&error)
    });
    click_selector(cx, "notice-toast-close");
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    wait_for(cx, &view, |root| root.ui.toast().is_none());
    let message = rendered(cx, "workspace-read-error-message");
    let banner = rendered(cx, "workspace-read-error");
    let page = rendered(cx, "meetings-page");
    assert!(within(banner, page));
    assert!(within(rendered(cx, "retry-workspace-read"), banner));
    assert!(rendered(cx, "meeting-content").size.height >= px(80.));
    let track = cx
        .debug_bounds("workspace-read-error-scrollbar-viewport")
        .expect("the long error must expose a draggable scrollbar");
    let text = rendered(cx, "workspace-read-error-text");
    assert!(
        text.size.height > message.size.height,
        "the real path needs scrolling"
    );
    assert_eq!(track.right(), message.right());
    assert!(within(track, message));
    focus_field(cx, &title);
    let focus = cx.update(|window, cx| window.focused(cx));
    let editor_scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.offset());
    let start = point(track.right() - px(7.), track.top() + px(8.));
    let end = point(start.x, track.bottom() - px(4.));
    cx.simulate_mouse_move(start, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let text = rendered(cx, "workspace-read-error-text");
    assert!(
        text.top() < message.top() && text.bottom() <= message.bottom() + px(1.),
        "dragging the edge reaches the end of the full error: {text:?} in {message:?}"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        editor_scroll
    );
    cx.simulate_input("!");
    assert_eq!(field_value(cx, &title), "Keep this draft!");
    assert!(view.read_with(cx, |root, _| root.ui.notice.is_empty()));
    click_selector(cx, "retry-workspace-read");
    let repeated_notice = i18n::message(locale, "documents_load_failed", &error);
    wait_for(cx, &view, |root| root.ui.notice == repeated_notice);
    assert_eq!(
        rendered(cx, "workspace-read-error-text").top(),
        text.top(),
        "a repeated failure keeps the error's reading position"
    );
    // A repeated failure reports a new notification. Dismiss it before
    // using the persistent panel's retry again, as with the first failure.
    click_selector(cx, "notice-toast-close");
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    wait_for(cx, &view, |root| root.ui.toast().is_none());
    drop(unreadable);
    click_selector(cx, "retry-workspace-read");
    wait_for(cx, &view, |root| root.projects_error.is_none());
    assert!(cx.debug_bounds("workspace-read-error").is_none());
    assert_eq!(field_value(cx, &title), "Keep this draft!");
    assert_eq!(documents.package_bytes("weekly"), before);
    let unreadable = UnreadableDocumentsRoot::new(&documents);
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| root.projects_error.is_some());
    assert!(
        rendered(cx, "workspace-read-error-text").top()
            >= rendered(cx, "workspace-read-error-message").top(),
        "a new failure starts at the beginning of its message"
    );
    assert_eq!(field_value(cx, &title), "Keep this draft!");
    drop(unreadable);
    assert_eq!(documents.package_bytes("weekly"), before);
}

#[gpui::test]
fn an_initial_workspace_read_failure_does_not_claim_the_workspace_is_empty(
    cx: &mut TestAppContext,
) {
    let documents = DocumentsRoot::new();
    let repository = documents.repository();
    let mut services = desktop_services(
        Arc::new(FakeAssistantUiService::default()),
        stored_key_settings(),
    );
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(repository.clone())),
    );
    let unreadable = UnreadableDocumentsRoot::new(&documents);
    let error = repository.snapshot().unwrap_err();
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
    assert!(
        cx.debug_bounds("assistant-first-import").is_none(),
        "an unreadable root is not known to need its first import"
    );
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        i18n::message(i18n::Locale::ZhCn, "documents_load_failed", &error)
    );
    assert!(
        cx.debug_bounds("empty-import-audio").is_none(),
        "an unreadable root is not known to be empty"
    );
    rendered(cx, "workspace-read-error");
    rendered(cx, "project-tree-unavailable");
    drop(unreadable);
    let notified = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = notified.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    click_selector(cx, "retry-workspace-read");
    wait_for(cx, &view, |_| notified.get());
    cx.run_until_parked();
    assert!(cx.debug_bounds("workspace-read-error").is_none());
    assert!(cx.debug_bounds("project-tree-unavailable").is_none());
    assert!(
        view.read_with(cx, |root, _| root.ui.notice.is_empty()),
        "a successful retry removes its previous failure notice"
    );
    rendered(cx, "empty-import-audio");
    view.update(cx, |root, cx| root.navigate(Route::Assistant, cx));
    cx.run_until_parked();
    rendered(cx, "assistant-first-import");
}

fn workspace_retry_after_locale_change(
    cx: &mut TestAppContext,
    first: i18n::Locale,
    next: i18n::Locale,
    newer_error: bool,
) {
    let documents = DocumentsRoot::new();
    let repository = documents.repository();
    let settings = test_settings();
    let mut initial = settings.snapshot().unwrap();
    initial.ui_locale = first.code().into();
    settings.update(initial).unwrap();
    let mut services = desktop_services(
        Arc::new(FakeAssistantUiService::default()),
        settings.clone(),
    );
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(repository)),
    );
    let unreadable = UnreadableDocumentsRoot::new(&documents);
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
    let old_toast = view.read_with(cx, |root, _| root.ui.toast().unwrap().notice.clone());
    drop(unreadable);
    // Use the same controller path as the Settings language control and
    // wait for its actual settings write, not just the immediate relabel.
    view.update(cx, |root, cx| {
        root.navigate(Route::Settings, cx);
        root.set_locale(next, cx);
    });
    wait_for(cx, &view, |root| {
        root.ui.notice == i18n::text(next, "language_changed")
    });
    assert_eq!(settings.snapshot().unwrap().ui_locale, next.code());
    assert_eq!(view.read_with(cx, |root, _| root.ui.locale), next);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.toast().unwrap().notice.clone()),
        old_toast
    );
    let expected_notice = if newer_error {
        i18n::message(next, "document_read_failed", "another/state.json")
    } else {
        i18n::text(next, "language_changed").into()
    };
    view.update(cx, |root, cx| {
        root.navigate(Route::Meetings, cx);
        if newer_error {
            root.ui.notice = expected_notice.clone();
        }
        cx.notify();
    });
    cx.run_until_parked();
    click_selector(cx, "retry-workspace-read");
    wait_for(cx, &view, |root| root.projects_error.is_none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice, expected_notice,
            "a newer notice is retained"
        );
        if newer_error {
            assert_eq!(root.ui.toast().unwrap().notice, expected_notice);
        } else {
            assert!(
                root.ui.toast().is_none(),
                "the recovered workspace must not retain its old-language error toast"
            );
        }
    });
    rendered(cx, "empty-import-audio");
}

#[gpui::test]
fn workspace_retry_clears_the_chinese_error_after_settings_switches_to_english(
    cx: &mut TestAppContext,
) {
    workspace_retry_after_locale_change(cx, i18n::Locale::ZhCn, i18n::Locale::EnUs, false);
}

#[gpui::test]
fn workspace_retry_clears_the_english_error_after_settings_switches_to_chinese(
    cx: &mut TestAppContext,
) {
    workspace_retry_after_locale_change(cx, i18n::Locale::EnUs, i18n::Locale::ZhCn, false);
}

#[gpui::test]
fn workspace_retry_preserves_a_newer_error_after_settings_changes_language(
    cx: &mut TestAppContext,
) {
    workspace_retry_after_locale_change(cx, i18n::Locale::ZhCn, i18n::Locale::EnUs, true);
}

#[gpui::test]
fn a_successfully_read_empty_workspace_offers_import(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    let (_view, cx, _changes) = open_on(cx, &documents);
    rendered(cx, "empty-import-audio");
    assert!(cx.debug_bounds("workspace-read-error").is_none());
    assert!(cx.debug_bounds("project-tree-unavailable").is_none());
}

#[gpui::test]
fn a_stale_import_snapshot_failure_preserves_the_newer_feedback(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(processed("a", "A"), "# A\n");
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("late-failure.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let write_gate = crate::tests::Gate::default();
    let snapshot_gate = crate::tests::Gate::default();
    let (view, cx) = open_import_with_snapshot_gate_on(cx, &documents, &write_gate, &snapshot_gate);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    write_gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| write_gate.counts().1 == 1);
    let moved = audio_files.path().join("temporarily-moved-workspace");
    std::fs::rename(documents.path(), &moved).unwrap();
    std::fs::write(documents.path(), b"root temporarily occupied").unwrap();
    snapshot_gate.arm();
    write_gate.release();
    wait_for(cx, &view, |_| snapshot_gate.counts().1 == 1);
    // The old snapshot has genuinely failed, but cannot return yet. A
    // newer successful reload must remain authoritative when it does.
    std::fs::remove_file(documents.path()).unwrap();
    std::fs::rename(moved, documents.path()).unwrap();
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects.len() == 2 && !root.audio.pending()
    });
    cx.run_until_parked();
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let notified = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = notified.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    snapshot_gate.release();
    // Import completion always notifies even when its stale snapshot is
    // ignored, so this waits for that callback rather than sleeping.
    wait_for(cx, &view, |_| notified.get());
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.notice, notice);
        assert!(
            root.projects_error.is_none(),
            "the old failure is not the latest read outcome"
        );
        assert_eq!(root.projects.len(), 2);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("a")));
    });
    assert_eq!(
        documents.repository().snapshot().unwrap().documents.len(),
        2
    );
}

#[gpui::test]
fn first_import_is_selected_by_a_reload_before_its_completion(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("watcher-first.wav");
    std::fs::write(&audio, b"fixture audio").unwrap();
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_import_on(cx, &documents, &gate);
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    // As a watcher does, reload the real committed package before the
    // still-blocked import service is allowed to publish its completion.
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects.len() == 1 && !root.audio.pending()
    });
    let (chosen_before_completion, imported) = view.read_with(cx, |root, _| {
        (
            root.ui.selected_project.clone(),
            root.projects[0].id().clone(),
        )
    });
    gate.release();
    let notice = i18n::message(i18n::Locale::ZhCn, "imported", "watcher-first");
    wait_for(cx, &view, |root| root.ui.notice == notice);
    assert_eq!(chosen_before_completion, Some(imported.clone()), "the latest successful snapshot initializes selection, independently of the late import callback");
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.selected_project, Some(imported.clone()));
        assert_eq!(root.ui.route, Route::Assistant);
        assert!(
            !root.ui.can_go_back(),
            "initial selection adds no history entry"
        );
        assert!(root.ui.expanded_projects.contains(&imported));
    });
    assert_eq!(
        documents.read(imported.as_str()).state.audio_path(),
        audio.display().to_string()
    );
}

#[gpui::test]
fn the_transcript_reads_as_turns_in_a_measured_column(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let mut weekly = processed("weekly", "产品周会");
    let said = [
        ("speaker-0", 0, 4_000, "先过一下页面"),
        ("speaker-0", 4_000, 9_000, "然后看截图"),
        ("speaker-1", 9_000, 12_000, "那就开始迁移"),
    ];
    weekly.results.transcript = said
        .iter()
        .enumerate()
        .map(|(index, (_, start_ms, end_ms, text))| TranscriptSegment {
            id: format!("t-{index}"),
            start_ms: *start_ms,
            end_ms: *end_ms,
            text: (*text).into(),
        })
        .collect();
    weekly.results.diarization = said
        .iter()
        .enumerate()
        .map(
            |(index, (speaker, start_ms, end_ms, _))| DiarizationSegment {
                id: format!("d-{index}"),
                speaker_id: (*speaker).into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: *start_ms,
                end_ms: *end_ms,
                confidence: None,
            },
        )
        .collect();
    weekly.results.speaker_segments = domain::align_transcript_and_speakers(
        &weekly.results.transcript,
        &weekly.results.diarization,
    );
    root.put(weekly, "# 产品周会\n");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    click_selector(cx, "content-mode-original");

    // Two turns: speaker-0's two segments, then speaker-1's.
    let content = rendered(cx, "meeting-content");
    let turns = [0, 1].map(|index| rendered(cx, leak(format!("transcript-turn-{index}"))));
    assert!(cx.debug_bounds("transcript-turn-2").is_none());
    assert!(turns[0].bottom() <= turns[1].top());
    for turn in turns {
        assert!(within(turn, content));
        assert!(
            turn.size.width <= meetings::READING_WIDTH,
            "the transcript keeps to the reading width: {turn:?}"
        );
    }
    assert!(
        content.size.width > meetings::READING_WIDTH,
        "the column is narrower than the plane"
    );
}

fn assert_centered_empty_state(cx: &mut VisualTestContext, action: Option<&'static str>) {
    let region = rendered(cx, "meeting-content");
    let empty = rendered(cx, "document-empty");
    assert!(within(empty, region));
    assert!(
        (empty.center().x - region.center().x).abs() < px(1.)
            && (empty.center().y - region.center().y).abs() < px(1.),
        "the empty state is centered: {empty:?} in {region:?}"
    );
    assert!(empty.size.height < region.size.height / 2., "and compact");
    for selector in [
        "empty-process",
        "empty-relink-audio",
        "go-to-models",
        "empty-import-audio",
    ] {
        let shown = cx.debug_bounds(selector);
        if Some(selector) == action {
            assert!(within(shown.expect("the action is shown"), empty));
        } else {
            assert!(shown.is_none(), "`{selector}` is not offered");
        }
    }
}

#[gpui::test]
fn an_unprocessed_document_offers_its_next_step_in_place(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let audio = root.path().join("present.wav");
    std::fs::write(&audio, b"audio").unwrap();
    root.put(
        DocumentState::new(
            ProjectId::new("present"),
            "Present",
            audio.display().to_string(),
        ),
        "# Present\n",
    );
    let mut gone = DocumentState::new(ProjectId::new("gone"), "Gone", "gone.wav");
    gone.hierarchy.sibling_order = 1;
    root.put(gone, "# Gone\n");
    let (view, cx, _changes) = open_on(cx, &root);

    select_row(cx, &view, "present");
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    cx.run_until_parked();
    // The test settings choose backends without models.
    assert_centered_empty_state(cx, Some("go-to-models"));
    let before = view.read_with(cx, |root, _| root.ui.notice.clone());
    click_selector(cx, "process-project");
    assert!(
        !view.read_with(cx, |root, _| root.ui.processing),
        "processing waits for the models"
    );
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), before);

    // With the default models ready, the document can be processed.
    view.update(cx, |root, cx| {
        root.settings_draft.asr_backend_id = None;
        root.settings_draft.diarization_backend_id = None;
        root.installation.default_asr_ready = true;
        root.installation.default_diarization_ready = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert_centered_empty_state(cx, Some("empty-process"));
    click_selector(cx, "empty-process");
    wait_for(cx, &view, |root| {
        !root.ui.processing && root.ui.notice != before
    });

    // Missing audio is relinked first.
    select_row(cx, &view, "gone");
    wait_for(cx, &view, |root| {
        root.audio.missing(&ProjectId::new("gone"))
    });
    cx.run_until_parked();
    assert_centered_empty_state(cx, Some("empty-relink-audio"));

    // While it is processed, the state says so and offers nothing.
    view.update(cx, |root, cx| {
        root.ui.begin_processing(ProjectId::new("gone"));
        cx.notify();
    });
    cx.run_until_parked();
    assert_centered_empty_state(cx, None);
}

#[gpui::test]
fn an_empty_workspace_offers_to_import_audio(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let (_view, cx, _changes) = open_on(cx, &root);
    assert_centered_empty_state(cx, Some("empty-import-audio"));
}

#[gpui::test]
fn switching_from_a_transcript_to_an_imported_document_selects_summary(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "周会"), "# 周会\n");
    let mut imported = DocumentState::new(ProjectId::new("new"), "新录音", "new.wav");
    imported.hierarchy.sibling_order = 1;
    documents.put(imported, "# 新录音\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "weekly");
    click_selector(cx, "content-mode-original");
    rendered(cx, "transcript-turn-0");
    select_row(cx, &view, "new");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Summary,
        "the imported document has no Original tab, so Summary must be selected"
    );
    assert!(cx.debug_bounds("content-mode-original").is_none());
    rendered(cx, "document-empty");

    select_row(cx, &view, "weekly");
    click_selector(cx, "content-mode-original");
    click_selector(cx, "navigate-back");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.content_mode),
        crate::state::ContentMode::Summary,
        "history navigation also keeps a visible tab selected"
    );
}

#[gpui::test]
fn switching_documents_and_reading_modes_starts_at_the_top(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    for (index, id) in ["a", "b"].into_iter().enumerate() {
        let mut document = processed(id, id);
        document.hierarchy.sibling_order = index as u32;
        document.content.summary = Some("一段很长的会议总结。\n".repeat(120));
        documents.put(document, "# Meeting\n");
    }
    let (view, cx, _changes) = open_on(cx, &documents);
    select_row(cx, &view, "a");
    let scroll = view.read_with(cx, |root, _| root.scroll.meeting_content.clone());
    assert!(scroll.max_offset().y > px(200.));
    scroll.set_offset(point(px(0.), px(-200.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(scroll.offset().y, px(-200.));
    select_row(cx, &view, "a");
    assert_eq!(
        scroll.offset().y,
        px(-200.),
        "selecting the current document keeps its reading position"
    );
    select_row(cx, &view, "b");
    assert_eq!(
        scroll.offset().y,
        px(0.),
        "another document opens at its beginning"
    );

    scroll.set_offset(point(px(0.), px(-200.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    click_selector(cx, "navigate-back");
    assert_eq!(
        scroll.offset().y,
        px(0.),
        "history opens the document at its beginning"
    );
    click_selector(cx, "navigate-forward");

    scroll.set_offset(point(px(0.), px(-200.)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("b"))
    });
    assert_eq!(scroll.offset().y, px(0.), "the editor opens at its title");
}

#[gpui::test]
fn document_actions_fit_beside_detail_in_both_locales(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(processed("weekly", "产品周会"), "# 产品周会\n");
    let (view, cx, _changes) = open_on(cx, &documents);
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    select_row(cx, &view, "weekly");
    wait_for(cx, &view, |root| {
        root.audio.missing(&ProjectId::new("weekly"))
    });
    click_selector(cx, "open-project-detail");
    for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
        view.update(cx, |root, cx| root.set_locale(locale, cx));
        cx.run_until_parked();
        let page = rendered(cx, "meetings-page");
        for selector in [
            "document-status",
            "audio-problem",
            "process-project",
            "relink-audio",
        ] {
            let control = rendered(cx, selector);
            assert!(
                within(control, page),
                "{locale:?}: {selector} overflows the reading plane: {control:?} in {page:?}"
            );
        }
    }
}
