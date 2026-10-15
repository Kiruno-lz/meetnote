//! L2: the Documents slice of the Meetings route on real document packages
//! in a temporary documents root (`FileDocumentRepository`): scanner issues,
//! the Detail facts, the revision-checked editor and tree moves, and the
//! conflict notice when another writer changed a document first.
//!
//! The desktop opens without a file watcher; a test that changes the
//! directory reports it through a change stream it feeds, as the watcher
//! would.

use crate::tests::{
    click_selector, desktop_services, field_value, focus_field, open_desktop_on, temp_directory,
    test_settings, wait_for, FakeAssistantUiService, Field,
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

    // The broken package is repaired and the stray file removed.
    std::fs::remove_file(&stray).unwrap();
    root.put(
        DocumentState::new(ProjectId::new("broken"), "Repaired", "broken.wav"),
        "# Repaired\n",
    );
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
    assert_eq!(value("detail_document_status").as_deref(), Some("review"));
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

    click_selector(cx, "save-document");
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
    click_selector(cx, "retry-document-read");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("a"))
    });
    cx.run_until_parked();
    let (title, _, _, body) = editor_fields(cx, &view);
    assert_eq!(field_value(cx, &title), "A");
    assert_eq!(area_value(cx, &body), "# A\n");
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

/// Gates the real import write's return, after its package and locks are committed.
struct ImportWriteRepository {
    inner: FileDocumentRepository,
    gate: crate::tests::Gate,
    snapshot_failure_gate: crate::tests::Gate,
}

impl application::DocumentRepository for ImportWriteRepository {
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
        self.inner.read(id)
    }

    fn watch(&self) -> Result<std::sync::mpsc::Receiver<DocumentChange>, String> {
        self.inner.watch()
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
        DocumentService::new(Arc::new(ImportWriteRepository {
            inner: documents.repository(),
            gate: gate.clone(),
            snapshot_failure_gate: snapshot_failure_gate.clone(),
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
    wait_for(cx, &view, |root| root.projects.is_empty());
    let (selected, notice) = view.read_with(cx, |root, _| {
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
