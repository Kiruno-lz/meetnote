//! Independent B2 execution/resource probe; counts are not DER or quality scores.
use application::DiarizationBackend;
use contracts::{ComputeDevice, DiarizationRequest, DiarizationStageConfiguration, ModelRef};
use diarization_native::NativeDiarizationBackend;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use runtime::JobRuntime;
use serde_json::json;
use std::{fs, io::Write, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: diarization_benchmark <backend> <model-directory> <audio> <output.jsonl>"
                .into(),
        );
    }
    let backend: Box<dyn DiarizationBackend> = match args[0].as_str() {
        "diarization-native" => Box::new(NativeDiarizationBackend::default()),
        "sherpa-onnx" => Box::new(SherpaOnnxDiarizationBackend::default()),
        _ => return Err("unknown diarization backend".into()),
    };
    let model = ModelRef {
        backend_id: args[0].clone(),
        model_id: if args[0] == "diarization-native" {
            "speakrs-community-1-cpu"
        } else {
            "sherpa-onnx-speaker-diarization-bundle"
        }
        .into(),
        revision: if args[0] == "diarization-native" {
            Some(diarization_native::SPEAKRS_MODEL_REVISION.into())
        } else {
            None
        },
        artifact_path: args[1].clone(),
        artifacts: Vec::new(),
    };
    backend.probe(&model).map_err(|e| e.message)?;
    let mut output = fs::File::create(&args[3])?;
    let runtime = JobRuntime::default();
    for repeat in 0..2 {
        let (context, _) = runtime.create_job();
        let request = DiarizationRequest {
            audio_path: args[2].clone(),
            model: model.clone(),
            expected_speakers: None,
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            stage_configuration: DiarizationStageConfiguration::default(),
        };
        let start = Instant::now();
        let result = futures::executor::block_on(backend.diarize(request, context));
        let seconds = start.elapsed().as_secs_f64();
        let row = match result {
            Ok(result) => {
                json!({"backend":args[0],"repeat":repeat,"wallSeconds":seconds,"status":"ok",
                "segments":result.segments.len(),"speakerCount":result.speaker_count,
                "observations":result.embedding_observations.len(),"qualityStatus":"not_assessed",
                "speakerSegments":result.segments,"embeddingObservations":result.embedding_observations})
            }
            Err(error) => {
                json!({"backend":args[0],"repeat":repeat,"wallSeconds":seconds,"status":"failed","error":error.message})
            }
        };
        writeln!(output, "{row}")?;
        output.flush()?;
        eprintln!("diarization-benchmark: {row}");
    }
    Ok(())
}
