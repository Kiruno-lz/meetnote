use crate::engine::{execution_mode, validate_cluster_layout, SPEAKRS_MODEL_REVISION};
use crate::model::{prepare_model_bundle, PreparedNativeModelBundle};
use crate::{
    EmbeddingFeatureExtractor, EmbeddingFrontend, EmbeddingNormalization, EmbeddingSpace,
    NativeDiarizationError, SpeakerEmbeddingObservation,
};
use audio::{AudioWindow, TARGET_SAMPLE_RATE_HZ};
use contracts::{BackendDescriptor, DiarizationRequest, DiarizationResult};
use runtime::JobContext;
use speakrs::pipeline::InferenceArtifacts;
use speakrs::{EmbeddingObservations, OwnedDiarizationPipeline, PipelineConfig};
use std::collections::BTreeSet;
use std::sync::Arc;

pub trait NativeDiarizationWindowSession: Send {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), NativeDiarizationError>;
    fn finish(self: Box<Self>) -> Result<DiarizationResult, NativeDiarizationError>;
}

#[derive(Debug, Clone, Copy)]
struct WindowTimeline {
    sequence: u64,
    global_start_sample: u64,
    valid_core_start_sample: u64,
    valid_core_end_sample: u64,
    is_final: bool,
}

struct WindowInference {
    timeline: WindowTimeline,
    artifacts: InferenceArtifacts,
}

pub(crate) struct SpeakrsWindowSession {
    request: DiarizationRequest,
    descriptor: BackendDescriptor,
    pipeline: OwnedDiarizationPipeline,
    pipeline_config: PipelineConfig,
    _prepared_bundle: PreparedNativeModelBundle,
    windows: Vec<WindowInference>,
    clustering_vectors: Vec<Vec<f32>>,
}

impl SpeakrsWindowSession {
    pub(crate) fn new(
        request: DiarizationRequest,
        descriptor: BackendDescriptor,
    ) -> Result<Self, NativeDiarizationError> {
        if request.expected_speakers == Some(0) {
            return Err(NativeDiarizationError::InvalidOutput(
                "expected speaker count must be greater than zero".into(),
            ));
        }
        let prepared_bundle = prepare_model_bundle(&request.model, &request.stage_configuration)?;
        prepared_bundle.probe_cpu()?;
        let mode = execution_mode(request.device)?;
        let pipeline = OwnedDiarizationPipeline::from_dir(prepared_bundle.root(), mode)
            .map_err(|error| NativeDiarizationError::Inference(error.to_string()))?;
        let pipeline_config = pipeline.pipeline_config();
        Ok(Self {
            request,
            descriptor,
            pipeline,
            pipeline_config,
            _prepared_bundle: prepared_bundle,
            windows: Vec::new(),
            clustering_vectors: Vec::new(),
        })
    }

    fn extract_window(&mut self, window: &AudioWindow) -> Result<(), NativeDiarizationError> {
        if window.sample_rate != TARGET_SAMPLE_RATE_HZ {
            return Err(NativeDiarizationError::InvalidAudio(format!(
                "window {} has sample rate {}, expected {}",
                window.sequence, window.sample_rate, TARGET_SAMPLE_RATE_HZ
            )));
        }
        let artifacts = self
            .pipeline
            .run_inference(&window.samples)
            .map_err(|error| NativeDiarizationError::Inference(error.to_string()))?;
        let timeline = WindowTimeline {
            sequence: window.sequence,
            global_start_sample: window.global_start_sample,
            valid_core_start_sample: window.valid_core_start_sample,
            valid_core_end_sample: window.valid_core_end_sample,
            is_final: window.is_final,
        };
        let observations = artifacts.embedding_observations();
        let step_samples =
            (self.pipeline.segmentation_step() * f64::from(TARGET_SAMPLE_RATE_HZ)).round() as u64;
        let inference_window_samples = self.pipeline.segmentation_window_samples() as u64;
        for observation in observations.iter() {
            let origin = observation.origin.ok_or_else(|| {
                NativeDiarizationError::InvalidOutput(
                    "inference embedding observation is missing its chunk origin".into(),
                )
            })?;
            let local_start = (origin.chunk_index as u64).saturating_mul(step_samples);
            let local_midpoint = local_start.saturating_add(inference_window_samples / 2);
            let global_midpoint = window.global_start_sample.saturating_add(local_midpoint);
            if core_owns(timeline, global_midpoint) {
                self.clustering_vectors.push(observation.vector.clone());
            }
        }
        self.windows.push(WindowInference {
            timeline,
            artifacts,
        });
        Ok(())
    }

    fn finish_result(mut self) -> Result<DiarizationResult, NativeDiarizationError> {
        let observations = EmbeddingObservations::from_vectors(&self.clustering_vectors)
            .map_err(|error| NativeDiarizationError::InvalidOutput(error.to_string()))?;
        let clustering = self
            .pipeline
            .cluster_embedding_observations(&observations, &self.pipeline_config);
        let mut segments = Vec::new();
        let mut embedding_observations = Vec::new();
        let mut embedding_space: Option<EmbeddingSpace> = None;
        let step_samples =
            (self.pipeline.segmentation_step() * f64::from(TARGET_SAMPLE_RATE_HZ)).round() as u64;
        let inference_window_samples = self.pipeline.segmentation_window_samples() as u64;

        for window in self.windows.drain(..) {
            let result = self
                .pipeline
                .finish_post_inference_with_clustering(
                    window.artifacts,
                    &clustering,
                    &self.pipeline_config,
                )
                .map_err(|error| NativeDiarizationError::Inference(error.to_string()))?;
            if !validate_cluster_layout(
                result.hard_clusters.shape(),
                result.embeddings.shape(),
                !result.segments.is_empty(),
            )? {
                continue;
            }
            let space = embedding_space_for(&self.request, result.embeddings.shape())?;
            if let Some(existing) = &embedding_space {
                if !existing.compatible_with(&space) {
                    return Err(NativeDiarizationError::InvalidOutput(
                        "embedding space changed between audio windows".into(),
                    ));
                }
            } else {
                embedding_space = Some(space.clone());
            }
            append_window_observations(
                &mut embedding_observations,
                &result,
                window.timeline,
                step_samples,
                inference_window_samples,
                &space,
            );
            append_window_segments(&mut segments, result.segments, window.timeline)?;
        }

        embedding_observations
            .sort_by_key(|observation| (observation.start_ms, observation.end_ms));
        segments.sort_by_key(|(cluster, start, end)| (*start, *end, *cluster));
        let segments = merge_segments(segments)
            .into_iter()
            .enumerate()
            .map(
                |(index, (cluster, start_ms, end_ms))| domain::DiarizationSegment {
                    id: format!("native-diarization-segment-{index}"),
                    speaker_id: format!("speaker-{cluster}"),
                    speaker_profile_id: None,
                    speaker_label: None,
                    start_ms,
                    end_ms,
                    confidence: None,
                },
            )
            .collect::<Vec<_>>();
        let speaker_count = segments
            .iter()
            .filter_map(|segment| parse_domain_speaker_id(&segment.speaker_id))
            .collect::<BTreeSet<_>>()
            .len() as u32;
        debug_assert!(speaker_count as usize <= clustering.cluster_count().max(1));
        Ok(DiarizationResult {
            backend: self.descriptor,
            model: self.request.model,
            segments,
            speaker_count: Some(speaker_count),
            embedding_observations,
            speaker_profile_candidates: Vec::new(),
        })
    }
}

impl NativeDiarizationWindowSession for SpeakrsWindowSession {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), NativeDiarizationError> {
        if context.is_cancelled() {
            return Err(NativeDiarizationError::Cancelled);
        }
        self.extract_window(&window)
    }

    fn finish(self: Box<Self>) -> Result<DiarizationResult, NativeDiarizationError> {
        self.finish_result()
    }
}

fn embedding_space_for(
    request: &DiarizationRequest,
    shape: &[usize],
) -> Result<EmbeddingSpace, NativeDiarizationError> {
    if shape.len() != 3 || shape[2] == 0 {
        return Err(NativeDiarizationError::InvalidOutput(
            "window returned an invalid embedding tensor".into(),
        ));
    }
    Ok(EmbeddingSpace {
        backend_id: "diarization-native".into(),
        model_id: "wespeaker-voxceleb-resnet34".into(),
        model_revision: request
            .model
            .revision
            .clone()
            .or_else(|| Some(SPEAKRS_MODEL_REVISION.into())),
        dimension: shape[2],
        sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
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
    })
}

fn append_window_observations(
    output: &mut Vec<SpeakerEmbeddingObservation>,
    result: &speakrs::DiarizationResult,
    timeline: WindowTimeline,
    step_samples: u64,
    inference_window_samples: u64,
    space: &EmbeddingSpace,
) {
    let shape = result.embeddings.shape();
    let segmentation_shape = result.segmentations.shape();
    for chunk_index in 0..shape[0] {
        for speaker_index in 0..shape[1] {
            let cluster = result.hard_clusters[[chunk_index, speaker_index]];
            if cluster < 0 {
                continue;
            }
            let vector = (0..shape[2])
                .map(|dimension| result.embeddings[[chunk_index, speaker_index, dimension]])
                .collect::<Vec<_>>();
            if vector.iter().any(|value| !value.is_finite()) {
                continue;
            }
            let local_start = (chunk_index as u64).saturating_mul(step_samples);
            let local_end = local_start.saturating_add(inference_window_samples);
            let global_start = timeline.global_start_sample.saturating_add(local_start);
            let global_end = timeline.global_start_sample.saturating_add(local_end);
            let midpoint = global_start.saturating_add((global_end - global_start) / 2);
            if !core_owns(timeline, midpoint) {
                continue;
            }
            let activity = if segmentation_shape.len() == 3
                && chunk_index < segmentation_shape[0]
                && speaker_index < segmentation_shape[2]
            {
                let frames = segmentation_shape[1].max(1);
                (0..frames)
                    .map(|frame| result.segmentations[[chunk_index, frame, speaker_index]])
                    .sum::<f32>()
                    / frames as f32
            } else {
                1.0
            };
            let start = global_start.max(timeline.valid_core_start_sample);
            let end = global_end.min(timeline.valid_core_end_sample);
            let Some((start_ms, end_ms)) = positive_millisecond_range(start, end) else {
                continue;
            };
            output.push(SpeakerEmbeddingObservation {
                id: format!(
                    "native-window-{}-embedding-{}-{}",
                    timeline.sequence, chunk_index, speaker_index
                ),
                cluster_id: cluster as u32,
                start_ms,
                end_ms,
                vector,
                quality: activity.clamp(0.0, 1.0),
                embedding_space: space.clone(),
            });
        }
    }
}

fn append_window_segments(
    output: &mut Vec<(u32, u64, u64)>,
    segments: Vec<speakrs::Segment>,
    timeline: WindowTimeline,
) -> Result<(), NativeDiarizationError> {
    for segment in segments {
        let cluster = parse_speaker_label(&segment.speaker)?;
        let start = timeline
            .global_start_sample
            .saturating_add(seconds_to_sample(segment.start)?)
            .max(timeline.valid_core_start_sample);
        let end = timeline
            .global_start_sample
            .saturating_add(seconds_to_sample(segment.end)?)
            .min(timeline.valid_core_end_sample);
        if let Some((start_ms, end_ms)) = positive_millisecond_range(start, end) {
            output.push((cluster, start_ms, end_ms));
        }
    }
    Ok(())
}

fn merge_segments(segments: Vec<(u32, u64, u64)>) -> Vec<(u32, u64, u64)> {
    let mut merged: Vec<(u32, u64, u64)> = Vec::new();
    for segment in segments {
        if let Some(previous) = merged.last_mut() {
            if previous.0 == segment.0 && segment.1 <= previous.2 {
                previous.2 = previous.2.max(segment.2);
                continue;
            }
        }
        merged.push(segment);
    }
    merged
}

fn core_owns(timeline: WindowTimeline, sample: u64) -> bool {
    sample >= timeline.valid_core_start_sample
        && (sample < timeline.valid_core_end_sample
            || (timeline.is_final && sample == timeline.valid_core_end_sample))
}

fn seconds_to_sample(seconds: f64) -> Result<u64, NativeDiarizationError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(NativeDiarizationError::InvalidOutput(format!(
            "invalid timestamp {seconds}"
        )));
    }
    Ok((seconds * f64::from(TARGET_SAMPLE_RATE_HZ)).round() as u64)
}

fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / u64::from(TARGET_SAMPLE_RATE_HZ)
}

fn positive_millisecond_range(start: u64, end: u64) -> Option<(u64, u64)> {
    let start_ms = sample_to_ms(start);
    let end_ms = sample_to_ms(end);
    (start_ms < end_ms).then_some((start_ms, end_ms))
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

fn parse_domain_speaker_id(label: &str) -> Option<u32> {
    label.strip_prefix("speaker-")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, Array3};
    use speakrs::pipeline::{
        ChunkEmbeddings, ChunkSpeakerClusters, DecodedSegmentations, DiscreteDiarization,
        SpeakerCountTrack,
    };

    fn single_sample_tail(context_seconds: u64, extra_core_samples: u64) -> WindowTimeline {
        let core = 60 * u64::from(TARGET_SAMPLE_RATE_HZ) + extra_core_samples;
        let plan =
            audio::AudioWindowPlan::new(core, context_seconds * u64::from(TARGET_SAMPLE_RATE_HZ))
                .unwrap();
        let bounds = plan.bounds_for_total(core + 1).last().unwrap();
        WindowTimeline {
            sequence: bounds.sequence,
            global_start_sample: bounds.global_start_sample,
            valid_core_start_sample: bounds.valid_core_start_sample,
            valid_core_end_sample: bounds.valid_core_end_sample,
            is_final: true,
        }
    }

    // A typed export fixture, not a model inference or quality measurement.
    fn one_speaker_result() -> speakrs::DiarizationResult {
        let mut embeddings = Array3::zeros((1, 1, 256));
        embeddings[[0, 0, 0]] = 1.0;
        speakrs::DiarizationResult {
            segmentations: DecodedSegmentations(Array3::from_elem((1, 1, 1), 1.0)),
            embeddings: ChunkEmbeddings(embeddings),
            speaker_count: SpeakerCountTrack(vec![1]),
            hard_clusters: ChunkSpeakerClusters(Array2::zeros((1, 1))),
            discrete_diarization: DiscreteDiarization(Array2::from_elem((1, 1), 1.0)),
            segments: vec![speakrs::Segment::new(0.0, 10.0, "SPEAKER_00")],
        }
    }

    fn fixture_space() -> EmbeddingSpace {
        embedding_space_for(
            &DiarizationRequest {
                audio_path: "unused.wav".into(),
                model: contracts::ModelRef {
                    backend_id: "diarization-native".into(),
                    model_id: "fixture".into(),
                    revision: None,
                    artifact_path: "unused".into(),
                    artifacts: Vec::new(),
                },
                expected_speakers: None,
                device: contracts::ComputeDevice::Cpu,
                speaker_library_path: None,
                stage_configuration: Default::default(),
            },
            &[1, 1, 256],
        )
        .unwrap()
    }

    #[test]
    fn sample_clipped_speaker_segments_cannot_collapse_to_zero_milliseconds() {
        let mut output = Vec::new();
        append_window_segments(
            &mut output,
            vec![speakrs::Segment::new(0.0, 16.0, "SPEAKER_00")],
            single_sample_tail(15, 0),
        )
        .unwrap();
        assert!(output.is_empty(), "zero-ms speaker segment: {output:?}");
    }

    #[test]
    fn sample_clipped_observations_cannot_collapse_to_zero_milliseconds() {
        let mut output = Vec::new();
        append_window_observations(
            &mut output,
            &one_speaker_result(),
            single_sample_tail(5, 0),
            16_000,
            160_000,
            &fixture_space(),
        );
        assert!(
            output.is_empty(),
            "zero-ms embedding observation: {output:?}"
        );
    }

    #[test]
    fn source_sample_tail_crossing_a_millisecond_keeps_segments_and_observations() {
        let mut segments = Vec::new();
        append_window_segments(
            &mut segments,
            vec![speakrs::Segment::new(0.0, 16.0, "SPEAKER_00")],
            single_sample_tail(15, 15),
        )
        .unwrap();
        assert_eq!(segments, [(0, 60_000, 60_001)]);
        let mut observations = Vec::new();
        let result = one_speaker_result();
        let space = fixture_space();
        append_window_observations(
            &mut observations,
            &result,
            single_sample_tail(5, 15),
            16_000,
            160_000,
            &space,
        );
        assert_eq!(observations.len(), 1);
        let observation = &observations[0];
        assert_eq!((observation.start_ms, observation.end_ms), (60_000, 60_001));
        assert_eq!(observation.cluster_id, 0);
        assert_eq!(
            observation.vector,
            result.embeddings.iter().copied().collect::<Vec<_>>()
        );
        assert_eq!(observation.quality, 1.0);
        assert!(observation.embedding_space.compatible_with(&space));
    }

    #[test]
    fn seam_reconciliation_preserves_overlap_between_different_speakers() {
        let merged = merge_segments(vec![(0, 0, 1_000), (1, 500, 1_500), (0, 900, 2_000)]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], (0, 0, 1_000));
    }

    #[test]
    fn adjacent_same_speaker_fragments_are_merged() {
        let merged = merge_segments(vec![(0, 0, 1_000), (0, 1_000, 2_000)]);
        assert_eq!(merged, vec![(0, 0, 2_000)]);
    }
}
