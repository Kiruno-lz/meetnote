use application::{
    run_pipeline, AsrBackend, BackendInventory, DiarizationBackend, ProcessingService,
    WorkspaceService,
};
use asr::{AsrBackendManager, AsrBackendRegistry};
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AsrRequest, ComputeDevice, DiarizationRequest, ModelRef};
use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::SpeakerTaggedSegment;
use persistence::JsonProjectRepository;
use runtime::JobRuntime;
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use ui::DesktopServices;

fn main() {
    let whisper_backend = Arc::new(WhisperRsBackend::default());
    let diarization_backend = Arc::new(SherpaOnnxDiarizationBackend::default());

    let mut asr_registry = AsrBackendRegistry::default();
    asr_registry.register(whisper_backend.clone());
    let asr_manager = AsrBackendManager::new(asr_registry);

    let mut diarization_registry = DiarizationBackendRegistry::default();
    diarization_registry.register(diarization_backend.clone());
    let diarization_manager = DiarizationBackendManager::new(diarization_registry);

    let repository = Arc::new(
        JsonProjectRepository::open("data/projects/projects.json")
            .expect("failed to open project repository"),
    );
    let workspace = WorkspaceService::new(
        ProcessingService::new(whisper_backend.clone(), diarization_backend.clone()),
        repository,
        "data/markdown",
    );
    let desktop_services = DesktopServices {
        workspace,
        asr_model: ModelRef {
            backend_id: "whisper-rs".into(),
            model_id: "whisper-base-en-ggml".into(),
            revision: None,
            artifact_path: "models/asr/whisper-large-v3-ggml/ggml-base.en.bin".into(),
        },
        diarization_model: ModelRef {
            backend_id: "sherpa-onnx".into(),
            model_id: "sherpa-onnx-speaker-diarization-bundle".into(),
            revision: None,
            artifact_path: "models/diarization/sherpa-onnx-speaker-diarization-bundle".into(),
        },
    };

    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("process") {
        if args.len() != 6 {
            eprintln!(
                "usage: meetnote process <audio.wav> <whisper-model> <diarization-dir> <output-prefix>"
            );
            std::process::exit(2);
        }
        if let Err(error) = process_audio(
            &whisper_backend,
            &diarization_backend,
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
            Path::new(&args[5]),
        ) {
            eprintln!("processing failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    ui::run(
        BackendInventory::new(
            asr_manager.available_backends(),
            diarization_manager.available_backends(),
        ),
        desktop_services,
    );
}

#[derive(Serialize)]
struct ProcessReport<'a> {
    audio_path: &'a str,
    asr_model: &'a ModelRef,
    diarization_model: &'a ModelRef,
    asr: &'a contracts::AsrResult,
    diarization: &'a contracts::DiarizationResult,
    aligned: &'a [SpeakerTaggedSegment],
}

fn process_audio(
    whisper_backend: &WhisperRsBackend,
    diarization_backend: &SherpaOnnxDiarizationBackend,
    audio_path: &Path,
    whisper_model_path: &Path,
    diarization_model_path: &Path,
    output_prefix: &Path,
) -> Result<(), String> {
    let asr_model = ModelRef {
        backend_id: "whisper-rs".into(),
        model_id: "whisper-base-en-ggml".into(),
        revision: None,
        artifact_path: whisper_model_path.display().to_string(),
    };
    let diarization_model = ModelRef {
        backend_id: "sherpa-onnx".into(),
        model_id: "sherpa-onnx-speaker-diarization-bundle".into(),
        revision: None,
        artifact_path: diarization_model_path.display().to_string(),
    };
    whisper_backend
        .probe(&asr_model)
        .map_err(|error| error.message.clone())?;
    diarization_backend
        .probe(&diarization_model)
        .map_err(|error| error.message.clone())?;

    let runtime = JobRuntime::default();
    let (asr_context, _) = runtime.create_job();
    let (diarization_context, _) = runtime.create_job();
    asr_context.started();
    diarization_context.started();

    let result = futures::executor::block_on(run_pipeline(
        whisper_backend,
        diarization_backend,
        AsrRequest {
            audio_path: audio_path.display().to_string(),
            model: asr_model.clone(),
            language: Some("en".into()),
            device: ComputeDevice::Auto,
        },
        DiarizationRequest {
            audio_path: audio_path.display().to_string(),
            model: diarization_model.clone(),
            expected_speakers: Some(2),
            device: ComputeDevice::Auto,
        },
        asr_context.clone(),
        diarization_context.clone(),
    ))
    .map_err(|error| {
        asr_context.failed(error.message.clone());
        diarization_context.failed(error.message.clone());
        error.message
    })?;
    asr_context.completed();
    diarization_context.completed();

    let audio_path_string = audio_path.display().to_string();
    let report = ProcessReport {
        audio_path: &audio_path_string,
        asr_model: &asr_model,
        diarization_model: &diarization_model,
        asr: &result.asr,
        diarization: &result.diarization,
        aligned: &result.aligned,
    };
    let json_path = with_extension(output_prefix, "json");
    let markdown_path = with_extension(output_prefix, "md");
    if let Some(parent) = json_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let json = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    fs::write(&json_path, json).map_err(|error| error.to_string())?;
    fs::write(&markdown_path, render_markdown(audio_path, &result.aligned))
        .map_err(|error| error.to_string())?;
    println!(
        "processed {} transcript segments and {} diarization segments",
        result.asr.segments.len(),
        result.diarization.segments.len()
    );
    println!("json: {}", json_path.display());
    println!("markdown: {}", markdown_path.display());
    Ok(())
}

fn with_extension(prefix: &Path, extension: &str) -> PathBuf {
    PathBuf::from(format!("{}.{}", prefix.display(), extension))
}

fn render_markdown(audio_path: &Path, segments: &[SpeakerTaggedSegment]) -> String {
    let mut output = format!(
        "# Meeting transcript\n\nAudio: `{}`\n\n",
        audio_path.display()
    );
    for segment in segments {
        let text = segment.text.replace('\n', " ");
        output.push_str(&format!(
            "- **[{}–{}] {}** {}\n",
            timestamp(segment.start_ms),
            timestamp(segment.end_ms),
            segment.speaker_id,
            text
        ));
    }
    output
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let seconds = seconds % 60;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % 1_000
    )
}
