use application::{diarization_descriptor, BackendFuture, DiarizationBackend};
use audio::read_wav_mono_16k;
use contracts::{
    BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice, DiarizationRequest,
    DiarizationResult, ModelRef,
};
use runtime::{spawn_blocking, JobContext};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SherpaOnnxDiarizationBackend {
    pub segmentation_filename: String,
    pub embedding_filename: String,
    pub clustering_threshold: f32,
}

impl Default for SherpaOnnxDiarizationBackend {
    fn default() -> Self {
        Self {
            segmentation_filename: "segmentation.onnx".into(),
            embedding_filename: "embedding.onnx".into(),
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

    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult> {
        let backend = self.clone();
        Box::pin(spawn_blocking(move || {
            diarize_sync(backend, request, context)
        }))
    }
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

    backend.probe(&request.model)?;
    let audio = read_wav_mono_16k(&request.audio_path).map_err(|error| BackendError {
        code: BackendErrorCode::InvalidAudio,
        message: error.to_string(),
    })?;
    context.report(runtime::Progress {
        completed: 1,
        total: Some(2),
    });

    let directory = PathBuf::from(&request.model.artifact_path);
    let provider = provider_for(request.device).to_string();
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

    let segments = result
        .sort_by_start_time()
        .into_iter()
        .enumerate()
        .map(|(index, segment)| domain::DiarizationSegment {
            id: format!("sherpa-diarization-segment-{index}"),
            speaker_id: format!("speaker-{}", segment.speaker),
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
    })
}

fn provider_for(device: ComputeDevice) -> &'static str {
    match device {
        ComputeDevice::CoreMl | ComputeDevice::Metal => "coreml",
        ComputeDevice::Auto | ComputeDevice::Cpu => "cpu",
    }
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_rejects_a_missing_diarization_bundle() {
        let result = SherpaOnnxDiarizationBackend::default().probe(&ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "sherpa-pyannote-3dspeaker".into(),
            revision: None,
            artifact_path: "/missing/diarization".into(),
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
