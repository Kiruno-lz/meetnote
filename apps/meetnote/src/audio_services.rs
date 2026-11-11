use std::path::Path;
use std::sync::Arc;

use application::{BackendInventory, ProcessingOptions, ProcessingService};
use asr::{AsrBackendManager, AsrBackendRegistry};
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::ModelRef;
use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};
use diarization_native::{NativeDiarizationBackend, NativeDiarizationConfig};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use models::{ModelCatalog, ModelManager, manager::BackendProbe};
use runtime::JobRuntime;
use settings::AppSettings;

pub fn model_catalog() -> Result<ModelCatalog, models::CatalogError> {
    ModelCatalog::parse(include_str!("../../../resources/models/catalog.json"))
}

#[derive(Clone)]
pub struct AudioBackends {
    asr: Arc<AsrBackendManager>,
    diarization: Arc<DiarizationBackendManager>,
}

impl AudioBackends {
    pub fn registered(speaker_library: &Path) -> Self {
        let mut asr = AsrBackendRegistry::default();
        asr.register(Arc::new(WhisperRsBackend::default()));
        asr.register(Arc::new(SherpaOnnxAsrBackend::default()));

        let mut diarization = DiarizationBackendRegistry::default();
        diarization.register(Arc::new(SherpaOnnxDiarizationBackend::default()));
        diarization.register(Arc::new(NativeDiarizationBackend::new(
            NativeDiarizationConfig {
                speaker_library_path: speaker_library.into(),
                ..NativeDiarizationConfig::default()
            },
        )));

        Self {
            asr: Arc::new(AsrBackendManager::new(asr)),
            diarization: Arc::new(DiarizationBackendManager::new(diarization)),
        }
    }

    pub fn processing(&self, jobs: Arc<JobRuntime>) -> ProcessingService {
        ProcessingService::new_with_providers(self.asr.clone(), self.diarization.clone())
            .with_job_runtime(jobs)
    }

    pub fn inventory(&self) -> BackendInventory {
        BackendInventory::new(
            self.asr.available_backends(),
            self.diarization.available_backends(),
        )
    }

    pub fn models(&self, catalog: ModelCatalog, root: &Path) -> ModelManager {
        ModelManager::with_backend_probe(catalog, root, Some(Arc::new(self.clone())))
    }
}

impl BackendProbe for AudioBackends {
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

pub fn processing_options(settings: &AppSettings) -> ProcessingOptions {
    ProcessingOptions {
        language: settings.language.clone(),
        expected_speakers: settings.expected_speakers,
        device: settings.compute_device,
        speaker_library_path: Some(settings.speaker_library_path.clone()),
        diarization_stages: settings.diarization_stages.clone(),
    }
}
