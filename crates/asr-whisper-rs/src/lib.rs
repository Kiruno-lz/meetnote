use application::{asr_descriptor, AsrBackend, BackendFuture};
use audio::read_audio_mono_16k;
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice,
    ModelRef,
};
use runtime::{spawn_blocking, JobContext};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Clone)]
pub struct WhisperRsBackend {
    pub threads: i32,
}

impl Default for WhisperRsBackend {
    fn default() -> Self {
        Self { threads: 4 }
    }
}

impl AsrBackend for WhisperRsBackend {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor(
            "whisper-rs",
            &["offline", "timestamps", "metal", "ggml", "gguf"],
        )
    }

    fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
        if !Path::new(&model.artifact_path).is_file() {
            return Err(BackendError {
                code: BackendErrorCode::ModelMissing,
                message: format!("Whisper model file does not exist: {}", model.artifact_path),
            });
        }
        Ok(())
    }

    fn transcribe(&self, request: AsrRequest, context: JobContext) -> BackendFuture<AsrResult> {
        let threads = self.threads.max(1);
        Box::pin(spawn_blocking(move || {
            transcribe_sync(request, context, threads)
        }))
    }
}

fn transcribe_sync(
    request: AsrRequest,
    context: JobContext,
    threads: i32,
) -> Result<AsrResult, BackendError> {
    if context.is_cancelled() {
        return Err(BackendError {
            code: BackendErrorCode::Cancelled,
            message: "Whisper transcription was cancelled before loading audio".into(),
        });
    }

    whisper_rs::install_logging_hooks();
    let audio = read_audio_mono_16k(&request.audio_path).map_err(|error| BackendError {
        code: BackendErrorCode::InvalidAudio,
        message: error.to_string(),
    })?;
    let context_parameters = WhisperContextParameters {
        use_gpu: !matches!(request.device, ComputeDevice::Cpu),
        ..WhisperContextParameters::default()
    };

    let whisper_context =
        WhisperContext::new_with_params(&request.model.artifact_path, context_parameters).map_err(
            |error| BackendError {
                code: BackendErrorCode::ModelLoadFailed,
                message: error.to_string(),
            },
        )?;
    let mut state = whisper_context
        .create_state()
        .map_err(|error| BackendError {
            code: BackendErrorCode::ModelLoadFailed,
            message: error.to_string(),
        })?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    match request.language.as_deref() {
        Some("auto") | None => params.set_detect_language(true),
        Some(language) => params.set_language(Some(language)),
    }

    state
        .full(params, &audio.samples)
        .map_err(|error| BackendError {
            code: BackendErrorCode::InferenceFailed,
            message: error.to_string(),
        })?;

    let mut segments = Vec::new();
    for (index, segment) in state.as_iter().enumerate() {
        if context.is_cancelled() {
            return Err(BackendError {
                code: BackendErrorCode::Cancelled,
                message: "Whisper transcription was cancelled".into(),
            });
        }
        let text = segment
            .to_str_lossy()
            .map_err(|error| BackendError {
                code: BackendErrorCode::OutputValidationFailed,
                message: error.to_string(),
            })?
            .trim()
            .to_string();
        let start_ms = (segment.start_timestamp().max(0) as u64) * 10;
        let end_ms = (segment.end_timestamp().max(0) as u64) * 10;
        if text.is_empty() || end_ms <= start_ms {
            continue;
        }
        segments.push(domain::TranscriptSegment {
            id: format!("whisper-segment-{index}"),
            start_ms,
            end_ms,
            text,
        });
    }

    Ok(AsrResult {
        backend: WhisperRsBackend::default().descriptor(),
        model: request.model,
        language: request.language,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_rejects_a_missing_whisper_artifact() {
        let result = WhisperRsBackend::default().probe(&ModelRef {
            backend_id: "whisper-rs".into(),
            model_id: "whisper-large-v3".into(),
            revision: None,
            artifact_path: "/missing/ggml-large-v3.bin".into(),
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
