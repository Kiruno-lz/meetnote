//! L2: speaker correction in the Original view on real document packages
//! (`FileDocumentRepository` in a temporary documents root), and the Speaker
//! Library section of Settings on a real library file
//! (`JsonSpeakerProfileStore` in a temporary directory).
//!
//! Every correction is one revision-checked command; the conflict notice and
//! the document helpers are the Documents slice's (`document_tests`).

use crate::document_tests::{
    leak, open_on, processed, rendered, select_row, within, DocumentsRoot, View,
};
use crate::tests::{
    click_selector, desktop_services, open_desktop_on, temp_directory, test_settings, wait_for,
    FakeAssistantUiService,
};
use crate::{i18n, route::Route, DesktopServices};
use application::{
    DocumentService, JsonSpeakerProfileStore, ProcessingService, SpeakerProfileStore as _,
    WorkspaceService,
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
fn two_speakers(id: &str) -> DocumentState {
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
    for _ in 0..500 {
        cx.run_until_parked();
        if cx.debug_bounds(selector).is_some() == rendered {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("`{selector}` rendered: expected {rendered}");
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

    click_selector(cx, "reload-conflicted-document");
    wait_for(cx, &view, |root| {
        root.document_conflict.is_none()
            && root
                .projects
                .iter()
                .any(|project| project.title() == "外部修改的标题")
    });
    // Renaming on the reloaded revision succeeds.
    cx.run_until_parked();
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
    view.update(cx, |root, cx| {
        root.navigate(Route::Settings, cx);
        root.scroll
            .settings
            .scroll_to_top_of_item(crate::pages::settings::SPEAKER_LIBRARY_SECTION);
    });
    cx.run_until_parked();
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
}

impl application::SpeakerProfileStore for GatedLibrary {
    fn load(&self) -> Result<SpeakerLibraryDocument, application::SpeakerLibraryError> {
        self.inner.load()
    }

    fn replace(
        &self,
        value: &SpeakerLibraryDocument,
    ) -> Result<(), application::SpeakerLibraryError> {
        self.gate.pass();
        self.inner.replace(value)
    }
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
