use application::AsrBackend;
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use contracts::{AsrRequest, ComputeDevice, ModelRef};
use runtime::JobRuntime;
use std::path::Path;

#[test]
#[ignore = "requires MEETNOTE_SHERPA_ONNX_MODEL_DIR and MEETNOTE_SHERPA_ONNX_AUDIO"]
fn paraformer_transcribes_real_audio() {
    let model_dir = std::env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR")
        .expect("MEETNOTE_SHERPA_ONNX_MODEL_DIR is required");
    let audio_path = std::env::var("MEETNOTE_SHERPA_ONNX_AUDIO")
        .expect("MEETNOTE_SHERPA_ONNX_AUDIO is required");
    let model = ModelRef {
        backend_id: "sherpa-onnx".into(),
        model_id: "paraformer-zh".into(),
        revision: None,
        artifact_path: model_dir,
    };
    let backend = SherpaOnnxAsrBackend::default();
    backend
        .probe(&model)
        .expect("sherpa-onnx ASR model probe should succeed");
    let (context, _) = JobRuntime::default().create_job();

    let result = futures::executor::block_on(backend.transcribe(
        AsrRequest {
            audio_path,
            model,
            language: Some("zh".into()),
            device: ComputeDevice::Cpu,
        },
        context,
    ))
    .expect("sherpa-onnx ASR transcription should succeed");

    assert!(!result.segments.is_empty());
    assert!(!result.segments[0].text.is_empty());
    assert!(Path::new(&result.model.artifact_path).is_dir());
}
