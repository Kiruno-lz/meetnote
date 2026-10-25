use application::{
    diarization_descriptor, BackendFuture, DiarizationBackend, DiarizationStageProvenance,
};
use audio::read_audio_mono_16k;
use contracts::{
    BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice, DiarizationRequest,
    DiarizationResult, EmbeddingFeatureExtractor, EmbeddingFrontend, EmbeddingNormalization,
    EmbeddingSpace, ModelRef, SpeakerEmbeddingObservation,
};
use runtime::JobContext;
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SherpaOnnxDiarizationBackend {
    pub segmentation_filename: String,
    pub embedding_filename: String,
    pub embedding_model_id: String,
    pub embedding_model_revision: Option<String>,
    pub clustering_threshold: f32,
}

impl Default for SherpaOnnxDiarizationBackend {
    fn default() -> Self {
        Self {
            segmentation_filename: "segmentation.onnx".into(),
            embedding_filename: "embedding.onnx".into(),
            embedding_model_id: "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k".into(),
            embedding_model_revision: None,
            clustering_threshold: 0.5,
        }
    }
}

impl DiarizationBackend for SherpaOnnxDiarizationBackend {
    fn descriptor(&self) -> BackendDescriptor {
        diarization_descriptor(
            "sherpa-onnx",
            &["offline", "onnx", "pyannote-segmentation", "clustering"],
        )
    }

    fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
        let directory = Path::new(&model.artifact_path);
        if !directory.is_dir() {
            return Err(BackendError {
                code: BackendErrorCode::ModelMissing,
                message: format!(
                    "Diarization model directory does not exist: {}",
                    directory.display()
                ),
            });
        }
        for filename in [&self.segmentation_filename, &self.embedding_filename] {
            let path = directory.join(filename);
            if !path.is_file() {
                return Err(BackendError {
                    code: BackendErrorCode::ModelMissing,
                    message: format!(
                        "Diarization model artifact does not exist: {}",
                        path.display()
                    ),
                });
            }
        }
        Ok(())
    }

    fn stage_provenance(&self, model: &ModelRef) -> DiarizationStageProvenance {
        let directory = Path::new(&model.artifact_path);
        DiarizationStageProvenance {
            segmentation: Some(ModelRef {
                backend_id: model.backend_id.clone(),
                model_id: "pyannote-segmentation-3.0".into(),
                revision: model.revision.clone(),
                artifact_path: directory
                    .join(&self.segmentation_filename)
                    .display()
                    .to_string(),
                artifacts: matching_artifacts(model, &self.segmentation_filename),
            }),
            embedding: Some(ModelRef {
                backend_id: model.backend_id.clone(),
                model_id: self.embedding_model_id.clone(),
                revision: self
                    .embedding_model_revision
                    .clone()
                    .or_else(|| model.revision.clone()),
                artifact_path: directory
                    .join(&self.embedding_filename)
                    .display()
                    .to_string(),
                artifacts: matching_artifacts(model, &self.embedding_filename),
            }),
            clustering: Some(domain::AlgorithmProvenance {
                id: "sherpa-fast-clustering".into(),
                revision: Some("sherpa-onnx-1.13.6".into()),
                parameters: Some(format!("threshold={}", self.clustering_threshold)),
            }),
        }
    }

    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult> {
        let backend = self.clone();
        let worker_context = context.clone();
        Box::pin(context.spawn_blocking(move || diarize_sync(backend, request, worker_context)))
    }
}

fn matching_artifacts(model: &ModelRef, filename: &str) -> Vec<contracts::ModelArtifactRef> {
    model
        .artifacts
        .iter()
        .filter(|artifact| artifact.filename == filename)
        .cloned()
        .collect()
}

fn diarize_sync(
    backend: SherpaOnnxDiarizationBackend,
    request: DiarizationRequest,
    context: JobContext,
) -> Result<DiarizationResult, BackendError> {
    if context.is_cancelled() {
        return Err(BackendError {
            code: BackendErrorCode::Cancelled,
            message: "Diarization was cancelled before loading audio".into(),
        });
    }

    let provider = provider_for(request.device)?.to_string();
    backend.probe(&request.model)?;
    let audio = read_audio_mono_16k(&request.audio_path).map_err(|error| BackendError {
        code: BackendErrorCode::InvalidAudio,
        message: error.to_string(),
    })?;
    context.report(runtime::Progress {
        completed: 1,
        total: Some(2),
    });

    let directory = PathBuf::from(&request.model.artifact_path);
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(path_string(directory.join(&backend.segmentation_filename))),
                ..Default::default()
            },
            provider: Some(provider.clone()),
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(path_string(directory.join(&backend.embedding_filename))),
            provider: Some(provider),
            ..Default::default()
        },
        clustering: FastClusteringConfig {
            num_clusters: request
                .expected_speakers
                .map(|value| value as i32)
                .unwrap_or(-1),
            threshold: backend.clustering_threshold,
        },
        ..Default::default()
    };
    let diarizer = OfflineSpeakerDiarization::create(&config).ok_or_else(|| BackendError {
        code: BackendErrorCode::ModelLoadFailed,
        message: "sherpa-onnx failed to create the diarization pipeline".into(),
    })?;
    let result = diarizer
        .process(&audio.samples)
        .ok_or_else(|| BackendError {
            code: BackendErrorCode::InferenceFailed,
            message: "sherpa-onnx failed to process the audio waveform".into(),
        })?;

    if context.is_cancelled() {
        return Err(BackendError {
            code: BackendErrorCode::Cancelled,
            message: "Diarization was cancelled".into(),
        });
    }

    let raw_segments = result.sort_by_start_time();
    let embedding_space = EmbeddingSpace {
        backend_id: "sherpa-onnx".into(),
        model_id: backend.embedding_model_id.clone(),
        model_revision: backend
            .embedding_model_revision
            .clone()
            .or_else(|| request.model.revision.clone()),
        dimension: 0,
        sample_rate_hz: 16_000,
        frontend: EmbeddingFrontend {
            extractor: EmbeddingFeatureExtractor::KaldiFbank,
            mel_bins: 80,
            frame_length_ms: 25,
            frame_shift_ms: 10,
            mean_normalization: true,
            dither: false,
            overlap_aware: false,
        },
        legacy_feature_config: None,
        normalization: EmbeddingNormalization::L2,
    };
    let extractor = SpeakerEmbeddingExtractor::create(&SpeakerEmbeddingExtractorConfig {
        model: Some(path_string(directory.join(&backend.embedding_filename))),
        provider: Some(provider_for(request.device)?.into()),
        ..Default::default()
    })
    .ok_or_else(|| BackendError {
        code: BackendErrorCode::ModelLoadFailed,
        message: "sherpa-onnx failed to create the speaker embedding extractor".into(),
    })?;
    let embedding_space = EmbeddingSpace {
        dimension: extractor.dim().max(0) as usize,
        ..embedding_space
    };
    let embedding_observations = raw_segments
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| {
            let start_ms = (segment.start.max(0.0) * 1_000.0) as u64;
            let end_ms = (segment.end.max(0.0) * 1_000.0) as u64;
            let start = (start_ms.saturating_mul(16) as usize).min(audio.samples.len());
            let end = (end_ms.saturating_mul(16) as usize).min(audio.samples.len());
            if end <= start {
                return None;
            }
            let stream = extractor.create_stream()?;
            stream.accept_waveform(16_000, &audio.samples[start..end]);
            stream.input_finished();
            if !extractor.is_ready(&stream) {
                return None;
            }
            let vector = extractor.compute(&stream)?;
            if vector.len() != embedding_space.dimension
                || vector.iter().any(|value| !value.is_finite())
            {
                return None;
            }
            Some(SpeakerEmbeddingObservation {
                id: format!("sherpa-embedding-{index}"),
                cluster_id: segment.speaker.max(0) as u32,
                start_ms,
                end_ms,
                vector,
                quality: 1.0,
                embedding_space: embedding_space.clone(),
            })
        })
        .collect::<Vec<_>>();
    let segments = raw_segments
        .into_iter()
        .enumerate()
        .map(|(index, segment)| domain::DiarizationSegment {
            id: format!("sherpa-diarization-segment-{index}"),
            speaker_id: format!("speaker-{}", segment.speaker),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: (segment.start.max(0.0) * 1_000.0) as u64,
            end_ms: (segment.end.max(0.0) * 1_000.0) as u64,
            confidence: None,
        })
        .collect();
    let speaker_count = Some(result.num_speakers().max(0) as u32);
    context.report(runtime::Progress {
        completed: 2,
        total: Some(2),
    });

    Ok(DiarizationResult {
        backend: backend.descriptor(),
        model: request.model,
        segments,
        speaker_count,
        embedding_observations,
        speaker_profile_candidates: Vec::new(),
    })
}

fn provider_for(device: ComputeDevice) -> Result<&'static str, BackendError> {
    match device {
        ComputeDevice::Auto | ComputeDevice::Cpu => Ok("cpu"),
        ComputeDevice::CoreMl | ComputeDevice::Metal => Err(BackendError {
            code: BackendErrorCode::DeviceUnavailable,
            message:
                "the pinned sherpa-onnx runtime cannot verify accelerated execution; select CPU"
                    .into(),
        }),
    }
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unverified_accelerators_before_loading_assets() {
        let runtime = runtime::JobRuntime::default();
        for device in [ComputeDevice::Metal, ComputeDevice::CoreMl] {
            let (context, _) = runtime.create_job();
            let error = diarize_sync(
                SherpaOnnxDiarizationBackend::default(),
                DiarizationRequest {
                    audio_path: "missing.wav".into(),
                    model: ModelRef {
                        backend_id: "sherpa-onnx".into(),
                        model_id: "bundle".into(),
                        revision: None,
                        artifact_path: "/missing/bundle".into(),
                        artifacts: Vec::new(),
                    },
                    expected_speakers: None,
                    device,
                    speaker_library_path: None,
                    stage_configuration: contracts::DiarizationStageConfiguration::default(),
                },
                context,
            )
            .unwrap_err();
            assert_eq!(error.code, BackendErrorCode::DeviceUnavailable);
        }
    }

    #[test]
    fn descriptor_does_not_expose_stage_overrides() {
        let descriptor = SherpaOnnxDiarizationBackend::default().descriptor();
        assert!(descriptor.configurable_diarization_stages.is_empty());
        assert!(descriptor.diarization_stage_options.is_empty());
    }

    #[test]
    fn rejects_stage_overrides_instead_of_silently_ignoring_them() {
        let backend = SherpaOnnxDiarizationBackend::default();
        let error = application::DiarizationBackend::validate_stage_configuration(
            &backend,
            &ModelRef {
                backend_id: "sherpa-onnx".into(),
                model_id: "bundle".into(),
                revision: None,
                artifact_path: "bundle".into(),
                artifacts: Vec::new(),
            },
            &contracts::DiarizationStageConfiguration {
                clustering_algorithm: Some(contracts::AlgorithmRef {
                    id: "unsupported".into(),
                    revision: None,
                    parameters: None,
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::ModelIncompatible);
    }

    #[test]
    fn probe_rejects_a_missing_diarization_bundle() {
        let result = SherpaOnnxDiarizationBackend::default().probe(&ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "sherpa-pyannote-3dspeaker".into(),
            revision: None,
            artifact_path: "/missing/diarization".into(),
            artifacts: Vec::new(),
        });

        assert!(matches!(
            result,
            Err(BackendError {
                code: BackendErrorCode::ModelMissing,
                ..
            })
        ));
    }
}
