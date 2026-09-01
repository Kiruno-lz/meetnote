use crate::{
    EmbeddingFeatureExtractor, EmbeddingFrontend, EmbeddingNormalization, EmbeddingSpace,
    NativeDiarizationError, NativeModelBundle, SpeakerEmbeddingObservation,
};
use contracts::{ComputeDevice, ModelRef};
use speakrs::{ExecutionMode, OwnedDiarizationPipeline};
use std::collections::BTreeMap;

pub const SPEAKRS_MODEL_ID: &str = "speakrs-community-1-cpu";
pub const SPEAKRS_MODEL_REVISION: &str = "5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f";

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedTrack {
    pub track_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SegmentationOutput {
    pub tracks: Vec<SegmentedTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnclusteredEmbedding {
    pub track_id: String,
    pub vector: Vec<f32>,
    pub quality: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EmbeddingOutput {
    pub embedding_space: Option<EmbeddingSpace>,
    pub embeddings: Vec<UnclusteredEmbedding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterAssignment {
    pub track_id: String,
    pub cluster_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClusteringOutput {
    pub assignments: Vec<ClusterAssignment>,
}

pub trait SegmentationAdapter: Send + Sync {
    fn probe(
        &self,
        bundle: &NativeModelBundle,
        device: ComputeDevice,
    ) -> Result<(), NativeDiarizationError>;
    fn segment(
        &self,
        samples: &[f32],
        bundle: &NativeModelBundle,
        device: ComputeDevice,
    ) -> Result<SegmentationOutput, NativeDiarizationError>;
}

pub trait EmbeddingAdapter: Send + Sync {
    fn probe(
        &self,
        bundle: &NativeModelBundle,
        device: ComputeDevice,
    ) -> Result<(), NativeDiarizationError>;
    fn embed(
        &self,
        samples: &[f32],
        segmentation: &SegmentationOutput,
        bundle: &NativeModelBundle,
        model: &ModelRef,
        device: ComputeDevice,
    ) -> Result<EmbeddingOutput, NativeDiarizationError>;
}

pub trait ClusteringAdapter: Send + Sync {
    fn cluster(
        &self,
        segmentation: &SegmentationOutput,
        embeddings: &EmbeddingOutput,
        expected_speakers: Option<u32>,
    ) -> Result<ClusteringOutput, NativeDiarizationError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineSegment {
    pub cluster_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EngineOutput {
    pub segments: Vec<EngineSegment>,
    pub observations: Vec<SpeakerEmbeddingObservation>,
    pub speaker_count: u32,
}

pub trait NativeDiarizationEngine: Send + Sync {
    fn probe(&self, model: &ModelRef) -> Result<(), NativeDiarizationError>;
    fn run(
        &self,
        samples: &[f32],
        model: &ModelRef,
        device: ComputeDevice,
        expected_speakers: Option<u32>,
    ) -> Result<EngineOutput, NativeDiarizationError>;
}

pub struct StagePipelineEngine<S, E, C> {
    segmentation: S,
    embedding: E,
    clustering: C,
}

impl<S, E, C> StagePipelineEngine<S, E, C> {
    pub fn new(segmentation: S, embedding: E, clustering: C) -> Self {
        Self {
            segmentation,
            embedding,
            clustering,
        }
    }
}

impl<S, E, C> NativeDiarizationEngine for StagePipelineEngine<S, E, C>
where
    S: SegmentationAdapter,
    E: EmbeddingAdapter,
    C: ClusteringAdapter,
{
    fn probe(&self, model: &ModelRef) -> Result<(), NativeDiarizationError> {
        let bundle = NativeModelBundle::from_model(model)?;
        self.segmentation.probe(&bundle, ComputeDevice::Cpu)?;
        self.embedding.probe(&bundle, ComputeDevice::Cpu)
    }

    fn run(
        &self,
        samples: &[f32],
        model: &ModelRef,
        device: ComputeDevice,
        expected_speakers: Option<u32>,
    ) -> Result<EngineOutput, NativeDiarizationError> {
        let bundle = NativeModelBundle::from_model(model)?;
        self.segmentation.probe(&bundle, device)?;
        self.embedding.probe(&bundle, device)?;
        let segmentation = self.segmentation.segment(samples, &bundle, device)?;
        let embedding = self
            .embedding
            .embed(samples, &segmentation, &bundle, model, device)?;
        let embedding_space = embedding.embedding_space.clone().ok_or_else(|| {
            NativeDiarizationError::InvalidOutput("embedding adapter omitted its space".into())
        })?;
        let clustering = self
            .clustering
            .cluster(&segmentation, &embedding, expected_speakers)?;
        let assignments = clustering
            .assignments
            .into_iter()
            .map(|assignment| (assignment.track_id, assignment.cluster_id))
            .collect::<BTreeMap<_, _>>();
        let tracks = segmentation
            .tracks
            .iter()
            .map(|track| (track.track_id.as_str(), track))
            .collect::<BTreeMap<_, _>>();
        let mut segments = Vec::with_capacity(segmentation.tracks.len());
        for track in &segmentation.tracks {
            let cluster_id = assignments.get(&track.track_id).ok_or_else(|| {
                NativeDiarizationError::InvalidOutput(format!(
                    "clustering omitted track {}",
                    track.track_id
                ))
            })?;
            segments.push(EngineSegment {
                cluster_id: *cluster_id,
                start_ms: track.start_ms,
                end_ms: track.end_ms,
                confidence: Some(track.confidence),
            });
        }
        let mut observations = Vec::with_capacity(embedding.embeddings.len());
        for item in embedding.embeddings {
            let track = tracks.get(item.track_id.as_str()).ok_or_else(|| {
                NativeDiarizationError::InvalidOutput(format!(
                    "embedding references unknown track {}",
                    item.track_id
                ))
            })?;
            let cluster_id = assignments.get(&item.track_id).ok_or_else(|| {
                NativeDiarizationError::InvalidOutput(format!(
                    "clustering omitted embedded track {}",
                    item.track_id
                ))
            })?;
            observations.push(SpeakerEmbeddingObservation {
                id: uuid::Uuid::new_v4().to_string(),
                cluster_id: *cluster_id,
                start_ms: track.start_ms,
                end_ms: track.end_ms,
                vector: item.vector,
                quality: item.quality,
                embedding_space: embedding_space.clone(),
            });
        }
        let speaker_count = assignments
            .values()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len() as u32;
        Ok(EngineOutput {
            segments,
            observations,
            speaker_count,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpeakrsEngine;

impl NativeDiarizationEngine for SpeakrsEngine {
    fn probe(&self, model: &ModelRef) -> Result<(), NativeDiarizationError> {
        if model.model_id != SPEAKRS_MODEL_ID {
            return Err(NativeDiarizationError::ModelIncompatible(format!(
                "SpeakrsEngine requires model `{SPEAKRS_MODEL_ID}`, got `{}`",
                model.model_id
            )));
        }
        NativeModelBundle::from_model(model)?.probe_cpu()
    }

    fn run(
        &self,
        samples: &[f32],
        model: &ModelRef,
        device: ComputeDevice,
        expected_speakers: Option<u32>,
    ) -> Result<EngineOutput, NativeDiarizationError> {
        if expected_speakers == Some(0) {
            return Err(NativeDiarizationError::InvalidOutput(
                "expected speaker count must be greater than zero".into(),
            ));
        }
        self.probe(model)?;
        let bundle = NativeModelBundle::from_model(model)?;
        let mode = execution_mode(device)?;
        let mut pipeline = OwnedDiarizationPipeline::from_dir(bundle.root(), mode)
            .map_err(|error| NativeDiarizationError::Inference(error.to_string()))?;
        let result = pipeline
            .run(samples)
            .map_err(|error| NativeDiarizationError::Inference(error.to_string()))?;
        let embedding_space = EmbeddingSpace {
            backend_id: "diarization-native".into(),
            model_id: "wespeaker-voxceleb-resnet34".into(),
            model_revision: model
                .revision
                .clone()
                .or_else(|| Some(SPEAKRS_MODEL_REVISION.into())),
            dimension: result.embeddings.shape().get(2).copied().unwrap_or(0),
            sample_rate_hz: 16_000,
            frontend: EmbeddingFrontend {
                extractor: EmbeddingFeatureExtractor::KaldiFbank,
                mel_bins: 80,
                frame_length_ms: 25,
                frame_shift_ms: 10,
                mean_normalization: false,
                dither: true,
                overlap_aware: true,
            },
            legacy_feature_config: None,
            normalization: EmbeddingNormalization::L2,
        };
        let audio_end_ms = (samples.len() as u64).saturating_mul(1_000) / 16_000;
        let step_ms = (pipeline.segmentation_step() * 1_000.0).round() as u64;
        let mut observations = Vec::new();
        let shape = result.embeddings.shape();
        if shape.len() == 3 {
            for chunk_idx in 0..shape[0] {
                for speaker_idx in 0..shape[1] {
                    let cluster = result.hard_clusters[[chunk_idx, speaker_idx]];
                    if cluster < 0 {
                        continue;
                    }
                    let vector = (0..shape[2])
                        .map(|dimension| result.embeddings[[chunk_idx, speaker_idx, dimension]])
                        .collect::<Vec<_>>();
                    if vector.iter().any(|value| !value.is_finite()) {
                        continue;
                    }
                    let activity = if result.segmentations.shape().len() == 3
                        && chunk_idx < result.segmentations.shape()[0]
                        && speaker_idx < result.segmentations.shape()[2]
                    {
                        let frames = result.segmentations.shape()[1].max(1);
                        (0..frames)
                            .map(|frame| result.segmentations[[chunk_idx, frame, speaker_idx]])
                            .sum::<f32>()
                            / frames as f32
                    } else {
                        1.0
                    };
                    let start_ms = chunk_idx as u64 * step_ms;
                    observations.push(SpeakerEmbeddingObservation {
                        id: uuid::Uuid::new_v4().to_string(),
                        cluster_id: cluster as u32,
                        start_ms,
                        end_ms: start_ms.saturating_add(10_000).min(audio_end_ms),
                        vector,
                        quality: activity.clamp(0.0, 1.0),
                        embedding_space: embedding_space.clone(),
                    });
                }
            }
        }
        let segments = result
            .segments
            .into_iter()
            .map(|segment| {
                let cluster_id = parse_speaker_label(&segment.speaker)?;
                let start_ms = seconds_to_ms(segment.start)?;
                let end_ms = seconds_to_ms(segment.end)?;
                if end_ms <= start_ms {
                    return Err(NativeDiarizationError::InvalidOutput(format!(
                        "segment has invalid range {start_ms}..{end_ms}"
                    )));
                }
                Ok(EngineSegment {
                    cluster_id,
                    start_ms,
                    end_ms,
                    confidence: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let speaker_count = segments
            .iter()
            .map(|segment| segment.cluster_id)
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);
        Ok(EngineOutput {
            segments,
            observations,
            speaker_count,
        })
    }
}

fn execution_mode(device: ComputeDevice) -> Result<ExecutionMode, NativeDiarizationError> {
    match device {
        ComputeDevice::Auto | ComputeDevice::Cpu => Ok(ExecutionMode::Cpu),
        ComputeDevice::Metal | ComputeDevice::CoreMl => {
            #[cfg(feature = "coreml")]
            {
                Ok(ExecutionMode::CoreMl)
            }
            #[cfg(not(feature = "coreml"))]
            {
                Err(NativeDiarizationError::DeviceUnavailable(
                    "build diarization-native with the `coreml` feature".into(),
                ))
            }
        }
    }
}

fn parse_speaker_label(label: &str) -> Result<u32, NativeDiarizationError> {
    label
        .strip_prefix("SPEAKER_")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            NativeDiarizationError::InvalidOutput(format!(
                "unexpected speakrs speaker label `{label}`"
            ))
        })
}

fn seconds_to_ms(seconds: f64) -> Result<u64, NativeDiarizationError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(NativeDiarizationError::InvalidOutput(format!(
            "invalid timestamp {seconds}"
        )));
    }
    Ok((seconds * 1_000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::REQUIRED_CPU_ARTIFACTS;

    struct FakeSegmentation;

    impl SegmentationAdapter for FakeSegmentation {
        fn probe(
            &self,
            _bundle: &NativeModelBundle,
            _device: ComputeDevice,
        ) -> Result<(), NativeDiarizationError> {
            Ok(())
        }

        fn segment(
            &self,
            _samples: &[f32],
            _bundle: &NativeModelBundle,
            _device: ComputeDevice,
        ) -> Result<SegmentationOutput, NativeDiarizationError> {
            Ok(SegmentationOutput {
                tracks: vec![SegmentedTrack {
                    track_id: "track-a".into(),
                    start_ms: 100,
                    end_ms: 900,
                    confidence: 0.9,
                }],
            })
        }
    }

    struct FakeEmbedding;

    impl EmbeddingAdapter for FakeEmbedding {
        fn probe(
            &self,
            _bundle: &NativeModelBundle,
            _device: ComputeDevice,
        ) -> Result<(), NativeDiarizationError> {
            Ok(())
        }

        fn embed(
            &self,
            _samples: &[f32],
            _segmentation: &SegmentationOutput,
            _bundle: &NativeModelBundle,
            _model: &ModelRef,
            _device: ComputeDevice,
        ) -> Result<EmbeddingOutput, NativeDiarizationError> {
            Ok(EmbeddingOutput {
                embedding_space: Some(EmbeddingSpace {
                    backend_id: "diarization-native".into(),
                    model_id: "fake-embedding".into(),
                    model_revision: Some("1".into()),
                    dimension: 2,
                    sample_rate_hz: 16_000,
                    frontend: EmbeddingFrontend {
                        extractor: EmbeddingFeatureExtractor::KaldiFbank,
                        mel_bins: 80,
                        frame_length_ms: 25,
                        frame_shift_ms: 10,
                        mean_normalization: false,
                        dither: false,
                        overlap_aware: false,
                    },
                    legacy_feature_config: None,
                    normalization: EmbeddingNormalization::L2,
                }),
                embeddings: vec![UnclusteredEmbedding {
                    track_id: "track-a".into(),
                    vector: vec![1.0, 0.0],
                    quality: 0.9,
                }],
            })
        }
    }

    struct FakeClustering;

    impl ClusteringAdapter for FakeClustering {
        fn cluster(
            &self,
            _segmentation: &SegmentationOutput,
            _embeddings: &EmbeddingOutput,
            _expected_speakers: Option<u32>,
        ) -> Result<ClusteringOutput, NativeDiarizationError> {
            Ok(ClusteringOutput {
                assignments: vec![ClusterAssignment {
                    track_id: "track-a".into(),
                    cluster_id: 7,
                }],
            })
        }
    }

    #[test]
    fn independent_stage_adapters_form_a_complete_pipeline() {
        let root = std::env::temp_dir().join(format!("meetnote-stages-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        for filename in REQUIRED_CPU_ARTIFACTS {
            std::fs::write(root.join(filename), []).unwrap();
        }
        let model = ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "fixture".into(),
            revision: Some("1".into()),
            artifact_path: root.display().to_string(),
        };
        let engine = StagePipelineEngine::new(FakeSegmentation, FakeEmbedding, FakeClustering);

        let output = engine
            .run(&[0.0; 16_000], &model, ComputeDevice::Cpu, Some(1))
            .unwrap();

        assert_eq!(output.speaker_count, 1);
        assert_eq!(output.segments[0].cluster_id, 7);
        assert_eq!(output.observations[0].cluster_id, 7);
        assert_eq!(output.observations[0].start_ms, 100);
        std::fs::remove_dir_all(root).unwrap();
    }
}
