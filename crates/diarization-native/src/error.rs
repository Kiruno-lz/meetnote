use contracts::{BackendError, BackendErrorCode};
use thiserror::Error;

pub use application::SpeakerLibraryError;

#[derive(Debug, Error)]
pub enum NativeDiarizationError {
    #[error("diarization model is missing: {0}")]
    ModelMissing(String),
    #[error("diarization model is incompatible: {0}")]
    ModelIncompatible(String),
    #[error("requested compute device is unavailable: {0}")]
    DeviceUnavailable(String),
    #[error("audio is invalid: {0}")]
    InvalidAudio(String),
    #[error("diarization inference failed: {0}")]
    Inference(String),
    #[error("diarization output is invalid: {0}")]
    InvalidOutput(String),
    #[error("diarization was cancelled")]
    Cancelled,
    #[error(transparent)]
    SpeakerLibrary(#[from] SpeakerLibraryError),
}

impl From<NativeDiarizationError> for BackendError {
    fn from(error: NativeDiarizationError) -> Self {
        let code = match &error {
            NativeDiarizationError::ModelMissing(_) => BackendErrorCode::ModelMissing,
            NativeDiarizationError::ModelIncompatible(_) => BackendErrorCode::ModelIncompatible,
            NativeDiarizationError::DeviceUnavailable(_) => BackendErrorCode::DeviceUnavailable,
            NativeDiarizationError::InvalidAudio(_) => BackendErrorCode::InvalidAudio,
            NativeDiarizationError::Inference(_) => BackendErrorCode::InferenceFailed,
            NativeDiarizationError::InvalidOutput(_) => BackendErrorCode::OutputValidationFailed,
            NativeDiarizationError::Cancelled => BackendErrorCode::Cancelled,
            NativeDiarizationError::SpeakerLibrary(_) => BackendErrorCode::PersistenceFailed,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}
