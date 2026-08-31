use contracts::DiarizationResult;
use serde::{Deserialize, Serialize};

pub const SPEAKER_LIBRARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingNormalization {
    L2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSpace {
    pub backend_id: String,
    pub model_id: String,
    pub model_revision: Option<String>,
    pub dimension: usize,
    pub sample_rate_hz: u32,
    pub feature_config: String,
    pub normalization: EmbeddingNormalization,
}

impl EmbeddingSpace {
    pub fn compatible_with(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSampleSource {
    pub project_id: Option<String>,
    pub source_segment_id: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDiarizationOutput {
    pub result: DiarizationResult,
    pub observations: Vec<SpeakerEmbeddingObservation>,
    pub identity_matches: Vec<IdentityMatch>,
    pub anonymous_clusters: Vec<u32>,
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
