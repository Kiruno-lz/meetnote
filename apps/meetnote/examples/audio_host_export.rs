//! Headless Audio Host export for the real-data quality matrix.
//!
//! This is intentionally a small harness around the production
//! `WorkspaceService`; it does not introduce another processing runtime.

use application::{ProcessingOptions, ProcessingService, WorkspaceService};
use asr::{AsrBackendManager, AsrBackendRegistry};
use asr_sherpa_onnx::SherpaOnnxAsrBackend;
use asr_whisper_rs::WhisperRsBackend;
use contracts::{AlgorithmRef, ComputeDevice, DiarizationStageConfiguration, ModelRef};
use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};
use diarization_native::{NativeDiarizationBackend, NativeDiarizationConfig};
use futures::executor::block_on;
use persistence::FileDocumentRepository;
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let meeting_id = args
        .next()
        .ok_or("usage: audio_host_export <meeting-id> <audio> <documents-root> <output-jsonl>")?;
    let audio = PathBuf::from(args.next().ok_or("missing audio path")?);
    let documents_root = PathBuf::from(args.next().ok_or("missing documents root")?);
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    let asr_backend_id = env::var("MEETNOTE_ASR_BACKEND").unwrap_or_else(|_| "whisper-rs".into());
    let is_paraformer = asr_backend_id == "sherpa-onnx";
    let asr_model_path = PathBuf::from(if is_paraformer {
        env::var("MEETNOTE_SHERPA_ONNX_MODEL_DIR")
            .map_err(|_| "MEETNOTE_SHERPA_ONNX_MODEL_DIR is required")?
    } else {
        env::var("MEETNOTE_WHISPER_MODEL").map_err(|_| "MEETNOTE_WHISPER_MODEL is required")?
    });
    let diarization_model = PathBuf::from(
        env::var("MEETNOTE_DIARIZATION_MODEL_DIR")
            .map_err(|_| "MEETNOTE_DIARIZATION_MODEL_DIR is required")?,
    );

    let mut asr_registry = AsrBackendRegistry::default();
    if is_paraformer {
        asr_registry.register(Arc::new(SherpaOnnxAsrBackend::default()));
    } else {
        asr_registry.register(Arc::new(WhisperRsBackend::default()));
    }
    let asr_manager = Arc::new(AsrBackendManager::new(asr_registry));
    let mut diarization_registry = DiarizationBackendRegistry::default();
    diarization_registry.register(Arc::new(NativeDiarizationBackend::new(
        NativeDiarizationConfig::default(),
    )));
    let diarization_manager = Arc::new(DiarizationBackendManager::new(diarization_registry));
    let repository = Arc::new(FileDocumentRepository::open(&documents_root)?);
    let documents = application::DocumentService::new(repository);
    let stage_configuration = DiarizationStageConfiguration {
        segmentation_model: Some(ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "pyannote-segmentation-3.0".into(),
            revision: None,
            artifact_path: diarization_model
                .join("segmentation-3.0.onnx")
                .display()
                .to_string(),
            artifacts: Vec::new(),
        }),
        embedding_model: Some(ModelRef {
            backend_id: "diarization-native".into(),
            model_id: "wespeaker-voxceleb-resnet34".into(),
            revision: None,
            artifact_path: diarization_model
                .join("wespeaker-voxceleb-resnet34.onnx")
                .display()
                .to_string(),
            artifacts: Vec::new(),
        }),
        clustering_algorithm: Some(AlgorithmRef {
            id: "speakrs-ahc-plda-vbx".into(),
            revision: Some("speakrs-community-1-cpu".into()),
            parameters: Some("community-1-default".into()),
        }),
    };
    let workspace = WorkspaceService::with_options(
        ProcessingService::new_with_providers(asr_manager, diarization_manager),
        documents.clone(),
        ProcessingOptions {
            language: Some("en".into()),
            expected_speakers: Some(4),
            device: ComputeDevice::Cpu,
            speaker_library_path: None,
            diarization_stages: stage_configuration,
        },
    );
    let asr_model = ModelRef {
        backend_id: asr_backend_id.clone(),
        model_id: if is_paraformer {
            "paraformer-zh".into()
        } else {
            "whisper-large-v3-ggml".into()
        },
        revision: None,
        artifact_path: asr_model_path.display().to_string(),
        artifacts: Vec::new(),
    };
    let diarization_model_ref = ModelRef {
        backend_id: "diarization-native".into(),
        model_id: "speakrs-community-1-cpu".into(),
        revision: None,
        artifact_path: diarization_model.display().to_string(),
        artifacts: Vec::new(),
    };
    let project = workspace.import_audio(audio.display().to_string())?;
    eprintln!("audio-host: imported {}", project.id().as_str());
    eprintln!("audio-host: processing");
    let result =
        block_on(workspace.process_project(project.id().clone(), asr_model, diarization_model_ref))
            .map_err(|error| error.message)?;
    eprintln!("audio-host: processed {}", result.id().as_str());
    let record = documents
        .read(result.id())?
        .ok_or("processed document disappeared")?;
    let prediction = audio_prediction(&meeting_id, &record.state);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, format!("{}\n", serde_json::to_string(&prediction)?))?;
    Ok(())
}

fn audio_prediction(meeting_id: &str, state: &domain::DocumentState) -> serde_json::Value {
    let transcript = state
        .results
        .transcript
        .iter()
        .map(|segment| {
            json!({
                "id": segment.id,
                "startMs": segment.start_ms,
                "endMs": segment.end_ms,
                "text": segment.text,
            })
        })
        .collect::<Vec<_>>();
    let speaker_segments = state
        .results
        .speaker_segments
        .iter()
        .map(|segment| {
            json!({
                "transcriptSegmentId": segment.transcript_segment_id,
                "speaker": segment.speaker_id,
                "startMs": segment.start_ms,
                "endMs": segment.end_ms,
                "text": segment.text,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schemaVersion": 2,
        "predictionKind": "audio_host_prediction",
        "sourceDomain": "audio",
        "assistantSemanticQualityStatus": "not_assessed",
        "meetingId": meeting_id,
        "transcript": transcript,
        "speakerSegments": speaker_segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn normalized_export_is_scored_by_the_audio_metrics_consumer() {
        let mut state =
            domain::DocumentState::new(domain::ProjectId::new("fixture"), "Fixture", "fixture.wav");
        state.results.transcript = vec![domain::TranscriptSegment {
            id: "segment-1".into(),
            start_ms: 0,
            end_ms: 1_000,
            text: "Hello".into(),
        }];
        state.results.speaker_segments = vec![domain::SpeakerTaggedSegment {
            transcript_segment_id: "segment-1".into(),
            speaker_id: "model-speaker-X".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "Hello".into(),
        }];
        let prediction = audio_prediction("fixture", &state);
        let metrics = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/audio-eval");
        let program = r#"
import json, sys
sys.path.insert(0, sys.argv[1])
from metrics import score_record
prediction = json.load(sys.stdin)
reference = {
    'meetingId': 'fixture', 'language': 'en',
    'transcript': [{'startMs': 0, 'endMs': 1000, 'speaker': 'reference-A', 'text': 'Hello'}],
    'referenceSpeakerSegments': [{'startMs': 0, 'endMs': 1000, 'speaker': 'reference-A'}],
}
result = score_record(reference, prediction)
assert result['asr']['werOrCer'] == 0, result
assert result['diarization']['der'] == 0, result
assert result['assistantSemanticQualityStatus'] == 'not_assessed', result
"#;
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(program)
            .arg(metrics)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("audio evaluation tests require python3");
        serde_json::to_writer(child.stdin.take().unwrap(), &prediction).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
