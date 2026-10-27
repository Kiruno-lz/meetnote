use application::{
    asr_descriptor, run_backend_worker, AsrBackend, AsrWindowSession, BackendFuture,
};
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
        run_backend_worker(context, move || {
            transcribe_sync(request, worker_context, threads)
        })
    }

    fn supports_windowed_audio(&self) -> bool {
        true
    }

    fn create_window_session(
        &self,
        request: AsrRequest,
    ) -> Result<Box<dyn AsrWindowSession>, BackendError> {
        validate_device(request.device)?;
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
        if context.is_cancelled() {
            return Err(cancelled_error("Whisper transcription was cancelled"));
        }
        if is_digital_silence(&window.samples) {
            return Ok(());
        }
        let mut state = self.whisper.create_state().map_err(|error| BackendError {
            code: BackendErrorCode::ModelLoadFailed,
            message: error.to_string(),
        })?;
        let params = whisper_params(
            self.request.language.as_deref(),
            self.threads,
            context.clone(),
        );
        params.full(&mut state, &window.samples).map_err(|error| {
            if context.is_cancelled() {
                cancelled_error("Whisper transcription was cancelled")
            } else {
                BackendError {
                    code: BackendErrorCode::InferenceFailed,
                    message: error.to_string(),
                }
            }
        })?;
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|error| BackendError {
                    code: BackendErrorCode::OutputValidationFailed,
                    message: error.to_string(),
                })?
                .trim()
                .to_string();
            let Some((start_ms, end_ms)) =
                owned_window_range(&window, segment.start_timestamp(), segment.end_timestamp())
            else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            self.segments.push(domain::TranscriptSegment {
                id: String::new(),
                start_ms,
                end_ms,
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
    validate_device(request.device)?;
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
    if context.is_cancelled() {
        return Err(cancelled_error("Whisper transcription was cancelled"));
    }
    if is_digital_silence(&audio.samples) {
        return Ok(AsrResult {
            backend: WhisperRsBackend::default().descriptor(),
            model: request.model,
            language: request.language,
            segments: Vec::new(),
        });
    }
    let mut state = whisper_context
        .create_state()
        .map_err(|error| BackendError {
            code: BackendErrorCode::ModelLoadFailed,
            message: error.to_string(),
        })?;

    let params = whisper_params(request.language.as_deref(), threads, context.clone());

    params.full(&mut state, &audio.samples).map_err(|error| {
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
        let Some((start, end)) = bounded_sample_range(
            segment.start_timestamp(),
            segment.end_timestamp(),
            audio.samples.len() as u64,
        ) else {
            continue;
        };
        let start_ms = sample_to_ms(start);
        let end_ms = sample_to_ms(end);
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

fn is_digital_silence(samples: &[f32]) -> bool {
    !samples.is_empty() && samples.iter().all(|sample| *sample == 0.0)
}

fn validate_device(device: ComputeDevice) -> Result<(), BackendError> {
    if device == ComputeDevice::CoreMl {
        return Err(BackendError {
            code: BackendErrorCode::DeviceUnavailable,
            message: "this whisper-rs build enables Metal, not Core ML; select Auto, CPU or Metal"
                .into(),
        });
    }
    Ok(())
}

/// The raw callback's owned pointee stays alive until synchronous inference ends.
struct WhisperParameters<'a> {
    params: FullParams<'a, 'static>,
    cancellation: Box<JobContext>,
}

impl WhisperParameters<'_> {
    fn full(
        self,
        state: &mut whisper_rs::WhisperState,
        samples: &[f32],
    ) -> Result<(), whisper_rs::WhisperError> {
        let result = state.full(self.params, samples);
        drop(self.cancellation);
        result
    }
}

unsafe extern "C" fn should_abort(user_data: *mut std::ffi::c_void) -> bool {
    // SAFETY: WhisperParameters owns this stable Box<JobContext> for the whole
    // synchronous full call. whisper.cpp only reads it; it never owns or frees it.
    let context = unsafe { &*(user_data.cast::<JobContext>()) };
    context.is_cancelled()
}

fn whisper_params<'a>(
    language: Option<&'a str>,
    threads: i32,
    context: JobContext,
) -> WhisperParameters<'a> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let mut cancellation = Box::new(context);
    // whisper-rs 0.16.0's safe wrapper passes Box<dyn FnMut> userdata to a
    // trampoline that casts it to the concrete closure F. Use the typed C API
    // with an owned stable pointee instead; no closure leaks or ABI mismatch.
    unsafe {
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data((&mut *cancellation as *mut JobContext).cast());
    }
    match language {
        Some("auto") | None => {
            params.set_language(Some("auto"));
            params.set_detect_language(false);
        }
        Some(language) => params.set_language(Some(language)),
    }
    WhisperParameters {
        params,
        cancellation,
    }
}

fn timestamp_to_sample(timestamp_10ms: i64) -> u64 {
    (timestamp_10ms.max(0) as u64).saturating_mul(u64::from(TARGET_SAMPLE_RATE_HZ) / 100)
}

fn bounded_sample_range(start: i64, end: i64, source_samples: u64) -> Option<(u64, u64)> {
    // Whisper timestamps include its padded inference context. Both whole-file
    // and windowed output belong only to the decoded source samples.
    let start = timestamp_to_sample(start).min(source_samples);
    let end = timestamp_to_sample(end).min(source_samples);
    (start < end).then_some((start, end))
}

fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / u64::from(TARGET_SAMPLE_RATE_HZ)
}

fn owned_window_range(window: &AudioWindow, start: i64, end: i64) -> Option<(u64, u64)> {
    let (local_start, local_end) = bounded_sample_range(start, end, window.samples.len() as u64)?;
    let global_start = window.global_start_sample.saturating_add(local_start);
    let global_end = window.global_start_sample.saturating_add(local_end);
    let midpoint = global_start.saturating_add((global_end - global_start) / 2);
    if !core_owns(window, midpoint) {
        return None;
    }
    let start_ms = sample_to_ms(global_start);
    let end_ms = sample_to_ms(global_end);
    (start_ms < end_ms).then_some((start_ms, end_ms))
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

    fn final_single_sample_core(core_extra_samples: u64) -> AudioWindow {
        let core_samples = 60 * u64::from(TARGET_SAMPLE_RATE_HZ) + core_extra_samples;
        let plan = audio::AudioWindowPlan::new(core_samples, 15 * u64::from(TARGET_SAMPLE_RATE_HZ))
            .unwrap();
        let end = core_samples + 1;
        let bounds = plan.bounds_for_total(end).last().unwrap();
        AudioWindow {
            sequence: bounds.sequence,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
            samples: vec![0.5; (bounds.global_end_sample - bounds.global_start_sample) as usize]
                .into(),
            global_start_sample: bounds.global_start_sample,
            global_end_sample: bounds.global_end_sample,
            valid_core_start_sample: bounds.valid_core_start_sample,
            valid_core_end_sample: bounds.valid_core_end_sample,
            source_total_samples: Some(end),
            is_final: true,
        }
    }

    #[test]
    fn window_sample_clipping_does_not_create_zero_millisecond_segments() {
        let window = final_single_sample_core(0);
        assert_eq!(
            bounded_sample_range(1_500, 1_501, window.samples.len() as u64),
            Some((240_000, 240_001))
        );
        assert_eq!(owned_window_range(&window, 1_500, 1_501), None);
    }

    #[test]
    fn keeps_a_short_tail_that_crosses_a_global_millisecond_boundary() {
        let window = final_single_sample_core(15);
        assert_eq!(
            owned_window_range(&window, 1_500, 1_501),
            Some((60_000, 60_001))
        );
    }

    #[test]
    fn window_time_mapping_preserves_midpoint_ownership_and_source_bounds() {
        let window = AudioWindow {
            sequence: 2,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
            samples: vec![0.5; 48_000].into(),
            global_start_sample: 16_000,
            global_end_sample: 64_000,
            valid_core_start_sample: 32_000,
            valid_core_end_sample: 48_000,
            source_total_samples: Some(80_000),
            is_final: false,
        };
        assert_eq!(owned_window_range(&window, 0, 100), None);
        assert_eq!(owned_window_range(&window, 100, 200), Some((2_000, 3_000)));
        assert_eq!(owned_window_range(&window, 200, 400), None);
    }

    #[test]
    fn digital_silence_requires_a_nonempty_exact_zero_waveform() {
        assert!(is_digital_silence(&[0.0, -0.0, 0.0]));
        assert!(!is_digital_silence(&[]));
    }

    #[test]
    fn weak_or_nonfinite_samples_are_never_classified_as_digital_silence() {
        for sample in [
            f32::from_bits(1),
            -f32::from_bits(1),
            f32::NAN,
            f32::INFINITY,
        ] {
            assert!(!is_digital_silence(&[0.0, sample, 0.0]));
        }
    }

    #[test]
    fn timestamp_ranges_preserve_source_overlap_without_exposing_padding() {
        let source_samples = 8670 * 16;
        assert_eq!(
            bounded_sample_range(0, 900, source_samples),
            Some((0, source_samples))
        );
        assert_eq!(
            bounded_sample_range(100, 200, source_samples),
            Some((16000, 32000))
        );
        assert_eq!(
            bounded_sample_range(-10, 10, source_samples),
            Some((0, 1600))
        );
        assert_eq!(bounded_sample_range(900, 1000, source_samples), None);
        assert_eq!(bounded_sample_range(10, 10, source_samples), None);
        assert_eq!(bounded_sample_range(20, 10, source_samples), None);
        assert_eq!(
            bounded_sample_range(0, i64::MAX, source_samples),
            Some((0, source_samples))
        );
    }

    #[test]
    fn an_uncompiled_coreml_backend_cannot_be_labelled_as_metal() {
        let request = AsrRequest {
            audio_path: "unused.wav".into(),
            model: ModelRef {
                backend_id: "whisper-rs".into(),
                model_id: "whisper-base-en-ggml".into(),
                revision: None,
                artifact_path: "missing.bin".into(),
                artifacts: Vec::new(),
            },
            language: Some("en".into()),
            device: ComputeDevice::CoreMl,
        };
        let error = WhisperRsBackend::default()
            .create_window_session(request)
            .err()
            .unwrap();
        assert_eq!(error.code, BackendErrorCode::DeviceUnavailable);
    }

    #[test]
    fn cancellation_userdata_survives_parameter_moves_and_observes_cancellation() {
        let (context, _) = runtime::JobRuntime::default().create_job();
        let prepared = whisper_params(Some("en"), 4, context.clone());
        let pointer = (&*prepared.cancellation as *const JobContext)
            .cast_mut()
            .cast();
        let moved = Box::new(prepared);
        // SAFETY: moved still owns the exact pointee passed to the native API.
        assert!(!unsafe { should_abort(pointer) });
        context.cancel();
        assert!(unsafe { should_abort(pointer) });
        drop(moved);
    }

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
