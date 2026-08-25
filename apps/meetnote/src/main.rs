use application::BackendInventory;
use asr::{AsrBackendManager, AsrBackendRegistry};
use asr_whisper_rs::WhisperRsBackend;
use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use std::sync::Arc;

fn main() {
    let mut asr_registry = AsrBackendRegistry::default();
    asr_registry.register(Arc::new(WhisperRsBackend::default()));
    let asr_manager = AsrBackendManager::new(asr_registry);

    let mut diarization_registry = DiarizationBackendRegistry::default();
    diarization_registry.register(Arc::new(SherpaOnnxDiarizationBackend::default()));
    let diarization_manager = DiarizationBackendManager::new(diarization_registry);

    ui::run(BackendInventory::new(
        asr_manager.available_backends(),
        diarization_manager.available_backends(),
    ));
}
