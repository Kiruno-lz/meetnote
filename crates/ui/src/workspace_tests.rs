//! L2: the documents root chosen in Settings, and the recovery of a
//! document's missing source audio, on real document packages in temporary
//! documents roots (`FileDocumentRepository`) and real `settings.json`
//! files.
//!
//! The platform's folder and file prompts do not exist on the test
//! platform; the tests call what their callbacks call:
//! `choose_documents_root` and `relink_audio_path`.

use crate::document_tests::{
    open_on, processed, rendered, select_row, within, DocumentsRoot, UnreadableDocumentsRoot, View,
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
        Self::with_gates(read_gate, None)
    }

    fn with_save_gates(old: &crate::tests::Gate, new: &crate::tests::Gate) -> Self {
        Self::with_gates(None, Some((old, new)))
    }

    fn with_gates(
        read_gate: Option<&crate::tests::Gate>,
        save_gates: Option<(&crate::tests::Gate, &crate::tests::Gate)>,
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
                (_, Some((gate, _))) => {
                    crate::document_tests::gated_save_repository(old.repository(), gate)
                }
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
