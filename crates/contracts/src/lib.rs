use domain::{DiarizationSegment, TranscriptSegment};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub backend_id: String,
    pub model_id: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputeDevice {
    Auto,
    Cpu,
    Metal,
    CoreMl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendTask {
    Asr,
    Diarization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendDescriptor {
    pub id: String,
    pub task: BackendTask,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrRequest {
    pub audio_path: String,
    pub model: ModelRef,
    pub language: Option<String>,
    pub device: ComputeDevice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrResult {
    pub backend: BackendDescriptor,
    pub model: ModelRef,
    pub language: Option<String>,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiarizationRequest {
    pub audio_path: String,
    pub model: ModelRef,
    pub expected_speakers: Option<u32>,
    pub device: ComputeDevice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiarizationResult {
    pub backend: BackendDescriptor,
    pub model: ModelRef,
    pub segments: Vec<DiarizationSegment>,
    pub speaker_count: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendErrorCode {
    BackendUnavailable,
    ModelMissing,
    ModelIncompatible,
    ModelLoadFailed,
    DeviceUnavailable,
    InvalidAudio,
    InferenceFailed,
    Cancelled,
    OutputValidationFailed,
    NotImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendError {
    pub code: BackendErrorCode,
    pub message: String,
}

impl BackendError {
    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            code: BackendErrorCode::NotImplemented,
            message: message.into(),
        }
    }
}
