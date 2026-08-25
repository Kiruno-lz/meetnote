use application::{diarization_descriptor, BackendFuture, DiarizationBackend};
use contracts::{BackendDescriptor, BackendError, DiarizationRequest, DiarizationResult, ModelRef};
use runtime::JobContext;

pub struct NativeDiarizationBackend;

impl DiarizationBackend for NativeDiarizationBackend {
    fn descriptor(&self) -> BackendDescriptor {
        diarization_descriptor("diarization-native", &["offline", "coreml", "onnx"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "diarization crate integration is scheduled after v0.1",
        ))
    }

    fn diarize(
        &self,
        _request: DiarizationRequest,
        _context: JobContext,
    ) -> BackendFuture<DiarizationResult> {
        Box::pin(async {
            Err(BackendError::not_implemented(
                "diarization crate integration is scheduled after v0.1",
            ))
        })
    }
}
