pub mod assistant_ui;
mod components;
mod controller;
mod controls;
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

pub use controller::{run, DesktopServices, ModelService};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_ui::{
        AssistantCancellation, AssistantJobEvent, AssistantMutationSummary, AssistantPendingReview,
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
        /// The request id of every run.
        run_request_ids: Arc<Mutex<Vec<String>>>,
        /// Holds each run until released, when set.
        run_gate: Option<Gate>,
        /// Subscribers of `job_events`; tests publish through `publish_job`.
        job_subscribers:
            Arc<Mutex<Vec<futures::channel::mpsc::UnboundedSender<AssistantJobEvent>>>>,
    }

    impl FakeAssistantUiService {
        fn publish_job(&self, request_id: Option<&str>, event: runtime::JobEvent) {
            for subscriber in self.job_subscribers.lock().unwrap().iter() {
                subscriber
                    .unbounded_send(AssistantJobEvent {
                        request_id: request_id.map(str::to_string),
                        event: event.clone(),
                    })
                    .unwrap();
            }
        }

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
            self.run_request_ids
                .lock()
                .unwrap()
                .push(request.request_id.clone());
            if let Some(gate) = &self.run_gate {
                gate.pass();
            }
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

        fn job_events(&self) -> futures::channel::mpsc::UnboundedReceiver<AssistantJobEvent> {
            let (sender, receiver) = futures::channel::mpsc::unbounded();
            self.job_subscribers.lock().unwrap().push(sender);
            receiver
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

    fn test_model_manager() -> ModelManager {
        ModelManager::new(
            ModelCatalog::from_path(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../resources/models/catalog.json"),
            )
            .unwrap(),
            "models",
        )
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
        let model_manager = test_model_manager();
        DesktopServices {
            workspace,
            settings,
            installation: application::InstallationService::new(model_manager.clone()),
            model_manager: Arc::new(model_manager),
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
        open_desktop_on(cx, desktop_services(assistant, settings))
    }

    /// The desktop once its startup project list has loaded, so a test's
    /// own project list is not replaced by it.
    fn open_desktop_on(
        cx: &mut TestAppContext,
        services: DesktopServices,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        let (view, cx) = open_desktop_loading(cx, services);
        wait_for(cx, &view, |root| root.projects_loaded);
        (view, cx)
    }

    /// The desktop as soon as its window opens.
    fn open_desktop_loading(
        cx: &mut TestAppContext,
        services: DesktopServices,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
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

    /// The desktop window exactly as `run` opens it, once its startup project
    /// list has loaded, so a test's own project list is not replaced by it.
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
        wait_for(cx, &view, |root| root.projects_loaded);
        (view, cx)
    }

    #[gpui::test]
    fn rendering_meetings_and_models_does_not_probe_installation(cx: &mut TestAppContext) {
        // The real probe, counted.
        let mut services =
            desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
        let manager = test_model_manager();
        let real = application::InstallationService::new(manager.clone());
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = probes.clone();
        services.installation = application::InstallationService::with_probe(
            manager,
            Arc::new(move |query| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                real.probe(query)
            }),
        );
        let (view, cx) = open_desktop_on(cx, services);
        wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
        // Count from after the startup probe.
        probes.store(0, std::sync::atomic::Ordering::SeqCst);

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

        view.read_with(cx, |root, _| root.refresh_installation());
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
                let (changes, receiver) = futures::channel::mpsc::unbounded();
                root.follow_document_changes(receiver, cx);
                let (downloads, events) = futures::channel::mpsc::unbounded();
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
            .unbounded_send(application::DocumentChange::Changed)
            .unwrap();
        wait_for(cx, &view, |root| root.projects.len() == 1);
        senders[1]
            .1
            .unbounded_send(models::manager::DownloadEvent::Updated(
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

    /// Serializes the tests that hold a `runtime::spawn_blocking` worker in a
    /// gate: every test in the process shares that pool of at most four
    /// workers, and a held test still needs a free one for its settings save.
    fn hold_a_worker_alone() -> std::sync::MutexGuard<'static, ()> {
        static HELD: Mutex<()> = Mutex::new(());
        HELD.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Holds the calls that pass it while armed, until released.
    #[derive(Clone, Default)]
    struct Gate(Arc<(Mutex<GateState>, std::sync::Condvar)>);

    #[derive(Default)]
    struct GateState {
        armed: bool,
        /// Calls that entered since the gate was armed.
        entered: usize,
        /// Calls waiting in the gate now.
        holding: usize,
    }

    impl Gate {
        fn arm(&self) {
            let mut state = self.0 .0.lock().unwrap();
            state.armed = true;
            state.entered = 0;
        }

        /// Lets the held calls go and waits until each has left, so arming
        /// the gate again cannot catch a call that was already released.
        #[track_caller]
        fn release(&self) {
            let (lock, changed) = &*self.0;
            let mut state = lock.lock().unwrap();
            state.armed = false;
            changed.notify_all();
            let (state, left) = changed
                .wait_timeout_while(state, std::time::Duration::from_secs(5), |state| {
                    state.holding > 0
                })
                .unwrap();
            drop(state);
            assert!(!left.timed_out(), "a released call did not leave the gate");
        }

        fn counts(&self) -> (usize, usize) {
            let state = self.0 .0.lock().unwrap();
            (state.entered, state.holding)
        }

        /// Waits while armed. The wait is bounded, so a call made on the UI
        /// thread stalls the test briefly and then fails it instead of
        /// hanging it.
        fn pass(&self) {
            let (lock, released) = &*self.0;
            let mut state = lock.lock().unwrap();
            if !state.armed {
                return;
            }
            state.entered += 1;
            state.holding += 1;
            let (mut state, _) = released
                .wait_timeout_while(state, std::time::Duration::from_secs(2), |state| {
                    state.armed
                })
                .unwrap();
            state.holding -= 1;
            // `release` waits for the held calls to leave.
            released.notify_all();
        }
    }

    /// Documents in memory whose `snapshot`, which `list_projects` reads,
    /// waits in `list_gate`, and whose `get` waits in `get_gate`.
    struct GatedRepository {
        inner: InMemoryDocumentRepository,
        list_gate: Gate,
        get_gate: Gate,
    }

    impl application::DocumentRepository for GatedRepository {
        fn snapshot(&self) -> Result<application::DocumentSnapshot, String> {
            self.list_gate.pass();
            self.inner.snapshot()
        }

        fn get(&self, document_id: &domain::DocumentId) -> Result<Option<DocumentState>, String> {
            self.get_gate.pass();
            self.inner.get(document_id)
        }

        fn read(
            &self,
            document_id: &domain::DocumentId,
        ) -> Result<Option<contracts::DocumentRecord>, String> {
            self.inner.read(document_id)
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<application::DocumentChange>, String> {
            self.inner.watch()
        }

        fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
            self.inner.save(document, markdown)
        }
    }

    /// A workspace on `inner` whose list and get calls wait in the gates.
    fn gated_workspace(
        inner: InMemoryDocumentRepository,
        list_gate: &Gate,
        get_gate: &Gate,
    ) -> WorkspaceService {
        WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            DocumentService::new(Arc::new(GatedRepository {
                inner,
                list_gate: list_gate.clone(),
                get_gate: get_gate.clone(),
            })),
        )
    }

    /// The real catalog and download list; `model_ref` and `start` wait in
    /// `gate` and then succeed, the download completing at once.
    struct GatedModels {
        inner: ModelManager,
        gate: Gate,
    }

    impl ModelService for GatedModels {
        fn catalog(&self) -> &ModelCatalog {
            self.inner.catalog()
        }

        fn list(
            &self,
        ) -> Result<Vec<models::manager::DownloadProgress>, models::manager::ModelManagerError>
        {
            self.inner.list()
        }

        fn model_ref(
            &self,
            model_id: &str,
            backend_id: &str,
        ) -> Result<ModelRef, models::manager::ModelManagerError> {
            self.gate.pass();
            Ok(ModelRef {
                backend_id: backend_id.into(),
                model_id: model_id.into(),
                revision: None,
                artifact_path: "gated-model.bin".into(),
                artifacts: Vec::new(),
            })
        }

        fn start(
            &self,
            model_id: &str,
        ) -> Result<models::manager::DownloadTask, models::manager::ModelManagerError> {
            self.gate.pass();
            let (sender, updates) = futures::channel::mpsc::unbounded();
            sender
                .unbounded_send(models::manager::DownloadEvent::Updated(
                    models::manager::DownloadProgress {
                        id: models::manager::DownloadId(9),
                        model_id: model_id.into(),
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
            Ok(models::manager::DownloadTask {
                id: models::manager::DownloadId(9),
                events: std::sync::mpsc::channel().1,
                updates,
            })
        }

        fn pause(
            &self,
            id: models::manager::DownloadId,
        ) -> Result<(), models::manager::ModelManagerError> {
            self.inner.pause(id)
        }

        fn resume(
            &self,
            id: models::manager::DownloadId,
        ) -> Result<(), models::manager::ModelManagerError> {
            self.inner.resume(id)
        }

        fn cancel(
            &self,
            id: models::manager::DownloadId,
        ) -> Result<(), models::manager::ModelManagerError> {
            self.inner.cancel(id)
        }
    }

    /// While `gate` holds the one call the UI just started, navigating and
    /// an instant settings save still complete; then the call is released.
    #[track_caller]
    fn assert_ui_responsive_while_held(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
        gate: &Gate,
        settings: &SettingsService,
        backend: &str,
    ) {
        wait_for(cx, view, |_| gate.counts().0 >= 1);
        assert_eq!(
            gate.counts(),
            (1, 1),
            "the gated call already returned: it held the UI thread"
        );
        view.update(cx, |root, cx| {
            root.navigate(route::Route::Settings, cx);
            root.select_diarization_backend(backend.into(), cx);
        });
        wait_for(cx, view, |root| {
            root.ui.route == route::Route::Settings
                && settings
                    .snapshot()
                    .unwrap()
                    .diarization_backend_id
                    .as_deref()
                    == Some(backend)
        });
        assert_eq!(gate.counts().1, 1, "the gated call is still pending");
        view.update(cx, |root, cx| root.navigate(route::Route::Meetings, cx));
        gate.release();
    }

    #[gpui::test]
    fn project_list_reloads_leave_the_ui_thread_free(cx: &mut TestAppContext) {
        let _alone = hold_a_worker_alone();
        let gate = Gate::default();
        let inner = InMemoryDocumentRepository::default();
        let mut merged = DocumentState::new(ProjectId::new("merge"), "Merge", "merge.wav");
        merged.results.speaker_segments = vec![domain::SpeakerTaggedSegment {
            transcript_segment_id: "t-1".into(),
            speaker_id: "speaker-1".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "merge me".into(),
        }];
        application::DocumentRepository::save(&inner, merged, None).unwrap();
        let settings = test_settings();
        let mut services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        services.workspace = gated_workspace(inner.clone(), &gate, &Gate::default());
        let (view, cx) = open_desktop_on(cx, services);
        assert_eq!(view.read_with(cx, |root, _| root.projects.len()), 1);
        let (changes, receiver) = futures::channel::mpsc::unbounded();
        view.update(cx, |root, cx| root.follow_document_changes(receiver, cx));

        // Another writer adds a document to the directory.
        let external = DocumentState::new(ProjectId::new("external"), "External", "external.wav");
        application::DocumentRepository::save(&inner, external, None).unwrap();
        gate.arm();
        changes
            .unbounded_send(application::DocumentChange::Changed)
            .unwrap();
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-change");
        wait_for(cx, &view, |root| root.projects.len() == 2);

        // An import.
        gate.arm();
        view.update(cx, |root, cx| {
            root.import_audio_path("imported.wav".into(), cx)
        });
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-import");
        wait_for(cx, &view, |root| root.projects.len() == 3);

        // A speaker merge.
        gate.arm();
        view.update_in(cx, |root, window, cx| {
            root.ui.selected_project = Some(ProjectId::new("merge"));
            root.merge_speaker(&gpui::ClickEvent::default(), window, cx)
        });
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-merge");
        let merged = i18n::text(i18n::Locale::ZhCn, "speaker_merged");
        wait_for(cx, &view, |root| root.ui.notice == merged);
    }

    /// A repository holding the `first` document.
    fn repository_with_first_document() -> InMemoryDocumentRepository {
        let inner = InMemoryDocumentRepository::default();
        let first = DocumentState::new(ProjectId::new("first"), "First", "first.wav");
        application::DocumentRepository::save(&inner, first, None).unwrap();
        inner
    }

    #[gpui::test]
    fn the_startup_project_list_loads_off_the_ui_thread(cx: &mut TestAppContext) {
        let _alone = hold_a_worker_alone();
        let gate = Gate::default();
        let settings = test_settings();
        let mut services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        services.workspace =
            gated_workspace(repository_with_first_document(), &gate, &Gate::default());
        gate.arm();

        let (view, cx) = open_desktop_loading(cx, services);
        assert!(!view.read_with(cx, |root, _| root.projects_loaded));
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-startup");
        wait_for(cx, &view, |root| root.projects_loaded);

        view.read_with(cx, |root, _| {
            let first = ProjectId::new("first");
            assert_eq!(root.projects.len(), 1);
            assert_eq!(root.ui.selected_project.as_ref(), Some(&first));
            assert!(root.ui.expanded_projects.contains(&first));
        });
    }

    #[gpui::test]
    fn starting_processing_leaves_the_ui_thread_free(cx: &mut TestAppContext) {
        let _alone = hold_a_worker_alone();
        let gate = Gate::default();
        // Selected models that are not installed.
        let settings = test_settings();
        let mut value = settings.snapshot().unwrap();
        let missing = |backend_id: &str| ModelRef {
            backend_id: backend_id.into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "missing-model".into(),
            artifacts: Vec::new(),
        };
        value.asr_model = Some(missing("whisper-rs"));
        value.diarization_backend_id = Some("sherpa-onnx".into());
        value.diarization_model = Some(missing("sherpa-onnx"));
        settings.update(value).unwrap();
        let mut services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        services.workspace =
            gated_workspace(repository_with_first_document(), &Gate::default(), &gate);
        let (view, cx) = open_desktop_on(cx, services);

        // Starting reads the document the job processes.
        gate.arm();
        view.update_in(cx, |root, window, cx| {
            root.process_selected(&gpui::ClickEvent::default(), window, cx)
        });
        assert!(view.read_with(cx, |root, _| root.ui.processing));
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-start");
        // The fixture has no model, so the job fails and processing ends.
        wait_for(cx, &view, |root| !root.ui.processing);
    }

    #[gpui::test]
    fn model_selection_and_download_start_leave_the_ui_thread_free(cx: &mut TestAppContext) {
        let _alone = hold_a_worker_alone();
        let gate = Gate::default();
        let settings = test_settings();
        let mut services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        services.model_manager = Arc::new(GatedModels {
            inner: test_model_manager(),
            gate: gate.clone(),
        });
        let (view, cx) = open_desktop_on(cx, services);

        gate.arm();
        view.update(cx, |root, cx| {
            root.use_model("gated-model".into(), "asr".into(), "gated-asr".into(), cx)
        });
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-model-ref");
        wait_for(cx, &view, |_| {
            settings
                .snapshot()
                .unwrap()
                .asr_model
                .is_some_and(|model| model.model_id == "gated-model")
        });

        gate.arm();
        view.update(cx, |root, cx| root.download_model("gated-model".into(), cx));
        assert_ui_responsive_while_held(cx, &view, &gate, &settings, "dia-during-start");
        let installed = i18n::message(i18n::Locale::ZhCn, "installed", "gated-model");
        wait_for(cx, &view, |root| root.ui.notice == installed);
    }

    /// A desktop whose Speaker Library lives in a fresh directory, asked to
    /// move it to a path without a library: the migration dialog opens.
    /// Returns the view, the services, the directory and both paths.
    fn open_speaker_library_prompt(
        cx: &mut TestAppContext,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
        DesktopServices,
        [std::path::PathBuf; 3],
    ) {
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
        wait_for(cx, &view, |root| root.speaker_library_prompt.is_some());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("confirm-dialog").is_some(),
            "the migration dialog is shown"
        );
        (view, cx, services, [directory, source, destination])
    }

    #[track_caller]
    fn click_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` is not rendered"));
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_speaker_library_migration_dialog_opens_and_migrates(cx: &mut TestAppContext) {
        let (view, cx, services, [directory, _, destination]) = open_speaker_library_prompt(cx);
        // The scrim keeps presses from reaching the page behind the dialog.
        let route = view.read_with(cx, |root, _| root.ui.route);
        assert_ne!(route, route::Route::Models);
        click_selector(cx, "nav-Models");
        assert_eq!(view.read_with(cx, |root, _| root.ui.route), route);
        assert!(cx.debug_bounds("confirm-dialog").is_some());

        click_selector(cx, "dialog-confirm");
        let expected = destination.display().to_string();
        wait_for(cx, &view, |_| {
            services.settings.snapshot().unwrap().speaker_library_path == expected
        });
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "{}");
        assert!(
            cx.debug_bounds("confirm-dialog").is_none(),
            "the dialog closed"
        );
        std::fs::remove_dir_all(directory).ok();
    }

    #[gpui::test]
    fn cancelling_the_migration_dialog_switches_the_path_without_migrating(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, services, [directory, source, destination]) =
            open_speaker_library_prompt(cx);

        click_selector(cx, "dialog-cancel");
        let expected = destination.display().to_string();
        wait_for(cx, &view, |_| {
            services.settings.snapshot().unwrap().speaker_library_path == expected
        });
        assert!(!destination.exists(), "cancel must not migrate the library");
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "{}");
        assert!(
            cx.debug_bounds("confirm-dialog").is_none(),
            "the dialog closed"
        );
        std::fs::remove_dir_all(directory).ok();
    }

    /// Answers the migration dialog with `key` alone: the dialog holds
    /// focus from the moment it opens.
    fn answer_migration_with_key(cx: &mut TestAppContext, key: &str, migrates: bool) {
        let (view, cx, services, [directory, source, destination]) =
            open_speaker_library_prompt(cx);

        cx.simulate_keystrokes(key);
        let expected = destination.display().to_string();
        wait_for(cx, &view, |_| {
            services.settings.snapshot().unwrap().speaker_library_path == expected
        });
        assert_eq!(destination.exists(), migrates, "{key} migrates: {migrates}");
        assert!(source.exists());
        assert!(view.read_with(cx, |root, _| root.speaker_library_prompt.is_none()));
        std::fs::remove_dir_all(directory).ok();
    }

    #[gpui::test]
    fn enter_confirms_the_migration_dialog(cx: &mut TestAppContext) {
        answer_migration_with_key(cx, "enter", true);
    }

    #[gpui::test]
    fn escape_cancels_the_migration_dialog(cx: &mut TestAppContext) {
        answer_migration_with_key(cx, "escape", false);
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
        let (download, events) = futures::channel::mpsc::unbounded();
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
            .unbounded_send(models::manager::DownloadEvent::Updated(progress))
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
    fn job_events_of_the_run_drive_the_assistant_page(cx: &mut TestAppContext) {
        let _alone = hold_a_worker_alone();
        let gate = Gate::default();
        let assistant = Arc::new(FakeAssistantUiService {
            run_gate: Some(gate.clone()),
            ..Default::default()
        });
        let (view, cx) = open_desktop_with(cx, assistant.clone(), stored_key_settings());
        gate.arm();
        start_run(cx, &view);
        wait_for(cx, &view, |_| gate.counts().0 == 1);
        let request_id = assistant.run_request_ids.lock().unwrap()[0].clone();
        let run = Some(request_id.as_str());
        let job_id = runtime::JobId(7);
        let task_shown = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            cx.run_until_parked();
            cx.debug_bounds(selector).is_some()
        };

        assistant.publish_job(run, runtime::JobEvent::Started { job_id });
        // A probe's job and another run's job are not this run's.
        assistant.publish_job(
            None,
            runtime::JobEvent::Failed {
                job_id: runtime::JobId(8),
                message: "probe failed".into(),
            },
        );
        assistant.publish_job(
            Some("another-run"),
            runtime::JobEvent::Cancelled {
                job_id: runtime::JobId(9),
            },
        );
        wait_for(cx, &view, |root| root.assistant.job == Some(job_id));
        assert!(task_shown(cx, "assistant-task-running"));
        assert!(view.read_with(cx, |root, _| matches!(
            root.assistant.task,
            AssistantTaskState::Running { .. }
        )));

        let progress = runtime::Progress {
            completed: 1,
            total: Some(3),
        };
        assistant.publish_job(run, runtime::JobEvent::Progress { job_id, progress });
        wait_for(cx, &view, |root| {
            root.assistant.job_progress == Some(progress)
        });
        assert!(cx.debug_bounds("assistant-job-progress").is_some());

        assistant.publish_job(
            run,
            runtime::JobEvent::Failed {
                job_id,
                message: "worker failed".into(),
            },
        );
        wait_for(cx, &view, |root| {
            matches!(root.assistant.task, AssistantTaskState::Failed(_))
        });
        assert!(task_shown(cx, "assistant-task-failed"));
        assert_eq!(
            gate.counts().1,
            1,
            "the job event, not the run's result, ended the task"
        );

        // The run's typed error replaces the job's message.
        gate.release();
        wait_for(cx, &view, |root| {
            matches!(
                &root.assistant.task,
                AssistantTaskState::Failed(error) if error.code == "ASSISTANT_RUNTIME_NOT_CONNECTED"
            )
        });
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

    #[gpui::test]
    fn the_next_queued_review_awaits_after_the_first_expires(cx: &mut TestAppContext) {
        let assistant = Arc::new(FakeAssistantUiService::default());
        *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
            pending_reviews: vec![
                pending_review("expires", crate::assistant_ui::now_ms() + 60_000),
                pending_review("next", u64::MAX),
            ],
            ..empty_result()
        });
        *assistant.confirm_result.lock().unwrap() = Some(empty_result());
        let (view, cx) = open_desktop_with(cx, assistant.clone(), stored_key_settings());
        start_run(cx, &view);
        wait_for(cx, &view, |root| root.assistant.pending.is_some());

        cx.executor()
            .advance_clock(std::time::Duration::from_secs(61));
        assert_eq!(
            view.read_with(cx, |root, _| root.assistant.pending.clone()),
            Some(PendingReviewState::Awaiting(pending_review(
                "next",
                u64::MAX
            ))),
            "the queued review must await once the first expires"
        );

        view.update_in(cx, |root, window, cx| {
            root.accept_assistant_pending(&gpui::ClickEvent::default(), window, cx)
        });
        wait_for(cx, &view, |root| {
            root.assistant.pending.is_none() && root.assistant.task == AssistantTaskState::Completed
        });
        assert_eq!(assistant.grants_for("confirm"), [vec!["first".to_string()]]);
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
    fn a_conflict_reloads_another_writers_settings_so_the_retry_succeeds(cx: &mut TestAppContext) {
        let path = temp_directory().join("settings.json");
        let open = || SettingsService::open(Arc::new(JsonSettingsRepository::open(&path))).unwrap();
        let (view, cx) = open_desktop_with(cx, Arc::new(FakeAssistantUiService::default()), open());
        view.update(cx, |root, cx| root.navigate(route::Route::Settings, cx));
        cx.run_until_parked();
        // Another process changes settings.json; the desktop's snapshot
        // cannot see it.
        let other = open();
        let mut external = other.snapshot().unwrap();
        external.expected_speakers = Some(3);
        other.update(external).unwrap();

        view.update(cx, |root, cx| {
            root.select_asr_backend("test-asr".into(), cx)
        });
        wait_for(cx, &view, |root| root.settings_error.is_some());
        let error = view.read_with(cx, |root, _| root.settings_error.clone().unwrap());
        assert_eq!(error.code, "SETTINGS_REVISION_CONFLICT");
        cx.run_until_parked();
        let shown = view.read_with(cx, |root, cx| {
            root.settings_selectors
                .as_ref()
                .expect("selectors are rebuilt on render")
                .expected_speakers
                .read(cx)
                .selected_value()
                .map(str::to_owned)
        });
        assert_eq!(
            shown.as_deref(),
            Some("3"),
            "the page shows the external value"
        );

        view.update(cx, |root, cx| {
            // A new failure would set the error again.
            root.settings_error = None;
            root.select_asr_backend("test-asr".into(), cx)
        });
        wait_for(cx, &view, |root| {
            root.settings_error.is_some()
                || open().snapshot().unwrap().asr_backend_id.as_deref() == Some("test-asr")
        });
        assert_eq!(
            view.read_with(cx, |root, _| root.settings_error.clone()),
            None,
            "the retry must succeed after the reload"
        );
        let stored = open().snapshot().unwrap();
        assert_eq!(stored.asr_backend_id.as_deref(), Some("test-asr"));
        assert_eq!(stored.expected_speakers, Some(3));
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
                cx.emit(controls::DropdownEvent::Confirm(label.to_string()))
            })
        };
        let appearance = |settings: &SettingsService| settings.snapshot().unwrap().appearance;
        assert_eq!(
            select(cx, false).read_with(cx, |state, _| state.selected_value().map(str::to_owned)),
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
                .map(|select| select.read(cx).selected_value().map(str::to_owned))
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
            cx.emit(controls::DropdownEvent::Confirm("English".to_string()))
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

    /// The Settings route of the desktop window as `run` opens it, once the
    /// startup installation state, which rebuilds every select, has arrived.
    fn open_settings_route(
        cx: &mut TestAppContext,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
        SettingsService,
    ) {
        let settings = test_settings();
        let services = desktop_services(
            Arc::new(FakeAssistantUiService::default()),
            settings.clone(),
        );
        let (view, cx) = open_main_window(cx, services);
        wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
        view.update(cx, |root, cx| root.navigate(route::Route::Settings, cx));
        cx.run_until_parked();
        (view, cx, settings)
    }

    fn settings_select(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<controller::RootView>,
        pick: impl Fn(&controller::SettingsSelectors) -> &gpui::Entity<controls::Dropdown>,
    ) -> gpui::Entity<controls::Dropdown> {
        view.read_with(cx, |root, _| {
            pick(
                root.settings_selectors
                    .as_ref()
                    .expect("selectors are built on render"),
            )
            .clone()
        })
    }

    fn trigger_focused(
        cx: &mut gpui::VisualTestContext,
        select: &gpui::Entity<controls::Dropdown>,
    ) -> bool {
        cx.update(|window, cx| {
            gpui::Focusable::focus_handle(select.read(cx), cx).is_focused(window)
        })
    }

    #[track_caller]
    fn assert_select(
        cx: &mut gpui::VisualTestContext,
        select: &gpui::Entity<controls::Dropdown>,
        open: bool,
        highlighted: Option<&str>,
        selected: &str,
    ) {
        select.read_with(cx, |select, _| {
            assert_eq!(select.is_open(), open, "open");
            assert_eq!(select.highlighted_value(), highlighted, "highlight");
            assert_eq!(select.selected_value(), Some(selected), "selection");
        });
        assert_eq!(
            cx.debug_bounds("select-menu").is_some(),
            open,
            "the menu is rendered exactly while open"
        );
    }

    #[gpui::test]
    fn the_device_select_opens_moves_and_saves_with_the_keyboard(cx: &mut TestAppContext) {
        let (view, cx, settings) = open_settings_route(cx);
        let select = settings_select(cx, &view, |selectors| &selectors.compute_device);
        let revision = settings.revision().unwrap();
        cx.update(|window, cx| {
            gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx)
        });
        cx.run_until_parked();

        cx.simulate_keystrokes("enter");
        assert_select(cx, &select, true, Some("自动"), "自动");
        assert!(!trigger_focused(cx, &select), "the open menu holds focus");
        cx.simulate_keystrokes("down");
        assert_select(cx, &select, true, Some("CPU"), "自动");
        cx.simulate_keystrokes("down down up");
        assert_select(cx, &select, true, Some("Metal"), "自动");

        cx.simulate_keystrokes("enter");
        assert_select(cx, &select, false, None, "Metal");
        assert!(trigger_focused(cx, &select), "focus returns to the trigger");
        wait_for(cx, &view, |_| {
            settings.snapshot().unwrap().compute_device == contracts::ComputeDevice::Metal
        });
        assert_ne!(
            settings.revision().unwrap(),
            revision,
            "a revision-checked save"
        );
        assert!(view.read_with(cx, |root, _| root.settings_error.is_none()));
        assert_eq!(
            view.read_with(cx, |root, _| root.settings_draft.compute_device),
            contracts::ComputeDevice::Metal
        );
    }

    #[gpui::test]
    fn escape_dismisses_the_device_select_without_changing_it(cx: &mut TestAppContext) {
        let (view, cx, settings) = open_settings_route(cx);
        let select = settings_select(cx, &view, |selectors| &selectors.compute_device);
        let revision = settings.revision().unwrap();
        cx.update(|window, cx| {
            gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx)
        });
        cx.run_until_parked();

        // Down opens a closed select on its selection.
        cx.simulate_keystrokes("down");
        assert_select(cx, &select, true, Some("自动"), "自动");
        cx.simulate_keystrokes("down");
        assert_select(cx, &select, true, Some("CPU"), "自动");
        cx.simulate_keystrokes("escape");
        assert_select(cx, &select, false, None, "自动");
        assert!(trigger_focused(cx, &select), "focus returns to the trigger");
        cx.run_until_parked();
        assert_eq!(settings.revision().unwrap(), revision, "nothing is saved");
        assert_eq!(
            settings.snapshot().unwrap().compute_device,
            contracts::ComputeDevice::Auto
        );
    }

    #[gpui::test]
    fn a_click_opens_the_theme_select_and_a_click_outside_closes_it(cx: &mut TestAppContext) {
        let (view, cx, settings) = open_settings_route(cx);
        let select = settings_select(cx, &view, |selectors| &selectors.theme);
        let revision = settings.revision().unwrap();

        click_selector(cx, "select-theme");
        assert_select(cx, &select, true, Some("跟随系统"), "跟随系统");
        let menu = cx.debug_bounds("select-menu").unwrap();
        let trigger = cx.debug_bounds("select-theme").unwrap();
        assert!(
            menu.top() >= trigger.bottom()
                && menu.left() >= trigger.left()
                && menu.right() <= trigger.right()
                && menu.left() - trigger.left() <= gpui::px(1.),
            "the menu opens below the trigger at its width: {menu:?} under {trigger:?}"
        );
        // A press on the page beside the settings title.
        let page = cx.debug_bounds("settings-page").unwrap();
        cx.simulate_click(
            gpui::point(page.right() - gpui::px(24.), page.top() + gpui::px(12.)),
            Modifiers::none(),
        );
        cx.run_until_parked();
        assert_select(cx, &select, false, None, "跟随系统");
        assert!(trigger_focused(cx, &select), "focus returns to the trigger");
        assert_eq!(settings.revision().unwrap(), revision, "nothing is saved");

        // A click on the trigger of an open select only closes it.
        click_selector(cx, "select-theme");
        assert_select(cx, &select, true, Some("跟随系统"), "跟随系统");
        click_selector(cx, "select-theme");
        assert_select(cx, &select, false, None, "跟随系统");

        click_selector(cx, "select-theme");
        click_selector(cx, "select-option-2");
        assert_select(cx, &select, false, None, "深色");
        assert!(trigger_focused(cx, &select), "focus returns to the trigger");
        wait_for(cx, &view, |_| {
            settings.snapshot().unwrap().appearance.theme == settings::ThemePreference::Dark
        });
        cx.update(|_, cx| assert_eq!(theme::tokens(cx).mode, theme::Mode::Dark));
    }

    #[gpui::test]
    fn every_settings_select_carries_its_row_label(cx: &mut TestAppContext) {
        let (view, cx, _settings) = open_settings_route(cx);
        let labels = view.read_with(cx, |root, cx| {
            let selectors = root.settings_selectors.as_ref().unwrap();
            [
                (&selectors.theme, "theme"),
                (&selectors.material, "material"),
                (&selectors.language, "language"),
                (&selectors.asr_backend, "asr_backend"),
                (&selectors.asr_model, "asr_model"),
                (&selectors.transcription_language, "transcription_language"),
                (&selectors.diarization_backend, "diarization_backend"),
                (&selectors.diarization_model, "diarization_model"),
                (
                    &selectors.diarization_segmentation_model,
                    "diarization_segmentation_model",
                ),
                (
                    &selectors.diarization_embedding_model,
                    "diarization_embedding_model",
                ),
                (&selectors.diarization_clustering, "diarization_clustering"),
                (&selectors.expected_speakers, "expected_speakers"),
                (&selectors.compute_device, "compute_device"),
            ]
            .map(|(select, key)| {
                (
                    select.read(cx).accessibility_label().to_string(),
                    i18n::text(i18n::Locale::ZhCn, key),
                )
            })
        });
        for (label, expected) in labels {
            assert!(!expected.is_empty());
            assert_eq!(label, expected);
        }
        let progress = controls::ProgressBar::new(
            "download-progress",
            i18n::text(i18n::Locale::EnUs, "model_download_progress"),
        );
        assert_eq!(
            progress.accessibility_label().as_ref(),
            "Model download progress"
        );
    }
}
