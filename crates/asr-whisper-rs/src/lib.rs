use application::{asr_descriptor, AsrBackend, AsrWindowSession, BackendFuture};
use audio::{read_audio_mono_16k, AudioWindow, TARGET_SAMPLE_RATE_HZ};
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice,
    ModelRef,
};
use runtime::JobContext;
use std::path::Path;
use std::sync::Arc;
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
            &[
                "offline",
                "bounded-offline-windows",
                "timestamps",
                "metal",
                "ggml",
                "gguf",
            ],
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
        let worker_context = context.clone();
        Box::pin(context.spawn_blocking(move || transcribe_sync(request, worker_context, threads)))
    }

    fn supports_windowed_audio(&self) -> bool {
        true
    }

    fn create_window_session(
        &self,
        request: AsrRequest,
    ) -> Result<Box<dyn AsrWindowSession>, BackendError> {
        self.probe(&request.model)?;
        whisper_rs::install_logging_hooks();
        let context_parameters = WhisperContextParameters {
            use_gpu: !matches!(request.device, ComputeDevice::Cpu),
            ..WhisperContextParameters::default()
        };
        let whisper =
            WhisperContext::new_with_params(&request.model.artifact_path, context_parameters)
                .map_err(|error| BackendError {
                    code: BackendErrorCode::ModelLoadFailed,
                    message: error.to_string(),
                })?;
        Ok(Box::new(WhisperWindowSession {
            request,
            whisper,
            threads: self.threads.max(1),
            segments: Vec::new(),
        }))
    }
}

struct WhisperWindowSession {
    request: AsrRequest,
    whisper: WhisperContext,
    threads: i32,
    segments: Vec<domain::TranscriptSegment>,
}

impl AsrWindowSession for WhisperWindowSession {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), BackendError> {
        let mut state = self.whisper.create_state().map_err(|error| BackendError {
            code: BackendErrorCode::ModelLoadFailed,
            message: error.to_string(),
        })?;
        let params = whisper_params(
            self.request.language.as_deref(),
            self.threads,
            context.clone(),
        );
        state.full(params, &window.samples).map_err(|error| {
            if context.is_cancelled() {
                cancelled_error("Whisper transcription was cancelled")
            } else {
                BackendError {
                    code: BackendErrorCode::InferenceFailed,
                    message: error.to_string(),
                }
            }
        })?;
        let window_len = window.samples.len() as u64;
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|error| BackendError {
                    code: BackendErrorCode::OutputValidationFailed,
                    message: error.to_string(),
                })?
                .trim()
                .to_string();
            let local_start = timestamp_to_sample(segment.start_timestamp()).min(window_len);
            let local_end = timestamp_to_sample(segment.end_timestamp()).min(window_len);
            if text.is_empty() || local_end <= local_start {
                continue;
            }
            let global_start = window.global_start_sample.saturating_add(local_start);
            let global_end = window.global_start_sample.saturating_add(local_end);
            let midpoint = global_start.saturating_add((global_end - global_start) / 2);
            if !core_owns(&window, midpoint) {
                continue;
            }
            self.segments.push(domain::TranscriptSegment {
                id: String::new(),
                start_ms: sample_to_ms(global_start),
                end_ms: sample_to_ms(global_end),
                text,
            });
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<AsrResult, BackendError> {
        self.segments
            .sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        let mut stitched: Vec<domain::TranscriptSegment> = Vec::new();
        for segment in self.segments.drain(..) {
            if let Some(previous) = stitched.last_mut() {
                if duplicate_segments(previous, &segment) {
                    if segment.text.chars().count() > previous.text.chars().count() {
                        *previous = segment;
                    }
                    continue;
                }
            }
            stitched.push(segment);
        }
        for (index, segment) in stitched.iter_mut().enumerate() {
            segment.id = format!("whisper-segment-{index}");
        }
        Ok(AsrResult {
            backend: WhisperRsBackend::default().descriptor(),
            model: self.request.model,
            language: self.request.language,
            segments: stitched,
        })
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

    let params = whisper_params(request.language.as_deref(), threads, context.clone());

    state.full(params, &audio.samples).map_err(|error| {
        if context.is_cancelled() {
            cancelled_error("Whisper transcription was cancelled")
        } else {
            BackendError {
                code: BackendErrorCode::InferenceFailed,
                message: error.to_string(),
            }
        }
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

fn whisper_params<'a>(
    language: Option<&'a str>,
    threads: i32,
    context: JobContext,
) -> FullParams<'a, 'static> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_abort_callback_safe(move || context.is_cancelled());
    match language {
        Some("auto") | None => {
            params.set_language(Some("auto"));
            params.set_detect_language(false);
        }
        Some(language) => params.set_language(Some(language)),
    }
    params
}

fn timestamp_to_sample(timestamp_10ms: i64) -> u64 {
    (timestamp_10ms.max(0) as u64).saturating_mul(u64::from(TARGET_SAMPLE_RATE_HZ) / 100)
}

fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / u64::from(TARGET_SAMPLE_RATE_HZ)
}

fn core_owns(window: &AudioWindow, sample: u64) -> bool {
    sample >= window.valid_core_start_sample
        && (sample < window.valid_core_end_sample
            || (window.is_final && sample == window.valid_core_end_sample))
}

fn duplicate_segments(left: &domain::TranscriptSegment, right: &domain::TranscriptSegment) -> bool {
    let overlap = left
        .end_ms
        .min(right.end_ms)
        .saturating_sub(left.start_ms.max(right.start_ms));
    let shorter = (left.end_ms - left.start_ms).min(right.end_ms - right.start_ms);
    if shorter == 0 || overlap.saturating_mul(2) < shorter {
        return false;
    }
    let normalize = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let left = normalize(&left.text);
    let right = normalize(&right.text);
    left == right || left.contains(&right) || right.contains(&left)
}

fn cancelled_error(message: &str) -> BackendError {
    BackendError {
        code: BackendErrorCode::Cancelled,
        message: message.into(),
    }
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

    #[test]
    fn overlap_deduplication_keeps_conflicting_text_visible() {
        let left = domain::TranscriptSegment {
            id: "left".into(),
            start_ms: 1_000,
            end_ms: 2_000,
            text: "same text".into(),
        };
        let duplicate = domain::TranscriptSegment {
            id: "right".into(),
            start_ms: 1_200,
            end_ms: 2_100,
            text: " same text ".into(),
        };
        let conflict = domain::TranscriptSegment {
            text: "different text".into(),
            ..duplicate.clone()
        };

        assert!(duplicate_segments(&left, &duplicate));
        assert!(!duplicate_segments(&left, &conflict));
    }
}
