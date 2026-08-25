use application::{asr_descriptor, AsrBackend, BackendFuture};
use contracts::{AsrRequest, AsrResult, BackendDescriptor, BackendError, ModelRef};
use runtime::JobContext;

pub struct WhisperRsBackend;

impl AsrBackend for WhisperRsBackend {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor("whisper-rs", &["offline", "timestamps", "metal"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "whisper-rs model loading is scheduled after v0.1",
        ))
    }

    fn transcribe(&self, _request: AsrRequest, _context: JobContext) -> BackendFuture<AsrResult> {
        Box::pin(async {
            Err(BackendError::not_implemented(
                "whisper-rs inference is scheduled after v0.1",
            ))
        })
    }
}
