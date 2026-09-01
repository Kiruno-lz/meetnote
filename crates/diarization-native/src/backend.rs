use crate::{
    IdentityResolverConfig, JsonSpeakerProfileStore, NativeDiarizationEngine,
    NativeDiarizationError, NativeDiarizationOutput, SpeakerEmbeddingObservation,
    SpeakerLibraryService, SpeakrsEngine,
};
use application::{diarization_descriptor, BackendFuture, DiarizationBackend};
use audio::read_audio_mono_16k;
use contracts::{BackendDescriptor, BackendError, DiarizationRequest, DiarizationResult, ModelRef};
use runtime::{spawn_blocking, JobContext, Progress};
use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

pub type NativeBackendFuture<T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send>>;

#[derive(Debug, Clone)]
pub struct NativeDiarizationConfig {
    pub speaker_library_path: PathBuf,
    pub identity: IdentityResolverConfig,
}

impl Default for NativeDiarizationConfig {
    fn default() -> Self {
        Self {
            speaker_library_path: PathBuf::from("data/speakers/library.json"),
            identity: IdentityResolverConfig::default(),
        }
    }
}

#[derive(Clone)]
pub struct NativeDiarizationBackend {
    engine: Arc<dyn NativeDiarizationEngine>,
    library: SpeakerLibraryService,
    config: NativeDiarizationConfig,
}

impl Default for NativeDiarizationBackend {
    fn default() -> Self {
        Self::new(NativeDiarizationConfig::default())
    }
}

impl NativeDiarizationBackend {
    pub fn new(config: NativeDiarizationConfig) -> Self {
        let store = Arc::new(JsonSpeakerProfileStore::new(
            config.speaker_library_path.clone(),
        ));
        Self::with_components(
            Arc::new(SpeakrsEngine),
            SpeakerLibraryService::new(store),
            config,
        )
    }

    pub fn with_components(
        engine: Arc<dyn NativeDiarizationEngine>,
        library: SpeakerLibraryService,
        config: NativeDiarizationConfig,
    ) -> Self {
        Self {
            engine,
            library,
            config,
        }
    }

    pub fn library(&self) -> &SpeakerLibraryService {
        &self.library
    }

    pub fn config(&self) -> &NativeDiarizationConfig {
        &self.config
    }

    pub fn analyze(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> NativeBackendFuture<NativeDiarizationOutput> {
        let backend = self.clone();
        Box::pin(spawn_blocking(move || {
            backend
                .analyze_sync(request, context)
                .map_err(BackendError::from)
        }))
    }

    fn analyze_sync(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> Result<NativeDiarizationOutput, NativeDiarizationError> {
        ensure_not_cancelled(&context)?;
        self.engine.probe(&request.model)?;
        let audio = read_audio_mono_16k(&request.audio_path)
            .map_err(|error| NativeDiarizationError::InvalidAudio(error.to_string()))?;
        context.report(Progress {
            completed: 1,
            total: Some(4),
        });
        ensure_not_cancelled(&context)?;

        let engine_output = self.engine.run(
            &audio.samples,
            &request.model,
            request.device,
            request.expected_speakers,
        )?;
        validate_engine_output(&engine_output.observations, &engine_output.segments)?;
        context.report(Progress {
            completed: 3,
            total: Some(4),
        });
        ensure_not_cancelled(&context)?;

        let all_clusters = engine_output
            .segments
            .iter()
            .map(|segment| segment.cluster_id)
            .collect::<BTreeSet<_>>();
        let segments = engine_output
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| domain::DiarizationSegment {
                id: format!("native-diarization-segment-{index}"),
                speaker_id: format!("speaker-{}", segment.cluster_id),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                confidence: segment.confidence,
            })
            .collect();
        let speaker_count = if engine_output.speaker_count == 0 && !all_clusters.is_empty() {
            Some(all_clusters.len() as u32)
        } else {
            Some(engine_output.speaker_count)
        };
        context.report(Progress {
            completed: 4,
            total: Some(4),
        });

        Ok(NativeDiarizationOutput {
            result: DiarizationResult {
                backend: self.descriptor(),
                model: request.model,
                segments,
                speaker_count,
                embedding_observations: engine_output.observations.clone(),
            },
            observations: engine_output.observations,
            anonymous_clusters: all_clusters.into_iter().collect(),
        })
    }
}

impl DiarizationBackend for NativeDiarizationBackend {
    fn descriptor(&self) -> BackendDescriptor {
        diarization_descriptor(
            "diarization-native",
            &[
                "offline",
                "onnx",
                "overlap-aware",
                "segmentation",
                "speaker-embedding",
                "ahc-plda-vbx",
                "speaker-profile-persistence",
                "automatic-speaker-labeling",
                "speaker-count-hint",
            ],
        )
    }

    fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
        self.engine.probe(model).map_err(BackendError::from)
    }

    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult> {
        let future = self.analyze(request, context);
        Box::pin(async move { future.await.map(|output| output.result) })
    }
}

fn ensure_not_cancelled(context: &JobContext) -> Result<(), NativeDiarizationError> {
    if context.is_cancelled() {
        Err(NativeDiarizationError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_engine_output(
    observations: &[SpeakerEmbeddingObservation],
    segments: &[crate::EngineSegment],
) -> Result<(), NativeDiarizationError> {
    for segment in segments {
        if segment.end_ms <= segment.start_ms {
            return Err(NativeDiarizationError::InvalidOutput(format!(
                "segment {} has an empty or reversed time range",
                segment.cluster_id
            )));
        }
        if segment
            .confidence
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(NativeDiarizationError::InvalidOutput(
                "segment confidence must be between zero and one".into(),
            ));
        }
    }
    for observation in observations {
        if observation.vector.len() != observation.embedding_space.dimension
            || observation.vector.iter().any(|value| !value.is_finite())
            || observation.quality < 0.0
            || observation.quality > 1.0
            || !observation.quality.is_finite()
        {
            return Err(NativeDiarizationError::InvalidOutput(format!(
                "embedding observation {} is invalid",
                observation.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EmbeddingFeatureExtractor, EmbeddingFrontend, EmbeddingNormalization, EmbeddingSpace,
        EngineOutput, EngineSegment, InMemorySpeakerProfileStore, SpeakerSampleSource,
    };
    use contracts::{ComputeDevice, ModelRef};
    use runtime::JobRuntime;

    struct FakeEngine {
        output: EngineOutput,
    }

    impl NativeDiarizationEngine for FakeEngine {
        fn probe(&self, _model: &ModelRef) -> Result<(), NativeDiarizationError> {
            Ok(())
        }

        fn run(
            &self,
            _samples: &[f32],
            _model: &ModelRef,
            _device: ComputeDevice,
            _expected_speakers: Option<u32>,
        ) -> Result<EngineOutput, NativeDiarizationError> {
            Ok(self.output.clone())
        }
    }

    fn observation(vector: Vec<f32>) -> SpeakerEmbeddingObservation {
        SpeakerEmbeddingObservation {
            id: "observation".into(),
            cluster_id: 0,
            start_ms: 0,
            end_ms: 1_000,
            quality: 1.0,
            embedding_space: EmbeddingSpace {
                backend_id: "diarization-native".into(),
                model_id: "wespeaker".into(),
                model_revision: Some("1".into()),
                dimension: vector.len(),
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
            },
            vector,
        }
    }

    fn write_test_wav() -> PathBuf {
        let path = std::env::temp_dir().join(format!("meetnote-{}.wav", uuid::Uuid::new_v4()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn end_to_end_analysis_applies_a_persistent_identity_match() {
        let store = Arc::new(InMemorySpeakerProfileStore::default());
        let library = SpeakerLibraryService::new(store);
        library
            .enroll(
                "Alice",
                &observation(vec![1.0, 0.0]),
                SpeakerSampleSource {
                    project_id: Some("enrollment".into()),
                    source_segment_id: Some("segment".into()),
                    start_ms: 0,
                    end_ms: 1_000,
                },
                0.7,
            )
            .unwrap();
        let engine = FakeEngine {
            output: EngineOutput {
                segments: vec![EngineSegment {
                    cluster_id: 0,
                    start_ms: 0,
                    end_ms: 1_000,
                    confidence: Some(0.95),
                }],
                observations: vec![observation(vec![0.99, 0.01])],
                speaker_count: 1,
            },
        };
        let backend = NativeDiarizationBackend::with_components(
            Arc::new(engine),
            library,
            NativeDiarizationConfig {
                speaker_library_path: "unused.json".into(),
                identity: IdentityResolverConfig {
                    default_threshold: 0.7,
                    ambiguity_margin: 0.03,
                },
            },
        );
        let audio_path = write_test_wav();
        let (context, _) = JobRuntime::default().create_job();

        let mut output = futures::executor::block_on(backend.analyze(
            DiarizationRequest {
                audio_path: audio_path.display().to_string(),
                model: ModelRef {
                    backend_id: "diarization-native".into(),
                    model_id: "fixture".into(),
                    revision: Some("1".into()),
                    artifact_path: "unused".into(),
                },
                expected_speakers: Some(1),
                device: ComputeDevice::Cpu,
                speaker_library_path: None,
            },
            context,
        ))
        .unwrap();

        let identity_matches = application::apply_identity_matches(
            &mut output.result,
            backend.library(),
            &application::ClusterIdentityResolver::new(IdentityResolverConfig {
                default_threshold: 0.7,
                ambiguity_margin: 0.03,
            }),
        )
        .unwrap();

        assert_eq!(identity_matches[0].display_name, "Alice");
        assert_eq!(
            output.result.segments[0].speaker_label.as_deref(),
            Some("Alice")
        );
        assert!(output.result.segments[0].speaker_profile_id.is_some());
        std::fs::remove_file(audio_path).unwrap();
    }
}
