use application::{asr_descriptor, AsrBackend, BackendFuture};
use contracts::{AsrRequest, AsrResult, BackendDescriptor, BackendError, ModelRef};
use runtime::JobContext;

pub struct SherpaOnnxAsrBackend;

impl AsrBackend for SherpaOnnxAsrBackend {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor("sherpa-onnx", &["offline", "streaming", "onnx"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "sherpa-onnx model loading is scheduled after v0.1",
        ))
    }

    fn transcribe(&self, _request: AsrRequest, _context: JobContext) -> BackendFuture<AsrResult> {
        Box::pin(async {
            Err(BackendError::not_implemented(
                "sherpa-onnx inference is scheduled after v0.1",
            ))
        })
    }
}
