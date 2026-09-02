use application::{asr_descriptor, AsrBackend, BackendFuture};
use audio::read_audio_mono_16k;
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, BackendErrorCode, ComputeDevice,
    ModelRef,
};
use runtime::{JobContext, Progress};
use sherpa_onnx::{
    OfflineParaformerModelConfig, OfflineQwen3ASRModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig,
};
use std::path::{Path, PathBuf};

const PARAFORMER_MODEL_FILES: &[&str] = &["model.int8.onnx", "model.onnx"];
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
            &["offline", "onnx", "paraformer", "qwen3-asr"],
        )
    }

    fn probe(&self, model: &ModelRef) -> Result<(), BackendError> {
        validate_model_ref(model)?;
        let directory = model_directory(model);
        match model_family(model) {
            ModelFamily::Paraformer => {
                require_any_file(&directory, PARAFORMER_MODEL_FILES)?;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Paraformer,
    Qwen3Asr,
    Unsupported,
}

fn model_family(model: &ModelRef) -> ModelFamily {
    let model_id = model.model_id.to_ascii_lowercase();
    if model_id.contains("paraformer") {
        ModelFamily::Paraformer
    } else if model_id.contains("qwen3") {
        ModelFamily::Qwen3Asr
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
        ComputeDevice::CoreMl => "coreml",
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
            let model = require_any_file(&directory, PARAFORMER_MODEL_FILES)?;
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
