//! The fixture the L2 route tests (`routes.rs`) and the L3 screenshots
//! (`visual.rs`) both render: settings in a directory of their own, three
//! in-memory documents (one a semantic child), the `zh-CN` locale, a pinned
//! appearance (dark opaque unless a test picks another), a model root in
//! which Paraformer is installed with a passing probe record and the native
//! diarization bundle failed its download probe (`install_fixture_models`),
//! and an Assistant service that is never reached. The
//! weekly recording both of its documents name exists and the interview's
//! does not (`fixture_directory`). Every value is fixed, so a route renders
//! the same frame on every run. `pixels.rs` beside this file is not part of
//! the fixture; `visual.rs` and the `pixels` target include it directly.

use application::{
    AsrBackend as _, BackendInventory, DiarizationBackend as _, DocumentRepository as _,
    DocumentService, ProcessingService, WorkspaceService,
};
use asr_whisper_rs::WhisperRsBackend;
use contracts::ModelRef;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::{
    DiarizationProvenance, DiarizationSegment, DocumentState, ModelProvenance, ProcessingRecord,
    ProjectId, RuntimeProvenance, SpeakerTaggedSegment, TranscriptSegment,
};
use gpui::{px, size, App, Bounds, WindowBounds, WindowOptions};
use models::{ModelCatalog, ModelManager};
use persistence::InMemoryDocumentRepository;
use settings::{JsonSettingsRepository, SettingsService};
use std::sync::Arc;
use ui::assistant_ui::{
    AssistantCancellation, AssistantMutationSummary, AssistantRunRequest, AssistantRunResult,
    AssistantSettingsSave, AssistantUiError, AssistantUiService, ConnectionDiagnostic,
    ConnectionProbeKind,
};
use ui::test_support::MainWindow;

pub const LOCALE: &str = "zh-CN";
/// The window size `run` opens.
pub const WINDOW_WIDTH: f32 = 1180.;
pub const WINDOW_HEIGHT: f32 = 760.;

/// The selected root, its semantic child and a second root, in list order.
pub const DOCUMENT_IDS: [&str; 3] = [
    "doc-1-weekly-sync",
    "doc-1-weekly-sync-decisions",
    "doc-2-customer-interview",
];

/// A fixed instant, so no document carries the time the test ran.
const FIXTURE_TIME_MS: u64 = 1_788_000_000_000;
/// The recordings the documents name, relative to `fixture_directory`.
const WEEKLY_AUDIO: &str = "fixtures/weekly-sync.wav";
const INTERVIEW_AUDIO: &str = "fixtures/customer-interview.wav";

/// The app-wide setup `run` performs.
pub fn init(cx: &mut App) {
    ui::test_support::init(cx);
}

/// Opens the desktop window on the fixture in the dark opaque appearance,
/// at the size `run` uses.
#[allow(dead_code, reason = "`visual.rs` opens every appearance itself")]
pub fn open_main_window(cx: &mut App) -> MainWindow {
    open_main_window_in(
        settings::AppearanceSettings {
            theme: settings::ThemePreference::Dark,
            material: settings::MaterialPreference::Opaque,
        },
        cx,
    )
}

/// Opens the desktop window on the fixture in `appearance`, which must be
/// pinned: with `ThemePreference::System` the frame would follow the
/// machine's appearance.
pub fn open_main_window_in(appearance: settings::AppearanceSettings, cx: &mut App) -> MainWindow {
    assert_ne!(
        appearance.theme,
        settings::ThemePreference::System,
        "the fixture pins its appearance"
    );
    MainWindow::open(inventory(), services(appearance), window_options(), cx)
        .expect("the fixture window opens")
}

pub fn catalog() -> ModelCatalog {
    ModelCatalog::from_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/models/catalog.json"),
    )
    .expect("the repository model catalog loads")
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: gpui::point(px(0.), px(0.)),
            size: size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

fn inventory() -> BackendInventory {
    BackendInventory::new(
        vec![WhisperRsBackend::default().descriptor()],
        vec![SherpaOnnxDiarizationBackend::default().descriptor()],
    )
}

/// The directory the test process runs in from the first fixture window on.
/// It holds the weekly recording at the relative path its documents name,
/// so its audio check finds the file, and not the interview's, whose audio
/// is missing. Owning the working directory keeps every relative path of
/// the fixture off the checkout.
fn fixture_directory() -> &'static std::path::Path {
    static DIRECTORY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let directory =
            std::env::temp_dir().join(format!("meetnote-ui-fixture-cwd-{}", std::process::id()));
        let weekly = directory.join(WEEKLY_AUDIO);
        std::fs::create_dir_all(weekly.parent().expect("the recording has a folder"))
            .expect("the fixture directory");
        std::fs::write(&weekly, b"fixture recording").expect("the weekly recording");
        std::env::set_current_dir(&directory).expect("the fixture working directory");
        directory
    })
}

fn services(appearance: settings::AppearanceSettings) -> ui::DesktopServices {
    fixture_directory();
    let repository = Arc::new(InMemoryDocumentRepository::default());
    for document in documents() {
        // The repository keeps the fixed revision and timestamps;
        // `DocumentService::save` would stamp the current time.
        repository.save(document, None).expect("fixture document");
    }
    let temp = temp_directory();
    install_fixture_models(&temp.join("models"));
    let model_manager = ModelManager::new(catalog(), temp.join("models"));
    ui::DesktopServices {
        workspace: WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            DocumentService::new(repository),
        ),
        settings: settings(&temp, appearance),
        installation: application::InstallationService::new(model_manager.clone()),
        model_manager: Arc::new(model_manager),
        asr_model: missing_model("whisper-rs"),
        diarization_model: missing_model("sherpa-onnx"),
        assistant: Arc::new(UnreachableAssistant),
        speaker_library: speaker_library(),
    }
}

/// The installed model with a passing record and the failed one.
const PASSING_MODEL: &str = "paraformer-zh";
const FAILING_MODEL: &str = "speakrs-community-1-cpu";
const REJECTION: &str = "the embedding graph cannot find its external data";

/// Writes the probe records the manager would have written at the fixed
/// instant, in its `meetnote.model-probe.v1` format: Paraformer's download
/// verified its archive and passed its backend, and its files are
/// installed; the native bundle's backend rejected the download, so the
/// manager removed the files and kept the failure. Every other model has no
/// file and no record.
fn install_fixture_models(model_root: &std::path::Path) {
    let catalog = catalog();
    let model = |id: &str| {
        catalog
            .find(id)
            .expect("the catalog lists the fixture model")
    };
    let checksums = |entry: &models::ModelCatalogEntry, result: &str| {
        entry
            .artifacts
            .iter()
            .map(|artifact| {
                format!(
                    r#"{{"filename": "{}", "expectedSha256": "{}", "result": "{result}"}}"#,
                    artifact.filename,
                    artifact
                        .sha256
                        .as_deref()
                        .expect("the fixture pins every checksum")
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let write = |path: std::path::PathBuf, content: String| {
        std::fs::create_dir_all(path.parent().expect("a model file has a folder"))
            .expect("the fixture model folder");
        std::fs::write(path, content).expect("a fixture model file");
    };

    let passing = model(PASSING_MODEL);
    for artifact in passing.artifact_paths(model_root) {
        write(artifact, "fixture weights".into());
    }
    write(
        passing.probe_record_path(model_root),
        format!(
            r#"{{"schemaVersion": "meetnote.model-probe.v1", "modelId": "{PASSING_MODEL}",
  "probe": {{"kind": "download", "probedAtMs": {FIXTURE_TIME_MS}, "revision": "{}",
    "checksums": [{}],
    "backends": [{{"backendId": "sherpa-onnx", "result": "passed"}}],
    "outcome": {{"result": "passed"}}}}}}"#,
            passing.revision.as_deref().unwrap_or_default(),
            checksums(passing, "archive-matched"),
        ),
    );

    let failing = model(FAILING_MODEL);
    let reason = format!(
        r#"{{"kind": "backend-probe", "backendId": "diarization-native", "message": "{REJECTION}"}}"#
    );
    let revision = failing.revision.as_deref().unwrap_or_default();
    write(
        failing.probe_record_path(model_root),
        format!(
            r#"{{"schemaVersion": "meetnote.model-probe.v1", "modelId": "{FAILING_MODEL}",
  "probe": {{"kind": "download", "probedAtMs": {FIXTURE_TIME_MS}, "revision": "{revision}",
    "checksums": [{}],
    "backends": [{{"backendId": "diarization-native", "result": "failed", "message": "{REJECTION}"}}],
    "outcome": {{"result": "failed", "reason": {reason}}}}},
  "failure": {{"operation": "download", "failedAtMs": {FIXTURE_TIME_MS}, "revision": "{revision}",
    "reason": {reason}}}}}"#,
            checksums(failing, "matched"),
        ),
    );
}

/// Three fixed profiles in memory, so no test reads or writes a library file.
fn speaker_library() -> application::SpeakerLibraryService {
    use application::SpeakerProfileStore as _;
    let space = contracts::EmbeddingSpace {
        backend_id: "diarization-native".into(),
        model_id: "wespeaker-resnet34".into(),
        model_revision: Some("1".into()),
        dimension: 2,
        sample_rate_hz: 16_000,
        frontend: contracts::EmbeddingFrontend::default(),
        legacy_feature_config: None,
        normalization: contracts::EmbeddingNormalization::L2,
    };
    let profile = |id: &str, name: &str, aliases: &[&str], vector: [f32; 2], samples: usize| {
        contracts::SpeakerProfile {
            id: id.into(),
            display_name: name.into(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            embedding_space: space.clone(),
            samples: (0..samples)
                .map(|index| contracts::SpeakerEmbeddingSample {
                    id: format!("{id}-sample-{index}"),
                    vector: vector.to_vec(),
                    quality: 0.9,
                    source: contracts::SpeakerSampleSource {
                        project_id: Some(DOCUMENT_IDS[0].into()),
                        source_segment_id: Some(format!("t-{index}")),
                        start_ms: 0,
                        end_ms: 4_000,
                        audio_path: None,
                        audio_sha256: None,
                        backup_path: None,
                    },
                    created_at_ms: FIXTURE_TIME_MS,
                })
                .collect(),
            match_threshold: 0.7,
            created_at_ms: FIXTURE_TIME_MS,
            updated_at_ms: FIXTURE_TIME_MS,
        }
    };
    let store = application::InMemorySpeakerProfileStore::default();
    store
        .replace(&contracts::SpeakerLibraryDocument {
            schema_version: contracts::SPEAKER_LIBRARY_SCHEMA_VERSION,
            profiles: vec![
                profile("profile-wang", "王经理", &["Wang"], [1.0, 0.0], 3),
                profile("profile-li", "李工", &[], [0.0, 1.0], 2),
                profile("profile-guest", "访谈嘉宾", &[], [0.7, 0.7], 1),
            ],
        })
        .expect("the fixture library is valid");
    application::SpeakerLibraryService::new(Arc::new(store))
}

/// A directory no other test uses; it is not created.
fn temp_directory() -> std::path::PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "meetnote-ui-fixture-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ))
}

fn settings(temp: &std::path::Path, appearance: settings::AppearanceSettings) -> SettingsService {
    let settings = SettingsService::open(Arc::new(JsonSettingsRepository::open(
        temp.join("settings.json"),
    )))
    .expect("fixture settings open");
    let mut value = settings.snapshot().expect("fixture settings read");
    value.ui_locale = LOCALE.into();
    value.appearance = appearance;
    value.asr_backend_id = Some("whisper-rs".into());
    value.diarization_backend_id = Some("sherpa-onnx".into());
    value.speaker_library_path = "data/speakers/library.json".into();
    value.assistant.active_connection_id = Some("local".into());
    value.assistant.connections = vec![settings::AssistantConnectionProfile {
        id: "local".into(),
        name: "Local LiteLLM".into(),
        protocol: settings::LlmProtocol::OpenAiChatCompletions,
        base_url: "http://127.0.0.1:4000".into(),
        model: "assistant-default".into(),
        auth_mode: settings::AssistantAuthMode::Bearer,
        api_key: None,
        api_key_file: None,
        context_window_tokens: Some(128_000),
        timeout_ms: 120_000,
        max_output_tokens: 4_096,
    }];
    settings.update(value).expect("fixture settings write");
    settings
}

fn missing_model(backend_id: &str) -> ModelRef {
    ModelRef {
        backend_id: backend_id.into(),
        model_id: "fixture".into(),
        revision: None,
        artifact_path: "missing-model".into(),
        artifacts: Vec::new(),
    }
}

fn documents() -> Vec<DocumentState> {
    let segment =
        |id: &str, speaker: &str, start_ms: u64, end_ms: u64, text: &str| SpeakerTaggedSegment {
            transcript_segment_id: id.into(),
            speaker_id: speaker.into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms,
            end_ms,
            text: text.into(),
        };
    let mut weekly = DocumentState::new(ProjectId::new(DOCUMENT_IDS[0]), "产品周会", WEEKLY_AUDIO);
    weekly.content.tags = vec!["周会".into()];
    weekly.content.summary = Some(
        "确认 GPUI 依赖迁移前先建立各页面的截图基线；Models 页补充 license 与来源展示；\
         下周复盘长音频的处理耗时与内存占用。"
            .into(),
    );
    weekly.results.speaker_segments = vec![
        segment(
            "t-1",
            "speaker-0",
            0,
            42_000,
            "先过一下迁移前要冻结的页面。",
        ),
        segment(
            "t-2",
            "speaker-1",
            42_000,
            95_000,
            "截图基线和结构测试都要在当前依赖上生成。",
        ),
    ];
    // The transcript and speaker turns the aligned segments came from.
    weekly.results.transcript = weekly
        .results
        .speaker_segments
        .iter()
        .map(|segment| TranscriptSegment {
            id: segment.transcript_segment_id.clone(),
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text.clone(),
        })
        .collect();
    weekly.results.diarization = weekly
        .results
        .speaker_segments
        .iter()
        .map(|segment| DiarizationSegment {
            id: format!("d-{}", segment.transcript_segment_id),
            speaker_id: segment.speaker_id.clone(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            confidence: None,
        })
        .collect();
    // Complete provenance except the input audio checksum, so Detail lists
    // one missing fact.
    weekly.record_processing(ProcessingRecord {
        processing_revision: 1,
        completed_at_ms: FIXTURE_TIME_MS,
        input_audio_checksum: None,
        asr: ModelProvenance {
            backend_id: "whisper-rs".into(),
            model_id: "whisper-large-v3-turbo".into(),
            revision: Some("v3-turbo".into()),
            artifact_sha256: Some(
                "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69".into(),
            ),
            artifact_path: "models/asr/whisper-large-v3-turbo/ggml-large-v3-turbo.bin".into(),
            artifacts: Vec::new(),
        },
        diarization: DiarizationProvenance {
            backend_id: "sherpa-onnx".into(),
            model_revision: Some("2024-10".into()),
            segmentation: None,
            embedding: None,
            clustering: domain::AlgorithmProvenance {
                id: "fast-clustering".into(),
                revision: Some("1".into()),
                parameters: None,
            },
            identity_matching: None,
        },
        alignment: domain::AlgorithmProvenance {
            id: "max-overlap-speaker-alignment".into(),
            revision: Some("1".into()),
            parameters: None,
        },
        runtime: RuntimeProvenance {
            device: "Metal".into(),
            language: Some("zh".into()),
            sample_rate_hz: Some(16_000),
            ..Default::default()
        },
    });
    let decisions = weekly.semantic_child(
        ProjectId::new(DOCUMENT_IDS[1]),
        "决策与行动项",
        42_000,
        95_000,
        vec!["决策".into()],
    );
    let mut interview =
        DocumentState::new(ProjectId::new(DOCUMENT_IDS[2]), "客户访谈", INTERVIEW_AUDIO);
    interview.hierarchy.sibling_order = 1;
    let mut documents = vec![weekly, decisions, interview];
    for document in &mut documents {
        document.schema.updated_at_ms = FIXTURE_TIME_MS;
        document.identity.created_at_ms = FIXTURE_TIME_MS;
    }
    documents
}

/// The route pages render without the Assistant; any call is a test bug.
struct UnreachableAssistant;

impl UnreachableAssistant {
    fn unreachable<T>(&self, operation: &str) -> Result<T, AssistantUiError> {
        Err(AssistantUiError::unavailable(format!(
            "the route fixture does not {operation}"
        )))
    }
}

impl AssistantUiService for UnreachableAssistant {
    fn save_assistant_settings(
        &self,
        _: String,
        _: AssistantSettingsSave,
    ) -> Result<settings::AppSettings, AssistantUiError> {
        self.unreachable("save settings")
    }

    fn probe_connection(
        &self,
        _: settings::AssistantConnectionProfile,
        _: ConnectionProbeKind,
        _: AssistantCancellation,
    ) -> Result<ConnectionDiagnostic, AssistantUiError> {
        self.unreachable("probe connections")
    }

    fn run(
        &self,
        _: AssistantRunRequest,
        _: AssistantCancellation,
    ) -> Result<AssistantRunResult, AssistantUiError> {
        self.unreachable("run")
    }

    fn confirm(&self, _: String, _: Vec<String>) -> Result<AssistantRunResult, AssistantUiError> {
        self.unreachable("confirm")
    }

    fn reject(&self, _: String, _: Vec<String>) -> Result<(), AssistantUiError> {
        self.unreachable("reject")
    }

    fn undo(
        &self,
        _: String,
        _: Vec<String>,
    ) -> Result<AssistantMutationSummary, AssistantUiError> {
        self.unreachable("undo")
    }
}
