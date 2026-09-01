use contracts::DiarizationResult;
use serde::{Deserialize, Serialize};

pub use contracts::{
    EmbeddingFeatureExtractor, EmbeddingFrontend, EmbeddingNormalization, EmbeddingSpace,
    IdentityMatch, SpeakerEmbeddingObservation, SpeakerEmbeddingSample, SpeakerLibraryDocument,
    SpeakerProfile, SpeakerSampleSource,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDiarizationOutput {
    pub result: DiarizationResult,
    pub observations: Vec<SpeakerEmbeddingObservation>,
    pub anonymous_clusters: Vec<u32>,
}
