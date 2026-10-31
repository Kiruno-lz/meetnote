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
    #[error("diarization model failed to load: {0}")]
    ModelLoadFailed(String),
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
            NativeDiarizationError::ModelLoadFailed(_) => BackendErrorCode::ModelLoadFailed,
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

impl NativeDiarizationError {
    pub(crate) fn loading_pipeline(error: speakrs::PipelineError) -> Self {
        match error {
            speakrs::PipelineError::ModelLoad(_)
            | speakrs::PipelineError::UnsupportedExecutionMode(_) => error.into(),
            other => Self::ModelLoadFailed(other.to_string()),
        }
    }
}

impl From<speakrs::inference::ModelLoadError> for NativeDiarizationError {
    fn from(error: speakrs::inference::ModelLoadError) -> Self {
        let message = error.to_string();
        match error {
            speakrs::inference::ModelLoadError::UnsupportedExecutionMode(_) => {
                Self::DeviceUnavailable(message)
            }
            speakrs::inference::ModelLoadError::MissingNativeAsset { .. } => {
                Self::ModelMissing(message)
            }
            _ => Self::ModelLoadFailed(message),
        }
    }
}

impl From<speakrs::PipelineError> for NativeDiarizationError {
    fn from(error: speakrs::PipelineError) -> Self {
        match error {
            speakrs::PipelineError::ModelLoad(error) => error.into(),
            speakrs::PipelineError::UnsupportedExecutionMode(error) => {
                Self::DeviceUnavailable(error.to_string())
            }
            other => Self::Inference(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipeline_errors_keep_load_phase_missing_assets_and_inference_distinct() {
        use speakrs::inference::ModelLoadError;
        use speakrs::{ExecutionMode, PipelineError};
        // Typed SDK error fixtures; these do not execute a model.
        let lazy_load = PipelineError::ModelLoad(ModelLoadError::NativeAssetLoad {
            mode: ExecutionMode::CoreMl,
            path: "embedding.mlmodelc".into(),
            message: "invalid graph".into(),
        });
        let error: BackendError = NativeDiarizationError::from(lazy_load).into();
        assert_eq!(error.code, BackendErrorCode::ModelLoadFailed);
        assert!(error.message.contains("embedding.mlmodelc"));
        let missing = PipelineError::ModelLoad(ModelLoadError::MissingNativeAsset {
            mode: ExecutionMode::CoreMl,
            path: "missing.mlmodelc".into(),
        });
        let error: BackendError = NativeDiarizationError::from(missing).into();
        assert_eq!(error.code, BackendErrorCode::ModelMissing);
        let error: BackendError = NativeDiarizationError::loading_pipeline(
            PipelineError::Invariant("invalid init".into()),
        )
        .into();
        assert_eq!(error.code, BackendErrorCode::ModelLoadFailed);
        let error: BackendError =
            NativeDiarizationError::from(PipelineError::Invariant("invalid run".into())).into();
        assert_eq!(error.code, BackendErrorCode::InferenceFailed);
    }
}
