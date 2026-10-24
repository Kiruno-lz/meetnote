//! L2: the documents root chosen in Settings, and the recovery of a
//! document's missing source audio, on real document packages in temporary
//! documents roots (`FileDocumentRepository`) and real `settings.json`
//! files.
//!
//! The platform's folder and file prompts do not exist on the test
//! platform; the tests call what their callbacks call:
//! `choose_documents_root` and `relink_audio_path`.

use crate::document_tests::{
    open_on, processed, rendered, scroll_document, select_row, within, DocumentsRoot,
    UnreadableDocumentsRoot, View,
};
use crate::tests::{
    click_selector, desktop_services, empty_result, focus_field, open_desktop_on, pending_review,
    temp_directory, wait_for, FakeAssistantUiService,
};
use crate::{
    assistant_ui::{AssistantMutationSummary, AssistantRunResult, PendingReviewState},
    i18n::{self, Locale},
    pages::detail,
    route::Route,
};
use application::{
    AudioSourceError, AudioStatus, DocumentChange, DocumentRepository, DocumentService,
    DocumentsRootError, ProcessingService, ReservedLocation, WorkspaceService,
};
use asr_whisper_rs::WhisperRsBackend;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::{DocumentState, ProjectId};
use gpui::{TestAppContext, VisualTestContext};
use persistence::FileDocumentRepository;
use runtime::JobRuntime;
use settings::{JsonSettingsRepository, SettingsService};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The bytes of a recording and their SHA-256, as processing records it.
const AUDIO: &[u8] = b"meetnote fixture audio\n";
const AUDIO_SHA256: &str = "731e82f7248116585d832c2ea8784dc94a8dcfd6529a6babfc94605e8fda9e49";
/// Another recording.
const OTHER_AUDIO: &[u8] = b"a different recording\n";
const OTHER_SHA256: &str = "13c12877ebfb2e7edafd47f9c0b765a656e85a055c2b5cb35c3ff7770e611e93";

/// Holds a successful real settings save after file IO, while the switch
/// still waits to return its result to the desktop.
pub(crate) struct HeldSettingsSave {
    pub(crate) inner: JsonSettingsRepository,
    pub(crate) gate: crate::tests::Gate,
}

impl settings::SettingsRepository for HeldSettingsSave {
    fn load(&self) -> Result<settings::AppSettings, settings::SettingsError> {
        settings::SettingsRepository::load(&self.inner)
    }

    fn save(&self, value: &settings::AppSettings) -> Result<(), settings::SettingsError> {
        settings::SettingsRepository::save(&self.inner, value)
    }

    fn save_if_equal(
        &self,
        expected: &settings::AppSettings,
        next: &settings::AppSettings,
    ) -> Result<(), settings::SettingsError> {
        let result = settings::SettingsRepository::save_if_equal(&self.inner, expected, next);
        if result.is_ok() {
            self.gate.pass();
        }
        result
    }
}

/// Pauses before the real compare-and-save so another writer can win.
struct HeldSettingsBeforeSave {
    inner: JsonSettingsRepository,
    gate: crate::tests::Gate,
}

impl settings::SettingsRepository for HeldSettingsBeforeSave {
    fn load(&self) -> Result<settings::AppSettings, settings::SettingsError> {
        settings::SettingsRepository::load(&self.inner)
    }

    fn save(&self, value: &settings::AppSettings) -> Result<(), settings::SettingsError> {
        settings::SettingsRepository::save(&self.inner, value)
    }

    fn save_if_equal(
        &self,
        expected: &settings::AppSettings,
        next: &settings::AppSettings,
    ) -> Result<(), settings::SettingsError> {
        self.gate.pass();
        settings::SettingsRepository::save_if_equal(&self.inner, expected, next)
    }
}

/// Holds a real list/audio snapshot after IO has released its file locks.
struct HeldSnapshots {
    inner: FileDocumentRepository,
    gate: crate::tests::Gate,
}

impl DocumentRepository for HeldSnapshots {
    fn snapshot(&self) -> Result<application::DocumentSnapshot, String> {
        let snapshot = self.inner.snapshot();
        self.gate.pass();
        snapshot
    }

    fn get(&self, id: &ProjectId) -> Result<Option<DocumentState>, String> {
        self.inner.get(id)
    }

    fn read(&self, id: &ProjectId) -> Result<Option<contracts::DocumentRecord>, String> {
        self.inner.read(id)
    }

    fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
        self.inner.save(document, markdown)
    }

    fn watch(&self) -> Result<std::sync::mpsc::Receiver<DocumentChange>, String> {
        self.inner.watch()
    }

    fn subscribe_changes(&self) -> Result<application::DocumentChanges, String> {
        self.inner.subscribe_changes()
    }
}

/// Two documents roots, a model directory, the jobs the workspace runs on
/// and settings in a `settings.json` of their own. The workspace starts on
/// `old` and switches roots as `main`'s does.
struct Workspace {
    old: DocumentsRoot,
    new: DocumentsRoot,
    models: PathBuf,
    settings_path: PathBuf,
    settings: SettingsService,
    jobs: Arc<JobRuntime>,
    workspace: WorkspaceService,
}

impl Workspace {
    fn new() -> Self {
        Self::with_read_gate(None)
    }

    fn with_read_gate(read_gate: Option<&crate::tests::Gate>) -> Self {
        Self::with_gates(read_gate, None, None)
    }

    fn with_save_gates(old: &crate::tests::Gate, new: &crate::tests::Gate) -> Self {
        Self::with_gates(None, Some((old, new)), None)
    }

    fn with_import_gates(save: &crate::tests::Gate, snapshot_failure: &crate::tests::Gate) -> Self {
        Self::with_gates(
            None,
            Some((save, &crate::tests::Gate::default())),
            Some(snapshot_failure),
        )
    }

    fn with_gates(
        read_gate: Option<&crate::tests::Gate>,
        save_gates: Option<(&crate::tests::Gate, &crate::tests::Gate)>,
        snapshot_failure_gate: Option<&crate::tests::Gate>,
    ) -> Self {
        let old = DocumentsRoot::new();
        let new = DocumentsRoot::new();
        let models = temp_directory();
        std::fs::create_dir_all(&models).unwrap();
        let settings_path = temp_directory().join("settings.json");
        let settings =
            SettingsService::open(Arc::new(JsonSettingsRepository::open(&settings_path))).unwrap();
        let mut value = settings.snapshot().unwrap();
        value.documents_root = old.path().display().to_string();
        settings.update(value).unwrap();
        let jobs = Arc::new(JobRuntime::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            )
            .with_job_runtime(jobs.clone()),
            DocumentService::new(match (read_gate, save_gates) {
                (Some(gate), _) => {
                    crate::document_tests::gated_read_repository(old.repository(), gate)
                }
                (_, Some((gate, _))) => crate::document_tests::gated_import_repository(
                    old.repository(),
                    gate,
                    snapshot_failure_gate.unwrap_or(&crate::tests::Gate::default()),
                ),
                _ => Arc::new(old.repository()),
            }),
        )
        .with_documents_root(application::DocumentsRoot::new(old.path(), &models, {
            let gate = save_gates.map(|(_, gate)| gate.clone());
            move |root: &Path| {
                FileDocumentRepository::open(root).map(|repository| match &gate {
                    Some(gate) => crate::document_tests::gated_save_repository(repository, gate),
                    None => Arc::new(repository) as Arc<dyn DocumentRepository>,
                })
            }
        }));
        Self {
            old,
            new,
            models,
            settings_path,
            settings,
            jobs,
            workspace,
        }
    }

    /// `documentsRoot` as `settings.json` holds it, read by a repository
    /// of its own.
    fn saved_documents_root(&self) -> String {
        SettingsService::open(Arc::new(JsonSettingsRepository::open(&self.settings_path)))
            .unwrap()
            .snapshot()
            .unwrap()
            .documents_root
    }

    fn root(&self) -> PathBuf {
        self.workspace.documents_root().unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.models).ok();
        if let Some(directory) = self.settings_path.parent() {
            std::fs::remove_dir_all(directory).ok();
        }
    }
}

/// The desktop on `fixture`'s workspace, on the Settings route once the
/// list has loaded. It follows the watcher of the root it switches to.
fn open_workspace<'a>(
    cx: &'a mut TestAppContext,
    fixture: &Workspace,
) -> (View, &'a mut VisualTestContext) {
    let mut services = desktop_services(
        Arc::new(FakeAssistantUiService::default()),
        fixture.settings.clone(),
    );
    services.workspace = fixture.workspace.clone();
    let (view, cx) = open_desktop_on(cx, services);
    view.update(cx, |root, cx| root.navigate(Route::Settings, cx));
    cx.run_until_parked();
    (view, cx)
}

fn put(root: &DocumentsRoot, id: &str, title: &str) {
    root.put(
        DocumentState::new(ProjectId::new(id), title, format!("{id}.wav")),
        &format!("# {title}\n"),
    );
}

fn ids(view: &View, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |root, _| {
        root.projects
            .iter()
            .map(|project| project.id().to_string())
            .collect()
    })
}

fn choose(cx: &mut VisualTestContext, view: &View, path: &Path) {
    let path = path.to_path_buf();
    view.update(cx, |root, cx| root.choose_documents_root(path, cx));
    wait_for(cx, view, |root| !root.documents_root_switch.switching);
    cx.run_until_parked();
}

fn switch_status(cx: &mut VisualTestContext, view: &View) -> Option<(String, bool)> {
    view.read_with(cx, |root, _| root.documents_root_switch.status.clone())
}

#[gpui::test]
fn workspace_search_discards_a_late_result_after_switching_roots(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    fixture.old.put(
        DocumentState::new(ProjectId::new("old"), "Old meeting", "old.wav"),
        "预算",
    );
    fixture.new.put(
        DocumentState::new(ProjectId::new("new"), "New meeting", "new.wav"),
        "预算",
    );
    let (view, cx) = open_workspace(cx, &fixture);
    cx.simulate_keystrokes("cmd-shift-f");
    let input = view.read_with(cx, |root, _| root.search.input.clone());
    focus_field(cx, &input);
    gate.arm();
    cx.simulate_input("预算");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    // The held repository read must leave UI interaction and switching usable.
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
    assert!(crate::tests::field_value(cx, &input).is_empty());
    gate.release();
    crate::document_tests::search(cx, &view, "预算");
    let found = view.read_with(cx, |root, _| {
        root.search
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .hits
            .clone()
    });
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].document_id.as_str(), "new");
    assert!(cx.debug_bounds("search-open-old").is_none());
}

#[gpui::test]
fn workspace_search_clearing_or_editing_invalidates_a_held_query(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    fixture.old.put(
        DocumentState::new(ProjectId::new("old"), "Review", "old.wav"),
        "预算",
    );
    let (view, cx) = open_workspace(cx, &fixture);
    cx.simulate_keystrokes("cmd-shift-f");
    let input = view.read_with(cx, |root, _| root.search.input.clone());
    focus_field(cx, &input);
    gate.arm();
    cx.simulate_input("预算");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    // Clearing while a real worker is held does not block the desktop.
    click_selector(cx, "workspace-search-clear");
    assert!(!view.read_with(cx, |root, _| root.search.pending));
    assert!(cx.debug_bounds("project-tree").is_some());
    gate.release();
    crate::document_tests::search(cx, &view, "不存在");
    assert!(cx.debug_bounds("workspace-search-empty").is_some());
    // A new draft immediately hides the previous results, until Enter.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("预算");
    assert!(view.read_with(cx, |root, _| root.search.result.is_none()));
    assert!(!view.read_with(cx, |root, _| root.search.pending));
}

#[gpui::test]
fn saved_markdown_discards_an_old_workspace_read_with_the_same_document_id(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    fixture.old.put(
        DocumentState::new(ProjectId::new("same"), "Old", "old.wav"),
        "Old workspace notes",
    );
    fixture.new.put(
        DocumentState::new(ProjectId::new("same"), "New", "new.wav"),
        "New workspace notes",
    );
    let (view, cx) = open_workspace(cx, &fixture);
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    gate.arm();
    click_selector(cx, "content-mode-markdown");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    choose(cx, &view, fixture.new.path());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    click_selector(cx, "content-mode-markdown");
    wait_for(cx, &view, |root| {
        root.saved_document
            .as_ref()
            .and_then(|saved| saved.outcome.as_ref())
            .is_some_and(|outcome| {
                outcome
                    .as_ref()
                    .is_ok_and(|record| record.markdown == "New workspace notes")
            })
    });
    gate.release();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |root, _| root
            .saved_document
            .as_ref()
            .unwrap()
            .outcome
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .markdown
            .clone()),
        "New workspace notes"
    );
}

/// Same IDs and revisions in both roots, with b sharing a's recording.
fn drag_documents(root: &DocumentsRoot, prefix: &str) {
    put(root, "a", &format!("{prefix} a"));
    let mut b = DocumentState::new(ProjectId::new("b"), format!("{prefix} b"), "a.wav");
    b.source.audio_asset_id = Some(ProjectId::new("a"));
    b.hierarchy.sibling_order = 1;
    root.put(b, &format!("# {prefix} b\n"));
    for id in ["a", "b"] {
        std::fs::write(root.path().join(id).join("attachment.txt"), prefix).unwrap();
    }
}

fn start_row_drag(cx: &mut VisualTestContext, id: &str) {
    let start = rendered(cx, crate::document_tests::leak(format!("drag-{id}"))).center();
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(
        start + gpui::point(gpui::px(6.), gpui::px(6.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    assert!(cx.update(|_, cx| cx.has_active_drag()));
}

fn finish_row_drag(cx: &mut VisualTestContext, view: &View, target: &'static str) {
    let target = rendered(cx, target).center();
    cx.simulate_mouse_move(target, gpui::MouseButton::Left, gpui::Modifiers::none());
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(view, move |_, _| observed.set(true)));
    cx.simulate_mouse_up(target, gpui::MouseButton::Left, gpui::Modifiers::none());
    wait_for(cx, view, |_| completed.get());
    assert!(!cx.update(|_, cx| cx.has_active_drag()));
}

fn a_drag_cannot_move_the_workspace_opened_after_it_started(
    cx: &mut TestAppContext,
    reparent: bool,
) {
    let fixture = Workspace::new();
    drag_documents(&fixture.old, "Old");
    drag_documents(&fixture.new, "New");
    let old_bytes = [
        fixture.old.package_bytes("a"),
        fixture.old.package_bytes("b"),
    ];
    let new_bytes = [
        fixture.new.package_bytes("a"),
        fixture.new.package_bytes("b"),
    ];
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    cx.simulate_keystrokes("cmd-2");
    cx.run_until_parked();
    start_row_drag(cx, "b");
    cx.simulate_keystrokes("cmd-,");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Settings);
    // The test platform has no folder prompt. This is the same callback
    // the native folder selection invokes, including settings persistence.
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New b")
            && !root.audio.pending()
    });
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
    assert!(
        cx.update(|_, cx| cx.has_active_drag()),
        "changing roots does not cancel GPUI's drag payload"
    );
    cx.simulate_keystrokes("cmd-2");
    cx.run_until_parked();
    finish_row_drag(
        cx,
        &view,
        if reparent {
            "project-row-a"
        } else {
            "before-a"
        },
    );
    assert_eq!(
        [
            fixture.old.package_bytes("a"),
            fixture.old.package_bytes("b")
        ],
        old_bytes
    );
    assert_eq!(
        [
            fixture.new.package_bytes("a"),
            fixture.new.package_bytes("b")
        ],
        new_bytes,
        "an old workspace's drag must not modify matching new documents"
    );
    for (root, prefix) in [(&fixture.old, "Old"), (&fixture.new, "New")] {
        for id in ["a", "b"] {
            assert_eq!(
                std::fs::read(root.path().join(id).join("attachment.txt")).unwrap(),
                prefix.as_bytes()
            );
        }
    }
    // A new keyboard gesture uses the current workspace and still works.
    select_row(cx, &view, "b");
    cx.simulate_keystrokes("alt-up");
    wait_for(cx, &view, |_| fixture.new.revision("b") == 1);
    assert_eq!(fixture.new.read("b").state.hierarchy.sibling_order, 0);
}

#[gpui::test]
fn an_old_workspace_drag_cannot_reorder_matching_new_documents(cx: &mut TestAppContext) {
    a_drag_cannot_move_the_workspace_opened_after_it_started(cx, false);
}

#[gpui::test]
fn an_old_workspace_drag_cannot_reparent_matching_new_documents(cx: &mut TestAppContext) {
    a_drag_cannot_move_the_workspace_opened_after_it_started(cx, true);
}

#[gpui::test]
fn a_drag_still_moves_its_workspace_after_an_ordinary_list_refresh(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    drag_documents(&fixture.old, "Old");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    cx.simulate_keystrokes("cmd-2");
    cx.run_until_parked();
    start_row_drag(cx, "b");
    let mut a = fixture.old.read("a").state;
    a.content.title = "Updated externally without moving the recording".into();
    fixture.old.put(a, "# Updated externally\n");
    view.update(cx, |root, cx| root.reload_projects(cx));
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "Updated externally without moving the recording")
            && !root.audio.pending()
    });
    assert!(cx.update(|_, cx| cx.has_active_drag()));
    finish_row_drag(cx, &view, "before-a");
    assert_eq!(fixture.old.read("b").state.hierarchy.sibling_order, 0);
    assert_eq!(fixture.old.read("a").state.hierarchy.sibling_order, 1);
    assert_eq!(fixture.old.read("a").markdown, "# Updated externally\n");
}

/// A same-root choice or a settings-conflict rollback retains the gesture's
/// original workspace, even though reads are restarted against its repository.
fn a_drag_survives_a_root_choice_that_keeps_its_workspace(
    cx: &mut TestAppContext,
    rollback: bool,
    reparent: bool,
) {
    let fixture = Workspace::new();
    drag_documents(&fixture.old, "Old");
    drag_documents(&fixture.new, "New");
    let untouched = [
        fixture.new.package_bytes("a"),
        fixture.new.package_bytes("b"),
    ];
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    cx.simulate_keystrokes("cmd-2");
    cx.run_until_parked();
    start_row_drag(cx, "b");
    cx.simulate_keystrokes("cmd-,");
    if rollback {
        // Another real settings writer makes the desktop's expected value
        // stale, so a successful root replacement must be rolled back.
        let mut external = fixture.settings.snapshot().unwrap();
        external.expected_speakers = Some(3);
        fixture.settings.update(external).unwrap();
    }
    choose(
        cx,
        &view,
        if rollback {
            fixture.new.path()
        } else {
            fixture.old.path()
        },
    );
    assert_eq!(switch_status(cx, &view).unwrap().1, rollback);
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.old.path().display().to_string()
    );
    wait_for(cx, &view, |root| {
        root.projects_loaded && !root.audio.pending()
    });
    assert!(cx.update(|_, cx| cx.has_active_drag()));
    cx.simulate_keystrokes("cmd-2");
    cx.run_until_parked();
    finish_row_drag(
        cx,
        &view,
        if reparent {
            "project-row-a"
        } else {
            "before-a"
        },
    );
    assert_eq!(fixture.old.revision("b"), 1);
    if reparent {
        assert_eq!(
            fixture.old.read("b").state.hierarchy.parent_id,
            Some(ProjectId::new("a"))
        );
    } else {
        assert_eq!(fixture.old.read("b").state.hierarchy.sibling_order, 0);
        assert_eq!(fixture.old.read("a").state.hierarchy.sibling_order, 1);
    }
    assert_eq!(
        [
            fixture.new.package_bytes("a"),
            fixture.new.package_bytes("b")
        ],
        untouched
    );
    for (root, prefix) in [(&fixture.old, "Old"), (&fixture.new, "New")] {
        for id in ["a", "b"] {
            assert_eq!(
                std::fs::read(root.path().join(id).join("attachment.txt")).unwrap(),
                prefix.as_bytes()
            );
        }
    }
}

#[gpui::test]
fn a_drag_can_reorder_after_choosing_its_current_root(cx: &mut TestAppContext) {
    a_drag_survives_a_root_choice_that_keeps_its_workspace(cx, false, false);
}

#[gpui::test]
fn a_drag_can_reparent_after_choosing_its_current_root(cx: &mut TestAppContext) {
    a_drag_survives_a_root_choice_that_keeps_its_workspace(cx, false, true);
}

#[gpui::test]
fn a_drag_can_reorder_after_a_root_switch_rolls_back(cx: &mut TestAppContext) {
    a_drag_survives_a_root_choice_that_keeps_its_workspace(cx, true, false);
}

#[gpui::test]
fn a_drag_can_reparent_after_a_root_switch_rolls_back(cx: &mut TestAppContext) {
    a_drag_survives_a_root_choice_that_keeps_its_workspace(cx, true, true);
}

#[gpui::test]
fn an_import_queued_before_a_host_root_switch_stays_in_its_original_workspace(
    cx: &mut TestAppContext,
) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let old_bytes = fixture.old.package_bytes("old-doc");
    let new_bytes = fixture.new.package_bytes("new-doc");
    let audio = fixture.models.join("Queued recording.wav");
    std::fs::write(&audio, AUDIO).unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| !root.audio.pending());

    view.update(cx, |root, cx| {
        root.import_audio_path(audio.clone(), cx);
        // The UI accepted this import but its async task has not been
        // polled. Replace the repository through the real Host API that
        // Settings calls, before the queued worker can execute its write.
        assert_eq!(
            fixture.old.repository().snapshot().unwrap().documents.len(),
            1
        );
        fixture
            .workspace
            .switch_documents_root(fixture.new.path())
            .unwrap();
    });
    wait_for(cx, &view, |_| {
        fixture.old.repository().snapshot().unwrap().documents.len()
            + fixture.new.repository().snapshot().unwrap().documents.len()
            == 3
    });
    let old = fixture.old.repository().snapshot().unwrap().documents;
    let new = fixture.new.repository().snapshot().unwrap().documents;
    assert_eq!(
        old.len(),
        2,
        "the accepted import belongs to the workspace open at admission"
    );
    assert_eq!(
        new.len(),
        1,
        "the queued worker must not import into the replacement root"
    );
    let imported = old
        .iter()
        .find(|document| document.id().as_str() != "old-doc")
        .unwrap();
    assert_eq!(imported.title(), "Queued recording");
    assert_eq!(imported.audio_path(), audio.display().to_string());
    assert!(fixture
        .old
        .read(imported.id().as_str())
        .markdown
        .contains("Queued recording.wav"));
    assert_eq!(fixture.old.package_bytes("old-doc"), old_bytes);
    assert_eq!(fixture.new.package_bytes("new-doc"), new_bytes);
    assert_eq!(std::fs::read(audio).unwrap(), AUDIO);
}

#[gpui::test]
fn returning_to_an_unloaded_old_document_defers_its_read_until_the_switch_finishes(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let mut fixture = Workspace::new();
    put(&fixture.old, "draft", "Keep this draft");
    put(&fixture.old, "shared", "Old");
    put(&fixture.new, "shared", "New");
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: gate.clone(),
    }))
    .unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "draft");
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("draft"))
    });
    let field = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep my unsaved wording");
    gate.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.new.path().into(), cx)
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.new.path());
    view.update(cx, |root, cx| {
        root.select_project(ProjectId::new("shared"), cx)
    });
    cx.run_until_parked();
    let deferred = view.read_with(cx, |root, _| {
        root.editor.loading.is_empty() && !root.editor.has_draft(&ProjectId::new("shared"))
    });
    // Selecting the already cached draft remains local while a different
    // repository is open on the worker.
    view.update(cx, |root, cx| {
        root.select_project(ProjectId::new("draft"), cx)
    });
    cx.run_until_parked();
    let preserved = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.title);
    gate.release();
    wait_for(cx, &view, |root| !root.documents_root_switch.switching);
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
    });
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("shared"))
    });
    assert!(
        deferred,
        "an old page must not start reading the new root during the switch"
    );
    assert_eq!(preserved, "Keep my unsaved wording");
    assert_eq!(
        view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.body),
        "# New\n"
    );
}

/// A started read has already read "Old". A switch that keeps the old
/// root must restart it from the files, without losing another draft.
fn a_switch_that_keeps_the_root_restarts_pending_reads(cx: &mut TestAppContext, busy: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    put(&fixture.old, "draft", "Draft");
    put(&fixture.old, "pending", "Old");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "draft");
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("draft"))
    });
    let field = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Retain this draft");
    gate.arm();
    view.update(cx, |root, cx| {
        root.select_project(ProjectId::new("pending"), cx)
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    put(&fixture.old, "pending", "Fresh from disk");
    let (job, _events) = fixture.jobs.create_job();
    if busy {
        job.started();
    } else {
        job.completed();
    }
    choose(
        cx,
        &view,
        if busy {
            fixture.new.path()
        } else {
            fixture.old.path()
        },
    );
    let status = switch_status(cx, &view).unwrap();
    gate.release();
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("pending")) && root.editor.loading.is_empty()
    });
    let read_body = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.body);
    select_row(cx, &view, "draft");
    assert_eq!(
        view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.title),
        "Retain this draft"
    );
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(status.1, busy);
    job.completed();
    assert_eq!(
        read_body, "# Fresh from disk\n",
        "a deferred read must resume on the unchanged root"
    );
}

#[gpui::test]
fn a_busy_refusal_restarts_the_old_root_read_without_losing_its_draft(cx: &mut TestAppContext) {
    a_switch_that_keeps_the_root_restarts_pending_reads(cx, true);
}

#[gpui::test]
fn choosing_the_current_root_restarts_its_read_without_losing_its_draft(cx: &mut TestAppContext) {
    a_switch_that_keeps_the_root_restarts_pending_reads(cx, false);
}

fn a_late_snapshot_stays_out_of_the_switching_workspace(cx: &mut TestAppContext, audio: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let snapshots = crate::tests::Gate::default();
    let settings_save = crate::tests::Gate::default();
    let mut fixture = Workspace::new();
    put(&fixture.old, "shared", "Old");
    put(&fixture.new, "shared", "New");
    fixture.workspace = WorkspaceService::new(
        fixture.workspace.processing(),
        DocumentService::new(Arc::new(HeldSnapshots {
            inner: fixture.old.repository(),
            gate: snapshots.clone(),
        })),
    )
    .with_documents_root(application::DocumentsRoot::new(
        fixture.old.path(),
        &fixture.models,
        |root: &Path| {
            FileDocumentRepository::open(root)
                .map(|repository| Arc::new(repository) as Arc<dyn DocumentRepository>)
        },
    ));
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: settings_save.clone(),
    }))
    .unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    let original_audio = view.read_with(cx, |root, _| {
        root.audio.result_for(&ProjectId::new("shared")).cloned()
    });
    let existing_audio = fixture.models.join("existing.wav");
    std::fs::write(&existing_audio, AUDIO).unwrap();
    let mut changed = fixture.old.read("shared").state;
    changed.content.title = "Changed after the displayed snapshot".into();
    changed.source.audio_path = existing_audio.display().to_string();
    fixture.old.put(changed, "# Changed externally\n");
    snapshots.arm();
    view.update(cx, |root, cx| {
        if audio {
            root.select_project(ProjectId::new("shared"), cx);
        } else {
            root.reload_projects(cx);
        }
    });
    wait_for(cx, &view, |_| snapshots.counts().0 == 1);
    settings_save.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.new.path().into(), cx)
    });
    wait_for(cx, &view, |_| settings_save.counts().0 == 1);
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    snapshots.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    let title_during_switch = view.read_with(cx, |root, _| root.projects[0].title().to_string());
    let audio_during_switch = view.read_with(cx, |root, _| {
        root.audio.result_for(&ProjectId::new("shared")).cloned()
    });
    settings_save.release();
    wait_for(cx, &view, |root| !root.documents_root_switch.switching);
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
            && !root.audio.pending()
    });
    assert_eq!(
        title_during_switch, "Old",
        "a held list must not replace the visible workspace during a switch"
    );
    assert_eq!(
        audio_during_switch, original_audio,
        "a held audio check must not replace the visible workspace during a switch"
    );
}

#[gpui::test]
fn a_late_list_snapshot_does_not_land_while_the_workspace_is_switching(cx: &mut TestAppContext) {
    a_late_snapshot_stays_out_of_the_switching_workspace(cx, false);
}

#[gpui::test]
fn a_late_audio_check_does_not_land_while_the_workspace_is_switching(cx: &mut TestAppContext) {
    a_late_snapshot_stays_out_of_the_switching_workspace(cx, true);
}

/// Waits, on the wall clock, for the file watcher to report a change.
#[track_caller]
fn wait_for_watcher(
    cx: &mut VisualTestContext,
    view: &View,
    done: impl Fn(&crate::controller::RootView) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        if view.read_with(cx, |root, _| done(root)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the watcher did not report the change");
}

#[gpui::test]
fn choosing_a_documents_root_shows_its_documents_saves_it_and_follows_its_changes(
    cx: &mut TestAppContext,
) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let (view, cx) = open_workspace(cx, &fixture);
    assert_eq!(ids(&view, cx), ["old-doc"]);
    // The Workspace section shows the current root beside its button.
    let section = rendered(cx, "workspace-section");
    assert!(within(rendered(cx, "documents-root-path"), section));
    assert!(within(rendered(cx, "choose-documents-root"), section));
    assert!(
        section.bottom() <= rendered(cx, "select-language").top(),
        "Workspace comes before Interface"
    );
    // An Assistant run on the old root left a mutation and a review.
    view.update(cx, |root, _| {
        root.assistant.result = Some(AssistantRunResult {
            mutations: vec![AssistantMutationSummary {
                mutation_id: "old-mutation".into(),
                affected_documents: Vec::new(),
                undo_token: "undo-old".into(),
                uncovered_segment_ids: Vec::new(),
            }],
            ..empty_result()
        });
        root.assistant.pending = Some(PendingReviewState::Awaiting(pending_review(
            "old-review",
            u64::MAX,
        )));
    });

    choose(cx, &view, fixture.new.path());
    let new_root = fixture.new.path().display().to_string();
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "new-doc")
    });
    assert_eq!(ids(&view, cx), ["new-doc"]);
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.selected_project.as_ref().map(ProjectId::as_str),
            Some("new-doc"),
            "the new root's first document is selected"
        );
        assert!(
            root.assistant.result.is_none() && root.assistant.pending.is_none(),
            "the old root's Assistant entries are cleared"
        );
        assert_eq!(root.settings_draft.documents_root, new_root);
        assert_eq!(
            root.documents_root_switch.status,
            Some((
                i18n::message(Locale::ZhCn, "documents_root_switched", &new_root),
                false
            ))
        );
    });
    rendered(cx, "documents-root-status");
    assert_eq!(fixture.saved_documents_root(), new_root);
    assert_eq!(fixture.root(), fixture.new.path());

    // Another writer adds a document to the new root; its watcher reports it.
    put(&fixture.new, "later-doc", "Later");
    wait_for_watcher(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "later-doc")
    });
}

#[gpui::test]
fn a_new_workspace_starts_at_the_top_even_with_the_same_document_id(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    for (root, title) in [
        (&fixture.old, "Old document"),
        (&fixture.new, "New document"),
    ] {
        let mut document = processed("shared", title);
        document.content.summary = Some(format!("Reading {title}.\n").repeat(120));
        root.put(document, "# Original body\n");
    }
    let old_bytes = fixture.old.package_bytes("shared");
    let new_bytes = fixture.new.package_bytes("shared");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        root.projects_loaded && !root.audio.pending()
    });
    click_selector(cx, "nav-Meetings");
    scroll_document(cx, &view);
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects.iter().any(|p| p.title() == "New document") && !root.audio.pending()
    });
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.meeting_content.offset()),
        Default::default(),
        "matching IDs in different workspaces do not share a reading position"
    );
    assert_eq!(fixture.old.package_bytes("shared"), old_bytes);
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
}

#[gpui::test]
fn switching_workspaces_clears_the_old_read_failure(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let (view, cx) = open_workspace(cx, &fixture);
    let repository = fixture.old.repository();
    let unreadable = UnreadableDocumentsRoot::new(&fixture.old);
    let error = repository.snapshot().unwrap_err();
    let expected = i18n::message(Locale::ZhCn, "documents_load_failed", &error);
    view.update(cx, |root, cx| {
        root.navigate(Route::Meetings, cx);
        root.reload_projects(cx);
    });
    wait_for(cx, &view, |root| root.ui.notice == expected);
    rendered(cx, "workspace-read-error");
    drop(unreadable);
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.id().as_str() == "new-doc")
    });
    cx.run_until_parked();
    assert_eq!(ids(&view, cx), ["new-doc"]);
    assert!(cx.debug_bounds("workspace-read-error").is_none());
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(ProjectId::new("new-doc"))
    );
    assert_eq!(fixture.root(), fixture.new.path());
}

fn late_document_read_keeps_the_new_workspace(cx: &mut TestAppContext, succeeds: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    put(&fixture.old, "shared", "Old");
    put(&fixture.new, "shared", "New");
    assert_eq!(
        fixture.old.revision("shared"),
        fixture.new.revision("shared")
    );
    let old_bytes = fixture.old.package_bytes("shared");
    let new_bytes = fixture.new.package_bytes("shared");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    // Fail the initial read, then hold the retry after the real repository
    // has produced its record or error, with no repository IO lock held.
    std::fs::remove_file(fixture.old.path().join("shared/state.json")).unwrap();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor
            .read_errors
            .contains_key(&ProjectId::new("shared"))
    });
    if succeeds {
        std::fs::write(fixture.old.path().join("shared/state.json"), &old_bytes[0]).unwrap();
    }
    gate.arm();
    click_selector(cx, "retry-document-read");
    wait_for(cx, &view, |_| gate.counts().0 == 1);

    // Use the actual Settings switch, including persistence and UI reset.
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
            && !root.audio.pending()
    });
    assert_eq!(fixture.root(), fixture.new.path());
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("shared"))
    });
    let title = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &title);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("New workspace draft");
    let draft = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits);
    assert_eq!(draft.body, "# New\n");
    let focus = cx.update(|window, cx| window.focused(cx));
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, cx| {
        assert!(
            root.editor.read_errors.is_empty(),
            "the old failure must not block the new root"
        );
        assert_eq!(
            root.editor.current(cx).unwrap().edits,
            draft,
            "the old record must not replace the new root's draft"
        );
        assert_eq!(root.ui.notice, notice);
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("shared")));
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
}

#[gpui::test]
fn a_late_document_read_record_does_not_replace_a_new_workspaces_draft(cx: &mut TestAppContext) {
    late_document_read_keeps_the_new_workspace(cx, true);
}

#[gpui::test]
fn a_late_document_read_error_does_not_block_a_new_workspaces_draft(cx: &mut TestAppContext) {
    late_document_read_keeps_the_new_workspace(cx, false);
}

#[gpui::test]
fn a_rolled_back_workspace_restarts_reads_and_keeps_its_existing_drafts(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_read_gate(Some(&gate));
    put(&fixture.old, "pending", "Old");
    put(&fixture.old, "draft", "Draft");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "draft");
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("draft"))
    });
    let title = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &title);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep this local draft");
    gate.arm();
    select_row(cx, &view, "pending");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    // The old read already has "Old"; the restored repository must read
    // the real file again instead of retaining that in-flight marker.
    put(&fixture.old, "pending", "Fresh");
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("pending"))
            && root.editor.loading.is_empty()
            && !root.audio.pending()
    });
    assert_eq!(
        view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.body),
        "# Fresh\n"
    );
    select_row(cx, &view, "draft");
    assert_eq!(
        view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.title),
        "Keep this local draft"
    );
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), notice);
    select_row(cx, &view, "pending");
    assert_eq!(
        view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits.body),
        "# Fresh\n"
    );
}

/// Edits the real document fields and starts their revision-checked save.
fn edit_and_save(cx: &mut VisualTestContext, view: &View, title: &str) {
    click_selector(cx, "content-mode-edit");
    wait_for(cx, view, |root| root.editor.shown().is_some());
    let field = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(title);
    click_selector(cx, "save-document");
}

fn late_save_keeps_the_new_workspace(cx: &mut TestAppContext, same_id: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let old_gate = crate::tests::Gate::default();
    let new_gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&old_gate, &new_gate);
    put(&fixture.old, "shared", "Old");
    // A different ID makes the old command's final read fail without
    // repository pinning; the same ID would return a different document.
    let new_id = if same_id { "shared" } else { "new" };
    put(&fixture.new, new_id, "New");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    old_gate.arm();
    edit_and_save(cx, &view, "Saved in old workspace");
    wait_for(cx, &view, |_| old_gate.counts().0 == 1);
    let old_bytes = fixture.old.package_bytes("shared");
    assert_eq!(
        fixture.old.read("shared").state.title(),
        "Saved in old workspace"
    );

    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
            && !root.audio.pending()
    });
    assert_eq!(fixture.root(), fixture.new.path());
    assert!(
        view.read_with(cx, |root, _| root.editor.saving.is_none()),
        "a save in the previous workspace must not disable this workspace's save"
    );
    new_gate.arm();
    edit_and_save(cx, &view, "Saved in new workspace");
    wait_for(cx, &view, |_| new_gate.counts().0 == 1);
    // Settle the new root's real watcher before observing the old callback.
    wait_for_watcher(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "Saved in new workspace")
            && !root.audio.pending()
    });
    let draft = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap());
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let focus = cx.update(|window, cx| window.focused(cx));
    let new_bytes = fixture.new.package_bytes(new_id);
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    old_gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, cx| {
        assert_eq!(
            root.editor.saving,
            Some(ProjectId::new(new_id)),
            "the old completion must not finish the new save"
        );
        let current = root.editor.current(cx).unwrap();
        assert_eq!(current.revision, draft.revision);
        assert_eq!(current.base, draft.base);
        assert_eq!(current.edits, draft.edits);
        assert_eq!(root.ui.notice, notice);
        assert!(root.document_conflict.is_none());
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(fixture.old.package_bytes("shared"), old_bytes);
    assert_eq!(fixture.new.package_bytes(new_id), new_bytes);
    new_gate.release();
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    view.read_with(cx, |root, cx| {
        let current = root.editor.current(cx).unwrap();
        assert_eq!(current.revision, fixture.new.revision(new_id));
        assert_eq!(current.base.title, "Saved in new workspace");
        assert_eq!(current.edits, draft.edits);
        assert_eq!(
            root.ui.notice,
            i18n::message(root.ui.locale, "document_saved", "Saved in new workspace")
        );
    });
}

#[gpui::test]
fn a_new_workspace_can_save_while_the_old_save_finishes(cx: &mut TestAppContext) {
    late_save_keeps_the_new_workspace(cx, true);
}

#[gpui::test]
fn an_old_save_cannot_finish_a_different_documents_new_workspace_save(cx: &mut TestAppContext) {
    late_save_keeps_the_new_workspace(cx, false);
}

#[gpui::test]
fn a_save_in_a_rolled_back_workspace_still_rebases_its_draft(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&gate, &crate::tests::Gate::default());
    put(&fixture.old, "shared", "Old");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    gate.arm();
    edit_and_save(cx, &view, "Saved before rollback");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let saved_revision = fixture.old.revision("shared");
    let field = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep typing during save");
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(
        view.read_with(cx, |root, _| root.editor.saving.clone()),
        Some(ProjectId::new("shared"))
    );
    gate.release();
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    view.read_with(cx, |root, cx| {
        let current = root.editor.current(cx).unwrap();
        assert_eq!(current.revision, saved_revision);
        assert_eq!(current.base.title, "Saved before rollback");
        assert_eq!(current.edits.title, "Keep typing during save");
    });
    click_selector(cx, "save-document");
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    assert_eq!(
        fixture.old.read("shared").state.title(),
        "Keep typing during save"
    );
    assert_eq!(fixture.old.revision("shared"), saved_revision + 1);
    assert!(view.read_with(cx, |root, _| root.document_conflict.is_none()));
}

fn rename_document_speaker(cx: &mut VisualTestContext, name: &str) {
    click_selector(cx, "speaker-name-speaker-1");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(name);
    click_selector(cx, "rename-speaker-speaker-1");
}

fn late_speaker_command_keeps_the_new_workspace(cx: &mut TestAppContext, merge: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let old_gate = crate::tests::Gate::default();
    let new_gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&old_gate, &new_gate);
    fixture
        .old
        .put(crate::speaker_tests::two_speakers("shared"), "# Old\n");
    fixture
        .new
        .put(crate::speaker_tests::two_speakers("shared"), "# New\n");
    let initial_revision = fixture.old.revision("shared");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    click_selector(cx, "content-mode-original");
    if cx.debug_bounds("speaker-name-speaker-1").is_none() {
        click_selector(cx, "document-speakers-toggle");
    }
    old_gate.arm();
    if merge {
        click_selector(cx, "merge-speaker-speaker-1");
        click_selector(cx, "select-option-0");
        click_selector(cx, "dialog-confirm");
    } else {
        rename_document_speaker(cx, "Old name");
    }
    wait_for(cx, &view, |_| old_gate.counts().0 == 1);
    let old_bytes = fixture.old.package_bytes("shared");
    assert_eq!(fixture.old.revision("shared"), initial_revision + 1);
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        !root.projects.is_empty() && !root.audio.pending()
    });
    assert!(
        view.read_with(cx, |root, _| root.speaker_command.is_none()),
        "a previous workspace's correction must not disable the new workspace"
    );
    click_selector(cx, "content-mode-original");
    if cx.debug_bounds("speaker-name-speaker-1").is_none() {
        click_selector(cx, "document-speakers-toggle");
    }
    new_gate.arm();
    rename_document_speaker(cx, "New name");
    wait_for(cx, &view, |_| new_gate.counts().0 == 1);
    // Settle the actual watcher before observing only the old completion.
    wait_for_watcher(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.revision() == initial_revision + 1)
            && !root.audio.pending()
    });
    let new_bytes = fixture.new.package_bytes("shared");
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let focus = cx.update(|window, cx| window.focused(cx));
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    old_gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.speaker_command,
            Some(ProjectId::new("shared")),
            "the old correction must not complete the new correction"
        );
        assert_eq!(root.ui.notice, notice);
        assert!(root.document_conflict.is_none());
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("shared")));
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(fixture.old.package_bytes("shared"), old_bytes);
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
    new_gate.release();
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            i18n::message(root.ui.locale, "speaker_renamed", "New name")
        );
        assert!(root.document_conflict.is_none());
    });
    assert!(fixture
        .new
        .read("shared")
        .state
        .results
        .diarization
        .iter()
        .any(|turn| {
            turn.speaker_id == "speaker-1" && turn.speaker_label.as_deref() == Some("New name")
        }));
}

#[gpui::test]
fn a_late_speaker_rename_cannot_complete_a_new_workspaces_correction(cx: &mut TestAppContext) {
    late_speaker_command_keeps_the_new_workspace(cx, false);
}

#[gpui::test]
fn a_late_speaker_merge_cannot_complete_a_new_workspaces_correction(cx: &mut TestAppContext) {
    late_speaker_command_keeps_the_new_workspace(cx, true);
}

#[gpui::test]
fn a_speaker_correction_in_a_rolled_back_workspace_still_completes(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&gate, &crate::tests::Gate::default());
    fixture
        .old
        .put(crate::speaker_tests::two_speakers("shared"), "# Old\n");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    click_selector(cx, "content-mode-original");
    if cx.debug_bounds("speaker-name-speaker-1").is_none() {
        click_selector(cx, "document-speakers-toggle");
    }
    gate.arm();
    rename_document_speaker(cx, "Still saved");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let saved = fixture.old.package_bytes("shared");
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(
        view.read_with(cx, |root, _| root.speaker_command.clone()),
        Some(ProjectId::new("shared"))
    );
    gate.release();
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    assert_eq!(fixture.old.package_bytes("shared"), saved);
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            i18n::message(root.ui.locale, "speaker_renamed", "Still saved")
        );
        assert!(root.document_conflict.is_none());
    });
}

#[gpui::test]
fn a_late_tree_move_keeps_the_new_workspaces_tree_and_editor(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&gate, &crate::tests::Gate::default());
    for (documents, prefix) in [(&fixture.old, "Old"), (&fixture.new, "New")] {
        put(documents, "a", &format!("{prefix} a"));
        let mut second = DocumentState::new(ProjectId::new("b"), format!("{prefix} b"), "a.wav");
        second.source.audio_asset_id = Some(ProjectId::new("a"));
        second.hierarchy.sibling_order = 1;
        documents.put(second, &format!("# {prefix} b\n"));
    }
    let new_bytes = [
        fixture.new.package_bytes("a"),
        fixture.new.package_bytes("b"),
    ];
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "b");
    gate.arm();
    // Option-Right moves b under a through the real tree command binding.
    cx.simulate_keystrokes("alt-right");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(
        fixture.old.read("b").state.hierarchy.parent_id,
        Some(ProjectId::new("a"))
    );
    let old_bytes = [
        fixture.old.package_bytes("a"),
        fixture.old.package_bytes("b"),
    ];

    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New b")
            && !root.audio.pending()
    });
    select_row(cx, &view, "b");
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("b"))
    });
    let title = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &title);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep the new workspace draft");
    let draft = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap().edits);
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let expanded = view.read_with(cx, |root, _| root.ui.expanded_projects.clone());
    let focus = cx.update(|window, cx| window.focused(cx));
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, cx| {
        assert_eq!(
            root.ui.expanded_projects, expanded,
            "the old keyboard move must not open its parent in the new workspace"
        );
        assert_eq!(root.ui.notice, notice);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("b")));
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.editor.current(cx).unwrap().edits, draft);
        assert!(root.document_conflict.is_none());
        assert!(root
            .projects
            .iter()
            .all(|document| document.hierarchy.parent_id.is_none()));
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        [
            fixture.old.package_bytes("a"),
            fixture.old.package_bytes("b")
        ],
        old_bytes
    );
    assert_eq!(
        [
            fixture.new.package_bytes("a"),
            fixture.new.package_bytes("b")
        ],
        new_bytes
    );
}

fn late_import_keeps_the_new_workspace(cx: &mut TestAppContext, snapshot_fails: bool) {
    let _alone = crate::tests::hold_a_worker_alone();
    let save_gate = crate::tests::Gate::default();
    let snapshot_gate = crate::tests::Gate::default();
    let fixture = Workspace::with_import_gates(&save_gate, &snapshot_gate);
    put(&fixture.old, "shared", "Old");
    put(&fixture.new, "shared", "New");
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("arriving.wav");
    std::fs::write(&audio, AUDIO).unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    save_gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| save_gate.counts().0 == 1);
    let imported = fixture
        .old
        .repository()
        .snapshot()
        .unwrap()
        .documents
        .into_iter()
        .find(|document| document.title() == "arriving")
        .unwrap();
    let imported_bytes = fixture.old.package_bytes(imported.id().as_str());
    assert_eq!(imported.audio_path(), audio.display().to_string());
    // Capture a real failure from the original root before changing roots,
    // then restore its packages while the result waits outside repository IO.
    let completion_gate = if snapshot_fails {
        let unreadable = UnreadableDocumentsRoot::new(&fixture.old);
        snapshot_gate.arm();
        save_gate.release();
        wait_for(cx, &view, |_| snapshot_gate.counts().0 == 1);
        drop(unreadable);
        &snapshot_gate
    } else {
        &save_gate
    };
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
            && !root.audio.pending()
    });
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| {
        root.editor.shown() == Some(&ProjectId::new("shared"))
    });
    let title = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &title);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("New workspace draft");
    let draft = view.read_with(cx, |root, cx| root.editor.current(cx).unwrap());
    let new_bytes = fixture.new.package_bytes("shared");
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let toast = view.read_with(cx, |root, _| {
        root.ui.toast().map(|toast| toast.notice.clone())
    });
    let focus = cx.update(|window, cx| window.focused(cx));
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    completion_gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, cx| {
        assert_eq!(
            root.ui.notice, notice,
            "the old import must not replace the new workspace's notice"
        );
        assert_eq!(root.ui.toast().map(|toast| toast.notice.clone()), toast);
        assert_eq!(root.editor.current(cx).unwrap().edits, draft.edits);
        assert_eq!(root.editor.current(cx).unwrap().base, draft.base);
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("shared")));
        assert_eq!(root.projects.len(), 1);
        assert!(root.projects_error.is_none());
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
    assert_eq!(
        fixture.old.package_bytes(imported.id().as_str()),
        imported_bytes
    );
}

#[gpui::test]
fn a_late_import_success_keeps_the_new_workspaces_notice_and_draft(cx: &mut TestAppContext) {
    late_import_keeps_the_new_workspace(cx, false);
}

#[gpui::test]
fn a_late_import_snapshot_failure_keeps_the_new_workspaces_notice_and_draft(
    cx: &mut TestAppContext,
) {
    late_import_keeps_the_new_workspace(cx, true);
}

#[gpui::test]
fn an_import_in_a_rolled_back_workspace_still_reports_its_completion(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&gate, &crate::tests::Gate::default());
    put(&fixture.old, "shared", "Old");
    let audio_files = DocumentsRoot::new();
    let audio = audio_files.path().join("arriving.wav");
    std::fs::write(&audio, AUDIO).unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    gate.arm();
    view.update(cx, |root, cx| root.import_audio_path(audio.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    gate.release();
    wait_for(cx, &view, |root| {
        root.ui.notice == i18n::message(root.ui.locale, "imported", "arriving")
    });
    assert_eq!(
        fixture.old.repository().snapshot().unwrap().documents.len(),
        2
    );
    assert!(fixture
        .new
        .repository()
        .snapshot()
        .unwrap()
        .documents
        .is_empty());
}

fn assistant_switch_fixture(gate: &crate::tests::Gate) -> Workspace {
    let mut fixture = Workspace::new();
    put(&fixture.old, "old", "Old");
    put(&fixture.new, "new", "New");
    let mut saved = fixture.settings.snapshot().unwrap();
    saved.assistant = crate::tests::stored_key_settings()
        .snapshot()
        .unwrap()
        .assistant;
    fixture.settings.update(saved).unwrap();
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: gate.clone(),
    }))
    .unwrap();
    fixture
}

fn open_assistant_workspace<'a>(
    cx: &'a mut TestAppContext,
    fixture: &Workspace,
    assistant: Arc<FakeAssistantUiService>,
) -> (View, &'a mut VisualTestContext) {
    let mut services = desktop_services(assistant, fixture.settings.clone());
    services.workspace = fixture.workspace.clone();
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Assistant, cx));
    cx.run_until_parked();
    (view, cx)
}

fn send_workspace_prompt(cx: &mut VisualTestContext, view: &View, prompt: &str) {
    let field = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(prompt);
    click_selector(cx, "assistant-submit");
}

fn hold_workspace_switch(
    cx: &mut VisualTestContext,
    view: &View,
    fixture: &Workspace,
    gate: &crate::tests::Gate,
) {
    gate.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.new.path().into(), cx)
    });
    wait_for(cx, view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.new.path());
    view.read_with(cx, |root, _| {
        assert!(root.documents_root_switch.switching);
        assert_eq!(root.projects[0].id().as_str(), "old");
    });
}

/// The UI accepts these actions before their worker can register a shared
/// job. The fake Assistant only observes admission/dispatch; the actual
/// directory switch and settings write use the fixture's real files.
#[gpui::test]
fn an_accepted_assistant_run_keeps_its_workspace_until_its_failure_returns(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&crate::tests::Gate::default());
    let assistant = Arc::new(FakeAssistantUiService {
        run_gate: Some(gate.clone()),
        ..Default::default()
    });
    assistant
        .run_errors
        .lock()
        .unwrap()
        .push_back(crate::assistant_ui::AssistantUiError {
            code: "PROVIDER_TIMEOUT".into(),
            message: "timed out".into(),
            retryable: true,
        });
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    let field = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    focus_field(cx, &field);
    cx.simulate_input("keep this task on the old workspace");
    let settings_before = std::fs::read(&fixture.settings_path).unwrap();
    gate.arm();
    view.update_in(cx, |root, window, cx| {
        root.assistant_action(&gpui::ClickEvent::default(), window, cx);
        assert!(
            root.assistant_busy(),
            "the real UI entry must accept the run"
        );
        assert!(fixture.jobs.list().unwrap().is_empty());
        root.choose_documents_root(fixture.new.path().into(), cx);
        assert!(
            !root.documents_root_switch.switching,
            "an accepted Assistant run must keep its workspace before worker registration"
        );
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    view.update(cx, |root, cx| root.navigate(Route::Settings, cx));
    cx.run_until_parked();
    assert!(!cx.cx.did_prompt_for_paths());
    click_selector(cx, "choose-documents-root");
    assert!(
        !cx.cx.did_prompt_for_paths(),
        "the busy button stays disabled"
    );
    view.update_in(cx, |root, window, cx| {
        root.choose_documents_root_directory(&gpui::ClickEvent::default(), window, cx)
    });
    assert!(
        !cx.cx.did_prompt_for_paths(),
        "a stale activation opens no chooser"
    );
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(
        std::fs::read(&fixture.settings_path).unwrap(),
        settings_before
    );
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            i18n::text(root.ui.locale, "documents_root_operation_pending")
        );
    });
    gate.release();
    wait_for(cx, &view, |root| !root.assistant_busy());
    // Cancellation is also still in flight until its outcome arrives.
    assistant
        .run_errors
        .lock()
        .unwrap()
        .push_back(crate::assistant_ui::AssistantUiError {
            code: "TASK_CANCELLED".into(),
            message: "cancelled".into(),
            retryable: true,
        });
    gate.arm();
    view.update_in(cx, |root, window, cx| {
        root.retry_assistant_run(&gpui::ClickEvent::default(), window, cx);
        assert!(root.assistant_busy());
        root.cancel_assistant(&gpui::ClickEvent::default(), window, cx);
        assert!(matches!(
            root.assistant.task,
            crate::assistant_ui::AssistantTaskState::Cancelling { .. }
        ));
        root.choose_documents_root(fixture.new.path().into(), cx);
        assert!(!root.documents_root_switch.switching);
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    gate.release();
    wait_for(cx, &view, |root| !root.assistant_busy());
    assert!(view.read_with(cx, |root, _| root.assistant.task
        == crate::assistant_ui::AssistantTaskState::Cancelled));
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
}

#[gpui::test]
fn an_accepted_assistant_confirmation_keeps_its_workspace_until_it_returns(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&crate::tests::Gate::default());
    let assistant = Arc::new(FakeAssistantUiService {
        decision_gate: Some(gate.clone()),
        ..Default::default()
    });
    *assistant.run_result.lock().unwrap() = Some(review_with_undo());
    *assistant.confirm_result.lock().unwrap() = Some(empty_result());
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    send_workspace_prompt(cx, &view, "review");
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    assert!(!view.read_with(cx, |root, _| root.assistant_busy()));
    gate.arm();
    view.update_in(cx, |root, window, cx| {
        root.accept_assistant_pending(&gpui::ClickEvent::default(), window, cx);
        assert!(root.assistant.decision_in_flight());
        assert!(fixture.jobs.list().unwrap().is_empty());
        root.choose_documents_root(fixture.new.path().into(), cx);
        assert!(
            !root.documents_root_switch.switching,
            "an accepted confirmation must keep its workspace before worker dispatch"
        );
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.old.path());
    gate.release();
    wait_for(cx, &view, |root| !root.assistant_busy());
    assert_eq!(assistant.grants_for("confirm"), [vec!["old".to_string()]]);
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
}

#[gpui::test]
fn an_accepted_assistant_undo_keeps_its_workspace_until_it_returns(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&crate::tests::Gate::default());
    let assistant = Arc::new(FakeAssistantUiService {
        decision_gate: Some(gate.clone()),
        ..Default::default()
    });
    *assistant.run_result.lock().unwrap() = Some(review_with_undo());
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    send_workspace_prompt(cx, &view, "review");
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    gate.arm();
    view.update(cx, |root, cx| {
        root.undo_assistant_mutation_for("applied".into(), cx);
        assert!(root.assistant.is_mutation_undo_in_flight("applied"));
        assert!(fixture.jobs.list().unwrap().is_empty());
        root.choose_documents_root(fixture.new.path().into(), cx);
        assert!(
            !root.documents_root_switch.switching,
            "an accepted undo must keep its workspace before worker dispatch"
        );
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.old.path());
    gate.release();
    wait_for(cx, &view, |root| !root.assistant_busy());
    assert!(view.read_with(cx, |root, _| root.assistant.is_mutation_undone("applied")));
    assert_eq!(assistant.grants_for("undo"), [vec!["old".to_string()]]);
    // An awaiting review itself does not block a directory choice.
    assert!(matches!(
        view.read_with(cx, |root, _| root.assistant.pending.clone()),
        Some(PendingReviewState::Awaiting(_))
    ));
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
}

#[gpui::test]
fn accepted_processing_keeps_its_workspace_before_the_start_worker_runs(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    let audio = fixture.old.path().join("source.wav");
    std::fs::write(&audio, AUDIO).unwrap();
    fixture.old.put(
        DocumentState::new(ProjectId::new("old"), "Old", audio.display().to_string()),
        "# Old\n",
    );
    put(&fixture.new, "new", "New");
    // Custom model references pass the UI catalog check and reach the real
    // backend preflight on the worker, which reports their missing files.
    let mut settings = fixture.settings.snapshot().unwrap();
    let missing = |backend_id: &str| contracts::ModelRef {
        backend_id: backend_id.into(),
        model_id: "fixture".into(),
        revision: None,
        artifact_path: fixture
            .models
            .join(format!("unlisted-{backend_id}"))
            .display()
            .to_string(),
        artifacts: Vec::new(),
    };
    settings.asr_backend_id = Some("whisper-rs".into());
    settings.asr_model = Some(missing("whisper-rs"));
    settings.diarization_backend_id = Some("sherpa-onnx".into());
    settings.diarization_model = Some(missing("sherpa-onnx"));
    fixture.settings.update(settings).unwrap();
    let new_before = fixture.new.package_bytes("new");
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update_in(cx, |root, window, cx| {
        root.process_selected(&gpui::ClickEvent::default(), window, cx);
        assert!(
            root.ui.processing,
            "the real UI entry must accept processing"
        );
        assert!(
            fixture.jobs.list().unwrap().is_empty(),
            "the host worker has not registered a job"
        );
        root.choose_documents_root(fixture.new.path().into(), cx);
        assert!(
            !root.documents_root_switch.switching,
            "accepted processing must keep its workspace before worker registration"
        );
    });
    wait_for(cx, &view, |root| !root.ui.processing);
    assert_eq!(fixture.root(), fixture.old.path());
    let failed = fixture.old.read("old");
    assert_eq!(failed.state.title(), "Old");
    assert_eq!(failed.markdown, "# Old\n");
    assert_eq!(failed.state.source.audio_path, audio.display().to_string());
    assert!(failed.state.results.transcript.is_empty());
    assert!(failed
        .state
        .processing
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("model file does not exist")));
    assert_eq!(fixture.new.package_bytes("new"), new_before);
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
}

#[gpui::test]
fn switching_workspaces_blocks_assistant_enter_send_and_retry(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&gate);
    let assistant = Arc::new(FakeAssistantUiService::default());
    assistant
        .run_errors
        .lock()
        .unwrap()
        .push_back(crate::assistant_ui::AssistantUiError {
            code: "PROVIDER_TIMEOUT".into(),
            message: "timed out".into(),
            retryable: true,
        });
    *assistant.run_result.lock().unwrap() = Some(empty_result());
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    send_workspace_prompt(cx, &view, "old prompt");
    wait_for(cx, &view, |root| {
        matches!(
            root.assistant.task,
            crate::assistant_ui::AssistantTaskState::Failed(_)
        )
    });
    let before = assistant.run_request_ids.lock().unwrap().len();
    hold_workspace_switch(cx, &view, &fixture, &gate);
    let field = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    focus_field(cx, &field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("keep this next prompt");
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |root, _| {
        assert!(
            matches!(
                root.assistant.task,
                crate::assistant_ui::AssistantTaskState::Failed(_)
            ),
            "Enter must not replace the old result during a pending switch"
        );
    });
    click_selector(cx, "assistant-submit");
    click_selector(cx, "assistant-retry");
    // Exercise the controller entry too, independently of disabled buttons.
    view.update_in(cx, |root, window, cx| {
        root.retry_assistant_run(&gpui::ClickEvent::default(), window, cx)
    });
    cx.run_until_parked();
    let after = assistant.run_request_ids.lock().unwrap().len();
    view.read_with(cx, |root, cx| {
        assert_eq!(
            root.assistant_inputs.prompt.read(cx).value(cx).as_str(),
            "keep this next prompt"
        );
        assert_eq!(root.ui.notice, notice);
        assert!(matches!(
            root.assistant.task,
            crate::assistant_ui::AssistantTaskState::Failed(_)
        ));
    });
    gate.release();
    wait_for(cx, &view, |root| {
        !root.documents_root_switch.switching
            && root.projects.iter().any(|d| d.id().as_str() == "new")
    });
    assert_eq!(
        after, before,
        "the pending switch must not dispatch another Assistant request"
    );
    send_workspace_prompt(cx, &view, "new prompt");
    wait_for(cx, &view, |root| {
        root.assistant.task == crate::assistant_ui::AssistantTaskState::Completed
    });
    assert_eq!(assistant.grants_for("run"), [vec!["new".to_string()]]);
}

fn review_with_undo() -> AssistantRunResult {
    let mut pending = pending_review("review", u64::MAX);
    pending.affected_document_ids = vec!["old".into()];
    AssistantRunResult {
        pending_reviews: vec![pending],
        mutation: Some(AssistantMutationSummary {
            mutation_id: "applied".into(),
            affected_documents: Vec::new(),
            undo_token: "undo-applied".into(),
            uncovered_segment_ids: Vec::new(),
        }),
        ..empty_result()
    }
}

#[gpui::test]
fn switching_workspaces_blocks_assistant_writes_and_keeps_rejection_available(
    cx: &mut TestAppContext,
) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&gate);
    let assistant = Arc::new(FakeAssistantUiService::default());
    *assistant.run_result.lock().unwrap() = Some(review_with_undo());
    *assistant.confirm_result.lock().unwrap() = Some(empty_result());
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    send_workspace_prompt(cx, &view, "old review");
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    hold_workspace_switch(cx, &view, &fixture, &gate);
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    click_selector(cx, "assistant-confirm");
    click_selector(cx, "assistant-undo-applied");
    view.update_in(cx, |root, window, cx| {
        root.accept_assistant_pending(&gpui::ClickEvent::default(), window, cx);
        root.undo_assistant_mutation_for("applied".into(), cx);
    });
    cx.run_until_parked();
    let confirm_calls = assistant.grants_for("confirm");
    let undo_calls = assistant.grants_for("undo");
    view.read_with(cx, |root, _| {
        assert!(matches!(
            root.assistant.pending,
            Some(PendingReviewState::Awaiting(_))
        ));
        assert!(!root.assistant.is_mutation_undone("applied"));
        assert!(!root.assistant.is_mutation_undo_in_flight("applied"));
        assert_eq!(root.ui.notice, notice);
    });
    click_selector(cx, "assistant-reject");
    wait_for(cx, &view, |root| {
        root.assistant.pending == Some(PendingReviewState::Rejected)
    });
    assert_eq!(assistant.grants_for("reject"), [vec!["old".to_string()]]);
    gate.release();
    wait_for(cx, &view, |root| !root.documents_root_switch.switching);
    assert!(
        confirm_calls.is_empty(),
        "the old review grant must not reach confirm during the switch"
    );
    assert!(
        undo_calls.is_empty(),
        "the old mutation grant must not reach undo during the switch"
    );
}

#[gpui::test]
fn assistant_review_and_undo_resume_after_a_workspace_switch_rolls_back(cx: &mut TestAppContext) {
    let gate = crate::tests::Gate::default();
    let fixture = assistant_switch_fixture(&gate);
    let assistant = Arc::new(FakeAssistantUiService::default());
    *assistant.run_result.lock().unwrap() = Some(review_with_undo());
    *assistant.confirm_result.lock().unwrap() = Some(empty_result());
    let (view, cx) = open_assistant_workspace(cx, &fixture, assistant.clone());
    send_workspace_prompt(cx, &view, "old review");
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    click_selector(cx, "assistant-confirm");
    wait_for(cx, &view, |root| !root.assistant.decision_in_flight());
    click_selector(cx, "assistant-undo-applied");
    wait_for(cx, &view, |root| {
        root.assistant.is_mutation_undone("applied")
    });
    assert_eq!(assistant.grants_for("confirm"), [vec!["old".to_string()]]);
    assert_eq!(assistant.grants_for("undo"), [vec!["old".to_string()]]);
}

#[gpui::test]
fn an_old_edit_cannot_be_saved_while_the_workspace_switch_is_returning(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let mut fixture = Workspace::new();
    put(&fixture.old, "shared", "Old");
    put(&fixture.new, "shared", "New");
    let old_bytes = fixture.old.package_bytes("shared");
    let new_bytes = fixture.new.package_bytes("shared");
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: gate.clone(),
    }))
    .unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    click_selector(cx, "content-mode-edit");
    wait_for(cx, &view, |root| root.editor.shown().is_some());
    let title = view.read_with(cx, |root, _| root.editor.title.clone());
    focus_field(cx, &title);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep this old workspace draft");
    gate.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.new.path().into(), cx)
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.new.path());
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
    view.read_with(cx, |root, cx| {
        assert!(root.documents_root_switch.switching);
        assert_eq!(root.projects[0].title(), "Old");
        assert_eq!(
            root.editor.current(cx).unwrap().edits.title,
            "Keep this old workspace draft"
        );
    });
    // Return to the still-visible old page while the real switch result
    // is pending; the same-ID new package must not accept its old patch.
    cx.simulate_keystrokes("cmd-2");
    click_selector(cx, "save-document");
    // An activation queued before the disabled render also reaches this
    // entry point, so admission cannot rely on the button alone.
    let saving = view.update_in(cx, |root, window, cx| {
        root.save_document_draft(&gpui::ClickEvent::default(), window, cx);
        root.editor.saving.clone()
    });
    assert!(saving.is_none());
    wait_for(cx, &view, |root| root.editor.saving.is_none());
    let after_attempt = fixture.new.package_bytes("shared");
    gate.release();
    wait_for(cx, &view, |root| !root.documents_root_switch.switching);
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.id().as_str() == "shared")
            && !root.audio.pending()
    });
    assert_eq!(fixture.old.package_bytes("shared"), old_bytes);
    assert!(
        after_attempt == new_bytes,
        "the old edit must not write to the new root during a pending switch"
    );
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
    assert_eq!(fixture.new.read("shared").state.title(), "New");
}

#[gpui::test]
fn a_pending_root_choice_keeps_speaker_input_and_resumes_actions(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let mut fixture = Workspace::new();
    fixture
        .old
        .put(crate::speaker_tests::two_speakers("shared"), "# Old\n");
    let before = fixture.old.package_bytes("shared");
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: gate.clone(),
    }))
    .unwrap();
    let replacement = fixture.models.join("replacement.wav");
    std::fs::write(&replacement, AUDIO).unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    click_selector(cx, "content-mode-original");
    if cx.debug_bounds("speaker-name-speaker-1").is_none() {
        click_selector(cx, "document-speakers-toggle");
    }
    click_selector(cx, "speaker-name-speaker-1");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Keep this name draft");
    let focus = cx.update(|window, cx| window.focused(cx));
    gate.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.old.path().into(), cx)
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    // Text input stays usable, but Enter must not dispatch its old record
    // while the root choice has not finished returning to the UI.
    cx.simulate_keystrokes("enter");
    let speaker_pending = view.read_with(cx, |root, _| root.speaker_command.clone());
    let relink_pending = view.update(cx, |root, cx| {
        root.relink_audio_path(replacement.clone(), cx);
        root.audio.relinking.clone()
    });
    let merge_pending = view.update_in(cx, |root, window, cx| {
        root.ask_merge_document_speaker("speaker-1".into(), "speaker-0".into(), window, cx);
        root.speaker_confirmation.is_some()
    });
    let during = fixture.old.package_bytes("shared");
    let focus_during = cx.update(|window, cx| window.focused(cx));
    gate.release();
    wait_for(cx, &view, |root| {
        !root.documents_root_switch.switching && !root.audio.pending()
    });
    assert!(
        speaker_pending.is_none(),
        "Enter must not start a speaker correction"
    );
    assert!(
        relink_pending.is_none(),
        "a queued file choice must not start a relink"
    );
    assert!(
        !merge_pending,
        "a queued merge choice must not open a review dialog"
    );
    assert_eq!(focus_during, focus);
    assert!(
        during == before,
        "a pending root choice must keep package bytes"
    );
    assert_eq!(fixture.old.package_bytes("shared"), before);
    // Choosing the current root retains the typed name and re-enables its
    // ordinary Enter submission once the settings write has finished.
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |root| root.speaker_command.is_none());
    let saved = fixture.old.read("shared");
    assert!(saved
        .state
        .results
        .diarization
        .iter()
        .filter(|turn| turn.speaker_id == "speaker-1")
        .all(|turn| turn.speaker_label.as_deref() == Some("Keep this name draft")));
    assert_eq!(saved.state.revision(), 4);
}

#[gpui::test]
fn settings_groups_have_more_room_without_spreading_their_rows(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    let (view, cx) = open_workspace(cx, &fixture);
    for size in [
        crate::pages::chrome::MIN_WINDOW_SIZE,
        gpui::size(gpui::px(1180.), gpui::px(760.)),
    ] {
        cx.simulate_resize(size);
        for locale in [Locale::ZhCn, Locale::EnUs] {
            view.update(cx, |root, cx| {
                root.ui.locale = locale;
                cx.notify();
            });
            cx.run_until_parked();
            let theme = rendered(cx, "select-theme");
            let material = rendered(cx, "select-material");
            assert_eq!(
                material.top() - theme.top(),
                gpui::px(68.),
                "the appearance rows retain their compact 52 px height and padding",
            );
            let sections = view.read_with(cx, |root, _| {
                let scroll = &root.scroll.settings;
                assert_eq!(
                    scroll.children_count(),
                    crate::pages::settings::SECTIONS.len()
                );
                (0..scroll.children_count())
                    .map(|index| scroll.bounds_for_item(index).unwrap())
                    .collect::<Vec<_>>()
            });
            for (index, pair) in sections.windows(2).enumerate() {
                let gap = pair[1].top() - pair[0].bottom();
                assert!(
                    gap >= gpui::px(40.),
                    "{locale:?} at {size:?}: groups {index} and {} need at least 40 px between their bounds, got {gap:?}",
                    index + 1,
                );
            }
        }
    }
}

fn settings_path_row_fits_the_minimum_width(cx: &mut TestAppContext, speakers: bool) {
    let fixture = Workspace::new();
    put(&fixture.old, "old", "Old");
    let (view, cx) = open_workspace(cx, &fixture);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    let (section, section_selector, path_selector, button_selector, label_key) = if speakers {
        (
            crate::pages::settings::SPEAKER_LIBRARY_SECTION,
            "speaker-library-section",
            "speaker-library-path",
            "choose-speaker-library",
            "speaker_library_path",
        )
    } else {
        (
            crate::pages::settings::WORKSPACE_SECTION,
            "workspace-section",
            "documents-root-path",
            "choose-documents-root",
            "documents_root",
        )
    };
    for locale in [Locale::ZhCn, Locale::EnUs] {
        for path in ["data/documents", "/Users/meeting-notes/Documents/团队工作区/a-long-project-name/meeting-recordings/资料与访谈/documents"] {
            view.update(cx, |root, cx| {
                root.ui.locale = locale;
                if speakers {
                    root.settings_draft.speaker_library_path = path.into();
                } else {
                    root.settings_draft.documents_root = path.into();
                }
                cx.notify();
            });
            cx.run_until_parked();
            click_selector(cx, crate::document_tests::leak(format!("settings-section-{section}")));
            let label = rendered(cx, crate::document_tests::leak(format!("setting-label-{}", i18n::text(locale, label_key))));
            let path_bounds = rendered(cx, path_selector);
            let button = rendered(cx, button_selector);
            let section_bounds = rendered(cx, section_selector);
            assert!(path_bounds.left() >= label.right(), "{locale:?}: the path must not cover its label: {path_bounds:?} versus {label:?}");
            assert!(path_bounds.right() <= button.left(), "{locale:?}: the path must not cover its choose button");
            assert!(path_bounds.size.width > gpui::px(0.), "the current path retains visible space");
            assert!(within(path_bounds, section_bounds));
            assert!(within(button, section_bounds));
            assert!(within(button, rendered(cx, "settings-page")));
            let mut reached = false;
            for _ in 0..80 {
                cx.simulate_keystrokes("tab");
                cx.run_until_parked();
                if cx.debug_bounds("focus-ring").is_some_and(|ring| within(ring, rendered(cx, button_selector))) {
                    reached = true;
                    break;
                }
            }
            assert!(reached, "{locale:?}: {button_selector} must stay keyboard reachable");
        }
    }
    assert!(!cx.cx.did_prompt_for_paths());
    let key = gpui::Keystroke::parse("enter").unwrap();
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: key.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke: key });
    cx.run_until_parked();
    assert!(
        cx.cx.did_prompt_for_paths(),
        "Enter activates the focused chooser"
    );
}

#[gpui::test]
fn the_documents_path_keeps_its_label_and_chooser_clear_at_minimum_width(cx: &mut TestAppContext) {
    settings_path_row_fits_the_minimum_width(cx, false);
}

#[gpui::test]
fn the_speaker_library_path_keeps_its_label_and_chooser_clear_at_minimum_width(
    cx: &mut TestAppContext,
) {
    settings_path_row_fits_the_minimum_width(cx, true);
}

#[gpui::test]
fn a_switch_while_a_job_runs_names_the_busy_jobs_and_changes_nothing(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let (view, cx) = open_workspace(cx, &fixture);
    // A processing or Assistant job on the shared runtime.
    let (job, _events) = fixture.jobs.create_job();
    job.started();
    let settings_before = std::fs::read(&fixture.settings_path).unwrap();

    choose(cx, &view, fixture.new.path());
    let busy = i18n::documents_root_error(
        Locale::ZhCn,
        &DocumentsRootError::WorkspaceBusy {
            job_ids: vec![job.job_id()],
        },
    );
    assert_eq!(switch_status(cx, &view), Some((busy.clone(), true)));
    assert_eq!(view.read_with(cx, |root, _| root.ui.notice.clone()), busy);
    rendered(cx, "documents-root-status");
    assert_eq!(
        std::fs::read(&fixture.settings_path).unwrap(),
        settings_before
    );
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(ids(&view, cx), ["old-doc"]);
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.settings_draft.documents_root,
            fixture.old.path().display().to_string()
        )
    });

    // Once the job ends the same choice switches.
    job.completed();
    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "new-doc")
    });
    assert_eq!(fixture.root(), fixture.new.path());
}

#[gpui::test]
fn a_reserved_or_invalid_documents_root_is_refused_with_its_reason(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    let (view, cx) = open_workspace(cx, &fixture);
    let settings_before = std::fs::read(&fixture.settings_path).unwrap();

    choose(cx, &view, &fixture.models);
    let reserved = DocumentsRootError::Reserved {
        path: fixture.models.clone(),
        location: ReservedLocation::ModelDirectory,
        reserved: fixture.models.clone(),
    };
    assert_eq!(
        switch_status(cx, &view),
        Some((i18n::documents_root_error(Locale::ZhCn, &reserved), true))
    );

    let file = fixture.new.path().join("notes.txt");
    std::fs::write(&file, "not a folder").unwrap();
    choose(cx, &view, &file);
    let not_a_directory = DocumentsRootError::NotADirectory { path: file.clone() };
    assert_eq!(
        switch_status(cx, &view),
        Some((
            i18n::documents_root_error(Locale::ZhCn, &not_a_directory),
            true
        ))
    );
    assert_eq!(
        std::fs::read(&fixture.settings_path).unwrap(),
        settings_before
    );
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(ids(&view, cx), ["old-doc"]);
}

#[gpui::test]
fn a_settings_conflict_after_the_switch_switches_the_workspace_back(cx: &mut TestAppContext) {
    let fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let (view, cx) = open_workspace(cx, &fixture);
    // Another writer saves the settings after the desktop loaded them.
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();

    choose(cx, &view, fixture.new.path());
    let (message, failed) = switch_status(cx, &view).expect("the switch reports its outcome");
    assert!(failed);
    assert!(
        message.starts_with(&i18n::message(Locale::ZhCn, "documents_root_not_saved", "")),
        "{message}"
    );
    assert_eq!(
        fixture.root(),
        fixture.old.path(),
        "the workspace switched back"
    );
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.old.path().display().to_string()
    );
    assert_eq!(
        fixture.settings.snapshot().unwrap().expected_speakers,
        Some(3)
    );
    wait_for(cx, &view, |root| root.projects_loaded);
    assert_eq!(ids(&view, cx), ["old-doc"]);
    // The switch ended the old root's change feed; the desktop follows the
    // root it switched back to.
    put(&fixture.old, "later-doc", "Later");
    wait_for_watcher(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.id().as_str() == "later-doc")
    });

    // The conflict reloaded the settings, so choosing again switches.
    choose(cx, &view, fixture.new.path());
    assert_eq!(fixture.root(), fixture.new.path());
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
    assert_eq!(
        fixture.settings.snapshot().unwrap().expected_speakers,
        Some(3)
    );
}

#[gpui::test]
fn a_failed_root_restore_shows_the_active_path_and_can_save_it_on_retry(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let mut fixture = Workspace::new();
    put(&fixture.old, "old-doc", "Old");
    put(&fixture.new, "new-doc", "New");
    let old_bytes = fixture.old.package_bytes("old-doc");
    let new_bytes = fixture.new.package_bytes("new-doc");
    fixture.settings = SettingsService::open(Arc::new(HeldSettingsBeforeSave {
        inner: JsonSettingsRepository::open(&fixture.settings_path),
        gate: gate.clone(),
    }))
    .unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    gate.arm();
    view.update(cx, |root, cx| {
        root.choose_documents_root(fixture.new.path().into(), cx)
    });
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert_eq!(fixture.root(), fixture.new.path());

    // A real competing write makes compare-and-save fail. Meanwhile the
    // previous directory becomes unavailable, so restoring it also fails.
    let external = SettingsService::open(Arc::new(JsonSettingsRepository::open(
        &fixture.settings_path,
    )))
    .unwrap();
    let mut value = external.snapshot().unwrap();
    value.expected_speakers = Some(3);
    external.update(value).unwrap();
    let unreadable = UnreadableDocumentsRoot::new(&fixture.old);
    gate.release();
    wait_for(cx, &view, |root| !root.documents_root_switch.switching);
    wait_for(cx, &view, |root| root.projects_loaded);
    let (message, failed) = switch_status(cx, &view).unwrap();
    assert!(failed);
    assert!(message.starts_with(&i18n::message(
        Locale::ZhCn,
        "documents_root_restore_failed",
        ""
    )));
    assert_eq!(fixture.root(), fixture.new.path());
    assert_eq!(ids(&view, cx), ["new-doc"]);
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.old.path().display().to_string()
    );
    let displayed = view.read_with(cx, |root, _| root.settings_draft.documents_root.clone());
    assert_eq!(
        displayed,
        fixture.new.path().display().to_string(),
        "Settings must show the active directory when restoring the saved path fails"
    );

    // Another settings conflict must not replace the active path with the
    // persisted one that the workspace could not restore.
    let mut value = external.snapshot().unwrap();
    value.expected_speakers = Some(4);
    external.update(value).unwrap();
    view.update(cx, |root, cx| root.set_locale(Locale::EnUs, cx));
    wait_for(cx, &view, |root| root.ui.locale == Locale::ZhCn);
    assert_eq!(
        view.read_with(cx, |root, _| root.settings_draft.documents_root.clone()),
        fixture.new.path().display().to_string(),
        "unrelated settings conflicts must retain the active path"
    );

    drop(unreadable);
    choose(cx, &view, fixture.new.path());
    assert!(!switch_status(cx, &view).unwrap().1);
    assert_eq!(
        fixture.saved_documents_root(),
        fixture.new.path().display().to_string()
    );
    assert_eq!(
        fixture.settings.snapshot().unwrap().expected_speakers,
        Some(4)
    );
    assert_eq!(fixture.old.package_bytes("old-doc"), old_bytes);
    assert_eq!(fixture.new.package_bytes("new-doc"), new_bytes);
}

/// A processed root `weekly` whose results came from `AUDIO` at `audio`,
/// and its semantic child `decisions` on the same recording.
fn recording(root: &DocumentsRoot, audio: &Path) {
    let mut weekly = processed("weekly", "产品周会");
    weekly.source.audio_path = audio.display().to_string();
    if let Some(record) = weekly.processing.latest.as_mut() {
        record.input_audio_checksum = Some(AUDIO_SHA256.into());
    }
    let mut decisions =
        weekly.semantic_child(ProjectId::new("decisions"), "决策", 0, 4_000, Vec::new());
    decisions.schema.revision = 2;
    root.put(weekly, "# 产品周会\n");
    root.put(decisions, "# 决策\n");
}

fn audio_result(
    cx: &mut VisualTestContext,
    view: &View,
    id: &str,
) -> Option<Result<AudioStatus, AudioSourceError>> {
    let id = ProjectId::new(id);
    view.read_with(cx, |root, _| root.audio.result_for(&id).cloned())
}

fn notice(cx: &mut VisualTestContext, view: &View) -> String {
    view.read_with(cx, |root, _| root.ui.notice.clone())
}

fn relink(cx: &mut VisualTestContext, view: &View, path: &Path) {
    let path = path.to_path_buf();
    view.update(cx, |root, cx| root.relink_audio_path(path, cx));
    wait_for(cx, view, |root| root.audio.relinking.is_none());
    cx.run_until_parked();
}

#[gpui::test]
fn an_audio_relink_queued_before_a_host_root_switch_keeps_its_original_asset(
    cx: &mut TestAppContext,
) {
    let fixture = Workspace::new();
    let missing = fixture.models.join("moved-away.wav");
    let restored = fixture.models.join("found-again.wav");
    std::fs::write(&restored, AUDIO).unwrap();
    for (root, attachment) in [
        (&fixture.old, b"old attachment"),
        (&fixture.new, b"new attachment"),
    ] {
        recording(root, &missing);
        for id in ["weekly", "decisions"] {
            std::fs::write(root.path().join(id).join("attachment.bin"), attachment).unwrap();
        }
    }
    let untouched = [
        fixture.new.package_bytes("weekly"),
        fixture.new.package_bytes("decisions"),
    ];
    let old_records = [fixture.old.read("weekly"), fixture.old.read("decisions")];
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    select_row(cx, &view, "weekly");
    wait_for(cx, &view, |root| !root.audio.pending());
    view.update(cx, |root, cx| {
        root.relink_audio_path(restored.clone(), cx);
        assert_eq!(
            fixture.old.read("weekly").state.audio_path(),
            missing.to_str().unwrap()
        );
        // The accepted UI task has not been polled. The real Host replaces
        // its repository before the queued relink starts reading or writing.
        fixture
            .workspace
            .switch_documents_root(fixture.new.path())
            .unwrap();
    });
    wait_for(cx, &view, |root| root.audio.relinking.is_none());
    assert_eq!(
        fixture.new.read("weekly").state.audio_path(),
        missing.to_str().unwrap(),
        "the queued relink must not alter the replacement workspace's same-ID asset"
    );
    assert_eq!(
        [
            fixture.new.package_bytes("weekly"),
            fixture.new.package_bytes("decisions")
        ],
        untouched
    );
    for previous in old_records {
        let current = fixture.old.read(previous.state.id().as_str());
        assert_eq!(current.state.audio_path(), restored.to_str().unwrap());
        assert_eq!(current.state.revision(), previous.state.revision() + 1);
        assert_eq!(current.state.results, previous.state.results);
        assert_eq!(current.state.processing, previous.state.processing);
        assert_eq!(current.state.hierarchy, previous.state.hierarchy);
        assert_eq!(current.markdown, previous.markdown);
    }
    for (root, attachment) in [
        (&fixture.old, b"old attachment"),
        (&fixture.new, b"new attachment"),
    ] {
        for id in ["weekly", "decisions"] {
            assert_eq!(
                std::fs::read(root.path().join(id).join("attachment.bin")).unwrap(),
                attachment
            );
        }
    }
    assert_eq!(std::fs::read(restored).unwrap(), AUDIO);
}

#[gpui::test]
fn a_late_audio_relink_cannot_complete_a_new_workspaces_relink(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let old_gate = crate::tests::Gate::default();
    let new_gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&old_gate, &new_gate);
    for (root, title) in [(&fixture.old, "Old"), (&fixture.new, "New")] {
        root.put(
            DocumentState::new(
                ProjectId::new("shared"),
                title,
                root.path().join("missing.wav").display().to_string(),
            ),
            &format!("# {title} notes\n"),
        );
        std::fs::write(
            root.path().join("shared/attachment.bin"),
            b"keep attachment",
        )
        .unwrap();
        std::fs::write(root.path().join("restored.wav"), AUDIO).unwrap();
    }
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    old_gate.arm();
    let old_path = fixture.old.path().join("restored.wav");
    view.update(cx, |root, cx| root.relink_audio_path(old_path.clone(), cx));
    wait_for(cx, &view, |_| old_gate.counts().0 == 1);
    let old_bytes = fixture.old.package_bytes("shared");
    assert_eq!(
        fixture.old.read("shared").state.audio_path(),
        old_path.to_str().unwrap()
    );

    choose(cx, &view, fixture.new.path());
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.title() == "New")
            && !root.audio.pending()
    });
    assert!(view.read_with(cx, |root, _| root.audio.relinking.is_none()));
    new_gate.arm();
    let new_path = fixture.new.path().join("restored.wav");
    view.update(cx, |root, cx| root.relink_audio_path(new_path.clone(), cx));
    wait_for(cx, &view, |_| new_gate.counts().0 == 1);
    wait_for_watcher(cx, &view, |root| {
        root.projects
            .iter()
            .any(|document| document.revision() == 1)
            && !root.audio.pending()
    });
    let notice = view.read_with(cx, |root, _| root.ui.notice.clone());
    let focus = cx.update(|window, cx| window.focused(cx));
    let new_bytes = fixture.new.package_bytes("shared");
    let completed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = completed.clone();
    let _completion = cx.update(|_, cx| cx.observe(&view, move |_, _| observed.set(true)));
    old_gate.release();
    wait_for(cx, &view, |_| completed.get());
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.audio.relinking,
            Some(ProjectId::new("shared")),
            "the old relink must not finish the new workspace's relink"
        );
        assert_eq!(root.ui.notice, notice);
        assert!(root.document_conflict.is_none());
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.selected_project, Some(ProjectId::new("shared")));
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(fixture.old.package_bytes("shared"), old_bytes);
    assert_eq!(fixture.new.package_bytes("shared"), new_bytes);
    new_gate.release();
    wait_for(cx, &view, |root| root.audio.relinking.is_none());
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            i18n::message(root.ui.locale, "audio_relinked", "New")
        );
        assert!(root.document_conflict.is_none());
    });
    assert_eq!(
        fixture.new.read("shared").state.audio_path(),
        new_path.to_str().unwrap()
    );
    for root in [&fixture.old, &fixture.new] {
        assert_eq!(
            std::fs::read(root.path().join("shared/attachment.bin")).unwrap(),
            b"keep attachment"
        );
        assert_eq!(
            std::fs::read(root.path().join("restored.wav")).unwrap(),
            AUDIO
        );
    }
}

#[gpui::test]
fn an_audio_relink_in_a_rolled_back_workspace_still_completes(cx: &mut TestAppContext) {
    let _alone = crate::tests::hold_a_worker_alone();
    let gate = crate::tests::Gate::default();
    let fixture = Workspace::with_save_gates(&gate, &crate::tests::Gate::default());
    put(&fixture.old, "shared", "Old");
    let path = fixture.old.path().join("restored.wav");
    std::fs::write(&path, AUDIO).unwrap();
    let (view, cx) = open_workspace(cx, &fixture);
    wait_for(cx, &view, |root| {
        !root.audio.pending() && !root.installation.catalog.is_empty()
    });
    view.update(cx, |root, cx| root.navigate(Route::Meetings, cx));
    cx.run_until_parked();
    gate.arm();
    view.update(cx, |root, cx| root.relink_audio_path(path.clone(), cx));
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let saved = fixture.old.package_bytes("shared");
    let mut external = fixture.settings.snapshot().unwrap();
    external.expected_speakers = Some(3);
    fixture.settings.update(external).unwrap();
    choose(cx, &view, fixture.new.path());
    assert!(switch_status(cx, &view).unwrap().1);
    assert_eq!(fixture.root(), fixture.old.path());
    assert_eq!(
        view.read_with(cx, |root, _| root.audio.relinking.clone()),
        Some(ProjectId::new("shared"))
    );
    gate.release();
    wait_for(cx, &view, |root| root.audio.relinking.is_none());
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            i18n::message(root.ui.locale, "audio_relinked", "Old")
        );
        assert!(root.document_conflict.is_none());
    });
    assert_eq!(fixture.old.package_bytes("shared"), saved);
    assert_eq!(
        fixture.old.read("shared").state.audio_path(),
        path.to_str().unwrap()
    );
}

#[gpui::test]
fn a_moved_audio_file_is_missing_until_relinked_to_the_same_recording(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let audio_directory = temp_directory();
    std::fs::create_dir_all(audio_directory.join("moved")).unwrap();
    let audio = audio_directory.join("weekly.wav");
    std::fs::write(&audio, AUDIO).unwrap();
    recording(&root, &audio);
    let (view, cx, changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    wait_for(cx, &view, |root| {
        matches!(
            root.audio.result_for(&ProjectId::new("weekly")),
            Some(Ok(AudioStatus::Present { .. }))
        )
    });
    assert!(cx.debug_bounds("audio-indicator-weekly").is_none());
    assert!(cx.debug_bounds("relink-audio").is_none());

    // The recording moves; the next reload checks it again.
    let moved = audio_directory.join("moved").join("weekly.wav");
    std::fs::rename(&audio, &moved).unwrap();
    changes.unbounded_send(DocumentChange::Changed).unwrap();
    let missing = AudioStatus::Missing {
        path: audio.display().to_string(),
    };
    wait_for(cx, &view, |root| {
        root.audio.result_for(&ProjectId::new("weekly")) == Some(&Ok(missing.clone()))
    });
    cx.run_until_parked();
    let row = rendered(cx, "project-row-weekly");
    assert!(within(rendered(cx, "audio-indicator-weekly"), row));
    assert!(within(
        rendered(cx, "relink-audio"),
        rendered(cx, "meetings-page")
    ));
    click_selector(cx, "open-project-detail");
    let detail_panel = rendered(cx, "detail-panel");
    assert!(within(rendered(cx, "detail-audio-status"), detail_panel));
    let shown = view.read_with(cx, |root, _| {
        let weekly = root
            .projects
            .iter()
            .find(|project| project.id().as_str() == "weekly")
            .unwrap();
        detail::detail_sections(
            weekly,
            &root.projects,
            root.ui.locale,
            root.audio.result_for(weekly.id()),
        )
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.key == "detail_audio_status")
        .map(|row| row.value)
    });
    assert_eq!(shown, Some(i18n::audio_status(Locale::ZhCn, &missing)));

    // Processing is disabled while the audio is missing.
    click_selector(cx, "process-project");
    cx.run_until_parked();
    assert!(!view.read_with(cx, |root, _| root.ui.processing));
    view.update_in(cx, |root, window, cx| {
        root.process_selected(&gpui::ClickEvent::default(), window, cx)
    });
    assert!(!view.read_with(cx, |root, _| root.ui.processing));
    assert_eq!(
        notice(cx, &view),
        i18n::text(Locale::ZhCn, "audio_missing_cannot_process")
    );

    // A different recording is refused and nothing is written.
    let written = [
        root.package_bytes("weekly"),
        root.package_bytes("decisions"),
    ];
    let other = audio_directory.join("other.wav");
    std::fs::write(&other, OTHER_AUDIO).unwrap();
    relink(cx, &view, &other);
    assert_eq!(
        notice(cx, &view),
        i18n::audio_source_error(
            Locale::ZhCn,
            &AudioSourceError::ChecksumMismatch {
                path: other.display().to_string(),
                expected: AUDIO_SHA256.into(),
                actual: OTHER_SHA256.into(),
            }
        )
    );
    // So is a file that does not exist.
    let gone = audio_directory.join("gone.wav");
    relink(cx, &view, &gone);
    assert_eq!(
        notice(cx, &view),
        i18n::audio_source_error(
            Locale::ZhCn,
            &AudioSourceError::FileMissing {
                path: gone.display().to_string(),
            }
        )
    );
    assert_eq!(
        [
            root.package_bytes("weekly"),
            root.package_bytes("decisions")
        ],
        written,
        "refused relinks write nothing"
    );
    assert_eq!(audio_result(cx, &view, "weekly"), Some(Ok(missing)));

    // The moved recording relinks every document of the recording.
    let revisions = [root.revision("weekly"), root.revision("decisions")];
    relink(cx, &view, &moved);
    assert_eq!(
        notice(cx, &view),
        i18n::message(Locale::ZhCn, "audio_relinked", "产品周会")
    );
    assert_eq!(
        [root.revision("weekly"), root.revision("decisions")],
        revisions.map(|revision| revision + 1)
    );
    for id in ["weekly", "decisions"] {
        assert_eq!(
            root.read(id).state.audio_path(),
            moved.display().to_string(),
            "{id} names the moved file"
        );
    }
    wait_for(cx, &view, |root| {
        matches!(
            root.audio.result_for(&ProjectId::new("weekly")),
            Some(Ok(AudioStatus::Present { .. }))
        ) && root.projects.iter().any(|project| {
            project.id().as_str() == "weekly" && project.revision() == revisions[0] + 1
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("audio-indicator-weekly").is_none());
    assert!(cx.debug_bounds("relink-audio").is_none());
    assert!(!view.read_with(cx, |root, _| root.audio.missing(&ProjectId::new("weekly"))));
    std::fs::remove_dir_all(audio_directory).ok();
}

#[gpui::test]
fn relinking_a_document_changed_elsewhere_conflicts_and_writes_nothing(cx: &mut TestAppContext) {
    let root = DocumentsRoot::new();
    let audio_directory = temp_directory();
    std::fs::create_dir_all(&audio_directory).unwrap();
    // The recording is not where the document says; its copy is.
    let audio = audio_directory.join("weekly.wav");
    let copy = audio_directory.join("copy.wav");
    std::fs::write(&copy, AUDIO).unwrap();
    recording(&root, &audio);
    let (view, cx, _changes) = open_on(cx, &root);
    select_row(cx, &view, "weekly");
    wait_for(cx, &view, |root| {
        root.audio.missing(&ProjectId::new("weekly"))
    });
    let loaded = root.revision("weekly");

    // Another writer changes the document after the tree was read.
    let mut external = root.read("weekly").state;
    external.content.title = "外部修改".into();
    root.external_save(external);
    let written = [
        root.package_bytes("weekly"),
        root.package_bytes("decisions"),
    ];

    relink(cx, &view, &copy);
    let conflict = AudioSourceError::RevisionConflict {
        document_id: ProjectId::new("weekly"),
        expected: loaded,
        actual: loaded + 1,
    };
    assert_eq!(
        notice(cx, &view),
        i18n::audio_source_error(Locale::ZhCn, &conflict)
    );
    let shown = view.read_with(cx, |root, _| root.document_conflict.clone());
    let shown = shown.expect("the conflict notice opens");
    assert_eq!(
        (shown.document_id.as_str(), shown.expected, shown.actual),
        ("weekly", loaded, loaded + 1)
    );
    rendered(cx, "document-conflict");
    assert_eq!(
        [
            root.package_bytes("weekly"),
            root.package_bytes("decisions")
        ],
        written
    );

    // Against the reloaded revision the relink succeeds.
    wait_for(cx, &view, |root| {
        root.projects
            .iter()
            .any(|project| project.title() == "外部修改")
    });
    relink(cx, &view, &copy);
    assert_eq!(root.revision("weekly"), loaded + 2);
    assert_eq!(
        root.read("weekly").state.audio_path(),
        copy.display().to_string()
    );
    std::fs::remove_dir_all(audio_directory).ok();
}
