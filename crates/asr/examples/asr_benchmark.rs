//! Isolated B1 ASR measurement using the production adapters, without DIA or UI.
//! Each call loads its own model, exactly as whole-file product transcription does.
use application::AsrBackend;
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AsrRequest, ComputeDevice, ModelRef};
use runtime::JobRuntime;
use serde::Deserialize;
use serde_json::json;
use std::{fs, io::Write, sync::Arc, time::Instant};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configuration {
    model: ModelRef,
    device: ComputeDevice,
    threads: i32,
    #[serde(default = "one")]
    repeats: usize,
    #[serde(default)]
    windowed: bool,
    window_core_samples: Option<u64>,
    window_context_samples: Option<u64>,
}

fn one() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    id: String,
    audio_path: String,
    language: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: asr_benchmark <configuration.json> <cases.json> <output.jsonl>".into());
    }
    let config: Configuration = serde_json::from_slice(&fs::read(&args[0])?)?;
    if config.threads < 1 || config.repeats == 0 {
        return Err("threads and repeats must be positive".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(&args[1])?)?;
    let backend: Box<dyn AsrBackend> = match config.model.backend_id.as_str() {
        "sherpa-onnx" => Box::new(SherpaOnnxAsrBackend {
            num_threads: config.threads,
        }),
        "whisper-rs" => Box::new(WhisperRsBackend {
            threads: config.threads,
        }),
        _ => return Err("unsupported benchmark backend".into()),
    };
    backend.probe(&config.model).map_err(|e| e.message)?;
    let runtime = JobRuntime::default();
    let mut output = fs::File::create(&args[2])?;
    for repeat in 0..config.repeats {
        for case in &cases {
            let (context, _) = runtime.create_job();
            let request = AsrRequest {
                audio_path: case.audio_path.clone(),
                model: config.model.clone(),
                language: Some(case.language.clone()),
                device: config.device,
            };
            let start = Instant::now();
            let result = if config.windowed {
                (|| {
                    let mut session = backend
                        .create_window_session(request)
                        .map_err(|e| e.message)?;
                    let mut plan = application::LongAudioConfig::default();
                    if let Some(core) = config.window_core_samples {
                        plan.window_plan = audio::AudioWindowPlan::new(
                            core,
                            config.window_context_samples.unwrap_or(0),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    let mut reader =
                        audio::AudioChunkReader::open(&case.audio_path, plan.decode_chunk_samples)
                            .map_err(|e| e.to_string())?;
                    let mut planner = audio::ProcessingWindowPlanner::new(plan.window_plan);
                    while let Some(chunk) = reader.next_chunk().map_err(|e| e.to_string())? {
                        for window in planner.push(chunk).map_err(|e| e.to_string())? {
                            session
                                .process_window(Arc::new(window), &context)
                                .map_err(|e| e.message)?;
                        }
                    }
                    planner.finish().map_err(|e| e.to_string())?;
                    session.finish().map_err(|e| e.message)
                })()
            } else {
                futures::executor::block_on(backend.transcribe(request, context))
                    .map_err(|e| e.message)
            };
            let seconds = start.elapsed().as_secs_f64();
            let row = match result {
                Ok(result) => json!({
                    "schemaVersion": 2, "predictionKind": "audio_host_prediction",
                    "sourceDomain": "audio", "assistantSemanticQualityStatus": "not_assessed",
                    "caseId": case.id, "repeat": repeat, "wallSeconds": seconds,
                    "modelReloadedPerCall": true, "windowed": config.windowed,
                    "transcript": result.segments.iter().map(|segment| json!({
                        "id":segment.id,"startMs":segment.start_ms,"endMs":segment.end_ms,"text":segment.text,
                    })).collect::<Vec<_>>(), "model": result.model,
                    "status": "ok",
                }),
                Err(error) => {
                    json!({"caseId":case.id,"repeat":repeat,"wallSeconds":seconds,"status":"failed","error":error})
                }
            };
            writeln!(output, "{}", row)?;
            output.flush()?;
            eprintln!(
                "asr-benchmark: {} repeat {} {:.3}s {}",
                case.id, repeat, seconds, row["status"]
            );
        }
    }
    Ok(())
}
