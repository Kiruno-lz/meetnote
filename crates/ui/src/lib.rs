pub mod assistant_ui;
mod components;
mod controller;
mod i18n;
mod pages;
mod route;
#[cfg(test)]
mod source_files;
mod state;
#[doc(hidden)]
pub mod test_support;
mod text_input;
mod theme;

pub use controller::{run, DesktopServices};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_ui::{
        AssistantCancellation, AssistantMutationSummary, AssistantPendingReview,
        AssistantRunRequest, AssistantRunResult, AssistantScopeMode, AssistantSettingsSave,
        AssistantTaskState, AssistantUiError, AssistantUiService, ConnectionDiagnostic,
        ConnectionProbeKind, PendingReviewState,
    };
    use application::{BackendInventory, DocumentService, ProcessingService, WorkspaceService};
    use asr_whisper_rs::WhisperRsBackend;
    use contracts::ModelRef;
    use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
    use domain::{DocumentState, ProjectId};
    use gpui::{AppContext, Modifiers, TestAppContext};
    use models::{ModelCatalog, ModelManager};
    use persistence::InMemoryDocumentRepository;
    use settings::{JsonSettingsRepository, SettingsService};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeAssistantUiService {
        undo_calls: Arc<Mutex<Vec<String>>>,
        run_result: Arc<Mutex<Option<AssistantRunResult>>>,
        confirm_result: Arc<Mutex<Option<AssistantRunResult>>>,
        /// `(operation, authorized_document_ids)` for every service call.
        grants: Arc<Mutex<Vec<(&'static str, Vec<String>)>>>,
        /// Settings that `save_assistant_settings` writes to, when exercised.
        settings: Option<SettingsService>,
        /// The write mode of every run request.
        write_modes: Arc<Mutex<Vec<contracts::AgentWriteMode>>>,
    }

    impl FakeAssistantUiService {
        fn record(&self, operation: &'static str, document_ids: Vec<String>) {
            self.grants.lock().unwrap().push((operation, document_ids));
        }

        fn grants_for(&self, operation: &str) -> Vec<Vec<String>> {
            self.grants
                .lock()
                .unwrap()
                .iter()
                .filter(|(recorded, _)| *recorded == operation)
                .map(|(_, document_ids)| document_ids.clone())
                .collect()
        }
    }

    impl AssistantUiService for FakeAssistantUiService {
        /// Edits of existing profiles, committed as the desktop service does.
        fn save_assistant_settings(
            &self,
            expected_revision: String,
            save: AssistantSettingsSave,
        ) -> Result<settings::AppSettings, AssistantUiError> {
            let service = self
                .settings
                .as_ref()
                .ok_or_else(|| AssistantUiError::unavailable("not exercised"))?;
            assert!(save.new_connection.is_none(), "not exercised");
            service
                .patch_if_equal(
                    &expected_revision,
                    save.patch,
                    settings::SettingsGrant::full(),
                )
                .map_err(|error| AssistantUiError::from_settings(&error))
        }

        fn probe_connection(
            &self,
            profile: settings::AssistantConnectionProfile,
            kind: ConnectionProbeKind,
            _: AssistantCancellation,
        ) -> Result<ConnectionDiagnostic, AssistantUiError> {
            Ok(ConnectionDiagnostic {
                kind,
                ready: true,
                local: profile.is_local(),
                models: None,
                text: Some(true),
                client_tools: Some(true),
                requested_model: profile.model.clone(),
                resolved_model: Some(profile.model),
                context_window_tokens: profile.context_window_tokens,
                message: "ready".into(),
            })
        }

        fn run(
            &self,
            request: AssistantRunRequest,
            _: AssistantCancellation,
        ) -> Result<AssistantRunResult, AssistantUiError> {
            self.write_modes.lock().unwrap().push(request.write_mode);
            self.record(
                "run",
                request
                    .authorized_documents
                    .into_iter()
                    .map(|document| document.document_id)
                    .collect(),
            );
            self.run_result
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| AssistantUiError::unavailable("not exercised"))
        }

        fn confirm(
            &self,
            _: String,
            authorized_document_ids: Vec<String>,
        ) -> Result<AssistantRunResult, AssistantUiError> {
            self.record("confirm", authorized_document_ids);
            self.confirm_result
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| AssistantUiError::unavailable("not exercised"))
        }

        fn reject(
            &self,
            _: String,
            authorized_document_ids: Vec<String>,
        ) -> Result<(), AssistantUiError> {
            self.record("reject", authorized_document_ids);
            Ok(())
        }

        fn undo(
            &self,
            undo_token: String,
            authorized_document_ids: Vec<String>,
        ) -> Result<AssistantMutationSummary, AssistantUiError> {
            self.record("undo", authorized_document_ids);
            self.undo_calls.lock().unwrap().push(undo_token.clone());
            Ok(AssistantMutationSummary {
                mutation_id: "undo-result".into(),
                affected_documents: Vec::new(),
                undo_token,
                uncovered_segment_ids: Vec::new(),
            })
        }
    }

    #[gpui::test]
    fn boots_an_empty_desktop_workspace(cx: &mut TestAppContext) {
        let services =
            desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
        // The startup installation probe completes on a worker thread.
        cx.executor().allow_parking();
        let assistant_input = cx.new(|cx| text_input::TextInput::new(cx, "Ask…"));
        let view = cx.new(|cx| {
            controller::RootView::new(
                BackendInventory::new(vec![], vec![]),
                services,
                assistant_input,
                cx,
            )
        });

        assert_eq!(view.read_with(cx, |view, _| view.projects.len()), 0);
    }

    #[gpui::test]
    fn renders_assistant_mutation_and_dispatches_undo_click(cx: &mut TestAppContext) {
        let assistant = Arc::new(FakeAssistantUiService::default());
        let services = desktop_services(assistant.clone(), test_settings());
        // Undo runs on a `runtime::spawn_blocking` worker thread.
        cx.executor().allow_parking();
        cx.update(|cx| controller::init_app(theme::Accessibility::default(), cx));
        let assistant_input = cx.new(|cx| text_input::TextInput::new(cx, "Ask…"));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            controller::RootView::new(
                BackendInventory::new(vec![], vec![]),
                services,
                assistant_input,
                cx,
            )
        });
        view.update(cx, |root, cx| {
            root.assistant.result = Some(AssistantRunResult {
                answer: Some("done".into()),
                warning: None,
                warnings: Vec::new(),
                evidence_segment_ids: Vec::new(),
                usage: None,
                requested_model: None,
                resolved_model: None,
                mutation: Some(AssistantMutationSummary {
                    mutation_id: "ui-mutation".into(),
                    affected_documents: Vec::new(),
                    undo_token: "undo-ui-mutation".into(),
                    uncovered_segment_ids: Vec::new(),
                }),
                mutations: Vec::new(),
                pending: None,
                pending_reviews: Vec::new(),
                diagnostics: None,
            });
            root.assistant.task = AssistantTaskState::Completed;
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("assistant-undo-ui-mutation")
            .expect("rendered Assistant mutation must expose an undo button");
        cx.simulate_click(bounds.center(), Modifiers::none());
        for _ in 0..100 {
            cx.run_until_parked();
            if !assistant.undo_calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            assistant.undo_calls.lock().unwrap().as_slice(),
            ["undo-ui-mutation"]
        );
    }

    /// A temporary directory no other test uses; it is not created.
    fn temp_directory() -> std::path::PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "meetnote-ui-test-{}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Settings in a directory of their own, so parallel tests never share a file.
    fn test_settings() -> SettingsService {
        SettingsService::open(Arc::new(JsonSettingsRepository::open(
            temp_directory().join("settings.json"),
        )))
        .unwrap()
    }

    const STORED_KEY: &str = "stored-secret";

    /// Settings whose active connection `local` has a stored API key.
    fn stored_key_settings() -> SettingsService {
        let settings = test_settings();
        let mut value = settings.snapshot().unwrap();
        value.assistant.active_connection_id = Some("local".into());
        value.assistant.connections = vec![settings::AssistantConnectionProfile {
            id: "local".into(),
            name: "Local".into(),
            protocol: settings::LlmProtocol::OpenAiChatCompletions,
            base_url: "http://127.0.0.1:4000".into(),
            model: "assistant-default".into(),
            auth_mode: settings::AssistantAuthMode::Bearer,
            api_key: Some(STORED_KEY.into()),
            api_key_file: None,
            context_window_tokens: None,
            timeout_ms: 120_000,
            max_output_tokens: 4_096,
        }];
        settings.update(value).unwrap();
        settings
    }

    /// An Agent changes a settings field after the desktop loaded them.
    fn external_patch(settings: &SettingsService, patch: settings::SettingsPatch) {
        settings
            .patch_if_equal(
                &settings.revision().unwrap(),
                patch,
                settings::SettingsGrant::full(),
            )
            .unwrap();
    }

    fn open_desktop(
        cx: &mut TestAppContext,
        assistant: Arc<FakeAssistantUiService>,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        open_desktop_with(cx, assistant, test_settings())
    }

    fn desktop_services(
        assistant: Arc<FakeAssistantUiService>,
        settings: SettingsService,
    ) -> DesktopServices {
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            DocumentService::new(Arc::new(InMemoryDocumentRepository::default())),
        );
        DesktopServices {
            workspace,
            settings,
            model_manager: ModelManager::new(
                ModelCatalog::from_path(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../resources/models/catalog.json"),
                )
                .unwrap(),
                "models",
            ),
            asr_model: ModelRef {
                backend_id: "whisper-rs".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "missing-model".into(),
                artifacts: Vec::new(),
            },
            diarization_model: ModelRef {
                backend_id: "sherpa-onnx".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "missing-bundle".into(),
                artifacts: Vec::new(),
            },
            assistant,
        }
    }

    /// The app setup `run` performs, with every accessibility flag off.
    fn init_gpui(cx: &mut TestAppContext) {
        // Service calls complete on `runtime::spawn_blocking` worker threads.
        cx.executor().allow_parking();
        cx.update(|cx| controller::init_app(theme::Accessibility::default(), cx));
    }

    fn open_desktop_with(
        cx: &mut TestAppContext,
        assistant: Arc<FakeAssistantUiService>,
        settings: SettingsService,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        let services = desktop_services(assistant, settings);
        init_gpui(cx);
        let assistant_input = cx.new(|cx| text_input::TextInput::new(cx, "Ask…"));
        cx.add_window_view(|_window, cx| {
            controller::RootView::new(
                BackendInventory::new(vec![], vec![]),
                services,
                assistant_input,
                cx,
            )
        })
    }

    /// The desktop window exactly as `run` opens it.
    fn open_main_window(
        cx: &mut TestAppContext,
        services: DesktopServices,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        init_gpui(cx);
        let (window, view) = cx
            .update(|cx| {
                controller::open_main_window(
                    BackendInventory::new(vec![], vec![]),
                    services,
                    gpui::WindowOptions::default(),
                    cx,
                )
            })
            .unwrap();
        let cx = gpui::VisualTestContext::from_window(window, cx).into_mut();
        cx.run_until_parked();
        (view, cx)
    }

    /// Drives the GPUI executor until the window satisfies `done`.
    #[track_caller]
    fn wait_until_window(
        cx: &mut gpui::VisualTestContext,
        done: impl Fn(&mut gpui::Window, &mut gpui::App) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if cx.update(|window, cx| done(window, cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("window state did not settle");
    }

    /// Replaces the installation probe with a counting wrapper around the
    /// real one; the count starts after the startup probe.
    fn count_installation_probes(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
    ) -> Arc<std::sync::atomic::AtomicUsize> {
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = probes.clone();
        view.update(cx, |root, _| {
            root.installation_probe = Arc::new(move |query| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                controller::probe_installation(query)
            });
        });
        probes
    }

    #[gpui::test]
    fn rendering_meetings_and_models_does_not_probe_installation(cx: &mut TestAppContext) {
        let (view, cx) = open_desktop(cx, Arc::new(FakeAssistantUiService::default()));
        wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
        let probes = count_installation_probes(cx, &view);

        for route in [route::Route::Meetings, route::Route::Models] {
            view.update(cx, |root, cx| root.navigate(route, cx));
            for _ in 0..5 {
                view.update(cx, |_, cx| cx.notify());
                cx.run_until_parked();
            }
        }
        assert_eq!(
            probes.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "rendering must read the cached installation state"
        );

        view.update(cx, |root, cx| root.refresh_installation(cx));
        wait_for(cx, &view, |_| {
            probes.load(std::sync::atomic::Ordering::SeqCst) == 1
        });
    }

    #[gpui::test]
    fn idle_change_and_download_waits_leave_blocking_workers_free(cx: &mut TestAppContext) {
        let settings = test_settings();
        let (view, cx) = open_desktop_with(
            cx,
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        // Twice as many idle waits as the shared `spawn_blocking` pool has
        // workers (at most 4); the senders stay alive, so every wait idles.
        let mut senders = Vec::new();
        view.update(cx, |root, cx| {
            for _ in 0..4 {
                let (changes, receiver) = std::sync::mpsc::channel();
                root.follow_document_changes(receiver, cx);
                let (downloads, events) = std::sync::mpsc::channel();
                root.follow_download(events, cx);
                senders.push((changes, downloads));
            }
        });
        cx.run_until_parked();

        view.update(cx, |root, cx| {
            root.select_asr_backend("test-asr".into(), cx)
        });
        wait_for(cx, &view, |_| {
            settings.snapshot().unwrap().asr_backend_id.as_deref() == Some("test-asr")
        });

        // The waits still deliver their events.
        let workspace = view.read_with(cx, |root, _| root.services.workspace.clone());
        workspace.import_audio("idle-wait.wav").unwrap();
        senders[0]
            .0
            .send(application::DocumentChange::Changed)
            .unwrap();
        wait_for(cx, &view, |root| root.projects.len() == 1);
        senders[1]
            .1
            .send(models::manager::DownloadEvent::Updated(
                models::manager::DownloadProgress {
                    id: models::manager::DownloadId(1),
                    model_id: "idle-model".into(),
                    state: models::manager::DownloadState::Completed,
                    completed_bytes: 1,
                    speed_bytes_per_second: None,
                    total_bytes: Some(1),
                    artifact_index: 1,
                    artifact_count: 1,
                    eta_seconds: Some(0),
                    retry_count: 0,
                },
            ))
            .unwrap();
        let installed = i18n::message(i18n::Locale::ZhCn, "installed", "idle-model");
        wait_for(cx, &view, |root| root.ui.notice == installed);
    }

    #[gpui::test]
    fn the_speaker_library_migration_dialog_opens_and_migrates(cx: &mut TestAppContext) {
        use gpui_component::WindowExt as _;
        let directory = temp_directory();
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("library.json");
        std::fs::write(&source, "{}").unwrap();
        let destination = directory.join("moved").join("library.json");
        let settings = test_settings();
        let mut value = settings.snapshot().unwrap();
        value.speaker_library_path = source.display().to_string();
        settings.update(value).unwrap();
        let services = desktop_services(Arc::new(FakeAssistantUiService::default()), settings);
        let (view, cx) = open_main_window(cx, services.clone());

        view.update(cx, |root, cx| {
            root.change_speaker_library_path(destination.clone(), cx)
        });
        wait_until_window(cx, |window, cx| window.has_active_dialog(cx));

        cx.dispatch_action(gpui_component::dialog::Confirm { secondary: false });
        let expected = destination.display().to_string();
        wait_for(cx, &view, |_| {
            services.settings.snapshot().unwrap().speaker_library_path == expected
        });
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "{}");
        std::fs::remove_dir_all(directory).ok();
    }

    #[gpui::test]
    fn detail_opens_beside_the_document_and_closing_it_keeps_jobs_running(cx: &mut TestAppContext) {
        let services =
            desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
        let (view, cx) = open_main_window(cx, services);
        // A processing job in flight, as `process_selected` leaves it, and a
        // download the view follows.
        let jobs = runtime::JobRuntime::with_max_concurrency(2);
        let (asr, _asr_events) = jobs.create_job();
        let (diarization, _diarization_events) = jobs.create_job();
        let (download, events) = std::sync::mpsc::channel();
        view.update(cx, |root, cx| {
            let document = DocumentState::new(ProjectId::new("first"), "First", "first.wav");
            root.ui.selected_project = Some(document.id().clone());
            root.projects = vec![document];
            root.ui.processing = true;
            root.cancel_contexts = Some((asr.clone(), diarization.clone()));
            root.follow_download(events, cx);
            root.navigate(route::Route::Meetings, cx);
        });
        cx.run_until_parked();

        let open = cx
            .debug_bounds("open-project-detail")
            .expect("the document offers Detail");
        cx.simulate_click(open.center(), Modifiers::none());
        cx.run_until_parked();
        let panel = cx
            .debug_bounds("detail-panel")
            .expect("Detail opens as a panel");
        let body = cx
            .debug_bounds("meeting-content")
            .expect("the document body stays visible beside Detail");
        assert!(
            !panel.intersects(&body) && body.right() <= panel.left(),
            "Detail lies right of the document body, not over it: {panel:?} / {body:?}"
        );
        let plane = cx
            .debug_bounds("content-plane")
            .expect("the content plane holds the page");
        let inside = |inner: gpui::Bounds<gpui::Pixels>| {
            inner.left() >= plane.left()
                && inner.right() <= plane.right()
                && inner.top() >= plane.top()
                && inner.bottom() <= plane.bottom()
        };
        assert!(inside(panel), "Detail is inside the content plane");

        let close = cx
            .debug_bounds("close-detail")
            .expect("Detail can be closed");
        cx.simulate_click(close.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("detail-panel").is_none(), "Detail closed");
        view.read_with(cx, |root, _| {
            assert!(root.ui.processing, "closing Detail keeps processing");
            assert!(root.cancel_contexts.is_some());
        });
        assert!(
            !asr.is_cancelled() && !diarization.is_cancelled(),
            "closing Detail cancels no job"
        );
        let progress = models::manager::DownloadProgress {
            id: models::manager::DownloadId(7),
            model_id: "detail-model".into(),
            state: models::manager::DownloadState::Downloading,
            completed_bytes: 2_000_000,
            speed_bytes_per_second: None,
            total_bytes: Some(4_000_000),
            artifact_index: 1,
            artifact_count: 1,
            eta_seconds: None,
            retry_count: 0,
        };
        download
            .send(models::manager::DownloadEvent::Updated(progress))
            .unwrap();
        let reported =
            i18n::download_progress(i18n::Locale::ZhCn, "detail-model", 1, 1, 2_000_000, None);
        wait_for(cx, &view, |root| root.ui.notice == reported);
    }

    /// Drives the GPUI executor until `done` holds; service calls run on real
    /// worker threads, so parked GPUI tasks may still be waiting on them.
    #[track_caller]
    fn wait_for(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
        done: impl Fn(&controller::RootView) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if view.read_with(cx, |root, _| done(root)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("assistant state did not settle");
    }

    fn empty_result() -> AssistantRunResult {
        AssistantRunResult {
            answer: None,
            warning: None,
            warnings: Vec::new(),
            evidence_segment_ids: Vec::new(),
            usage: None,
            requested_model: None,
            resolved_model: None,
            mutation: None,
            mutations: Vec::new(),
            pending: None,
            pending_reviews: Vec::new(),
            diagnostics: None,
        }
    }

    fn pending_review(id: &str, expires_at_ms: u64) -> AssistantPendingReview {
        AssistantPendingReview {
            confirmation_id: id.into(),
            expires_at_ms,
            affected_document_ids: vec!["first".into()],
            field_diffs: Vec::new(),
            tree_changes: Vec::new(),
            ranges: Vec::new(),
            uncovered_segment_ids: Vec::new(),
        }
    }

    /// Selects the `first` document and starts a run; the desktop must have
    /// saved settings with an active connection (`stored_key_settings`).
    fn start_run(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<controller::RootView>) {
        view.update_in(cx, |root, window, cx| {
            let first = DocumentState::new(ProjectId::new("first"), "First", "first.wav");
            let second = DocumentState::new(ProjectId::new("second"), "Second", "second.wav");
            root.ui.selected_project = Some(first.id().clone());
            root.projects = vec![first, second];
            root.assistant_inputs
                .prompt
                .update(cx, |input, _| input.set_value("organize"));
            root.assistant_action(&gpui::ClickEvent::default(), window, cx);
        });
    }

    #[gpui::test]
    fn the_assistant_uses_the_saved_settings_not_the_unsaved_draft(cx: &mut TestAppContext) {
        let assistant = Arc::new(FakeAssistantUiService::default());
        *assistant.run_result.lock().unwrap() = Some(empty_result());
        let (view, cx) = open_desktop_with(cx, assistant.clone(), stored_key_settings());
        view.update_in(cx, |root, window, cx| {
            let document = DocumentState::new(ProjectId::new("first"), "First", "first.wav");
            root.ui.selected_project = Some(document.id().clone());
            root.projects = vec![document];
            root.set_assistant_enabled(false, cx);
            root.set_assistant_write_mode(contracts::AgentWriteMode::ConfirmEach, cx);
            root.assistant_inputs
                .prompt
                .update(cx, |input, _| input.set_value("organize"));
            root.assistant_action(&gpui::ClickEvent::default(), window, cx);
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("assistant-write-mode-automatic").is_some()
                && cx
                    .debug_bounds("assistant-write-mode-confirm-each")
                    .is_none(),
            "the Assistant page must show the saved write mode"
        );
        wait_for(cx, &view, |_| {
            !assistant.write_modes.lock().unwrap().is_empty()
        });
        assert_eq!(
            assistant.write_modes.lock().unwrap().as_slice(),
            [contracts::AgentWriteMode::Automatic],
            "runs must use the saved write mode"
        );
    }

    #[gpui::test]
    fn confirm_reject_and_undo_use_the_grant_the_run_started_with(cx: &mut TestAppContext) {
        let assistant = Arc::new(FakeAssistantUiService::default());
        *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
            pending_reviews: vec![
                pending_review("accept-me", u64::MAX),
                pending_review("reject-me", u64::MAX),
            ],
            ..empty_result()
        });
        *assistant.confirm_result.lock().unwrap() = Some(AssistantRunResult {
            mutation: Some(AssistantMutationSummary {
                mutation_id: "confirmed".into(),
                affected_documents: Vec::new(),
                undo_token: "undo-confirmed".into(),
                uncovered_segment_ids: Vec::new(),
            }),
            ..empty_result()
        });
        let (view, cx) = open_desktop_with(cx, assistant.clone(), stored_key_settings());
        start_run(cx, &view);
        wait_for(cx, &view, |root| root.assistant.pending.is_some());
        assert_eq!(assistant.grants_for("run"), [vec!["first".to_string()]]);

        view.update(cx, |root, cx| {
            root.select_assistant_scope(AssistantScopeMode::AllVisibleDocuments, cx)
        });
        assert_eq!(
            view.read_with(cx, |root, _| root.assistant.authorized_documents.len()),
            2
        );

        view.update_in(cx, |root, window, cx| {
            root.accept_assistant_pending(&gpui::ClickEvent::default(), window, cx)
        });
        wait_for(cx, &view, |root| {
            matches!(
                &root.assistant.pending,
                Some(PendingReviewState::Awaiting(pending)) if pending.confirmation_id == "reject-me"
            )
        });
        view.update_in(cx, |root, window, cx| {
            root.reject_assistant_pending(&gpui::ClickEvent::default(), window, cx)
        });
        wait_for(cx, &view, |root| {
            root.assistant.pending == Some(PendingReviewState::Rejected)
        });
        view.update(cx, |root, cx| {
            root.undo_assistant_mutation_for("confirmed".into(), cx)
        });
        wait_for(cx, &view, |root| {
            root.assistant.is_mutation_undone("confirmed")
        });

        let original = vec![vec!["first".to_string()]];
        assert_eq!(assistant.grants_for("confirm"), original);
        assert_eq!(assistant.grants_for("reject"), original);
        assert_eq!(assistant.grants_for("undo"), original);
    }

    fn start_pending_run(
        cx: &mut TestAppContext,
        count: usize,
        expires_at_ms: u64,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        let assistant = Arc::new(FakeAssistantUiService::default());
        *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
            pending_reviews: (0..count)
                .map(|index| pending_review(&format!("confirmation-{index}"), expires_at_ms))
                .collect(),
            ..empty_result()
        });
        let (view, cx) = open_desktop_with(cx, assistant, stored_key_settings());
        start_run(cx, &view);
        wait_for(cx, &view, |root| root.assistant.pending.is_some());
        (view, cx)
    }

    #[gpui::test]
    fn pending_overflow_is_shown_on_the_page(cx: &mut TestAppContext) {
        let (_view, cx) = start_pending_run(cx, 10, u64::MAX);
        assert!(
            cx.debug_bounds("assistant-pending-overflow").is_some(),
            "overflowing pending reviews must be reported on the page"
        );
    }

    #[gpui::test]
    fn pending_expiry_is_driven_by_a_timer(cx: &mut TestAppContext) {
        let expires_at_ms = crate::assistant_ui::now_ms() + 60_000;
        let (view, cx) = start_pending_run(cx, 1, expires_at_ms);

        cx.executor()
            .advance_clock(std::time::Duration::from_secs(59));
        assert!(view.read_with(cx, |root, _| matches!(
            root.assistant.pending,
            Some(PendingReviewState::Awaiting(_))
        )));
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        assert_eq!(
            view.read_with(cx, |root, _| root.assistant.pending.clone()),
            Some(PendingReviewState::Expired)
        );
    }

    fn click_save_assistant_settings(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
    ) {
        view.update_in(cx, |root, window, cx| {
            root.save_assistant_settings(&gpui::ClickEvent::default(), window, cx)
        });
    }

    fn rename_connection(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
        name: &str,
    ) {
        view.update(cx, |root, cx| {
            root.assistant_inputs
                .connection_name
                .update(cx, |input, _| input.set_value(name))
        });
    }

    #[gpui::test]
    fn saving_a_profile_with_an_empty_secret_keeps_the_stored_key(cx: &mut TestAppContext) {
        let settings = stored_key_settings();
        let assistant = Arc::new(FakeAssistantUiService {
            settings: Some(settings.clone()),
            ..Default::default()
        });
        let (view, cx) = open_desktop_with(cx, assistant, settings.clone());
        rename_connection(cx, &view, "Renamed");
        click_save_assistant_settings(cx, &view);
        wait_for(cx, &view, |_| {
            settings.snapshot().unwrap().assistant.connections[0].name == "Renamed"
        });

        let saved = settings.snapshot().unwrap().assistant.connections[0].clone();
        assert_eq!(saved.api_key.as_deref(), Some(STORED_KEY));
        assert_eq!(
            view.read_with(cx, |root, cx| root
                .assistant_inputs
                .api_key
                .read(cx)
                .value()),
            ""
        );
    }

    #[gpui::test]
    fn loading_a_profile_never_puts_the_stored_key_into_the_secret_field(cx: &mut TestAppContext) {
        let (view, cx) = open_desktop_with(
            cx,
            Arc::new(FakeAssistantUiService::default()),
            stored_key_settings(),
        );
        view.update(cx, |root, cx| root.edit_assistant_profile("local", cx));

        view.read_with(cx, |root, cx| {
            assert_eq!(root.assistant_inputs.api_key.read(cx).value(), "");
            assert_eq!(
                root.assistant_inputs.connection_name.read(cx).value(),
                "Local"
            );
        });
    }

    #[gpui::test]
    fn a_stale_assistant_save_conflicts_and_succeeds_after_the_reload(cx: &mut TestAppContext) {
        let settings = stored_key_settings();
        let assistant = Arc::new(FakeAssistantUiService {
            settings: Some(settings.clone()),
            ..Default::default()
        });
        let (view, cx) = open_desktop_with(cx, assistant, settings.clone());
        view.update(cx, |root, cx| root.navigate(route::Route::Settings, cx));
        let initial_notice = view.read_with(cx, |root, _| root.ui.notice.clone());
        external_patch(
            &settings,
            settings::SettingsPatch {
                write_mode: Some(contracts::AgentWriteMode::ConfirmEach),
                ..Default::default()
            },
        );

        rename_connection(cx, &view, "Renamed");
        click_save_assistant_settings(cx, &view);
        wait_for(cx, &view, |root| root.ui.notice != initial_notice);

        let persisted = settings.snapshot().unwrap().assistant;
        assert_eq!(
            persisted.write_mode,
            contracts::AgentWriteMode::ConfirmEach,
            "the external change must survive a stale save"
        );
        assert_eq!(persisted.connections[0].name, "Local");
        let error = view
            .read_with(cx, |root, _| root.settings_error.clone())
            .expect("the conflict must be reported");
        assert_eq!(error.code, "SETTINGS_REVISION_CONFLICT");
        assert!(error.retryable);
        assert!(cx.debug_bounds("settings-error").is_some());

        click_save_assistant_settings(cx, &view);
        wait_for(cx, &view, |root| root.settings_error.is_none());
        let persisted = settings.snapshot().unwrap().assistant;
        assert_eq!(persisted.connections[0].name, "Renamed");
        assert_eq!(
            persisted.connections[0].api_key.as_deref(),
            Some(STORED_KEY)
        );
        assert_eq!(persisted.write_mode, contracts::AgentWriteMode::ConfirmEach);
    }

    #[gpui::test]
    fn a_stale_instant_save_conflicts_and_succeeds_after_the_reload(cx: &mut TestAppContext) {
        let settings = test_settings();
        let (view, cx) = open_desktop_with(
            cx,
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        external_patch(
            &settings,
            settings::SettingsPatch {
                max_tool_rounds: Some(5),
                ..Default::default()
            },
        );
        let asr_backend = |settings: &SettingsService| settings.snapshot().unwrap().asr_backend_id;

        view.update(cx, |root, cx| {
            root.select_asr_backend("test-asr".into(), cx)
        });
        wait_for(cx, &view, |root| {
            root.settings_error.is_some() || asr_backend(&settings).as_deref() == Some("test-asr")
        });

        assert_eq!(
            settings.snapshot().unwrap().assistant.max_tool_rounds,
            5,
            "the external change must survive a stale save"
        );
        assert_ne!(asr_backend(&settings).as_deref(), Some("test-asr"));
        let error = view
            .read_with(cx, |root, _| root.settings_error.clone())
            .expect("the conflict must be reported");
        assert_eq!(error.code, "SETTINGS_REVISION_CONFLICT");
        assert!(error.retryable);

        view.update(cx, |root, cx| {
            root.select_asr_backend("test-asr".into(), cx)
        });
        wait_for(cx, &view, |root| {
            root.settings_error.is_none() && asr_backend(&settings).as_deref() == Some("test-asr")
        });
        assert_eq!(settings.snapshot().unwrap().assistant.max_tool_rounds, 5);
    }

    #[gpui::test]
    fn an_instant_save_does_not_persist_the_unsaved_assistant_draft(cx: &mut TestAppContext) {
        let settings = test_settings();
        let (view, cx) = open_desktop_with(
            cx,
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        view.update(cx, |root, cx| {
            root.set_assistant_enabled(false, cx);
            root.set_assistant_write_mode(contracts::AgentWriteMode::ConfirmEach, cx);
            root.select_asr_backend("test-asr".into(), cx);
        });
        wait_for(cx, &view, |_| {
            settings.snapshot().unwrap().asr_backend_id.as_deref() == Some("test-asr")
        });

        let persisted = settings.snapshot().unwrap().assistant;
        assert_eq!(persisted.write_mode, contracts::AgentWriteMode::Automatic);
        assert!(persisted.enabled);
        view.read_with(cx, |root, _| {
            assert_eq!(
                root.settings_draft.assistant.write_mode,
                contracts::AgentWriteMode::ConfirmEach
            );
            assert!(!root.settings_draft.assistant.enabled);
        });
    }

    /// gpui-component's resolved colors except the window background.
    fn component_colors_without_background(cx: &mut gpui::VisualTestContext) -> String {
        cx.update(|_, cx| {
            let theme = gpui_component::Theme::global(cx);
            let mut colors = theme.colors;
            colors.background = gpui::Hsla::default();
            let mut tokens = theme.tokens;
            tokens.background = gpui::Hsla::default().into();
            format!("{colors:?}\n{tokens:?}")
        })
    }

    #[track_caller]
    fn assert_material(cx: &mut gpui::VisualTestContext, material: theme::Material) {
        cx.update(|_, cx| {
            let component = gpui_component::Theme::global(cx);
            let (alpha, background) = match material {
                theme::Material::Glass => (0., gpui::WindowBackgroundAppearance::Blurred),
                theme::Material::Opaque => (1., gpui::WindowBackgroundAppearance::Opaque),
            };
            assert_eq!(theme::tokens(cx).material, material);
            assert_eq!(
                component.background.a, alpha,
                "{material:?} component background"
            );
            assert_eq!(component.tokens.background.color.a, alpha, "Root's fill");
            assert_eq!(theme::window_background(cx), background);
        });
    }

    #[gpui::test]
    fn glass_clears_the_component_background_and_blurs_the_window(cx: &mut TestAppContext) {
        // Saved settings default to the glass material.
        let services =
            desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
        let (_view, cx) = open_main_window(cx, services);
        assert_material(cx, theme::Material::Glass);

        for theme in [
            settings::ThemePreference::Light,
            settings::ThemePreference::Dark,
        ] {
            let mut colors = Vec::new();
            for material in [
                settings::MaterialPreference::Glass,
                settings::MaterialPreference::Opaque,
            ] {
                cx.update(|_, cx| {
                    theme::set_preference(settings::AppearanceSettings { theme, material }, cx)
                });
                cx.run_until_parked();
                assert_material(
                    cx,
                    match material {
                        settings::MaterialPreference::Glass => theme::Material::Glass,
                        settings::MaterialPreference::Opaque => theme::Material::Opaque,
                    },
                );
                colors.push(component_colors_without_background(cx));
            }
            assert_eq!(
                colors[0], colors[1],
                "{theme:?}: the material may change only the background gpui-component paints"
            );
        }
    }

    #[gpui::test]
    fn appearance_selects_save_with_revision_checks_and_reapply_the_theme(cx: &mut TestAppContext) {
        let settings = test_settings();
        let services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        let (view, cx) = open_main_window(cx, services);
        view.update(cx, |root, cx| root.navigate(route::Route::Settings, cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("select-theme").is_some());
        assert!(cx.debug_bounds("select-material").is_some());
        let select = |cx: &mut gpui::VisualTestContext, material: bool| {
            view.read_with(cx, |root, _| {
                let selectors = root
                    .settings_selectors
                    .as_ref()
                    .expect("selectors are built on render");
                if material {
                    selectors.material.clone()
                } else {
                    selectors.theme.clone()
                }
            })
        };
        let confirm = |cx: &mut gpui::VisualTestContext, material: bool, label: &str| {
            select(cx, material).update(cx, |_, cx| {
                cx.emit(gpui_component::select::SelectEvent::Confirm(Some(
                    label.to_string(),
                )))
            })
        };
        let appearance = |settings: &SettingsService| settings.snapshot().unwrap().appearance;
        assert_eq!(
            select(cx, false).read_with(cx, |state, _| state.selected_value().cloned()),
            Some("跟随系统".to_string())
        );

        confirm(cx, false, "深色");
        wait_for(cx, &view, |_| {
            appearance(&settings).theme == settings::ThemePreference::Dark
        });
        cx.update(|_, cx| {
            assert_eq!(theme::tokens(cx).mode, theme::Mode::Dark);
            assert!(gpui_component::Theme::global(cx).is_dark());
        });
        assert_material(cx, theme::Material::Glass);

        // An external change makes the next save stale: it must conflict,
        // keep the saved material and re-apply it.
        external_patch(
            &settings,
            settings::SettingsPatch {
                max_tool_rounds: Some(5),
                ..Default::default()
            },
        );
        confirm(cx, true, "不透明");
        wait_for(cx, &view, |root| root.settings_error.is_some());
        let error = view.read_with(cx, |root, _| root.settings_error.clone().unwrap());
        assert_eq!(error.code, "SETTINGS_REVISION_CONFLICT");
        assert_eq!(
            appearance(&settings).material,
            settings::MaterialPreference::Glass
        );
        assert_material(cx, theme::Material::Glass);

        cx.run_until_parked();
        confirm(cx, true, "不透明");
        wait_for(cx, &view, |root| {
            root.settings_error.is_none()
                && appearance(&settings).material == settings::MaterialPreference::Opaque
        });
        assert_material(cx, theme::Material::Opaque);
        assert_eq!(
            appearance(&settings),
            settings::AppearanceSettings {
                theme: settings::ThemePreference::Dark,
                material: settings::MaterialPreference::Opaque,
            }
        );
        assert_eq!(settings.snapshot().unwrap().assistant.max_tool_rounds, 5);
    }

    #[gpui::test]
    fn the_system_preference_follows_the_window_appearance(cx: &mut TestAppContext) {
        let services =
            desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
        let (_view, cx) = open_main_window(cx, services);
        let mode = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                let mode = theme::tokens(cx).mode;
                assert_eq!(
                    gpui_component::Theme::global(cx).is_dark(),
                    mode == theme::Mode::Dark
                );
                mode
            })
        };
        // gpui's test window is always light.
        assert_eq!(mode(cx), theme::Mode::Light);

        cx.update(|window, cx| {
            theme::system_appearance_changed(gpui::WindowAppearance::Dark, window, cx)
        });
        assert_eq!(mode(cx), theme::Mode::Dark);
        assert_material(cx, theme::Material::Glass);

        cx.update(|_, cx| {
            theme::set_preference(
                settings::AppearanceSettings {
                    theme: settings::ThemePreference::Light,
                    ..Default::default()
                },
                cx,
            )
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            theme::system_appearance_changed(gpui::WindowAppearance::VibrantDark, window, cx)
        });
        assert_eq!(
            mode(cx),
            theme::Mode::Light,
            "a pinned theme ignores the system"
        );
    }

    #[gpui::test]
    fn changing_the_locale_rebuilds_the_localized_selects(cx: &mut TestAppContext) {
        let (view, cx) = open_desktop(cx, Arc::new(FakeAssistantUiService::default()));
        let selected = |cx: &mut gpui::VisualTestContext| {
            view.read_with(cx, |root, cx| {
                let selectors = root
                    .settings_selectors
                    .as_ref()
                    .expect("selectors are built on render");
                [
                    &selectors.compute_device,
                    &selectors.transcription_language,
                    &selectors.expected_speakers,
                ]
                .map(|select| select.read(cx).selected_value().cloned())
            })
        };
        cx.run_until_parked();
        assert_eq!(
            selected(cx),
            [
                Some("自动".to_string()),
                Some("自动".into()),
                Some("自动".into())
            ]
        );

        let language = view.read_with(cx, |root, _| {
            root.settings_selectors.as_ref().unwrap().language.clone()
        });
        language.update(cx, |_, cx| {
            cx.emit(gpui_component::select::SelectEvent::Confirm(Some(
                "English".to_string(),
            )))
        });
        cx.run_until_parked();
        assert_eq!(
            selected(cx),
            [
                Some("Auto".to_string()),
                Some("Auto".into()),
                Some("Auto".into())
            ]
        );
    }
}
