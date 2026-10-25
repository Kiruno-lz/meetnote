use application::{asr_descriptor, AsrBackend, AsrWindowSession, BackendFuture};
use audio::{read_audio_mono_16k, AudioWindow, TARGET_SAMPLE_RATE_HZ};
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice,
    ModelRef,
};
use runtime::{JobContext, Progress};
use sherpa_onnx::{
    OfflineParaformerModelConfig, OfflineQwen3ASRModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SINGLE_GRAPH_MODEL_FILES: &[&str] = &["model.int8.onnx", "model.onnx"];
const QWEN3_ENCODER_FILES: &[&str] = &["encoder.int8.onnx", "encoder.onnx"];
const QWEN3_DECODER_FILES: &[&str] = &["decoder.int8.onnx", "decoder.onnx"];

#[derive(Debug, Clone, Default)]
pub struct SherpaOnnxAsrBackend {
    pub num_threads: i32,
}

impl AsrBackend for SherpaOnnxAsrBackend {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor(
            "sherpa-onnx",
            &[
                "offline",
                "bounded-offline-windows",
                "onnx",
                "paraformer",
                "qwen3-asr",
                "sensevoice",
            ],
        )
    }

    fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
        validate_model_ref(model)?;
        let directory = model_directory(model);
        match model_family(model) {
            ModelFamily::Paraformer | ModelFamily::SenseVoice => {
                require_any_file(&directory, SINGLE_GRAPH_MODEL_FILES)?;
                require_file(&directory, "tokens.txt")?;
            }
            ModelFamily::Qwen3Asr => {
                require_file(&directory, "conv_frontend.onnx")?;
                require_any_file(&directory, QWEN3_ENCODER_FILES)?;
                require_any_file(&directory, QWEN3_DECODER_FILES)?;
                require_directory(&directory.join("tokenizer"))?;
                require_file(&directory, "tokens.txt")?;
            }
            ModelFamily::Unsupported => {
                return Err(BackendError {
                    code: BackendErrorCode::ModelIncompatible,
                    message: format!(
                        "sherpa-onnx ASR does not have an adapter for model {}",
                        model.model_id
                    ),
                });
            }
        }
        Ok(())
    }

    fn transcribe(&self, request: AsrRequest, context: JobContext) -> BackendFuture<AsrResult> {
        let num_threads = self.num_threads.max(1);
        let worker_context = context.clone();
        Box::pin(
            context.spawn_blocking(move || transcribe_sync(request, worker_context, num_threads)),
        )
    }

    fn supports_windowed_audio(&self) -> bool {
        true
    }

    fn create_window_session(
        &self,
        request: AsrRequest,
    ) -> Result<Box<dyn AsrWindowSession>, BackendError> {
        self.probe(&request.model)?;
        let config = build_config(&request, self.num_threads.max(1))?;
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| BackendError {
            code: BackendErrorCode::ModelLoadFailed,
            message: format!(
                "sherpa-onnx failed to create recognizer for {}",
                request.model.model_id
            ),
        })?;
        Ok(Box::new(SherpaWindowSession {
            request,
            recognizer,
            timestamped_segments: Vec::new(),
            untimestamped_windows: Vec::new(),
        }))
    }
}

struct SherpaWindowSession {
    request: AsrRequest,
    recognizer: OfflineRecognizer,
    timestamped_segments: Vec<domain::TranscriptSegment>,
    untimestamped_windows: Vec<UntimestampedWindow>,
}

struct UntimestampedWindow {
    global_start_ms: u64,
    global_end_ms: u64,
    core_start_ms: u64,
    text: String,
}

impl AsrWindowSession for SherpaWindowSession {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), BackendError> {
        if context.is_cancelled() {
            return Err(cancelled_error());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(window.sample_rate as i32, &window.samples);
        self.recognizer.decode(&stream);
        if context.is_cancelled() {
            return Err(cancelled_error());
        }
        let result = stream.get_result().ok_or_else(|| BackendError {
            code: BackendErrorCode::OutputValidationFailed,
            message: format!(
                "sherpa-onnx returned no recognition result for window {}",
                window.sequence
            ),
        })?;
        if let Some(timestamps) = result
            .timestamps
            .as_deref()
            .filter(|timestamps| !timestamps.is_empty() && timestamps.len() == result.tokens.len())
        {
            let durations = result.durations.as_deref();
            for (index, (token, timestamp)) in
                result.tokens.iter().zip(timestamps.iter()).enumerate()
            {
                let Some(local_start) = seconds_to_sample(*timestamp) else {
                    continue;
                };
                let local_end = durations
                    .and_then(|values| values.get(index))
                    .and_then(|duration| seconds_to_sample(*timestamp + *duration))
                    .or_else(|| {
                        timestamps
                            .get(index + 1)
                            .and_then(|value| seconds_to_sample(*value))
                    })
                    .unwrap_or(window.samples.len() as u64)
                    .min(window.samples.len() as u64);
                let local_start = local_start.min(window.samples.len() as u64);
                if local_end <= local_start {
                    continue;
                }
                let global_start = window.global_start_sample.saturating_add(local_start);
                let global_end = window.global_start_sample.saturating_add(local_end);
                let midpoint = global_start.saturating_add((global_end - global_start) / 2);
                if !core_owns(&window, midpoint) {
                    continue;
                }
                let text = normalize_token(token);
                if text.is_empty() {
                    continue;
                }
                self.timestamped_segments.push(domain::TranscriptSegment {
                    id: String::new(),
                    start_ms: sample_to_ms(global_start),
                    end_ms: sample_to_ms(global_end),
                    text,
                });
            }
        } else {
            let text = result.text.trim().to_string();
            if !text.is_empty() {
                self.untimestamped_windows.push(UntimestampedWindow {
                    global_start_ms: sample_to_ms(window.global_start_sample),
                    global_end_ms: sample_to_ms(window.global_end_sample),
                    core_start_ms: sample_to_ms(window.valid_core_start_sample),
                    text,
                });
            }
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<AsrResult, BackendError> {
        self.timestamped_segments
            .sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        let mut segments = merge_timestamped_tokens(self.timestamped_segments);
        let mut previous: Option<UntimestampedWindow> = None;
        for window in self.untimestamped_windows {
            let (start_ms, unique) = match previous.as_ref() {
                None => (window.global_start_ms, window.text.clone()),
                Some(previous) => strip_approximate_overlap(&previous.text, &window.text)
                    .map(|text| (previous.global_end_ms, text))
                    .unwrap_or_else(|| (window.core_start_ms, window.text.clone())),
            };
            if !unique.trim().is_empty() {
                segments.push(domain::TranscriptSegment {
                    id: String::new(),
                    start_ms,
                    end_ms: window.global_end_ms,
                    text: unique,
                });
            }
            previous = Some(window);
        }
        segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        for (index, segment) in segments.iter_mut().enumerate() {
            segment.id = format!("sherpa-onnx-segment-{index}");
        }
        Ok(AsrResult {
            backend: SherpaOnnxAsrBackend::default().descriptor(),
            model: self.request.model,
            language: self.request.language,
            segments,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Paraformer,
    Qwen3Asr,
    SenseVoice,
    Unsupported,
}

fn model_family(model: &ModelRef) -> ModelFamily {
    let model_id = model.model_id.to_ascii_lowercase();
    if model_id.contains("paraformer") {
        ModelFamily::Paraformer
    } else if model_id.contains("qwen3") {
        ModelFamily::Qwen3Asr
    } else if model_id.contains("sensevoice") || model_id.contains("sense-voice") {
        ModelFamily::SenseVoice
    } else {
        ModelFamily::Unsupported
    }
}

fn validate_model_ref(model: &ModelRef) -> Result<(), BackendError> {
    if model.backend_id != "sherpa-onnx" {
        return Err(BackendError {
            code: BackendErrorCode::ModelIncompatible,
            message: format!(
                "model {} belongs to backend {}, not sherpa-onnx",
                model.model_id, model.backend_id
            ),
        });
    }
    if !Path::new(&model.artifact_path).is_dir() {
        return Err(BackendError {
            code: BackendErrorCode::ModelMissing,
            message: format!(
                "sherpa-onnx model directory does not exist: {}",
                model.artifact_path
            ),
        });
    }
    Ok(())
}

fn model_directory(model: &ModelRef) -> PathBuf {
    PathBuf::from(&model.artifact_path)
}

fn require_file(directory: &Path, filename: &str) -> Result<(), BackendError> {
    let path = directory.join(filename);
    if path.is_file() {
        Ok(())
    } else {
        Err(BackendError {
            code: BackendErrorCode::ModelMissing,
            message: format!(
                "sherpa-onnx model artifact does not exist: {}",
                path.display()
            ),
        })
    }
}

fn require_directory(path: &Path) -> Result<(), BackendError> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(BackendError {
            code: BackendErrorCode::ModelMissing,
            message: format!(
                "sherpa-onnx model directory does not exist: {}",
                path.display()
            ),
        })
    }
}

fn require_any_file(directory: &Path, filenames: &[&str]) -> Result<PathBuf, BackendError> {
    filenames
        .iter()
        .map(|filename| directory.join(filename))
        .find(|path| path.is_file())
        .ok_or_else(|| BackendError {
            code: BackendErrorCode::ModelMissing,
            message: format!(
                "sherpa-onnx model directory has no supported model file: {}",
                directory.display()
            ),
        })
}

fn transcribe_sync(
    request: AsrRequest,
    context: JobContext,
    num_threads: i32,
) -> Result<AsrResult, BackendError> {
    if context.is_cancelled() {
        return Err(cancelled_error());
    }
    let backend = SherpaOnnxAsrBackend { num_threads };
    backend.probe(&request.model)?;
    context.started();
    context.report(Progress {
        completed: 1,
        total: Some(3),
    });

    let audio = read_audio_mono_16k(&request.audio_path).map_err(|error| BackendError {
        code: BackendErrorCode::InvalidAudio,
        message: error.to_string(),
    })?;
    if context.is_cancelled() {
        return Err(cancelled_error());
    }
    context.report(Progress {
        completed: 2,
        total: Some(3),
    });

    let config = build_config(&request, num_threads)?;
    let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| BackendError {
        code: BackendErrorCode::ModelLoadFailed,
        message: format!(
            "sherpa-onnx failed to create recognizer for {}",
            request.model.model_id
        ),
    })?;
    let stream = recognizer.create_stream();
    stream.accept_waveform(audio.sample_rate as i32, &audio.samples);
    recognizer.decode(&stream);
    if context.is_cancelled() {
        return Err(cancelled_error());
    }
    let result = stream.get_result().ok_or_else(|| BackendError {
        code: BackendErrorCode::OutputValidationFailed,
        message: "sherpa-onnx returned no recognition result".into(),
    })?;
    let text = result.text.trim().to_string();
    if text.is_empty() {
        return Err(BackendError {
            code: BackendErrorCode::OutputValidationFailed,
            message: "sherpa-onnx returned an empty recognition result".into(),
        });
    }
    let end_ms = (audio.samples.len() as u64 * 1_000) / u64::from(audio.sample_rate);
    context.report(Progress {
        completed: 3,
        total: Some(3),
    });
    context.completed();

    Ok(AsrResult {
        backend: backend.descriptor(),
        model: request.model,
        language: request.language,
        segments: vec![domain::TranscriptSegment {
            id: "sherpa-onnx-segment-0".into(),
            start_ms: 0,
            end_ms,
            text,
        }],
    })
}

fn build_config(
    request: &AsrRequest,
    num_threads: i32,
) -> Result<OfflineRecognizerConfig, BackendError> {
    let directory = model_directory(&request.model);
    let tokens = directory.join("tokens.txt");
    let provider = match request.device {
        ComputeDevice::Auto | ComputeDevice::Cpu => "cpu",
        // The pinned sherpa runtime has no provider-discovery/effective-provider
        // receipt. Its CoreML request was observed falling back to CPU. Reject
        // it until we can guarantee execution instead of mislabelling provenance.
        ComputeDevice::CoreMl => {
            return Err(BackendError {
                code: BackendErrorCode::DeviceUnavailable,
                message:
                    "the pinned sherpa-onnx runtime cannot verify Core ML execution; select CPU"
                        .into(),
            });
        }
        ComputeDevice::Metal => {
            return Err(BackendError {
                code: BackendErrorCode::DeviceUnavailable,
                message: "sherpa-onnx ASR supports CPU and Core ML, not Metal directly".into(),
            });
        }
    };
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.tokens = Some(tokens.display().to_string());
    config.model_config.num_threads = num_threads;
    config.model_config.provider = Some(provider.into());

    match model_family(&request.model) {
        ModelFamily::Paraformer => {
            let model = require_any_file(&directory, SINGLE_GRAPH_MODEL_FILES)?;
            config.model_config.paraformer = OfflineParaformerModelConfig {
                model: Some(model.display().to_string()),
            };
        }
        ModelFamily::Qwen3Asr => {
            let encoder = require_any_file(&directory, QWEN3_ENCODER_FILES)?;
            let decoder = require_any_file(&directory, QWEN3_DECODER_FILES)?;
            config.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
                conv_frontend: Some(directory.join("conv_frontend.onnx").display().to_string()),
                encoder: Some(encoder.display().to_string()),
                decoder: Some(decoder.display().to_string()),
                tokenizer: Some(directory.join("tokenizer").display().to_string()),
                ..Default::default()
            };
        }
        ModelFamily::SenseVoice => {
            let model = require_any_file(&directory, SINGLE_GRAPH_MODEL_FILES)?;
            config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: Some(model.display().to_string()),
                language: Some(request.language.clone().unwrap_or_else(|| "auto".into())),
                use_itn: true,
            };
        }
        ModelFamily::Unsupported => {
            return Err(BackendError {
                code: BackendErrorCode::ModelIncompatible,
                message: format!(
                    "sherpa-onnx ASR does not have an adapter for model {}",
                    request.model.model_id
                ),
            });
        }
    }
    Ok(config)
}

fn cancelled_error() -> BackendError {
    BackendError {
        code: BackendErrorCode::Cancelled,
        message: "sherpa-onnx transcription was cancelled".into(),
    }
}

fn seconds_to_sample(seconds: f32) -> Option<u64> {
    (seconds.is_finite() && seconds >= 0.0)
        .then(|| (f64::from(seconds) * f64::from(TARGET_SAMPLE_RATE_HZ)).round() as u64)
}

fn sample_to_ms(sample: u64) -> u64 {
    sample.saturating_mul(1_000) / u64::from(TARGET_SAMPLE_RATE_HZ)
}

fn core_owns(window: &AudioWindow, sample: u64) -> bool {
    sample >= window.valid_core_start_sample
        && (sample < window.valid_core_end_sample
            || (window.is_final && sample == window.valid_core_end_sample))
}

fn normalize_token(token: &str) -> String {
    token.replace('▁', " ").replace("@@", "").trim().to_string()
}

fn merge_timestamped_tokens(
    tokens: Vec<domain::TranscriptSegment>,
) -> Vec<domain::TranscriptSegment> {
    let mut segments: Vec<domain::TranscriptSegment> = Vec::new();
    for token in tokens {
        let should_merge = segments.last().is_some_and(|previous| {
            token.start_ms <= previous.end_ms.saturating_add(750)
                && token.end_ms.saturating_sub(previous.start_ms) <= 15_000
                && !ends_sentence(&previous.text)
        });
        if should_merge {
            let previous = segments.last_mut().expect("segment exists");
            append_token(&mut previous.text, &token.text);
            previous.end_ms = previous.end_ms.max(token.end_ms);
        } else {
            segments.push(token);
        }
    }
    segments
}

fn append_token(output: &mut String, token: &str) {
    let needs_space = output
        .chars()
        .last()
        .zip(token.chars().next())
        .is_some_and(|(left, right)| left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric());
    if needs_space {
        output.push(' ');
    }
    output.push_str(token);
}

fn ends_sentence(text: &str) -> bool {
    text.ends_with(['.', '!', '?', '。', '！', '？'])
}

fn strip_approximate_overlap(previous: &str, current: &str) -> Option<String> {
    const MAX_ALIGNMENT_CHARS: usize = 512;
    const MIN_OVERLAP_CHARS: usize = 8;
    const MIN_SIMILARITY: f32 = 0.55;
    let previous = previous.chars().collect::<Vec<_>>();
    let current = current.chars().collect::<Vec<_>>();
    let previous = &previous[previous.len().saturating_sub(MAX_ALIGNMENT_CHARS)..];
    let current_head_len = current.len().min(MAX_ALIGNMENT_CHARS);
    let current_head = &current[..current_head_len];
    if previous.len() < MIN_OVERLAP_CHARS || current_head.len() < MIN_OVERLAP_CHARS {
        return None;
    }
    let columns = current_head.len() + 1;
    let mut costs = vec![0u16; (previous.len() + 1) * columns];
    for (column, cost) in costs.iter_mut().take(columns).enumerate() {
        *cost = column as u16;
    }
    for row in 1..=previous.len() {
        costs[row * columns] = 0;
        for column in 1..columns {
            let substitution = costs[(row - 1) * columns + column - 1]
                + u16::from(previous[row - 1] != current_head[column - 1]);
            let deletion = costs[(row - 1) * columns + column] + 1;
            let insertion = costs[row * columns + column - 1] + 1;
            costs[row * columns + column] = substitution.min(deletion).min(insertion);
        }
    }

    let mut best: Option<(f32, usize)> = None;
    for current_prefix in MIN_OVERLAP_CHARS..=current_head.len() {
        let mut row = previous.len();
        let mut column = current_prefix;
        while column > 0 && row > 0 {
            let current_cost = costs[row * columns + column];
            let substitution = costs[(row - 1) * columns + column - 1]
                + u16::from(previous[row - 1] != current_head[column - 1]);
            if current_cost == substitution {
                row -= 1;
                column -= 1;
            } else if current_cost == costs[(row - 1) * columns + column] + 1 {
                row -= 1;
            } else {
                column -= 1;
            }
        }
        if column > 0 {
            continue;
        }
        let previous_suffix = previous.len() - row;
        let overlap = previous_suffix.min(current_prefix);
        if overlap < MIN_OVERLAP_CHARS {
            continue;
        }
        let edits = costs[previous.len() * columns + current_prefix] as f32;
        let similarity = 1.0 - edits / previous_suffix.max(current_prefix) as f32;
        if similarity < MIN_SIMILARITY {
            continue;
        }
        let score = similarity + overlap.min(256) as f32 / 1_000.0;
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, current_prefix));
        }
    }
    let (_, cut) = best?;
    Some(current[cut..].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn model_ref(model_id: &str, artifact_path: &Path) -> ModelRef {
        ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: model_id.into(),
            revision: None,
            artifact_path: artifact_path.display().to_string(),
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn probe_requires_paraformer_model_and_tokens() {
        let root = std::env::temp_dir().join(format!("meetnote-sherpa-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let backend = SherpaOnnxAsrBackend::default();
        let missing = backend.probe(&model_ref("paraformer-zh", &root));
        assert!(matches!(
            missing,
            Err(BackendError {
                code: BackendErrorCode::ModelMissing,
                ..
            })
        ));
        fs::write(root.join("model.int8.onnx"), b"fixture").unwrap();
        let missing_tokens = backend.probe(&model_ref("paraformer-zh", &root));
        assert!(matches!(
            missing_tokens,
            Err(BackendError {
                code: BackendErrorCode::ModelMissing,
                ..
            })
        ));
        fs::write(root.join("tokens.txt"), b"<blk> 0\n").unwrap();
        assert!(backend.probe(&model_ref("paraformer-zh", &root)).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn untimestamped_overlap_tolerates_small_recognition_differences() {
        assert_eq!(
            strip_approximate_overlap(
                "before the shared phrase near boundary",
                "shared phase near boundary then continues"
            )
            .unwrap(),
            " then continues"
        );
        assert!(strip_approximate_overlap("alpha beta", "different text").is_none());
    }

    #[test]
    fn timestamped_tokens_form_bounded_monotonic_segments() {
        let segments = merge_timestamped_tokens(vec![
            domain::TranscriptSegment {
                id: String::new(),
                start_ms: 0,
                end_ms: 500,
                text: "hello".into(),
            },
            domain::TranscriptSegment {
                id: String::new(),
                start_ms: 500,
                end_ms: 1_000,
                text: "world.".into(),
            },
            domain::TranscriptSegment {
                id: String::new(),
                start_ms: 20_000,
                end_ms: 20_500,
                text: "next".into(),
            },
        ]);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "hello world.");
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 1_000));
    }

    #[test]
    fn probe_rejects_unknown_model_families() {
        let root =
            std::env::temp_dir().join(format!("meetnote-sherpa-unknown-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let result = SherpaOnnxAsrBackend::default().probe(&model_ref("unknown", &root));
        assert!(matches!(
            result,
            Err(BackendError {
                code: BackendErrorCode::ModelIncompatible,
                ..
            })
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sensevoice_requires_its_assets_and_respects_language_and_device() {
        let root = std::env::temp_dir().join(format!("meetnote-sensevoice-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let model = model_ref("sensevoice-small-int8", &root);
        let backend = SherpaOnnxAsrBackend::default();
        assert_eq!(
            backend.probe(&model).unwrap_err().code,
            BackendErrorCode::ModelMissing
        );
        fs::write(root.join("model.int8.onnx"), b"fixture").unwrap();
        assert_eq!(
            backend.probe(&model).unwrap_err().code,
            BackendErrorCode::ModelMissing
        );
        fs::write(root.join("tokens.txt"), b"<blk> 0\n").unwrap();
        backend.probe(&model).unwrap();
        let mut request = AsrRequest {
            audio_path: "fixture.wav".into(),
            model,
            language: Some("zh".into()),
            device: ComputeDevice::Cpu,
        };
        let config = build_config(&request, 4).unwrap();
        assert_eq!(
            config.model_config.sense_voice.model.as_deref(),
            Some(root.join("model.int8.onnx").to_str().unwrap())
        );
        assert_eq!(
            config.model_config.sense_voice.language.as_deref(),
            Some("zh")
        );
        assert!(config.model_config.sense_voice.use_itn);
        assert!(config.model_config.paraformer.model.is_none());
        assert_eq!(config.model_config.provider.as_deref(), Some("cpu"));
        assert_eq!(config.model_config.num_threads, 4);
        request.language = None;
        assert_eq!(
            build_config(&request, 1)
                .unwrap()
                .model_config
                .sense_voice
                .language
                .as_deref(),
            Some("auto")
        );
        request.device = ComputeDevice::Metal;
        assert_eq!(
            build_config(&request, 1).unwrap_err().code,
            BackendErrorCode::DeviceUnavailable
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn build_config_selects_paraformer_adapter() {
        let root =
            std::env::temp_dir().join(format!("meetnote-sherpa-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("model.onnx"), b"fixture").unwrap();
        fs::write(root.join("tokens.txt"), b"<blk> 0\n").unwrap();
        let request = AsrRequest {
            audio_path: "fixture.wav".into(),
            model: model_ref("paraformer-zh", &root),
            language: None,
            device: ComputeDevice::Auto,
        };
        let config = build_config(&request, 4).unwrap();
        assert_eq!(config.model_config.num_threads, 4);
        assert_eq!(config.model_config.provider.as_deref(), Some("cpu"));
        assert_eq!(
            config.model_config.paraformer.model.as_deref(),
            Some(root.join("model.onnx").to_str().unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unverified_coreml_provider_must_not_silently_run_on_cpu() {
        let request = AsrRequest {
            audio_path: "fixture.wav".into(),
            model: model_ref("paraformer-zh", Path::new("unused")),
            language: None,
            device: ComputeDevice::CoreMl,
        };
        let error = build_config(&request, 1).unwrap_err();
        assert_eq!(error.code, BackendErrorCode::DeviceUnavailable);
        assert!(error.message.contains("Core ML"));
    }

    #[test]
    #[ignore = "requires a downloaded sherpa-onnx model and audio fixture"]
    fn transcribes_real_audio_with_paraformer() {
        let model_dir = std::env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR").unwrap();
        let audio_path = std::env::var("MEETNOTE_SHERPA_ONNX_AUDIO").unwrap();
        let runtime = runtime::JobRuntime::default();
        let (context, _) = runtime.create_job();
        let result = futures::executor::block_on(SherpaOnnxAsrBackend::default().transcribe(
            AsrRequest {
                audio_path,
                model: model_ref("paraformer-zh", Path::new(&model_dir)),
                language: Some("zh".into()),
                device: ComputeDevice::Cpu,
            },
            context,
        ))
        .unwrap();
        assert!(!result.segments.is_empty());
        assert!(!result.segments[0].text.is_empty());
    }
}
