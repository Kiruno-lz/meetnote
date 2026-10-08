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
    let (changes, receiver) = futures::channel::mpsc::unbounded();
    view.update(cx, |root, cx| {
        root.follow_document_changes(receiver, cx);
        root.navigate(Route::Meetings, cx);
    });
    cx.run_until_parked();
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
        root.ui.selected_project.as_ref() == Some(&id)
    });
}

/// Opens the editor of the selected document once its draft has loaded.
fn open_editor(cx: &mut VisualTestContext, view: &View, id: &str) {
    click_selector(cx, "content-mode-edit");
    let id = ProjectId::new(id);
    wait_for(cx, view, |root| root.editor.shown() == Some(&id));
    cx.run_until_parked();
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

/// A processing job of a document that the test drives: the contexts of
/// its transcription and diarization jobs emit the events the page
/// follows, and `outcome` ends it.
struct DrivenJob {
    transcription: runtime::JobContext,
    diarization: runtime::JobContext,
    outcome: futures::channel::oneshot::Sender<Result<DocumentState, contracts::BackendError>>,
}

/// Starts following a driven processing job of `id`, as `process_selected`
/// does once the workspace has started it.
fn drive_processing(cx: &mut VisualTestContext, view: &View, id: &str) -> DrivenJob {
    let jobs = runtime::JobRuntime::with_max_concurrency(2);
    let (transcription, asr_events) = jobs.create_job();
    let (diarization, diarization_events) = jobs.create_job();
    let (outcome, result) = futures::channel::oneshot::channel();
    let job = application::ProjectProcessingJob {
        future: Box::pin(async move {
            result.await.unwrap_or_else(|_| {
                Err(contracts::BackendError {
                    code: contracts::BackendErrorCode::Cancelled,
                    message: "dropped".into(),
                })
            })
        }),
        asr_context: transcription.clone(),
        diarization_context: diarization.clone(),
        asr_events,
        diarization_events,
    };
    let document = ProjectId::new(id);
    view.update(cx, |root, cx| {
        root.ui.begin_processing(document);
        root.ui.notice = i18n::message(i18n::Locale::ZhCn, "processing", "New");
        root.cancel_contexts = Some((transcription.clone(), diarization.clone()));
        cx.notify();
        cx.spawn(async move |this, cx| RootView::follow_processing(this, job, cx).await)
            .detach();
    });
    cx.run_until_parked();
    DrivenJob {
        transcription,
        diarization,
        outcome,
    }
}

fn phase(cx: &mut VisualTestContext, view: &View) -> Option<String> {
    view.read_with(cx, |root, _| {
        root.ui
            .progress
            .as_ref()
            .map(|progress| progress.phase(root.ui.locale))
    })
}

#[gpui::test]
fn processing_progress_shows_in_the_header_and_the_tree_not_the_notice(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(processed("weekly", "产品周会"), "# 产品周会\n");
    let mut new = DocumentState::new(ProjectId::new("new"), "New", "new.wav");
    new.hierarchy.sibling_order = 1;
    root.put(new.clone(), "# New\n");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "new");
    assert!(cx.debug_bounds("processing-progress").is_none());

    let job = drive_processing(cx, &view, "new");
    let starting = view.read_with(cx, |root, _| root.ui.notice.clone());
    // Before either job reports, the bar sweeps under "preparing".
    let page = rendered(cx, "meetings-page");
    let header_progress = rendered(cx, "processing-progress");
    assert!(within(header_progress, page));
    assert!(
        header_progress.bottom() <= rendered(cx, "meeting-content").top(),
        "the progress heads the document"
    );
    assert!(within(
        rendered(cx, "processing-indicator-new"),
        rendered(cx, "project-row-new")
    ));
    assert!(cx.debug_bounds("processing-indicator-weekly").is_none());
    assert_eq!(phase(cx, &view).as_deref(), Some("正在准备……"));

    job.transcription.started();
    job.diarization.started();
    job.transcription.report(runtime::Progress {
        completed: 3,
        total: Some(10),
    });
    wait_for(cx, &view, |root| {
        root.ui.progress.as_ref().is_some_and(|progress| {
            progress.transcription
                == crate::state::StageProgress::Running {
                    completed: 3,
                    total: Some(10),
                }
        })
    });
    assert_eq!(
        phase(cx, &view).as_deref(),
        Some("正在转录 3/10 · 等待说话人分离")
    );
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.progress.as_ref().unwrap().percent()),
        Some(15.)
    );
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        starting,
        "a progress tick leaves the notice alone"
    );

    // Another document shows no progress, and cannot start a second job.
    select_row(cx, &view, "weekly");
    assert!(cx.debug_bounds("processing-progress").is_none());
    rendered(cx, "processing-indicator-new");
    select_row(cx, &view, "new");

    // The outcome ends the progress and writes the notice.
    job.transcription.completed();
    job.diarization.completed();
    let mut finished = new.clone();
    finished.content.title = "New".into();
    job.outcome.send(Ok(finished)).ok();
    wait_for(cx, &view, |root| !root.ui.processing);
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.progress, None);
        assert_eq!(
            root.ui.notice,
            i18n::message(i18n::Locale::ZhCn, "finished", "New")
        );
    });
    assert!(cx.debug_bounds("processing-progress").is_none());
    assert!(cx.debug_bounds("processing-indicator-new").is_none());
}

#[gpui::test]
fn a_cancelled_job_notes_the_cancellation_not_a_failure(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(
        DocumentState::new(ProjectId::new("new"), "New", "new.wav"),
        "# New\n",
    );
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "new");
    let job = drive_processing(cx, &view, "new");
    job.transcription.cancel();
    job.outcome
        .send(Err(contracts::BackendError {
            code: contracts::BackendErrorCode::Cancelled,
            message: "cancelled".into(),
        }))
        .ok();
    wait_for(cx, &view, |root| !root.ui.processing);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        i18n::message(i18n::Locale::ZhCn, "cancelled", "New")
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

/// The empty state is one compact group centered in the content region.
#[track_caller]
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
    for selector in ["empty-process", "empty-relink-audio", "empty-import-audio"] {
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
    wait_for(cx, &view, |root| !root.audio.pending());
    cx.run_until_parked();
    assert_centered_empty_state(cx, Some("empty-process"));
    let before = view.read_with(cx, |root, _| root.ui.notice.clone());
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
