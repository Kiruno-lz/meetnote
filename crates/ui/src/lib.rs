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
    use application::{BackendInventory, ProcessingService, WorkspaceService};
    use asr_whisper_rs::WhisperRsBackend;
    use contracts::ModelRef;
    use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
    use gpui::{AppContext, TestAppContext};
    use models::{ModelCatalog, ModelManager};
    use persistence::InMemoryProjectRepository;
    use settings::{JsonSettingsRepository, SettingsService};
    use std::sync::Arc;

    #[gpui::test]
    fn boots_an_empty_desktop_workspace(cx: &mut TestAppContext) {
        let repository = Arc::new(InMemoryProjectRepository::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            repository,
            std::env::temp_dir().join("meetnote-ui-test-markdown"),
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
            },
            diarization_model: ModelRef {
                backend_id: "sherpa-onnx".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "missing-bundle".into(),
            },
        };
        let assistant_input = cx.new(|cx| text_input::TextInput::new(cx, "Ask…"));
        let view = cx.new(|_| {
            controller::RootView::new(
                BackendInventory::new(vec![], vec![]),
                services,
                assistant_input,
            )
        });

        assert_eq!(view.read_with(cx, |view, _| view.projects.len()), 0);
    }
}
