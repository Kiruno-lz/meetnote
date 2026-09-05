use domain::{DiarizationSegment, TranscriptSegment};
use serde::{Deserialize, Serialize};

pub const DOCUMENT_COMMAND_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRecord {
    pub state: domain::DocumentState,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCommandEnvelope {
    pub schema_version: u32,
    pub request_id: String,
    pub command: DocumentCommand,
}

impl DocumentCommandEnvelope {
    pub fn new(request_id: impl Into<String>, command: DocumentCommand) -> Self {
        Self {
            schema_version: DOCUMENT_COMMAND_SCHEMA_VERSION,
            request_id: request_id.into(),
            command,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "payload")]
pub enum DocumentCommand {
    List,
    Read {
        document_id: domain::DocumentId,
    },
    Create {
        title: String,
        audio_path: String,
        markdown: String,
        #[serde(default)]
        tags: Vec<String>,
    },
    Update {
        document_id: domain::DocumentId,
        expected_revision: u64,
        patch: DocumentPatch,
    },
    CreateSemanticChild {
        parent_id: domain::DocumentId,
        expected_parent_revision: u64,
        title: String,
        start_ms: u64,
        end_ms: u64,
        #[serde(default)]
        tags: Vec<String>,
        markdown: String,
    },
    Reparent {
        document_id: domain::DocumentId,
        expected_revision: u64,
        new_parent_id: Option<domain::DocumentId>,
        sibling_order: u32,
    },
    Reorder {
        document_id: domain::DocumentId,
        expected_revision: u64,
        target_id: domain::DocumentId,
        before: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPatch {
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub summary: Option<Option<String>>,
    #[serde(default)]
    pub document_status: Option<String>,
    pub markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "payload")]
pub enum DocumentCommandResult {
    Snapshot {
        documents: Vec<domain::DocumentState>,
        issues: Vec<DocumentCommandIssue>,
    },
    Document(Option<DocumentRecord>),
    Updated(DocumentRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCommandIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub backend_id: String,
    pub model_id: String,
    pub revision: Option<String>,
    pub artifact_path: String,
    #[serde(default)]
    pub artifacts: Vec<ModelArtifactRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifactRef {
    pub filename: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub archive_member: Option<String>,
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
    #[serde(default)]
    pub configurable_diarization_stages: Vec<DiarizationStageKind>,
    #[serde(default)]
    pub diarization_stage_options: Vec<DiarizationStageOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiarizationStageKind {
    Segmentation,
    Embedding,
    Clustering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationStageOption {
    pub stage: DiarizationStageKind,
    pub id: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub artifact_files: Vec<String>,
}

pub const SPEAKER_LIBRARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingNormalization {
    L2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingFeatureExtractor {
    KaldiFbank,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingFrontend {
    pub extractor: EmbeddingFeatureExtractor,
    pub mel_bins: u16,
    pub frame_length_ms: u16,
    pub frame_shift_ms: u16,
    pub mean_normalization: bool,
    pub dither: bool,
    pub overlap_aware: bool,
}

impl Default for EmbeddingFrontend {
    fn default() -> Self {
        Self {
            extractor: EmbeddingFeatureExtractor::KaldiFbank,
            mel_bins: 0,
            frame_length_ms: 0,
            frame_shift_ms: 0,
            mean_normalization: false,
            dither: false,
            overlap_aware: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSpace {
    /// Runtime provenance. It is deliberately excluded from compatibility checks.
    pub backend_id: String,
    pub model_id: String,
    pub model_revision: Option<String>,
    pub dimension: usize,
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub frontend: EmbeddingFrontend,
    /// Legacy v1 description. New records use `frontend` instead.
    #[serde(
        rename = "featureConfig",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub legacy_feature_config: Option<String>,
    pub normalization: EmbeddingNormalization,
}

impl EmbeddingSpace {
    pub fn compatible_with(&self, other: &Self) -> bool {
        self.model_id == other.model_id
            && self.model_revision == other.model_revision
            && self.dimension == other.dimension
            && self.sample_rate_hz == other.sample_rate_hz
            && self.normalization == other.normalization
            && match (&self.legacy_feature_config, &other.legacy_feature_config) {
                (Some(left), Some(right)) => left == right,
                (Some(legacy), None) => {
                    legacy_frontend(legacy).is_some_and(|frontend| frontend == other.frontend)
                }
                (None, Some(legacy)) => {
                    legacy_frontend(legacy).is_some_and(|frontend| self.frontend == frontend)
                }
                (None, None) => self.frontend == other.frontend,
            }
    }
}

fn legacy_frontend(description: &str) -> Option<EmbeddingFrontend> {
    match description {
        "80-bin-kaldi-fbank; masked-overlap-aware" => Some(EmbeddingFrontend {
            extractor: EmbeddingFeatureExtractor::KaldiFbank,
            mel_bins: 80,
            frame_length_ms: 25,
            frame_shift_ms: 10,
            mean_normalization: false,
            dither: true,
            overlap_aware: true,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSampleSource {
    pub project_id: Option<String>,
    pub source_segment_id: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default)]
    pub audio_path: Option<String>,
    #[serde(default)]
    pub audio_sha256: Option<String>,
    #[serde(default)]
    pub backup_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerEmbeddingSample {
    pub id: String,
    pub vector: Vec<f32>,
    pub quality: f32,
    pub source: SpeakerSampleSource,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerProfile {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub embedding_space: EmbeddingSpace,
    pub samples: Vec<SpeakerEmbeddingSample>,
    pub match_threshold: f32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerLibraryDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub profiles: Vec<SpeakerProfile>,
}

impl Default for SpeakerLibraryDocument {
    fn default() -> Self {
        Self {
            schema_version: SPEAKER_LIBRARY_SCHEMA_VERSION,
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerEmbeddingObservation {
    pub id: String,
    pub cluster_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub vector: Vec<f32>,
    pub quality: f32,
    pub embedding_space: EmbeddingSpace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityMatch {
    pub cluster_id: u32,
    pub profile_id: String,
    pub display_name: String,
    pub score: f32,
    pub threshold: f32,
    #[serde(default)]
    pub distribution_floor: f32,
    #[serde(default)]
    pub cluster_dispersion: f32,
    #[serde(default)]
    pub combined_dispersion: f32,
    #[serde(default)]
    pub competition_margin: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerProfileCandidate {
    pub cluster_id: u32,
    pub representative_observation_ids: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub observation_count: usize,
    pub mean_quality: f32,
    pub dispersion: f32,
    pub embedding_space: EmbeddingSpace,
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
    #[serde(default)]
    pub speaker_library_path: Option<String>,
    #[serde(default)]
    pub stage_configuration: DiarizationStageConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationStageConfiguration {
    #[serde(default)]
    pub segmentation_model: Option<ModelRef>,
    #[serde(default)]
    pub embedding_model: Option<ModelRef>,
    #[serde(default)]
    pub clustering_algorithm: Option<AlgorithmRef>,
}

impl DiarizationStageConfiguration {
    pub fn is_empty(&self) -> bool {
        self.segmentation_model.is_none()
            && self.embedding_model.is_none()
            && self.clustering_algorithm.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmRef {
    pub id: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub parameters: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiarizationResult {
    pub backend: BackendDescriptor,
    pub model: ModelRef,
    pub segments: Vec<DiarizationSegment>,
    pub speaker_count: Option<u32>,
    #[serde(default)]
    pub embedding_observations: Vec<SpeakerEmbeddingObservation>,
    #[serde(default)]
    pub speaker_profile_candidates: Vec<SpeakerProfileCandidate>,
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
    PersistenceFailed,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn frontend() -> EmbeddingFrontend {
        EmbeddingFrontend {
            extractor: EmbeddingFeatureExtractor::KaldiFbank,
            mel_bins: 80,
            frame_length_ms: 25,
            frame_shift_ms: 10,
            mean_normalization: false,
            dither: true,
            overlap_aware: false,
        }
    }

    fn space(backend_id: &str) -> EmbeddingSpace {
        EmbeddingSpace {
            backend_id: backend_id.into(),
            model_id: "resnet34".into(),
            model_revision: Some("rev-1".into()),
            dimension: 256,
            sample_rate_hz: 16_000,
            frontend: frontend(),
            legacy_feature_config: None,
            normalization: EmbeddingNormalization::L2,
        }
    }

    #[test]
    fn compatibility_ignores_runtime_wrapper_but_rejects_model_or_frontend_changes() {
        assert!(space("native").compatible_with(&space("sherpa-onnx")));

        let mut different_model = space("native");
        different_model.model_id = "eres2net".into();
        assert!(!space("native").compatible_with(&different_model));

        let mut different_frontend = space("native");
        different_frontend.frontend.mean_normalization = true;
        assert!(!space("native").compatible_with(&different_frontend));
    }

    #[test]
    fn known_legacy_feature_config_matches_its_structured_space() {
        let legacy: EmbeddingSpace = serde_json::from_str(
            r#"{
                "backendId":"native",
                "modelId":"resnet34",
                "modelRevision":"rev-1",
                "dimension":256,
                "sampleRateHz":16000,
                "featureConfig":"80-bin-kaldi-fbank; masked-overlap-aware",
                "normalization":"l2"
            }"#,
        )
        .unwrap();

        assert_eq!(
            legacy.legacy_feature_config.as_deref(),
            Some("80-bin-kaldi-fbank; masked-overlap-aware")
        );
        let mut structured = space("native");
        structured.frontend.overlap_aware = true;
        structured.frontend.dither = true;
        assert!(legacy.compatible_with(&structured));

        let mut unknown = legacy;
        unknown.legacy_feature_config = Some("unknown-feature-config".into());
        assert!(!unknown.compatible_with(&structured));
    }

    #[test]
    fn existing_speaker_sample_sources_keep_schema_one_compatibility() {
        let source: SpeakerSampleSource = serde_json::from_str(
            r#"{
                "projectId":"project",
                "sourceSegmentId":"segment",
                "startMs":10,
                "endMs":20
            }"#,
        )
        .unwrap();

        assert_eq!(SPEAKER_LIBRARY_SCHEMA_VERSION, 1);
        assert_eq!(source.project_id.as_deref(), Some("project"));
        assert!(source.audio_path.is_none());
        assert!(source.backup_path.is_none());
    }
}
