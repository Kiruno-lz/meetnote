//! L2: speaker correction in the Original view on real document packages
//! (`FileDocumentRepository` in a temporary documents root), and the Speaker
//! Library section of Settings on a real library file
//! (`JsonSpeakerProfileStore` in a temporary directory).
//!
//! Every correction is one revision-checked command; the conflict notice and
//! the document helpers are the Documents slice's (`document_tests`).

use crate::document_tests::{
    gated_save_repository, leak, open_on, processed, rendered, select_row, within, DocumentsRoot,
    View,
};
use crate::tests::{
    click_selector, desktop_services, field_value, open_desktop_on, temp_directory, test_settings,
    wait_for, FakeAssistantUiService,
};
use crate::{i18n, route::Route, DesktopServices};
use application::{
    DocumentChange, DocumentService, JsonSpeakerProfileStore, ProcessingService,
    SpeakerProfileStore as _, WorkspaceService,
};
use asr_whisper_rs::WhisperRsBackend;
use contracts::{ModelRef, SpeakerLibraryDocument, SpeakerProfile};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::{DiarizationSegment, DocumentState, ProjectId, TranscriptSegment};
use gpui::{TestAppContext, VisualTestContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A processed document in which `speaker-0` says the first and the last
/// segment and `speaker-1` the middle one.
pub(crate) fn two_speakers(id: &str) -> DocumentState {
    let mut state = processed(id, "产品周会");
    let turns = [
        ("speaker-0", 0, 4_000, "先过一下页面"),
        ("speaker-1", 4_000, 9_000, "截图基线已经生成"),
        ("speaker-0", 9_000, 12_000, "那就开始迁移"),
    ];
    state.results.transcript = turns
        .iter()
        .enumerate()
        .map(|(index, (_, start_ms, end_ms, text))| TranscriptSegment {
            id: format!("t-{index}"),
            start_ms: *start_ms,
            end_ms: *end_ms,
            text: (*text).into(),
        })
        .collect();
    state.results.diarization = turns
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
    state.results.speaker_segments = domain::align_transcript_and_speakers(
        &state.results.transcript,
        &state.results.diarization,
    );
    state
}

/// The speaker-labelled transcript without its speakers: what a correction
/// must keep.
fn spoken(state: &DocumentState) -> Vec<(String, u64, u64, String)> {
    state
        .results
        .speaker_segments
        .iter()
        .map(|segment| {
            (
                segment.transcript_segment_id.clone(),
                segment.start_ms,
                segment.end_ms,
                segment.text.clone(),
            )
        })
        .collect()
}

/// Shows the Original view of `id`.
fn open_original(cx: &mut VisualTestContext, view: &View, id: &str) {
    select_row(cx, view, id);
    click_selector(cx, "content-mode-original");
    cx.run_until_parked();
}

/// Replaces the text of the field rendered at `selector` as typing does.
fn retype_at(cx: &mut VisualTestContext, selector: &'static str, text: &str) {
    click_selector(cx, selector);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(text);
}

/// Opens the select rendered at `selector` and picks its option `index`.
fn choose(cx: &mut VisualTestContext, selector: &'static str, index: usize) {
    click_selector(cx, selector);
    click_selector(cx, leak(format!("select-option-{index}")));
}

/// Drives the executor until `selector` is rendered, or is not when
/// `rendered` is false.
#[track_caller]
fn wait_rendered(cx: &mut VisualTestContext, selector: &'static str, rendered: bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if cx.debug_bounds(selector).is_some() == rendered {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "`{selector}` rendered: expected {rendered}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn speaker_of(state: &DocumentState, transcript_segment_id: &str) -> (String, Option<String>) {
    let segment = state
        .results
        .speaker_segments
        .iter()
        .find(|segment| segment.transcript_segment_id == transcript_segment_id)
        .unwrap();
    (segment.speaker_id.clone(), segment.speaker_label.clone())
}

#[gpui::test]
fn renaming_a_speaker_writes_its_label_with_the_loaded_revision(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(two_speakers("weekly"), "# 产品周会\n");
    let before = root.read("weekly");
    let (view, cx, _changes) = open_on(cx, &root);
    open_original(cx, &view, "weekly");

    // The speakers head the transcript in the content plane.
    let content = rendered(cx, "meeting-content");
    let list = rendered(cx, "document-speakers");
    assert!(within(list, content), "{list:?} in {content:?}");
    for id in ["speaker-0", "speaker-1"] {
        let row = rendered(cx, leak(format!("speaker-row-{id}")));
        assert!(within(row, list), "`{id}` lies in the list");
        for control in ["speaker-name", "rename-speaker", "merge-speaker"] {
            assert!(within(rendered(cx, leak(format!("{control}-{id}"))), row));
        }
    }

    retype_at(cx, "speaker-name-speaker-1", "  王经理 ");
    assert_eq!(
        root.revision("weekly"),
        before.state.revision(),
        "typing writes nothing"
    );
    click_selector(cx, "rename-speaker-speaker-1");
    wait_for(cx, &view, |_| {
        root.revision("weekly") == before.state.revision() + 1
    });
    let after = root.read("weekly").state;
    assert_eq!(
        ["t-0", "t-1", "t-2"].map(|id| speaker_of(&after, id)),
        [
            ("speaker-0".into(), None),
            ("speaker-1".into(), Some("王经理".into())),
            ("speaker-0".into(), None),
        ]
    );
    assert!(after
        .results
        .diarization
        .iter()
        .all(|turn| (turn.speaker_id == "speaker-1")
            == (turn.speaker_label.as_deref() == Some("王经理"))));
    assert_eq!(after.results.transcript, before.state.results.transcript);
    assert_eq!(spoken(&after), spoken(&before.state));
    assert_eq!(after.processing, before.state.processing);

    // The list follows the reloaded document.
    wait_for(cx, &view, |root| {
        root.projects.iter().any(|project| {
            project.id().as_str() == "weekly" && project.revision() == after.revision()
        })
    });
    cx.run_until_parked();
    rendered(cx, "speaker-row-speaker-1");
    let renamed = i18n::message(i18n::Locale::ZhCn, "speaker_renamed", "王经理");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.notice.clone()),
        renamed
    );
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn an_unrelated_refresh_keeps_a_speaker_name_draft_focus_and_caret(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let (view, cx, changes) = open_on(cx, &documents);
    open_original(cx, &view, "weekly");
    retype_at(cx, "speaker-name-speaker-0", "主持人草稿");
    cx.simulate_keystrokes("left left");
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    let focus = cx.update(|window, cx| window.focused(cx));
    let mut external = documents.read("weekly").state;
    external.content.title = "Changed outside the speaker controls".into();
    documents.external_save(external);
    let before = documents.read("weekly");
    let bytes = documents.package_bytes("weekly");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.title() == "Changed outside the speaker controls")
    });
    let current = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(
        field_value(cx, &current),
        "主持人草稿",
        "a title refresh must not erase a name draft"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    cx.simulate_input("的");
    assert_eq!(
        field_value(cx, &name),
        "主持人的草稿",
        "the insertion point survives the refresh"
    );
    assert_eq!(documents.package_bytes("weekly"), bytes);
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none()
            && documents.revision("weekly") == before.state.revision() + 1
    });
    let after = documents.read("weekly");
    assert_eq!(after.state.title(), "Changed outside the speaker controls");
    assert_eq!(
        speaker_of(&after.state, "t-0").1.as_deref(),
        Some("主持人的草稿")
    );
    assert_eq!(
        after.state.results.transcript,
        before.state.results.transcript
    );
    assert_eq!(after.state.processing, before.state.processing);
    assert_eq!(after.markdown, before.markdown);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn an_unrelated_refresh_keeps_a_speaker_merge_menu_open_on_the_latest_revision(
    cx: &mut TestAppContext,
) {
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let (view, cx, changes) = open_on(cx, &documents);
    open_original(cx, &view, "weekly");
    click_selector(cx, "merge-speaker-speaker-0");
    rendered(cx, "select-menu");
    let focus = cx.update(|window, cx| window.focused(cx));
    let mut external = documents.read("weekly").state;
    external.document_status.value = "final".into();
    documents.external_save(external);
    let before = documents.read("weekly");
    let bytes = documents.package_bytes("weekly");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.document_status.value == "final")
    });
    assert!(
        cx.debug_bounds("select-menu").is_some(),
        "an unrelated status change must not close the merge menu"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    click_selector(cx, "select-option-0");
    rendered(cx, "confirm-dialog");
    assert_eq!(
        documents.package_bytes("weekly"),
        bytes,
        "selection still waits for confirmation"
    );
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none()
            && documents.revision("weekly") == before.state.revision() + 1
    });
    let after = documents.read("weekly");
    assert_eq!(after.state.document_status.value, "final");
    assert!(after
        .state
        .results
        .speaker_segments
        .iter()
        .all(|s| s.speaker_id == "speaker-1"));
    assert_eq!(
        after.state.results.transcript,
        before.state.results.transcript
    );
    assert_eq!(after.state.processing, before.state.processing);
    assert_eq!(after.markdown, before.markdown);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn an_external_speaker_correction_refreshes_the_name_controls(cx: &mut TestAppContext) {
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let (view, cx, changes) = open_on(cx, &documents);
    open_original(cx, &view, "weekly");
    let commands = application::DocumentCommandService::new(DocumentService::new(Arc::new(
        documents.repository(),
    )));
    let external = commands
        .set_speaker_label(
            &ProjectId::new("weekly"),
            documents.revision("weekly"),
            "speaker-0",
            Some("External name".into()),
        )
        .unwrap();
    let before = documents.package_bytes("weekly");
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.revision() == external.state.revision())
    });
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(
        field_value(cx, &name),
        "External name",
        "changed speaker data must not be treated as unrelated metadata"
    );
    assert_eq!(documents.package_bytes("weekly"), before);
}

#[gpui::test]
fn a_speaker_save_acknowledgement_keeps_later_input_in_another_row(cx: &mut TestAppContext) {
    speaker_save_later_input(cx, false);
}

#[gpui::test]
fn a_speaker_save_acknowledgement_keeps_an_edit_back_to_the_old_value(cx: &mut TestAppContext) {
    speaker_save_later_input(cx, true);
}

fn open_speakers_saving_on<'a>(
    cx: &'a mut TestAppContext,
    documents: &DocumentsRoot,
    gate: &crate::tests::Gate,
) -> (View, &'a mut VisualTestContext) {
    let mut services =
        desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(gated_save_repository(documents.repository(), gate)),
    );
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| {
        root.projects_loaded && !root.installation.catalog.is_empty() && !root.audio.pending()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    (view, cx)
}

fn speaker_save_later_input(cx: &mut TestAppContext, same_row: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let original = documents.read("weekly");
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_speakers_saving_on(cx, &documents, &gate);
    open_original(cx, &view, "weekly");
    retype_at(cx, "speaker-name-speaker-0", "Host");
    gate.arm();
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    let id = if same_row { "speaker-0" } else { "speaker-1" };
    let draft = if same_row { "" } else { "来宾草稿" };
    retype_at(cx, leak(format!("speaker-name-{id}")), draft);
    if same_row {
        cx.simulate_keystrokes("backspace");
    }
    if !same_row {
        cx.simulate_keystrokes("left left");
    }
    let focus = cx.update(|window, cx| window.focused(cx));
    // A watcher can publish the real committed revision before the write's
    // acknowledgement reaches the UI. Exercise that order explicitly.
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.revision() == original.state.revision() + 1)
    });
    let during = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row(id)
            .unwrap()
            .name
            .clone()
    });
    let during_value = field_value(cx, &during);
    gate.release();
    assert_eq!(
        during_value, draft,
        "a saved snapshot must not replace later input while its acknowledgement is pending"
    );
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row(id)
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(
        field_value(cx, &name),
        draft,
        "the acknowledgement keeps later edits"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    let saved = documents.read("weekly");
    assert_eq!(speaker_of(&saved.state, "t-0").1.as_deref(), Some("Host"));
    assert_eq!(speaker_of(&saved.state, "t-1").1, None);
    let expected = if same_row {
        None
    } else {
        Some("来宾的草稿")
    };
    if !same_row {
        cx.simulate_input("的");
        assert_eq!(
            field_value(cx, &name),
            "来宾的草稿",
            "the caret survives both snapshot and acknowledgement"
        );
    }
    // Enter is a separate second write, based on the acknowledged revision.
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none() && documents.revision("weekly") == saved.state.revision() + 1
    });
    let after = documents.read("weekly");
    let transcript_id = if same_row { "t-0" } else { "t-1" };
    assert_eq!(
        speaker_of(&after.state, transcript_id).1.as_deref(),
        expected
    );
    assert_eq!(after.state.processing, original.state.processing);
    assert_eq!(
        after.state.results.transcript,
        original.state.results.transcript
    );
    assert_eq!(after.markdown, original.markdown);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn a_speaker_save_acknowledgement_keeps_another_documents_input(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("first"), "# First body\n");
    let mut other = two_speakers("other");
    other.hierarchy.sibling_order = 1;
    documents.put(other, "# Other body\n");
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_speakers_saving_on(cx, &documents, &gate);
    open_original(cx, &view, "first");
    retype_at(cx, "speaker-name-speaker-0", "Host");
    gate.arm();
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    select_row(cx, &view, "other");
    retype_at(cx, "speaker-name-speaker-0", "乙方草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    let before = documents.package_bytes("other");
    gate.release();
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "乙方草稿");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(documents.package_bytes("other"), before);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("other"))
    );
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "乙方的草稿");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none() && documents.revision("other") == 4
    });
    assert_eq!(
        speaker_of(&documents.read("other").state, "t-0")
            .1
            .as_deref(),
        Some("乙方的草稿")
    );
    assert_eq!(
        speaker_of(&documents.read("first").state, "t-0")
            .1
            .as_deref(),
        Some("Host")
    );
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn a_speaker_save_acknowledgement_updates_clean_controls_reopened_while_saving(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("first"), "# First body\n");
    let mut other = two_speakers("other");
    other.hierarchy.sibling_order = 1;
    documents.put(other, "# Other body\n");
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_speakers_saving_on(cx, &documents, &gate);
    open_original(cx, &view, "first");
    retype_at(cx, "speaker-name-speaker-0", "Host");
    gate.arm();
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    select_row(cx, &view, "other");
    select_row(cx, &view, "first");
    let before = documents.package_bytes("first");
    gate.release();
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(
        field_value(cx, &name),
        "Host",
        "a fresh clean control follows the completed write"
    );
    assert_eq!(documents.package_bytes("first"), before);
}

#[gpui::test]
fn a_speaker_save_acknowledgement_keeps_a_remaining_name_after_merge(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let original = documents.read("weekly");
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_speakers_saving_on(cx, &documents, &gate);
    open_original(cx, &view, "weekly");
    choose(cx, "merge-speaker-speaker-0", 0);
    gate.arm();
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    retype_at(cx, "speaker-name-speaker-1", "来宾草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|p| p.revision() == original.state.revision() + 1)
    });
    let during = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-1")
            .unwrap()
            .name
            .clone()
    });
    let during_value = field_value(cx, &during);
    gate.release();
    assert_eq!(during_value, "来宾草稿");
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-1")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "来宾草稿");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert!(cx.debug_bounds("speaker-row-speaker-0").is_none());
    assert!(cx.debug_bounds("merge-speaker-speaker-1").is_none());
    let saved = documents.read("weekly");
    assert!(saved
        .state
        .results
        .speaker_segments
        .iter()
        .all(|segment| { segment.speaker_id == "speaker-1" && segment.speaker_label.is_none() }));
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "来宾的草稿");
    // The surviving row keeps its Enter subscription after the source leaves.
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none() && documents.revision("weekly") == saved.state.revision() + 1
    });
    let after = documents.read("weekly");
    assert!(after.state.results.speaker_segments.iter().all(|segment| {
        segment.speaker_id == "speaker-1" && segment.speaker_label.as_deref() == Some("来宾的草稿")
    }));
    assert_eq!(
        after.state.results.transcript,
        original.state.results.transcript
    );
    assert_eq!(spoken(&after.state), spoken(&original.state));
    assert_eq!(after.state.processing, original.state.processing);
    assert_eq!(after.markdown, original.markdown);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

#[gpui::test]
fn speaker_actions_wait_for_a_pending_write_without_blocking_typing(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# Original body\n");
    let original = documents.read("weekly");
    let gate = crate::tests::Gate::default();
    let (view, cx) = open_speakers_saving_on(cx, &documents, &gate);
    open_original(cx, &view, "weekly");
    retype_at(cx, "speaker-name-speaker-0", "Host");
    gate.arm();
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    let saved = documents.package_bytes("weekly");
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    retype_at(cx, "speaker-name-speaker-0", "");
    cx.simulate_keystrokes("backspace enter");
    let kept_notice = view.read_with(cx, |root, _| root.ui.notice == notice);
    retype_at(cx, "speaker-name-speaker-0", "主持人草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    click_selector(cx, "merge-speaker-speaker-0");
    let opened = cx.debug_bounds("select-menu").is_some();
    let kept_focus = cx.update(|window, cx| window.focused(cx)) == focus;
    if opened {
        cx.simulate_keystrokes("escape");
    }
    view.update_in(cx, |root, window, cx| {
        root.ask_merge_document_speaker("speaker-0".into(), "speaker-1".into(), window, cx)
    });
    cx.run_until_parked();
    let asked = cx.debug_bounds("confirm-dialog").is_some();
    if asked {
        click_selector(cx, "dialog-cancel");
    }
    retype_at(cx, "speaker-name-speaker-0", "主持人草稿");
    cx.simulate_keystrokes("left left tab");
    let next = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-1")
            .unwrap()
            .name
            .clone()
    });
    let skipped =
        cx.update(|window, cx| gpui::Focusable::focus_handle(next.read(cx), cx).is_focused(window));
    cx.simulate_keystrokes("shift-tab");
    let unchanged = documents.package_bytes("weekly") == saved;
    gate.release();
    assert_eq!(
        (opened, kept_focus, asked, skipped, kept_notice, unchanged),
        (false, true, false, true, true, true),
        "pending controls must be inert, skip Tab and keep input focus and the current outcome"
    );
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let name = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "主持人草稿");
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "主持人的草稿");
    choose(cx, "merge-speaker-speaker-0", 0);
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |root| {
        root.speaker_command.is_none()
            && documents.revision("weekly") == original.state.revision() + 2
    });
    let after = documents.read("weekly");
    assert!(after
        .state
        .results
        .speaker_segments
        .iter()
        .all(|segment| segment.speaker_id == "speaker-1"));
    assert_eq!(
        after.state.results.transcript,
        original.state.results.transcript
    );
    assert_eq!(after.state.processing, original.state.processing);
    assert_eq!(after.markdown, original.markdown);
}

#[gpui::test]
fn merging_a_speaker_attributes_its_segments_after_confirmation(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(two_speakers("weekly"), "# 产品周会\n");
    let before = root.read("weekly");
    let (view, cx, _changes) = open_on(cx, &root);
    open_original(cx, &view, "weekly");

    // `speaker-1` offers the one other speaker; choosing it asks first.
    let on_disk = root.package_bytes("weekly");
    choose(cx, "merge-speaker-speaker-1", 0);
    rendered(cx, "confirm-dialog");
    click_selector(cx, "dialog-cancel");
    wait_rendered(cx, "confirm-dialog", false);
    cx.run_until_parked();
    assert_eq!(
        root.package_bytes("weekly"),
        on_disk,
        "cancel writes nothing"
    );

    choose(cx, "merge-speaker-speaker-1", 0);
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| {
        root.revision("weekly") == before.state.revision() + 1
    });
    let after = root.read("weekly").state;
    assert!(
        after
            .results
            .speaker_segments
            .iter()
            .all(|segment| segment.speaker_id == "speaker-0"),
        "every segment of speaker-1 belongs to speaker-0"
    );
    assert!(after
        .results
        .diarization
        .iter()
        .all(|turn| turn.speaker_id == "speaker-0"));
    assert_eq!(after.results.transcript, before.state.results.transcript);
    assert_eq!(spoken(&after), spoken(&before.state));
    assert_eq!(after.processing, before.state.processing);

    // The merged speaker leaves the list.
    wait_rendered(cx, "speaker-row-speaker-1", false);
    rendered(cx, "speaker-row-speaker-0");
    assert!(
        cx.debug_bounds("merge-speaker-speaker-0").is_none(),
        "a lone speaker has nobody to merge into"
    );
}

#[gpui::test]
fn a_speaker_correction_on_a_stale_revision_shows_the_conflict_notice(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    root.put(two_speakers("weekly"), "# 产品周会\n");
    let loaded = root.revision("weekly");
    let (view, cx, _changes) = open_on(cx, &root);
    open_original(cx, &view, "weekly");
    retype_at(cx, "speaker-name-speaker-0", "主持人");

    // Another writer changes the document after the list was read.
    let mut external = root.read("weekly").state;
    external.content.title = "外部修改的标题".into();
    root.external_save(external);
    let on_disk = root.package_bytes("weekly");

    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |root| root.document_conflict.is_some());
    let conflict = view.read_with(cx, |root, _| root.document_conflict.clone().unwrap());
    assert_eq!(conflict.document_id.as_str(), "weekly");
    assert_eq!((conflict.expected, conflict.actual), (loaded, loaded + 1));
    assert!(within(
        rendered(cx, "document-conflict"),
        rendered(cx, "meetings-page")
    ));
    assert_eq!(
        root.package_bytes("weekly"),
        on_disk,
        "the refused correction wrote nothing"
    );

    wait_for(cx, &view, |root| {
        root.projects.iter().any(|p| p.title() == "外部修改的标题")
    });
    let draft = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(
        field_value(cx, &draft),
        "主持人",
        "a refused correction keeps the draft through the following refresh"
    );
    click_selector(cx, "reload-conflicted-document");
    wait_for(cx, &view, |root| {
        root.document_conflict.is_none()
            && root
                .projects
                .iter()
                .any(|project| project.title() == "外部修改的标题")
    });
    // An explicit reload discards the name draft, then renaming uses the
    // revision just read.
    cx.run_until_parked();
    let reloaded = view.read_with(cx, |root, _| {
        root.speaker_rows
            .as_ref()
            .unwrap()
            .row("speaker-0")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &reloaded), "");
    retype_at(cx, "speaker-name-speaker-0", "主持人");
    click_selector(cx, "rename-speaker-speaker-0");
    wait_for(cx, &view, |_| root.revision("weekly") == loaded + 2);
    assert_eq!(
        speaker_of(&root.read("weekly").state, "t-0").1.as_deref(),
        Some("主持人")
    );
}

#[gpui::test]
fn the_fixed_speaker_merge_is_gone(cx: &mut TestAppContext) {
    for key in ["merge_speaker", "speaker_merged"] {
        for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
            assert!(i18n::text(locale, key).is_empty(), "`{key}` is retired");
        }
    }
    let root = DocumentsRoot::new();
    root.put(two_speakers("weekly"), "# 产品周会\n");
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    assert!(cx.debug_bounds("merge-speaker").is_none());
    open_original(cx, &view, "weekly");
    assert!(cx.debug_bounds("merge-speaker").is_none());
    assert!(cx.debug_bounds("merge-speaker-speaker-1").is_some());
}

/// A Speaker Library file in a directory of its own, removed when the test
/// ends.
struct Library(PathBuf);

impl Library {
    fn new(profiles: Vec<SpeakerProfile>) -> Self {
        let directory = temp_directory();
        let library = Self(directory);
        JsonSpeakerProfileStore::new(library.path())
            .replace(&SpeakerLibraryDocument {
                profiles,
                ..SpeakerLibraryDocument::default()
            })
            .unwrap();
        library
    }

    fn path(&self) -> PathBuf {
        self.0.join("library.json")
    }

    fn load(&self) -> SpeakerLibraryDocument {
        JsonSpeakerProfileStore::new(self.path()).load().unwrap()
    }

    fn names(&self) -> Vec<(String, String)> {
        self.load()
            .profiles
            .into_iter()
            .map(|profile| (profile.id, profile.display_name))
            .collect()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A profile of `samples` samples at `vector`, each traceable to a
/// document segment.
fn profile(id: &str, name: &str, vector: [f32; 2], samples: usize) -> SpeakerProfile {
    SpeakerProfile {
        id: id.into(),
        display_name: name.into(),
        aliases: Vec::new(),
        embedding_space: contracts::EmbeddingSpace {
            backend_id: "diarization-native".into(),
            model_id: "embedding".into(),
            model_revision: Some("1".into()),
            dimension: 2,
            sample_rate_hz: 16_000,
            frontend: contracts::EmbeddingFrontend::default(),
            legacy_feature_config: None,
            normalization: contracts::EmbeddingNormalization::L2,
        },
        samples: (0..samples)
            .map(|index| contracts::SpeakerEmbeddingSample {
                id: format!("{id}-{index}"),
                vector: vector.to_vec(),
                quality: 0.9,
                source: contracts::SpeakerSampleSource {
                    project_id: Some("weekly".into()),
                    source_segment_id: Some(format!("t-{index}")),
                    start_ms: 0,
                    end_ms: 4_000,
                    audio_path: None,
                    audio_sha256: None,
                    backup_path: None,
                },
                created_at_ms: 1,
            })
            .collect(),
        match_threshold: 0.7,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

/// `alpha` and `alpha-2` are one voice; `beta` is another.
fn three_profiles() -> Vec<SpeakerProfile> {
    vec![
        profile("alpha", "Alpha", [1.0, 0.0], 2),
        profile("alpha-2", "Alpha (2)", [0.99, 0.14], 1),
        profile("beta", "Beta", [0.0, 1.0], 1),
    ]
}

/// The desktop on `documents` with its Speaker Library at `library`, on the
/// Settings route scrolled to the Speaker Library.
fn open_with_library<'a>(
    cx: &'a mut TestAppContext,
    library: &Path,
    documents: &DocumentsRoot,
) -> (View, &'a mut VisualTestContext, DesktopServices) {
    let settings = test_settings();
    let mut value = settings.snapshot().unwrap();
    value.speaker_library_path = library.display().to_string();
    settings.update(value).unwrap();
    let mut services = desktop_services(Arc::new(FakeAssistantUiService::default()), settings);
    services.workspace = WorkspaceService::new(
        ProcessingService::new(
            Arc::new(WhisperRsBackend::default()),
            Arc::new(SherpaOnnxDiarizationBackend::default()),
        ),
        DocumentService::new(Arc::new(documents.repository())),
    );
    services.speaker_library = crate::speaker_library_at(library);
    let (view, cx) = open_desktop_on(cx, services.clone());
    view.update(cx, |root, cx| root.navigate(Route::Settings, cx));
    cx.run_until_parked();
    // Select the section after Settings has laid out, as a user does.
    click_selector(cx, "settings-section-5");
    assert!(
        rendered(cx, "speaker-library-section").top() >= rendered(cx, "settings-scroll").top(),
        "the library begins inside the scroll viewport"
    );
    (view, cx, services)
}

#[gpui::test]
fn the_speaker_library_section_lists_and_renames_profiles(cx: &mut TestAppContext) {
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _services) = open_with_library(cx, &library.path(), &documents);

    wait_rendered(cx, "speaker-profile-alpha", true);
    let section = rendered(cx, "speaker-library-section");
    for id in ["alpha", "alpha-2", "beta"] {
        let row = rendered(cx, leak(format!("speaker-profile-{id}")));
        assert!(within(row, section), "`{id}` lies in the section");
        for control in [
            "speaker-profile-name",
            "rename-speaker-profile",
            "merge-speaker-profile",
            "delete-speaker-profile",
        ] {
            assert!(within(rendered(cx, leak(format!("{control}-{id}"))), row));
        }
    }

    retype_at(cx, "speaker-profile-name-alpha", "Alice");
    click_selector(cx, "rename-speaker-profile-alpha");
    wait_for(cx, &view, |_| library.names()[0].1 == "Alice");
    assert_eq!(
        library.names(),
        [
            ("alpha".into(), "Alice".into()),
            ("alpha-2".into(), "Alpha (2)".into()),
            ("beta".into(), "Beta".into())
        ]
    );
    wait_rendered(cx, "speaker-library-status", true);
    let renamed = i18n::message(i18n::Locale::ZhCn, "speaker_profile_renamed", "Alice");
    assert_eq!(
        view.read_with(cx, |root, _| root.speaker_profiles.status.clone()),
        Some((renamed, false))
    );
}

#[gpui::test]
fn deleting_a_profile_asks_first_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _services) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-beta", true);

    click_selector(cx, "delete-speaker-profile-beta");
    rendered(cx, "confirm-dialog");
    click_selector(cx, "dialog-cancel");
    wait_rendered(cx, "confirm-dialog", false);
    cx.run_until_parked();
    assert_eq!(library.load().profiles.len(), 3, "cancel keeps the profile");
    rendered(cx, "speaker-profile-beta");

    click_selector(cx, "delete-speaker-profile-beta");
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| library.load().profiles.len() == 2);
    assert!(library.names().iter().all(|(id, _)| id != "beta"));
    wait_rendered(cx, "speaker-profile-beta", false);
}

#[gpui::test]
fn a_long_chinese_profile_name_keeps_confirmation_actions_inside_the_minimum_window(
    cx: &mut TestAppContext,
) {
    long_profile_confirmation(
        cx,
        i18n::Locale::ZhCn,
        crate::pages::chrome::MIN_WINDOW_SIZE,
    );
}

#[gpui::test]
fn a_long_english_profile_name_keeps_confirmation_actions_inside_the_window(
    cx: &mut TestAppContext,
) {
    long_profile_confirmation(
        cx,
        i18n::Locale::EnUs,
        gpui::size(gpui::px(1180.), gpui::px(760.)),
    );
}

fn long_profile_confirmation(
    cx: &mut TestAppContext,
    locale: i18n::Locale,
    size: gpui::Size<gpui::Pixels>,
) {
    let name = match locale {
        i18n::Locale::ZhCn => "产品研发与用户访谈".repeat(80),
        i18n::Locale::EnUs => "Research and customer interviews ".repeat(100),
    };
    let library = Library::new(vec![profile("alpha", &name, [1.0, 0.0], 1)]);
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    cx.simulate_resize(size);
    view.update(cx, |root, cx| {
        root.ui.locale = locale;
        cx.notify();
    });
    cx.run_until_parked();
    click_selector(cx, "settings-section-5");
    let before = std::fs::read(library.path()).unwrap();
    click_selector(cx, "delete-speaker-profile-alpha");
    let window = gpui::Bounds::new(gpui::point(gpui::px(0.), gpui::px(0.)), size);
    let panel = rendered(cx, "confirm-dialog");
    assert!(within(panel, window), "a valid long profile name must not push the dialog outside the window: {panel:?} in {window:?}");
    for button in ["dialog-cancel", "dialog-confirm"] {
        assert!(
            within(rendered(cx, button), panel),
            "{button} remains visible and usable"
        );
    }
    let content = rendered(cx, "confirm-dialog-content");
    let text = rendered(cx, "confirm-dialog-text");
    assert!(
        text.size.height > content.size.height,
        "the full question is scrollable instead of discarded"
    );
    let buttons = [
        rendered(cx, "dialog-cancel"),
        rendered(cx, "dialog-confirm"),
    ];
    let background_scroll = view.read_with(cx, |root, _| root.scroll.settings.offset());
    let initial_position = text.top() - content.top();
    let focus = cx.update(|window, cx| window.focused(cx));
    cx.simulate_keystrokes("pagedown");
    cx.run_until_parked();
    assert!(
        rendered(cx, "confirm-dialog-text").top() - rendered(cx, "confirm-dialog-content").top()
            < initial_position,
        "Page Down must reveal the long question while the dialog root holds focus"
    );
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        focus,
        "reading keeps focus in place"
    );
    cx.simulate_keystrokes("pageup tab");
    cx.run_until_parked();
    let button_focus = cx.update(|window, cx| window.focused(cx));
    assert_ne!(button_focus, focus, "Tab reaches a dialog button");
    assert!(
        cx.update(|window, cx| view.read(cx).dialog_focus.contains_focused(window, cx)),
        "Tab stays inside the dialog while reading"
    );
    cx.simulate_keystrokes(
        "pagedown pagedown pagedown pagedown pagedown pagedown pagedown pagedown pagedown pagedown",
    );
    cx.run_until_parked();
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        button_focus,
        "paging from a button keeps its focus"
    );
    assert!(
        rendered(cx, "confirm-dialog-text").bottom()
            <= rendered(cx, "confirm-dialog-content").bottom() + gpui::px(1.),
        "paging reaches the description at the end without overscrolling"
    );
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.settings.offset()),
        background_scroll,
        "paging must not move the settings page behind the dialog"
    );
    cx.simulate_keystrokes("pageup pageup pageup pageup pageup pageup pageup pageup pageup pageup");
    cx.run_until_parked();
    assert_eq!(
        rendered(cx, "confirm-dialog-text").top() - rendered(cx, "confirm-dialog-content").top(),
        initial_position,
        "Page Up returns to the beginning without overscrolling"
    );
    let track = rendered(cx, "confirm-dialog-scrollbar-viewport");
    assert_eq!(
        track.right(),
        panel.right(),
        "the scrollbar reaches the dialog's right edge"
    );
    let start = gpui::point(track.right() - gpui::px(7.), track.top() + gpui::px(8.));
    let end = start + gpui::point(gpui::px(0.), gpui::px(50.));
    cx.simulate_mouse_move(start, None, gpui::Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        rendered(cx, "confirm-dialog-text").top() - rendered(cx, "confirm-dialog-content").top()
            < initial_position,
        "the scrollbar at the outer edge can be grabbed and dragged"
    );
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: content.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-10000.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let scrolled = rendered(cx, "confirm-dialog-text");
    let content = rendered(cx, "confirm-dialog-content");
    assert!(
        scrolled.top() - content.top() < initial_position,
        "the full name can be read by scrolling"
    );
    assert!(
        scrolled.bottom() <= content.bottom() + gpui::px(1.),
        "the description after the full name is reachable"
    );
    assert_eq!(
        rendered(cx, "dialog-cancel").top() - rendered(cx, "confirm-dialog").top(),
        buttons[0].top() - panel.top()
    );
    assert_eq!(
        rendered(cx, "dialog-confirm").top() - rendered(cx, "confirm-dialog").top(),
        buttons[1].top() - panel.top()
    );
    click_selector(cx, "dialog-cancel");
    wait_rendered(cx, "confirm-dialog", false);
    assert_eq!(
        std::fs::read(library.path()).unwrap(),
        before,
        "cancel keeps the real library unchanged"
    );
    click_selector(cx, "delete-speaker-profile-alpha");
    let text = rendered(cx, "confirm-dialog-text");
    let content = rendered(cx, "confirm-dialog-content");
    assert!(
        text.top() >= content.top(),
        "a new confirmation starts at the beginning of its question"
    );
    cx.simulate_keystrokes("escape");
    wait_rendered(cx, "confirm-dialog", false);
    assert_eq!(std::fs::read(library.path()).unwrap(), before);
    click_selector(cx, "delete-speaker-profile-alpha");
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| library.load().profiles.is_empty());
    assert!(
        library.path().is_file(),
        "the library asset remains present after confirming profile deletion"
    );
}

#[gpui::test]
fn deleting_a_profile_moves_focus_to_the_next_name(cx: &mut TestAppContext) {
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let field = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    cx.update(|window, cx| gpui::Focusable::focus_handle(field.read(cx), cx).focus(window, cx));
    click_selector(cx, "delete-speaker-profile-alpha");
    click_selector(cx, "dialog-confirm");
    wait_rendered(cx, "speaker-profile-alpha", false);
    let next = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha-2")
            .unwrap()
            .name
            .clone()
    });
    assert!(
        cx.update(|window, cx| gpui::Focusable::focus_handle(next.read(cx), cx).is_focused(window)),
        "the next surviving profile name receives focus"
    );
}

#[gpui::test]
fn deleting_the_only_profile_moves_focus_to_the_library_title(cx: &mut TestAppContext) {
    let library = Library::new(vec![profile("alpha", "Alpha", [1.0, 0.0], 1)]);
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let field = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    let removed_focus = cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(field.read(cx), cx);
        focus.focus(window, cx);
        focus
    });
    click_selector(cx, "delete-speaker-profile-alpha");
    click_selector(cx, "dialog-confirm");
    wait_rendered(cx, "speaker-profile-alpha", false);
    let focused = cx.update(|window, cx| window.focused(cx));
    assert!(
        focused.is_some(),
        "the empty library keeps a focus destination"
    );
    assert_ne!(
        focused,
        Some(removed_focus),
        "focus cannot remain on the removed profile"
    );
    let title = view.read_with(cx, |root, _| root.speaker_library_focus.clone());
    assert_eq!(
        focused,
        Some(title),
        "the Speaker Library title receives focus"
    );
    rendered(cx, "speaker-library-title");
}

/// Holds the real library's replacement until the test permits deletion.
struct GatedLibrary {
    inner: JsonSpeakerProfileStore,
    gate: crate::tests::Gate,
    pause_after_save: bool,
}

impl application::SpeakerProfileStore for GatedLibrary {
    fn load(&self) -> Result<SpeakerLibraryDocument, application::SpeakerLibraryError> {
        self.inner.load()
    }

    fn replace(
        &self,
        value: &SpeakerLibraryDocument,
    ) -> Result<(), application::SpeakerLibraryError> {
        if !self.pause_after_save {
            self.gate.pass();
        }
        self.inner.replace(value)?;
        if self.pause_after_save {
            self.gate.pass();
        }
        Ok(())
    }
}

fn pause_saved_library(
    view: &View,
    cx: &mut VisualTestContext,
    library: &Library,
) -> crate::tests::Gate {
    let gate = crate::tests::Gate::default();
    view.update(cx, |root, _| {
        root.services.speaker_library =
            application::SpeakerLibraryService::new(Arc::new(GatedLibrary {
                inner: JsonSpeakerProfileStore::new(library.path()),
                gate: gate.clone(),
                pause_after_save: true,
            }));
    });
    gate
}

#[gpui::test]
fn profile_actions_wait_for_a_pending_write_without_blocking_typing(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let gate = pause_saved_library(&view, cx, &library);
    retype_at(cx, "speaker-profile-name-alpha", "Alice");
    gate.arm();
    click_selector(cx, "rename-speaker-profile-alpha");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    let saved = std::fs::read(library.path()).unwrap();
    let status = view.read_with(cx, |root, _| root.speaker_profiles.status.clone());
    retype_at(cx, "speaker-profile-name-alpha", "Alpha");
    cx.simulate_keystrokes("enter");
    let kept_status = view.read_with(cx, |root, _| root.speaker_profiles.status == status);
    retype_at(cx, "speaker-profile-name-alpha", "主持人草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    click_selector(cx, "merge-speaker-profile-alpha");
    let opened = cx.debug_bounds("select-menu").is_some();
    let kept_focus = cx.update(|window, cx| window.focused(cx)) == focus;
    if opened {
        cx.simulate_keystrokes("escape");
    }
    view.update_in(cx, |root, window, cx| {
        root.ask_merge_speaker_profiles("alpha-2".into(), "alpha".into(), window, cx)
    });
    cx.run_until_parked();
    let asked_merge = cx.debug_bounds("confirm-dialog").is_some();
    if asked_merge {
        click_selector(cx, "dialog-cancel");
    }
    view.update_in(cx, |root, window, cx| {
        root.ask_delete_speaker_profile("alpha-2".into(), window, cx)
    });
    cx.run_until_parked();
    let asked_delete = cx.debug_bounds("confirm-dialog").is_some();
    if asked_delete {
        click_selector(cx, "dialog-cancel");
    }
    retype_at(cx, "speaker-profile-name-alpha", "主持人草稿");
    cx.simulate_keystrokes("left left tab");
    let next = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha-2")
            .unwrap()
            .name
            .clone()
    });
    let skipped =
        cx.update(|window, cx| gpui::Focusable::focus_handle(next.read(cx), cx).is_focused(window));
    cx.simulate_keystrokes("shift-tab");
    let unchanged = std::fs::read(library.path()).unwrap() == saved;
    gate.release();
    assert_eq!(
        (
            opened,
            kept_focus,
            asked_merge,
            asked_delete,
            skipped,
            kept_status,
            unchanged
        ),
        (false, true, false, false, true, true, true),
        "pending controls must be inert, skip Tab and keep input focus and the current outcome"
    );
    wait_for(cx, &view, |root| root.speaker_profiles.write_path.is_none());
    let name = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "主持人草稿");
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "主持人的草稿");
    choose(cx, "merge-speaker-profile-alpha-2", 0);
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |root| {
        root.speaker_profiles.write_path.is_none() && library.load().profiles.len() == 2
    });
    assert_eq!(library.load().profiles[0].samples.len(), 3);
    assert_eq!(library.load().profiles[0].display_name, "Alice");
    click_selector(cx, "speaker-profile-name-alpha");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_profiles.write_path.is_none() && library.names()[0].1 == "主持人的草稿"
    });
    assert_eq!(library.load().profiles[0].samples.len(), 3);
}

#[gpui::test]
fn a_profile_save_keeps_later_input_in_another_row(cx: &mut TestAppContext) {
    profile_save_later_input(cx, false);
}

#[gpui::test]
fn a_profile_save_keeps_an_edit_back_to_the_old_name(cx: &mut TestAppContext) {
    profile_save_later_input(cx, true);
}

fn profile_save_later_input(cx: &mut TestAppContext, same_row: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let gate = pause_saved_library(&view, cx, &library);
    retype_at(cx, "speaker-profile-name-alpha", "Alice");
    gate.arm();
    click_selector(cx, "rename-speaker-profile-alpha");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    let (id, draft) = if same_row {
        ("alpha", "Alpha")
    } else {
        ("beta", "来宾草稿")
    };
    retype_at(cx, leak(format!("speaker-profile-name-{id}")), draft);
    if !same_row {
        cx.simulate_keystrokes("left left");
    }
    let focus = cx.update(|window, cx| window.focused(cx));
    view.update(cx, |root, cx| root.reload_speaker_profiles(cx));
    wait_for(cx, &view, |root| {
        root.speaker_profiles
            .find("alpha")
            .is_some_and(|p| p.display_name == "Alice")
    });
    let during = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row(id)
            .unwrap()
            .name
            .clone()
    });
    let during_value = field_value(cx, &during);
    gate.release();
    assert_eq!(
        during_value, draft,
        "a read arriving before the acknowledgement keeps later input"
    );
    wait_for(cx, &view, |root| root.speaker_profiles.write_path.is_none());
    let name = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row(id)
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), draft);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        library.names()[0].1,
        "Alice",
        "later typing is not another write"
    );
    let before = library.load();
    if !same_row {
        cx.simulate_input("的");
        assert_eq!(field_value(cx, &name), "来宾的草稿");
    }
    let expected = if same_row { "Alpha" } else { "来宾的草稿" };
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_profiles.write_path.is_none()
            && library
                .names()
                .iter()
                .any(|(key, name)| key == id && name == expected)
    });
    let after = library.load();
    for prior in before.profiles {
        let saved = after.profiles.iter().find(|p| p.id == prior.id).unwrap();
        assert_eq!(saved.samples, prior.samples);
        assert_eq!(saved.embedding_space, prior.embedding_space);
        assert_eq!(saved.aliases, prior.aliases);
    }
    assert!(view.read_with(cx, |root, _| root
        .speaker_profiles
        .status
        .as_ref()
        .is_some_and(|(_, failed)| !failed)));
}

#[gpui::test]
fn a_profile_save_keeps_a_remaining_name_after_merge(cx: &mut TestAppContext) {
    profile_removal_later_input(cx, true);
}

#[gpui::test]
fn a_profile_save_keeps_a_remaining_name_after_deletion(cx: &mut TestAppContext) {
    profile_removal_later_input(cx, false);
}

fn profile_removal_later_input(cx: &mut TestAppContext, merge: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha-2", true);
    let gate = pause_saved_library(&view, cx, &library);
    if merge {
        choose(cx, "merge-speaker-profile-alpha-2", 0);
    } else {
        click_selector(cx, "delete-speaker-profile-alpha-2");
    }
    gate.arm();
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    retype_at(cx, "speaker-profile-name-alpha", "主持人草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    view.update(cx, |root, cx| root.reload_speaker_profiles(cx));
    wait_for(cx, &view, |root| root.speaker_profiles.profiles.len() == 2);
    let during = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    let during_value = field_value(cx, &during);
    gate.release();
    assert_eq!(during_value, "主持人草稿");
    wait_for(cx, &view, |root| root.speaker_profiles.write_path.is_none());
    let name = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "主持人草稿");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert!(cx.debug_bounds("speaker-profile-alpha-2").is_none());
    let before = library.load();
    assert_eq!(before.profiles[0].display_name, "Alpha");
    assert_eq!(before.profiles[0].samples.len(), if merge { 3 } else { 2 });
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "主持人的草稿");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| {
        root.speaker_profiles.write_path.is_none() && library.names()[0].1 == "主持人的草稿"
    });
    assert_eq!(
        library.load().profiles[0].samples,
        before.profiles[0].samples
    );
}

#[gpui::test]
fn a_profile_save_does_not_publish_its_outcome_in_another_library(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let library = Library::new(three_profiles());
    let mut profiles = three_profiles();
    profiles[0].display_name = "Another Alpha".into();
    let other = Library::new(profiles);
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let gate = pause_saved_library(&view, cx, &library);
    retype_at(cx, "speaker-profile-name-alpha", "Alice");
    gate.arm();
    click_selector(cx, "rename-speaker-profile-alpha");
    wait_for(cx, &view, |_| gate.counts() == (1, 1));
    view.update(cx, |root, cx| {
        root.change_speaker_library_path(other.path(), cx)
    });
    wait_for(cx, &view, |root| {
        root.settings_draft.speaker_library_path == other.path().display().to_string()
            && root
                .speaker_profiles
                .find("alpha")
                .is_some_and(|p| p.display_name == "Another Alpha")
    });
    retype_at(cx, "speaker-profile-name-beta", "新库草稿");
    cx.simulate_keystrokes("left left");
    let focus = cx.update(|window, cx| window.focused(cx));
    let before = std::fs::read(other.path()).unwrap();
    gate.release();
    wait_for(cx, &view, |root| root.speaker_profiles.write_path.is_none());
    let name = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("beta")
            .unwrap()
            .name
            .clone()
    });
    assert_eq!(field_value(cx, &name), "新库草稿");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(std::fs::read(other.path()).unwrap(), before);
    assert_eq!(library.names()[0].1, "Alice");
    assert!(
        view.read_with(cx, |root, _| root.speaker_profiles.status.is_none()),
        "the old library's success must not be shown in the new library"
    );
    cx.simulate_input("的");
    assert_eq!(field_value(cx, &name), "新库的草稿");
}

#[gpui::test]
fn deleting_a_profile_preserves_focus_moved_while_the_worker_is_pending(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);
    let gate = crate::tests::Gate::default();
    view.update(cx, |root, _| {
        root.services.speaker_library =
            application::SpeakerLibraryService::new(Arc::new(GatedLibrary {
                inner: JsonSpeakerProfileStore::new(library.path()),
                gate: gate.clone(),
                pause_after_save: false,
            }));
    });
    let field = view.read_with(cx, |root, _| {
        root.profile_rows
            .as_ref()
            .unwrap()
            .row("alpha")
            .unwrap()
            .name
            .clone()
    });
    let original_focus = cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(field.read(cx), cx);
        focus.focus(window, cx);
        focus
    });
    gate.arm();
    click_selector(cx, "delete-speaker-profile-alpha");
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    assert_eq!(
        library.load().profiles.len(),
        3,
        "the deletion is still blocked"
    );
    // From the first name, Shift-Tab reaches the stable directory control
    // before the profile rows; this is a real key event while the worker waits.
    cx.simulate_keystrokes("shift-tab");
    let moved_focus = cx.update(|window, cx| window.focused(cx));
    let held = gate.counts().1;
    gate.release();
    wait_rendered(cx, "speaker-profile-alpha", false);
    assert_eq!(held, 1, "focus moved before the deletion was released");
    assert!(moved_focus.is_some());
    assert_ne!(
        moved_focus,
        Some(original_focus),
        "Shift-Tab moved focus away"
    );
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        moved_focus,
        "the asynchronous deletion must keep the user's new focus"
    );
}

#[gpui::test]
fn merging_profiles_moves_samples_and_shows_refusals(cx: &mut TestAppContext) {
    let library = Library::new(three_profiles());
    let documents = DocumentsRoot::new();
    let (view, cx, _services) = open_with_library(cx, &library.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha-2", true);

    // `alpha-2` offers `alpha` first; the merge asks first.
    choose(cx, "merge-speaker-profile-alpha-2", 0);
    rendered(cx, "confirm-dialog");
    assert_eq!(library.load().profiles.len(), 3);
    click_selector(cx, "dialog-confirm");
    wait_for(cx, &view, |_| library.load().profiles.len() == 2);
    let alpha = library.load().profiles.remove(0);
    assert_eq!(alpha.id, "alpha");
    assert_eq!(alpha.samples.len(), 3, "the samples of both profiles");
    assert_eq!(alpha.aliases, ["Alpha (2)"]);
    wait_rendered(cx, "speaker-profile-alpha-2", false);

    // Two different voices are refused: the error shows, nothing changes.
    let unchanged = library.load();
    choose(cx, "merge-speaker-profile-beta", 0);
    click_selector(cx, "dialog-confirm");
    wait_rendered(cx, "speaker-library-status", true);
    wait_for(cx, &view, |root| {
        root.speaker_profiles
            .status
            .as_ref()
            .is_some_and(|(_, error)| *error)
    });
    assert_eq!(library.load(), unchanged);
    rendered(cx, "speaker-profile-beta");
}

#[gpui::test]
fn changing_the_library_path_rebuilds_the_service_and_the_processing_options(
    cx: &mut TestAppContext,
) {
    let first = Library::new(vec![profile("alpha", "Alpha", [1.0, 0.0], 1)]);
    let second = Library::new(vec![profile("beta", "Beta", [0.0, 1.0], 1)]);
    let documents = DocumentsRoot::new();
    documents.put(two_speakers("weekly"), "# 产品周会\n");
    let (view, cx, services) = open_with_library(cx, &first.path(), &documents);
    wait_rendered(cx, "speaker-profile-alpha", true);

    // The new path already holds a library, so it is used without migrating.
    view.update(cx, |root, cx| {
        root.change_speaker_library_path(second.path(), cx)
    });
    let expected = second.path().display().to_string();
    wait_for(cx, &view, |_| {
        services.settings.snapshot().unwrap().speaker_library_path == expected
    });
    wait_rendered(cx, "speaker-profile-beta", true);
    assert!(cx.debug_bounds("speaker-profile-alpha").is_none());
    let model = |backend_id: &str| ModelRef {
        backend_id: backend_id.into(),
        model_id: "fixture".into(),
        revision: None,
        artifact_path: "missing-model".into(),
        artifacts: Vec::new(),
    };
    let request = services
        .workspace
        .processing_request(
            &ProjectId::new("weekly"),
            model("whisper-rs"),
            model("sherpa-onnx"),
            None,
        )
        .unwrap();
    assert_eq!(
        request.speaker_library_path.as_deref(),
        Some(expected.as_str())
    );

    // Library calls reach the new file only.
    retype_at(cx, "speaker-profile-name-beta", "Bob");
    click_selector(cx, "rename-speaker-profile-beta");
    wait_for(cx, &view, |_| second.names()[0].1 == "Bob");
    assert_eq!(first.names(), [("alpha".into(), "Alpha".into())]);
}
