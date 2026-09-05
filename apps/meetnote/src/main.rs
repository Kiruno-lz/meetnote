use application::{
    BackendInventory, DocumentCommandService, DocumentService, ProcessingOptions,
    ProcessingService, WorkspaceService,
};
use asr::{AsrBackendManager, AsrBackendRegistry};
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::ModelRef;
use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};
use diarization_native::{NativeDiarizationBackend, NativeDiarizationConfig};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use models::{manager::BackendProbe, ModelCatalog, ModelManager};
use persistence::FileDocumentRepository;
use settings::{JsonSettingsRepository, SettingsService};
use std::env;
use std::sync::Arc;
use ui::DesktopServices;

fn main() {
    let settings = SettingsService::open(Arc::new(JsonSettingsRepository::open(
        "data/settings/settings.json",
    )))
    .expect("failed to open settings repository");
    let settings_snapshot = settings.snapshot().unwrap_or_default();

    let whisper_backend = Arc::new(WhisperRsBackend::default());
    let sherpa_asr_backend = Arc::new(SherpaOnnxAsrBackend::default());
    let sherpa_diarization_backend = Arc::new(SherpaOnnxDiarizationBackend::default());
    let native_diarization_backend =
        Arc::new(NativeDiarizationBackend::new(NativeDiarizationConfig {
            speaker_library_path: settings_snapshot.speaker_library_path.clone().into(),
            ..NativeDiarizationConfig::default()
        }));

    let mut asr_registry = AsrBackendRegistry::default();
    asr_registry.register(whisper_backend);
    asr_registry.register(sherpa_asr_backend);
    let asr_manager = Arc::new(AsrBackendManager::new(asr_registry));

    let mut diarization_registry = DiarizationBackendRegistry::default();
    diarization_registry.register(sherpa_diarization_backend);
    diarization_registry.register(native_diarization_backend);
    let diarization_manager = Arc::new(DiarizationBackendManager::new(diarization_registry));

    let repository = Arc::new(
        FileDocumentRepository::open("data/documents").expect("failed to open document repository"),
    );
    let documents = DocumentService::new(repository);
    let document_commands = DocumentCommandService::new(documents.clone());
    let workspace = WorkspaceService::with_options(
        ProcessingService::new_with_providers(asr_manager.clone(), diarization_manager.clone()),
        documents,
        ProcessingOptions {
            language: settings_snapshot.language.clone(),
            expected_speakers: settings_snapshot.expected_speakers,
            device: settings_snapshot.compute_device,
            speaker_library_path: Some(settings_snapshot.speaker_library_path.clone()),
            diarization_stages: settings_snapshot.diarization_stages.clone(),
        },
    );

    let catalog = ModelCatalog::from_path("resources/models/catalog.json")
        .expect("failed to open model catalog");
    let backend_probe = Arc::new(RegisteredBackendProbe {
        asr: asr_manager.clone(),
        diarization: diarization_manager.clone(),
    });
    let model_manager = ModelManager::with_backend_probe(catalog, "models", Some(backend_probe));
    let asr_backend_id = settings_snapshot
        .asr_backend_id
        .as_deref()
        .unwrap_or("whisper-rs");
    let diarization_backend_id = settings_snapshot
        .diarization_backend_id
        .as_deref()
        .unwrap_or("diarization-native");
    let asr_model = model_manager
        .configured_model_ref("asr", asr_backend_id, settings_snapshot.asr_model.as_ref())
        .unwrap_or_else(|_| unconfigured_model(asr_backend_id));
    let diarization_model = model_manager
        .configured_model_ref(
            "diarization",
            diarization_backend_id,
            settings_snapshot.diarization_model.as_ref(),
        )
        .unwrap_or_else(|_| unconfigured_model(diarization_backend_id));

    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("process") {
        if args.len() != 3 {
            eprintln!("usage: meetnote process <audio>");
            std::process::exit(2);
        }
        let project = workspace
            .import_audio(args[2].clone())
            .unwrap_or_else(|error| fail_process(error));
        let result = futures::executor::block_on(workspace.process_project(
            project.id().clone(),
            asr_model,
            diarization_model,
        ))
        .unwrap_or_else(|error| fail_process(error.message));
        println!("processed document {}", result.id().as_str());
        println!("document package: data/documents/{}", result.id().as_str());
        return;
    }

    let desktop_services = DesktopServices {
        workspace,
        document_commands,
        settings,
        model_manager,
        asr_model,
        diarization_model,
    };
    ui::run(
        BackendInventory::new(
            asr_manager.available_backends(),
            diarization_manager.available_backends(),
        ),
        desktop_services,
    );
}

fn unconfigured_model(backend_id: &str) -> ModelRef {
    ModelRef {
        backend_id: backend_id.into(),
        model_id: "unconfigured".into(),
        revision: None,
        artifact_path: String::new(),
        artifacts: Vec::new(),
    }
}

fn fail_process(error: String) -> ! {
    eprintln!("processing failed: {error}");
    std::process::exit(1);
}

struct RegisteredBackendProbe {
    asr: Arc<AsrBackendManager>,
    diarization: Arc<DiarizationBackendManager>,
}

impl BackendProbe for RegisteredBackendProbe {
    fn probe(&self, task: &str, model: &ModelRef) -> Result<(), String> {
        match task {
            "asr" => self
                .asr
                .probe(&model.backend_id, model)
                .map_err(|error| error.message),
            "diarization" => self
                .diarization
                .probe(&model.backend_id, model)
                .map_err(|error| error.message),
            task => Err(format!("backend probe is not registered for task: {task}")),
        }
    }
}
