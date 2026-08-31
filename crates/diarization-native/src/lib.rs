//! Observable Rust-native speaker diarization with persistent speaker identity.
//!
//! The crate keeps inference, three-stage orchestration, identity matching, and
//! persistence in separate modules. `NativeDiarizationBackend` implements the
//! application contract, while its `analyze` method exposes the richer
//! embedding observations needed for enrollment and diagnostics.

mod backend;
mod engine;
mod error;
mod identity;
mod library;
mod model;
mod store;
mod types;

pub use backend::{NativeBackendFuture, NativeDiarizationBackend, NativeDiarizationConfig};
pub use engine::{
    ClusterAssignment, ClusteringAdapter, ClusteringOutput, EmbeddingAdapter, EmbeddingOutput,
    EngineOutput, EngineSegment, NativeDiarizationEngine, SegmentationAdapter, SegmentationOutput,
    SegmentedTrack, SpeakrsEngine, StagePipelineEngine, UnclusteredEmbedding, SPEAKRS_MODEL_ID,
    SPEAKRS_MODEL_REVISION,
};
pub use error::{NativeDiarizationError, SpeakerLibraryError};
pub use identity::{cosine_similarity, ClusterIdentityResolver, IdentityResolverConfig};
pub use library::SpeakerLibraryService;
pub use model::{NativeModelBundle, REQUIRED_CPU_ARTIFACTS};
pub use store::{InMemorySpeakerProfileStore, JsonSpeakerProfileStore, SpeakerProfileStore};
pub use types::{
    EmbeddingNormalization, EmbeddingSpace, IdentityMatch, NativeDiarizationOutput,
    SpeakerEmbeddingObservation, SpeakerEmbeddingSample, SpeakerLibraryDocument, SpeakerProfile,
    SpeakerSampleSource,
};
