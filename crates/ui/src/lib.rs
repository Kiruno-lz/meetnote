pub mod assistant_ui;
mod components;
mod controller;
mod i18n;
mod pages;
mod route;
mod state;
mod text_input;

pub use controller::{run, DesktopServices};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_ui::{
        AssistantCancellation, AssistantMutationSummary, AssistantPendingReview,
        AssistantRunRequest, AssistantRunResult, AssistantScopeMode, AssistantTaskState,
        AssistantUiError, AssistantUiService, ConnectionDiagnostic, ConnectionProbeKind,
        PendingReviewState,
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
        fn save_assistant_settings(
            &self,
            settings: settings::AssistantSettings,
        ) -> Result<settings::AssistantSettings, AssistantUiError> {
            Ok(settings)
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
        let repository = Arc::new(InMemoryDocumentRepository::default());
        let documents = DocumentService::new(repository);
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            documents.clone(),
        );
        let services = DesktopServices {
            workspace,
            settings: SettingsService::open(Arc::new(JsonSettingsRepository::open(
                std::env::temp_dir().join("meetnote-ui-test-settings.json"),
            )))
            .unwrap(),
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
            assistant: Arc::new(FakeAssistantUiService::default()),
        };
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
        let repository = Arc::new(InMemoryDocumentRepository::default());
        let documents = DocumentService::new(repository);
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            documents.clone(),
        );
        let assistant = Arc::new(FakeAssistantUiService::default());
        let services = DesktopServices {
            workspace,
            settings: SettingsService::open(Arc::new(JsonSettingsRepository::open(
                std::env::temp_dir().join(format!(
                    "meetnote-ui-render-test-settings-{}-{}.json",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )),
            )))
            .unwrap(),
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
            assistant: assistant.clone(),
        };
        // Undo runs on a `runtime::spawn_blocking` worker thread.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        });
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

    fn open_desktop(
        cx: &mut TestAppContext,
        assistant: Arc<FakeAssistantUiService>,
    ) -> (
        gpui::Entity<controller::RootView>,
        &mut gpui::VisualTestContext,
    ) {
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            DocumentService::new(Arc::new(InMemoryDocumentRepository::default())),
        );
        static SETTINGS_SEQUENCE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let services = DesktopServices {
            workspace,
            settings: SettingsService::open(Arc::new(JsonSettingsRepository::open(
                std::env::temp_dir().join(format!(
                    "meetnote-ui-assistant-test-settings-{}-{}.json",
                    std::process::id(),
                    SETTINGS_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                )),
            )))
            .unwrap(),
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
        };
        // Service calls complete on `runtime::spawn_blocking` worker threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        });
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

    /// Drives the GPUI executor until `done` holds; service calls run on real
    /// worker threads, so parked GPUI tasks may still be waiting on them.
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

    /// Selects the `first` document, enables a connection and starts a run.
    fn start_run(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<controller::RootView>) {
        view.update_in(cx, |root, window, cx| {
            let first = DocumentState::new(ProjectId::new("first"), "First", "first.wav");
            let second = DocumentState::new(ProjectId::new("second"), "Second", "second.wav");
            root.ui.selected_project = Some(first.id().clone());
            root.projects = vec![first, second];
            root.settings_draft.assistant.enabled = true;
            root.settings_draft.assistant.active_connection_id = Some("fixture".into());
            root.assistant_inputs
                .prompt
                .update(cx, |input, _| input.set_value("organize"));
            root.assistant_action(&gpui::ClickEvent::default(), window, cx);
        });
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
        let (view, cx) = open_desktop(cx, assistant.clone());
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
        let (view, cx) = open_desktop(cx, assistant);
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
}
