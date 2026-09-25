//! The fixture the L2 route tests (`routes.rs`) and the L3 screenshots
//! (`visual.rs`) both render: settings in a directory of their own, three
//! in-memory documents (one a semantic child), the `zh-CN` locale, a pinned
//! appearance (dark opaque unless a test picks another), a model root that
//! holds no model, and an Assistant service that is never reached. Every
//! value is fixed, so a route renders the same frame on every run. `pixels.rs` beside this file is not part of
//! the fixture; `visual.rs` and the `pixels` target include it directly.

use application::{
    AsrBackend as _, BackendInventory, DiarizationBackend as _, DocumentRepository as _,
    DocumentService, ProcessingService, WorkspaceService,
};
use asr_whisper_rs::WhisperRsBackend;
use contracts::ModelRef;
use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
use domain::{DocumentState, ProcessingRecord, ProjectId, SpeakerTaggedSegment};
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

fn services(appearance: settings::AppearanceSettings) -> ui::DesktopServices {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    for document in documents() {
        // The repository keeps the fixed revision and timestamps;
        // `DocumentService::save` would stamp the current time.
        repository.save(document, None).expect("fixture document");
    }
    let temp = temp_directory();
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
        // A model root that does not exist: every catalog model is missing.
        installation: application::InstallationService::new(model_manager.clone()),
        model_manager: Arc::new(model_manager),
        asr_model: missing_model("whisper-rs"),
        diarization_model: missing_model("sherpa-onnx"),
        assistant: Arc::new(UnreachableAssistant),
    }
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
    let mut weekly = DocumentState::new(
        ProjectId::new(DOCUMENT_IDS[0]),
        "产品周会",
        "fixtures/weekly-sync.wav",
    );
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
    weekly.record_processing(ProcessingRecord {
        processing_revision: 1,
        completed_at_ms: FIXTURE_TIME_MS,
        input_audio_checksum: None,
        asr: Default::default(),
        diarization: Default::default(),
        alignment: Default::default(),
        runtime: Default::default(),
    });
    let decisions = weekly.semantic_child(
        ProjectId::new(DOCUMENT_IDS[1]),
        "决策与行动项",
        42_000,
        95_000,
        vec!["决策".into()],
    );
    let mut interview = DocumentState::new(
        ProjectId::new(DOCUMENT_IDS[2]),
        "客户访谈",
        "fixtures/customer-interview.wav",
    );
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
