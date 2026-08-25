use application::{diarization_descriptor, BackendFuture, DiarizationBackend};
use contracts::{BackendDescriptor, BackendError, DiarizationRequest, DiarizationResult, ModelRef};
use runtime::JobContext;

pub struct SherpaOnnxDiarizationBackend;

impl DiarizationBackend for SherpaOnnxDiarizationBackend {
    fn descriptor(&self) -> BackendDescriptor {
        diarization_descriptor("sherpa-onnx", &["offline", "onnx", "clustering"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "sherpa-onnx diarization loading is scheduled after v0.1",
        ))
    }

    fn diarize(
        &self,
        _request: DiarizationRequest,
        _context: JobContext,
    ) -> BackendFuture<DiarizationResult> {
        Box::pin(async {
            Err(BackendError::not_implemented(
                "sherpa-onnx diarization is scheduled after v0.1",
            ))
        })
    }
}
