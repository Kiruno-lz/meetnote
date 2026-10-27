use application::DiarizationBackend;
use contracts::{
    ComputeDevice, DiarizationRequest, DiarizationStageConfiguration, ModelArtifactRef, ModelRef,
};
use diarization_native::{
    NativeDiarizationBackend, NativeDiarizationEngine, NativeDiarizationError, SpeakrsEngine,
    REQUIRED_CPU_ARTIFACTS, SPEAKRS_MODEL_REVISION,
};
use models::ModelCatalog;
use std::path::{Path, PathBuf};

const COREML_ID: &str = "speakrs-community-1-coreml";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("meetnote-coreml-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn put(&self, filename: &str) {
        let path = self.0.join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"file-presence fixture; not a model").unwrap();
    }
    fn model(&self, id: &str) -> ModelRef {
        ModelRef {
            backend_id: "diarization-native".into(),
            model_id: id.into(),
            revision: Some(SPEAKRS_MODEL_REVISION.into()),
            artifact_path: self.0.display().to_string(),
            artifacts: vec![],
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn catalog() -> ModelCatalog {
    ModelCatalog::from_path(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/catalog.json"),
    )
    .unwrap()
}

#[test]
fn coreml_profile_declares_a_complete_pinned_offline_bundle() {
    let catalog = catalog();
    let entry = catalog.find(COREML_ID).expect("installable coreml profile");
    assert_eq!(entry.artifacts.len(), 76);
    assert_eq!(entry.artifact_files.len(), 76);
    assert_eq!(entry.revision.as_deref(), Some(SPEAKRS_MODEL_REVISION));
    assert!(entry
        .artifacts
        .iter()
        .all(|a| a.source.contains(SPEAKRS_MODEL_REVISION)
            && a.sha256.as_ref().is_some_and(|sha| sha.len() == 64)));
    assert!(entry
        .artifacts
        .iter()
        .any(|a| a.filename == "segmentation-3.0.mlmodelc/weights/weight.bin"));
    assert_eq!(entry.status, "ready");
    assert_eq!(entry.supported_platforms, ["macos-aarch64"]);
    assert!(entry.fixed_diarization_stages);
    assert_eq!(entry.candidate_backends, ["diarization-native"]);
}

#[test]
#[cfg(not(feature = "coreml"))]
fn coreml_profile_requires_compiled_execution_support() {
    let fixture = Fixture::new();
    for name in REQUIRED_CPU_ARTIFACTS {
        fixture.put(name);
    }
    let error = SpeakrsEngine.probe(&fixture.model(COREML_ID)).unwrap_err();
    assert!(
        matches!(error, NativeDiarizationError::DeviceUnavailable(_)),
        "{error:?}"
    );
}

#[test]
#[cfg(feature = "coreml")]
fn quick_probe_checks_every_coreml_artifact_without_loading_fixture_bytes() {
    let fixture = Fixture::new();
    let catalog = catalog();
    let entry = catalog.find(COREML_ID).unwrap();
    for filename in &entry.artifact_files {
        fixture.put(filename);
    }
    let model = fixture.model(COREML_ID);
    SpeakrsEngine
        .probe(&model)
        .expect("presence probe; not real model loading");
    for filename in &entry.artifact_files {
        let path = fixture.0.join(filename);
        std::fs::remove_file(path).unwrap();
        let error = SpeakrsEngine.probe(&model).unwrap_err();
        assert!(
            matches!(error, NativeDiarizationError::ModelMissing(_)),
            "{filename}: {error}"
        );
        assert!(error.to_string().contains(filename), "{filename}: {error}");
        fixture.put(filename);
    }
    let mut wrong_revision = model;
    wrong_revision.revision = Some("different-revision".into());
    assert!(SpeakrsEngine
        .probe(&wrong_revision)
        .unwrap_err()
        .to_string()
        .contains("revision"));
}

#[test]
fn a_cpu_request_for_the_coreml_profile_fails_before_audio_decoding() {
    let fixture = Fixture::new();
    for name in REQUIRED_CPU_ARTIFACTS {
        fixture.put(name);
    }
    let (context, _) = runtime::JobRuntime::default().create_job();
    let error = futures::executor::block_on(
        NativeDiarizationBackend::default().analyze(
            DiarizationRequest {
                audio_path: fixture
                    .0
                    .join("audio-that-must-not-be-opened.wav")
                    .display()
                    .to_string(),
                model: fixture.model(COREML_ID),
                device: ComputeDevice::Cpu,
                expected_speakers: None,
                speaker_library_path: None,
                stage_configuration: DiarizationStageConfiguration::default(),
            },
            context,
        ),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            contracts::BackendError {
                code: contracts::BackendErrorCode::DeviceUnavailable,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn coreml_stages_report_compiled_graphs_and_reject_partial_overrides() {
    let fixture = Fixture::new();
    let catalog = catalog();
    let entry = catalog.find(COREML_ID).unwrap();
    let mut model = fixture.model(COREML_ID);
    model.artifacts = entry
        .artifacts
        .iter()
        .map(|a| ModelArtifactRef {
            filename: a.filename.clone(),
            expected_sha256: a.sha256.clone(),
            archive_member: None,
        })
        .collect();
    let backend = NativeDiarizationBackend::default();
    let stages = backend.stage_provenance(&model);
    let segmentation = stages.segmentation.unwrap();
    let embedding = stages.embedding.unwrap();
    assert_eq!(segmentation.artifact_path, model.artifact_path);
    assert_eq!(embedding.artifact_path, model.artifact_path);
    assert!(segmentation
        .artifacts
        .iter()
        .any(|a| a.filename == "segmentation-3.0.mlmodelc/weights/weight.bin"));
    assert!(embedding
        .artifacts
        .iter()
        .any(|a| a.filename == "wespeaker-chunk-emb-s12-w22.mlmodelc/weights/weight.bin"));
    assert!(segmentation
        .artifacts
        .iter()
        .all(|a| a.filename.starts_with("segmentation-")));
    assert!(embedding
        .artifacts
        .iter()
        .all(|a| a.filename.starts_with("wespeaker-")));
    let overrides = DiarizationStageConfiguration {
        segmentation_model: Some(model.clone()),
        ..Default::default()
    };
    let error = backend
        .validate_stage_configuration(&model, &overrides)
        .unwrap_err();
    assert_eq!(error.code, contracts::BackendErrorCode::ModelIncompatible);
    assert!(error.message.contains("fixed"), "{error:?}");
}

#[test]
#[cfg(feature = "coreml")]
#[ignore = "requires explicit MEETNOTE_COREML_MODEL_ROOT with both profiles and <=10s single-speaker MEETNOTE_DIARIZATION_AUDIO"]
fn real_managed_coreml_profile_reprobes_and_preserves_the_short_baseline() {
    use std::sync::Arc;
    struct Probe;
    impl models::manager::BackendProbe for Probe {
        fn probe(&self, _task: &str, model: &ModelRef) -> Result<(), String> {
            NativeDiarizationBackend::default()
                .probe(model)
                .map_err(|error| error.message)
        }
    }
    let root = std::env::var("MEETNOTE_COREML_MODEL_ROOT").expect("explicit managed model root");
    let path =
        std::env::var("MEETNOTE_DIARIZATION_AUDIO").expect("explicit single-speaker short audio");
    let decoded = audio::read_audio_mono_16k(&path).unwrap();
    assert!(!decoded.samples.is_empty() && decoded.samples.len() <= 160_000);
    let end_ms = decoded.samples.len() as u64 * 1000 / 16000;
    let manager = models::ModelManager::with_backend_probe(catalog(), &root, Some(Arc::new(Probe)));
    let task = manager.reprobe(COREML_ID).unwrap();
    let record = futures::executor::block_on(task.finished).unwrap().unwrap();
    assert!(record.passed());
    assert_eq!(record.checksums.len(), 76);
    assert!(record.checksums.iter().all(|row| row.result.verified()));
    let model = manager.model_ref(COREML_ID, "diarization-native").unwrap();
    let cpu = manager
        .model_ref("speakrs-community-1-cpu", "diarization-native")
        .unwrap();
    let backend = NativeDiarizationBackend::default();
    let runtime = runtime::JobRuntime::default();
    let request = |model| DiarizationRequest {
        audio_path: path.clone(),
        model,
        device: ComputeDevice::Auto,
        expected_speakers: None,
        speaker_library_path: None,
        stage_configuration: DiarizationStageConfiguration::default(),
    };
    let (context, _) = runtime.create_job();
    let whole =
        futures::executor::block_on(backend.diarize(request(model.clone()), context)).unwrap();
    let (context, _) = runtime.create_job();
    let mut session = backend.create_window_session(request(model)).unwrap();
    let plan = application::LongAudioConfig::default();
    let mut reader = audio::AudioChunkReader::open(&path, plan.decode_chunk_samples).unwrap();
    let mut planner = audio::ProcessingWindowPlanner::new(plan.window_plan);
    while let Some(chunk) = reader.next_chunk().unwrap() {
        for window in planner.push(chunk).unwrap() {
            session.process_window(Arc::new(window), &context).unwrap();
        }
    }
    planner.finish().unwrap();
    let windowed = session.finish().unwrap();
    let (context, _) = runtime.create_job();
    let cpu = futures::executor::block_on(backend.diarize(request(cpu), context)).unwrap();
    let canonical = |result: &contracts::DiarizationResult| {
        result
            .segments
            .iter()
            .map(|s| (s.speaker_id.clone(), s.start_ms, s.end_ms))
            .collect::<Vec<_>>()
    };
    assert!(!whole.segments.is_empty());
    assert_eq!(whole.speaker_count, Some(1));
    assert_eq!(canonical(&whole), canonical(&windowed));
    for result in [&whole, &windowed, &cpu] {
        assert!(result
            .segments
            .iter()
            .all(|s| s.start_ms < s.end_ms && s.end_ms <= end_ms));
        assert_eq!(result.embedding_observations.len(), 1);
    }
    let whole_observation = &whole.embedding_observations[0];
    let window_observation = &windowed.embedding_observations[0];
    let cpu_observation = &cpu.embedding_observations[0];
    assert!(whole_observation
        .embedding_space
        .compatible_with(&cpu_observation.embedding_space));
    let within_mode =
        application::cosine_similarity(&whole_observation.vector, &window_observation.vector);
    let across_devices =
        application::cosine_similarity(&whole_observation.vector, &cpu_observation.vector);
    assert!(within_mode > 0.999);
    assert!(across_devices >= application::IdentityResolverConfig::default().default_threshold);
    eprintln!("managed coreml: checksums=76 whole/window speakers=1 within-mode-cosine={within_mode} cpu-cosine={across_devices}");
}
