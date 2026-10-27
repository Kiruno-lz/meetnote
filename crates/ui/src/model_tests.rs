//! L2: the Models route's diagnosis of each catalog model — its name, status
//! and one action in the header row; task, size, VRAM, backends, license and
//! the source's short form, with the full source and checksums in its
//! details; the latest probe record; a failed download or probe with its
//! localized reason and retry; an unreadable probe record — and the re-probe
//! of an installed model.
//!
//! Most tests render installation states they build, as
//! `InstallationService` would publish them, through a probe that returns
//! the state the test holds, with a fake `ModelService` that records the
//! calls the page makes. Real-manager tests re-probe fixture files in a
//! temporary model root, so the records shown come from the manager and the
//! installation service. They also check that background results leave an
//! active Settings menu and its keyboard choice intact.

use crate::controller::{ModelService, ReprobeWait};
use crate::document_tests::{leak, rendered, within, View};
use crate::pages::models::{family_summary, model_facts, readiness_text, FactRow, ModelFacts};
use crate::route::Route;
use crate::tests::{
    assert_ui_responsive_while_held, click_selector, desktop_services, hold_a_worker_alone,
    open_desktop_on, temp_directory, test_settings, wait_for, FakeAssistantUiService, Gate,
};
use application::{InstallationService, InstallationState};
use contracts::ModelRef;
use futures::channel::oneshot;
use gpui::{TestAppContext, VisualTestContext};
use models::manager::{
    DownloadEvent, DownloadId, DownloadProgress, DownloadState, DownloadTask, ModelManagerError,
    ReprobeTask,
};
use models::{
    ArtifactChecksum, BackendCheck, BackendProbeResult, CatalogArtifact, ChecksumResult,
    ModelAvailability, ModelCatalog, ModelCatalogEntry, ModelFailure, ModelFailureRecord,
    ModelInstallState, ModelManager, ModelOperation, ProbeOutcome, ProbeRecord, ProbeRecordError,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The instant every built record carries: 2026-08-29 10:40 UTC.
const PROBED_AT_MS: u64 = 1_788_000_000_000;
const PROBED_AT: &str = "2026-08-29 10:40 UTC";
/// Two archive members share their archive's checksum.
const ARCHIVE_SHA256: &str = "da92b3db5218c5be53aad53e57d1b6e63e7fc98a0e054fbdd6dbe18e9c6b1450";
const SEGMENTATION_SHA256: &str =
    "038b971741ed623af9773ecafdefa4b7bc523520099c2a68f8568b24189e8ad9";
const EMBEDDING_SHA256: &str = "203a4c67112167580ab1fcb62f4568c633499fb283805890aebe1c48564fcc0f";
const PLDA_SHA256: &str = "e20c9b012bebd1aabda5a38a127e63a43cf35debdc502715fc143e2fb6bc3c4b";
/// A model file's bytes and their SHA-256, and different bytes.
const MODEL_BYTES: &[u8] = b"meetnote fixture audio\n";
const MODEL_SHA256: &str = "731e82f7248116585d832c2ea8784dc94a8dcfd6529a6babfc94605e8fda9e49";
const CHANGED_BYTES: &[u8] = b"a different recording\n";
const CHANGED_SHA256: &str = "13c12877ebfb2e7edafd47f9c0b765a656e85a055c2b5cb35c3ff7770e611e93";
/// Longer than any card, so it must fade rather than widen the card.
const LONG_SOURCE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
                           sherpa-onnx-paraformer-zh-small-2024-03-09-with-a-very-long-release-\
                           asset-name-that-keeps-going-past-the-card.tar.bz2";
const LONG_LICENSE: &str = "apache-2.0-and-mit-and-cc-by-4.0-model-notices-required";

fn entry(id: &str, task: &str, family: &str, backends: &[&str]) -> ModelCatalogEntry {
    ModelCatalogEntry {
        supported_platforms: Vec::new(),
        id: id.into(),
        task: task.into(),
        model_family: family.into(),
        candidate_backends: backends.iter().map(|backend| backend.to_string()).collect(),
        revision: Some("fixture-v1".into()),
        source_format: "bin".into(),
        source: format!("https://models.example/{id}/model.bin"),
        size_bytes: Some(81_904_027),
        estimated_vram_bytes: Some(1 << 30),
        sha256: None,
        license: "apache-2.0".into(),
        requires_auth: false,
        status: "ready".into(),
        artifact_files: Vec::new(),
        artifacts: Vec::new(),
    }
}

fn artifact(filename: &str, sha256: &str, member: bool) -> CatalogArtifact {
    CatalogArtifact {
        filename: filename.into(),
        source: LONG_SOURCE.into(),
        sha256: Some(sha256.into()),
        archive_member: member.then(|| format!("bundle/{filename}")),
    }
}

/// A Paraformer-like bundle: two archive members of one long-named archive.
fn bundle_asr() -> ModelCatalogEntry {
    ModelCatalogEntry {
        source: LONG_SOURCE.into(),
        artifacts: vec![
            artifact("model.int8.onnx", ARCHIVE_SHA256, true),
            artifact("tokens.txt", ARCHIVE_SHA256, true),
        ],
        ..entry(
            "bundle-asr",
            "asr",
            "a-bundle",
            &["sherpa-onnx", "whisper-rs"],
        )
    }
}

/// A native diarization bundle: three files, each with its own checksum.
fn native_diarization() -> ModelCatalogEntry {
    ModelCatalogEntry {
        license: LONG_LICENSE.into(),
        artifacts: vec![
            artifact("segmentation-3.0.onnx", SEGMENTATION_SHA256, false),
            artifact("wespeaker-voxceleb-resnet34.onnx", EMBEDDING_SHA256, false),
            artifact("plda_lda.npy", PLDA_SHA256, false),
        ],
        ..entry(
            "native-diarization",
            "diarization",
            "b-native",
            &["diarization-native"],
        )
    }
}

/// A source without a download manifest or checksum.
fn candidate_asr() -> ModelCatalogEntry {
    ModelCatalogEntry {
        status: "candidate-source".into(),
        size_bytes: None,
        ..entry("candidate-asr", "asr", "c-candidate", &["sherpa-onnx"])
    }
}

fn catalog(models: Vec<ModelCatalogEntry>) -> ModelCatalog {
    ModelCatalog {
        schema_version: "meetnote.model-catalog.v1".into(),
        models,
    }
}

fn availability(model: ModelCatalogEntry, install_state: ModelInstallState) -> ModelAvailability {
    ModelAvailability {
        model,
        install_state,
        artifact_paths: Vec::new(),
        probe: None,
        failure: None,
        record_error: None,
    }
}

/// The probe right after a download of `bundle_asr`: both archive members
/// verified, one backend passed, one without a registered probe.
fn passing_download_probe() -> ProbeRecord {
    ProbeRecord {
        kind: ModelOperation::Download,
        probed_at_ms: PROBED_AT_MS,
        revision: Some("fixture-v1".into()),
        checksums: vec![
            ArtifactChecksum {
                filename: "model.int8.onnx".into(),
                expected_sha256: Some(ARCHIVE_SHA256.into()),
                result: ChecksumResult::ArchiveMatched,
            },
            ArtifactChecksum {
                filename: "tokens.txt".into(),
                expected_sha256: Some(ARCHIVE_SHA256.into()),
                result: ChecksumResult::Matched,
            },
        ],
        backends: vec![
            BackendProbeResult {
                backend_id: "sherpa-onnx".into(),
                result: BackendCheck::Passed,
            },
            BackendProbeResult {
                backend_id: "whisper-rs".into(),
                result: BackendCheck::NotRegistered,
            },
        ],
        outcome: ProbeOutcome::Passed,
    }
}

const BACKEND_MESSAGE: &str = "plda_psi.npy is missing";

fn backend_rejection() -> ModelFailure {
    ModelFailure::BackendProbe {
        backend_id: "diarization-native".into(),
        message: BACKEND_MESSAGE.into(),
    }
}

/// `native_diarization` after a download whose backend probe failed: the
/// manager removed the files and kept the record and the failure.
fn failed_download() -> ModelAvailability {
    let mut failed = availability(native_diarization(), ModelInstallState::Missing);
    failed.probe = Some(ProbeRecord {
        kind: ModelOperation::Download,
        probed_at_ms: PROBED_AT_MS,
        revision: Some("fixture-v1".into()),
        checksums: Vec::new(),
        backends: vec![BackendProbeResult {
            backend_id: "diarization-native".into(),
            result: BackendCheck::Failed {
                message: BACKEND_MESSAGE.into(),
            },
        }],
        outcome: ProbeOutcome::Failed {
            reason: backend_rejection(),
        },
    });
    failed.failure = Some(ModelFailureRecord {
        operation: ModelOperation::Download,
        failed_at_ms: PROBED_AT_MS,
        revision: Some("fixture-v1".into()),
        reason: backend_rejection(),
    });
    failed
}

/// The calls the page makes. `start` completes at once; `reprobe` waits in
/// `gate` while it is armed, then returns a task the test finishes; `list`
/// reports `downloads`, and pause, resume and cancel are recorded in
/// `controls`.
#[derive(Default)]
struct FakeModels {
    catalog: Option<ModelCatalog>,
    downloads: Mutex<Vec<DownloadProgress>>,
    controls: Mutex<Vec<(&'static str, DownloadId)>>,
    starts: Mutex<Vec<String>>,
    start_gate: Gate,
    fail_start: Mutex<bool>,
    reprobes: Mutex<Vec<String>>,
    gate: Gate,
    held_model: Mutex<Option<String>>,
    references: Mutex<std::collections::BTreeMap<String, ModelRef>>,
    finish: Mutex<Option<oneshot::Sender<Result<ProbeRecord, ModelManagerError>>>>,
}

impl FakeModels {
    fn new(catalog: ModelCatalog) -> Arc<Self> {
        Arc::new(Self {
            catalog: Some(catalog),
            ..Self::default()
        })
    }

    fn starts(&self) -> Vec<String> {
        self.starts.lock().unwrap().clone()
    }

    fn reprobes(&self) -> Vec<String> {
        self.reprobes.lock().unwrap().clone()
    }

    fn controls(&self) -> Vec<(&'static str, DownloadId)> {
        self.controls.lock().unwrap().clone()
    }

    fn control(&self, action: &'static str, id: DownloadId) -> Result<(), ModelManagerError> {
        self.controls.lock().unwrap().push((action, id));
        Ok(())
    }
}

impl ModelService for FakeModels {
    fn catalog(&self) -> &ModelCatalog {
        self.catalog.as_ref().expect("the fake has a catalog")
    }

    fn list(&self) -> Result<Vec<DownloadProgress>, ModelManagerError> {
        Ok(self.downloads.lock().unwrap().clone())
    }

    fn model_ref(&self, model_id: &str, backend_id: &str) -> Result<ModelRef, ModelManagerError> {
        let held = self.held_model.lock().unwrap().as_deref() == Some(model_id);
        if held {
            self.gate.pass();
        }
        self.references
            .lock()
            .unwrap()
            .get(model_id)
            .filter(|model| model.backend_id == backend_id)
            .cloned()
            .ok_or_else(|| ModelManagerError::ModelNotInstalled(model_id.into()))
    }

    fn start(&self, model_id: &str) -> Result<DownloadTask, ModelManagerError> {
        self.starts.lock().unwrap().push(model_id.into());
        self.start_gate.pass();
        if *self.fail_start.lock().unwrap() {
            return Err(ModelManagerError::ModelNotFound(model_id.into()));
        }
        let (sender, updates) = futures::channel::mpsc::unbounded();
        sender
            .unbounded_send(DownloadEvent::Updated(DownloadProgress {
                id: DownloadId(1),
                model_id: model_id.into(),
                state: DownloadState::Completed,
                completed_bytes: 1,
                speed_bytes_per_second: None,
                total_bytes: Some(1),
                artifact_index: 1,
                artifact_count: 1,
                eta_seconds: Some(0),
                retry_count: 0,
            }))
            .unwrap();
        Ok(DownloadTask {
            id: DownloadId(1),
            events: std::sync::mpsc::channel().1,
            updates,
        })
    }

    fn pause(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        self.control("pause", id)
    }

    fn resume(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        self.control("resume", id)
    }

    fn cancel(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        self.control("cancel", id)
    }

    fn reprobe(&self, model_id: &str) -> Result<ReprobeTask, ModelManagerError> {
        self.gate.pass();
        self.reprobes.lock().unwrap().push(model_id.into());
        let (sender, finished) = oneshot::channel();
        *self.finish.lock().unwrap() = Some(sender);
        Ok(ReprobeTask {
            model_id: model_id.into(),
            finished,
        })
    }
}

/// The installation state the service publishes, which the test replaces.
type Published = Arc<Mutex<InstallationState>>;

fn published(catalog: Vec<ModelAvailability>) -> Published {
    Arc::new(Mutex::new(InstallationState {
        catalog,
        ..InstallationState::default()
    }))
}

/// The desktop on the Models route, its installation service publishing
/// `state` and its model calls reaching `models`.
fn open_models<'a>(
    cx: &'a mut TestAppContext,
    models: Arc<FakeModels>,
    state: &Published,
    settings: settings::SettingsService,
) -> (View, &'a mut VisualTestContext) {
    let mut services = desktop_services(Arc::new(FakeAssistantUiService::default()), settings);
    let probe_state = state.clone();
    services.installation = InstallationService::with_probe(
        ModelManager::new(models.catalog().clone(), temp_directory()),
        Arc::new(move |_| probe_state.lock().unwrap().clone()),
    );
    services.model_manager = models;
    open_on_models(cx, services)
}

fn open_on_models(
    cx: &mut TestAppContext,
    services: crate::DesktopServices,
) -> (View, &mut VisualTestContext) {
    let (view, cx) = open_desktop_on(cx, services);
    wait_for(cx, &view, |root| !root.installation.catalog.is_empty());
    view.update(cx, |root, cx| root.navigate(Route::Models, cx));
    cx.run_until_parked();
    (view, cx)
}

/// What the card of `model_id` shows, from the state the view holds.
fn facts(cx: &mut VisualTestContext, view: &View, model_id: &str) -> ModelFacts {
    view.read_with(cx, |root, _| {
        let entry = root
            .installation
            .catalog
            .iter()
            .find(|entry| entry.model.id == model_id)
            .unwrap_or_else(|| panic!("{model_id} is in the published state"));
        model_facts(entry, root.ui.locale)
    })
}

fn selector(name: &str, model_id: &str) -> &'static str {
    leak(format!("model-{name}-{model_id}"))
}

fn shown(cx: &mut VisualTestContext, name: &str, model_id: &str) -> bool {
    cx.debug_bounds(selector(name, model_id)).is_some()
}

fn row(label: &str, value: impl Into<String>) -> FactRow {
    FactRow {
        label: label.into(),
        value: value.into(),
    }
}

/// The card's header row: its name, then the status chip and the one
/// action in fixed columns at its right edge.
fn assert_in_header(cx: &mut VisualTestContext, name: &str, model_id: &str) {
    let header = rendered(cx, selector("header", model_id));
    let status = rendered(cx, selector("status", model_id));
    let action = rendered(cx, selector(name, model_id));
    assert!(
        within(status, header) && within(action, header),
        "{name} of {model_id} shares the header row with its status"
    );
    assert!(
        status.right() <= action.left(),
        "the action follows the status"
    );
    assert_eq!(action.right(), header.right(), "the action ends the row");
}

#[gpui::test]
fn every_card_leads_with_its_name_status_and_action_over_quiet_facts_and_its_source(
    cx: &mut TestAppContext,
) {
    let models = [bundle_asr(), native_diarization(), candidate_asr()];
    let state = published(
        models
            .iter()
            .cloned()
            .map(|model| availability(model, ModelInstallState::Missing))
            .collect(),
    );
    let (view, cx) = open_models(
        cx,
        FakeModels::new(catalog(models.to_vec())),
        &state,
        test_settings(),
    );

    for model in &models {
        let card = rendered(cx, selector("card", &model.id));
        for name in [
            "header",
            "status",
            "facts",
            "license",
            "source",
            "details-toggle",
        ] {
            let fact = rendered(cx, selector(name, &model.id));
            assert!(within(fact, card), "{name} of {} is on its card", model.id);
        }
        // Checksums wait in the details.
        assert!(!shown(cx, "checksum", &model.id));
        let facts = facts(cx, &view, &model.id);
        assert_eq!(facts.license, model.license);
        assert_eq!(facts.source_url, model.source, "the full source in details");
        assert!(
            !facts.summary.contains(':') && !facts.summary.contains('：'),
            "facts read without labels: {}",
            facts.summary
        );
    }
    // The card names where a model comes from by host and project.
    let card = rendered(cx, "model-card-bundle-asr");
    assert!(within(rendered(cx, "model-source-bundle-asr"), card));

    let bundle = facts(cx, &view, "bundle-asr");
    assert_eq!(
        bundle.summary,
        "语音识别 · 78 MiB · 显存约 1.0 GiB · sherpa-onnx, whisper-rs"
    );
    assert_eq!(bundle.unavailable, None, "a ready model downloads");
    assert_eq!(bundle.source, "github.com/k2-fsa/sherpa-onnx");
    let native = facts(cx, &view, "native-diarization");
    assert_eq!(
        native.summary,
        "说话人分离 · 78 MiB · 显存约 1.0 GiB · diarization-native"
    );
    assert_eq!(native.source, "models.example");
    let candidate = facts(cx, &view, "candidate-asr");
    assert_eq!(
        candidate.summary, "语音识别 · 显存约 1.0 GiB · sherpa-onnx",
        "an unknown size is left out"
    );
    assert_eq!(
        candidate.unavailable.as_deref(),
        Some("来源尚未验证，暂不可下载")
    );
    assert!(shown(cx, "unavailable", "candidate-asr"));
    assert!(!shown(cx, "unavailable", "bundle-asr"));
    // Only ready models that are missing offer a download, in one column.
    assert!(!shown(cx, "download", "candidate-asr"));
    for model_id in ["bundle-asr", "native-diarization"] {
        assert_in_header(cx, "download", model_id);
    }
    let [first, second] =
        ["bundle-asr", "native-diarization"].map(|id| rendered(cx, selector("download", id)));
    assert_eq!(
        (first.left(), first.size.width),
        (second.left(), second.size.width)
    );

    // The full source and checksums are one expansion away, collapsed at
    // first.
    assert!(!shown(cx, "declared", "native-diarization-0"));
    assert!(!shown(cx, "source-url", "native-diarization"));
    click_selector(cx, "model-details-toggle-native-diarization");
    assert!(within(
        rendered(cx, "model-source-url-native-diarization"),
        rendered(cx, "model-card-native-diarization")
    ));
    assert_eq!(
        native.declared,
        [
            row("segmentation-3.0.onnx", SEGMENTATION_SHA256),
            row("wespeaker-voxceleb-resnet34.onnx", EMBEDDING_SHA256),
            row("plda_lda.npy", PLDA_SHA256),
        ]
    );
    let card = rendered(cx, "model-card-native-diarization");
    for index in 0..3 {
        let declared = rendered(
            cx,
            selector("declared", &format!("native-diarization-{index}")),
        );
        assert!(
            within(declared, card),
            "declared checksum {index} is on the card"
        );
    }
    assert!(
        !shown(cx, "declared", "bundle-asr-0"),
        "only that card expanded"
    );
    click_selector(cx, "model-details-toggle-native-diarization");
    assert!(
        !shown(cx, "declared", "native-diarization-0")
            && !shown(cx, "source-url", "native-diarization"),
        "collapsed again"
    );
    assert_eq!(candidate.declared, [row("model.bin", "未声明")]);
}

/// Starting a download persists its task on a worker. Two quick presses
/// must remain one request, with feedback on the card before IO completes.
#[gpui::test]
fn a_download_start_is_visible_and_single_flight_until_the_worker_returns(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let models = FakeModels::new(catalog(vec![bundle_asr()]));
    let state = published(vec![availability(bundle_asr(), ModelInstallState::Missing)]);
    let (view, cx) = open_models(cx, models.clone(), &state, test_settings());
    let mut pending_visible = Vec::new();
    for fail in [false, true] {
        *models.fail_start.lock().unwrap() = fail;
        models.start_gate.arm();
        click_selector(cx, "model-download-bundle-asr");
        wait_for(cx, &view, |_| models.start_gate.counts().0 > 0);
        pending_visible.push(shown(cx, "starting", "bundle-asr"));
        // The disabled button rejects another pointer click. The controller
        // also guards a repeated command, independent of render timing.
        if shown(cx, "starting", "bundle-asr") {
            assert_in_header(cx, "starting", "bundle-asr");
            click_selector(cx, "model-starting-bundle-asr");
        }
        view.update(cx, |root, cx| root.download_model("bundle-asr".into(), cx));
        models.start_gate.release();
        let expected = if fail { "下载失败" } else { "已安装" };
        wait_for(cx, &view, |root| root.ui.notice.contains(expected));
        assert!(
            shown(cx, "download", "bundle-asr"),
            "a completed or rejected start releases its button"
        );
    }
    assert_eq!(
        models.starts(),
        ["bundle-asr", "bundle-asr"],
        "each pair of commands starts only once"
    );
    assert_eq!(
        pending_visible,
        [true, true],
        "the card reports the pending start on success and failure"
    );
}

/// A download of `model_id` halfway through the first of its two files.
fn running(id: u64, model_id: &str, state: DownloadState) -> DownloadProgress {
    DownloadProgress {
        id: DownloadId(id),
        model_id: model_id.into(),
        state,
        completed_bytes: 33_659_290,
        speed_bytes_per_second: Some(5_452_595),
        total_bytes: Some(81_904_027),
        artifact_index: 1,
        artifact_count: 2,
        eta_seconds: Some(9),
        retry_count: 0,
    }
}

#[gpui::test]
fn retry_controls_follow_the_current_download_after_terminal_history(cx: &mut TestAppContext) {
    let models = FakeModels::new(catalog(vec![bundle_asr(), native_diarization()]));
    *models.downloads.lock().unwrap() = vec![
        running(
            1,
            "bundle-asr",
            DownloadState::Failed("connection lost".into()),
        ),
        running(2, "bundle-asr", DownloadState::Cancelled),
        running(3, "native-diarization", DownloadState::Completed),
        running(8, "native-diarization", DownloadState::Paused),
        running(7, "bundle-asr", DownloadState::Downloading),
    ];
    let state = published(vec![
        availability(bundle_asr(), ModelInstallState::Partial),
        failed_download(),
    ]);
    let (view, cx) = open_models(cx, models.clone(), &state, test_settings());

    assert_in_header(cx, "pause", "bundle-asr");
    assert_in_header(cx, "resume", "native-diarization");
    for model_id in ["bundle-asr", "native-diarization"] {
        assert!(shown(cx, "download-progress", model_id));
        assert!(!shown(cx, "download", model_id) && !shown(cx, "retry", model_id));
        assert!(
            !shown(cx, "failure", model_id),
            "the new attempt supersedes its failure"
        );
    }
    click_selector(cx, "model-pause-bundle-asr");
    click_selector(cx, "model-resume-native-diarization");
    click_selector(cx, "model-cancel-native-diarization");
    wait_for(cx, &view, |_| models.controls().len() == 3);
    let mut controls = models.controls();
    controls.sort();
    assert_eq!(
        controls,
        [
            ("cancel", DownloadId(8)),
            ("pause", DownloadId(7)),
            ("resume", DownloadId(8)),
        ]
    );

    // Cancelling the new attempt leaves only history; its controls must
    // disappear and the original failure can offer retry again.
    for download in models.downloads.lock().unwrap().iter_mut() {
        if !download.state.is_terminal() {
            download.state = DownloadState::Cancelled;
        }
    }
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(shown(cx, "download", "bundle-asr"));
    assert!(shown(cx, "retry", "native-diarization"));
    assert!(shown(cx, "failure", "native-diarization"));
    for model_id in ["bundle-asr", "native-diarization"] {
        assert!(!shown(cx, "download-progress", model_id));
        assert!(!shown(cx, "cancel", model_id));
    }
}

#[gpui::test]
fn a_running_download_shows_its_progress_with_pause_or_resume_and_cancel_in_the_action_column(
    cx: &mut TestAppContext,
) {
    let models = FakeModels::new(catalog(vec![bundle_asr(), native_diarization()]));
    *models.downloads.lock().unwrap() = vec![
        running(7, "bundle-asr", DownloadState::Downloading),
        running(8, "native-diarization", DownloadState::Paused),
    ];
    let state = published(vec![
        availability(bundle_asr(), ModelInstallState::Missing),
        availability(native_diarization(), ModelInstallState::Partial),
    ]);
    let (view, cx) = open_models(cx, models.clone(), &state, test_settings());

    // Pause or resume takes the download's place in the header.
    assert_in_header(cx, "pause", "bundle-asr");
    assert_in_header(cx, "resume", "native-diarization");
    for model_id in ["bundle-asr", "native-diarization"] {
        assert!(
            !shown(cx, "download", model_id),
            "{model_id} is downloading"
        );
        let card = rendered(cx, selector("card", model_id));
        let progress = rendered(cx, selector("download-progress", model_id));
        let cancel = rendered(cx, selector("cancel", model_id));
        assert!(within(progress, card) && within(cancel, card));
        assert!(
            progress.right() <= cancel.left(),
            "cancel ends the progress row"
        );
    }
    // Pause, resume and both cancels share one column.
    let actions = [
        ("pause", "bundle-asr"),
        ("resume", "native-diarization"),
        ("cancel", "bundle-asr"),
        ("cancel", "native-diarization"),
    ]
    .map(|(name, model_id)| rendered(cx, selector(name, model_id)));
    for action in &actions {
        assert_eq!(
            (action.left(), action.size.width),
            (actions[0].left(), actions[0].size.width),
            "{actions:?}"
        );
    }

    click_selector(cx, "model-pause-bundle-asr");
    click_selector(cx, "model-resume-native-diarization");
    click_selector(cx, "model-cancel-native-diarization");
    wait_for(cx, &view, |_| models.controls().len() == 3);
    let mut controls = models.controls();
    controls.sort();
    assert_eq!(
        controls,
        [
            ("cancel", DownloadId(8)),
            ("pause", DownloadId(7)),
            ("resume", DownloadId(8)),
        ]
    );
}

/// The manager retains old terminal tasks before a later retry. A card
/// must follow the active retry and send its controls to that task.
#[gpui::test]
fn a_retry_after_terminal_tasks_keeps_its_progress_and_controls_visible(cx: &mut TestAppContext) {
    let models = FakeModels::new(catalog(vec![bundle_asr()]));
    let state = published(vec![availability(bundle_asr(), ModelInstallState::Partial)]);
    *models.downloads.lock().unwrap() = vec![
        running(
            1,
            "bundle-asr",
            DownloadState::Failed("connection lost".into()),
        ),
        running(2, "bundle-asr", DownloadState::Cancelled),
        running(3, "bundle-asr", DownloadState::Completed),
        running(4, "bundle-asr", DownloadState::Downloading),
    ];
    let (view, cx) = open_models(cx, models.clone(), &state, test_settings());
    assert_in_header(cx, "pause", "bundle-asr");
    assert!(shown(cx, "download-progress", "bundle-asr"));
    assert!(!shown(cx, "download", "bundle-asr"));
    click_selector(cx, "model-pause-bundle-asr");
    click_selector(cx, "model-cancel-bundle-asr");
    wait_for(cx, &view, |_| models.controls().len() == 2);
    let mut controls = models.controls();
    controls.sort();
    assert_eq!(
        controls,
        [("cancel", DownloadId(4)), ("pause", DownloadId(4))]
    );
    models.downloads.lock().unwrap()[3].state = DownloadState::Paused;
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_in_header(cx, "resume", "bundle-asr");
    click_selector(cx, "model-resume-bundle-asr");
    wait_for(cx, &view, |_| models.controls().len() == 3);
    assert!(models.controls().contains(&("resume", DownloadId(4))));
}

#[gpui::test]
fn the_page_says_whether_the_default_models_are_ready_and_each_family_what_is_downloaded(
    cx: &mut TestAppContext,
) {
    let models = FakeModels::new(catalog(vec![
        bundle_asr(),
        native_diarization(),
        candidate_asr(),
    ]));
    let state = published(vec![
        availability(bundle_asr(), ModelInstallState::Installed),
        availability(native_diarization(), ModelInstallState::Partial),
        availability(candidate_asr(), ModelInstallState::Missing),
    ]);
    state.lock().unwrap().default_asr_ready = true;
    let (view, cx) = open_models(cx, models, &state, test_settings());

    // The line under the title answers whether audio can be processed.
    let page = rendered(cx, "models-page");
    let readiness = rendered(cx, "models-readiness");
    assert!(within(readiness, page));
    assert_eq!(
        view.read_with(cx, |root, _| readiness_text(
            &root.installation,
            root.ui.locale
        )),
        "说话人分离模型尚未就绪，可在下方下载或到设置中选择"
    );

    // The page keeps one rhythm of spacing tokens: the title and its
    // readiness line read as one block above the list, and every card sits
    // one step inside its family and keeps its content one step inside.
    let spacing = view.read_with(cx, |_, cx| crate::theme::tokens(cx).spacing);
    let title = rendered(cx, "models-title");
    let list = rendered(cx, "models-scroll");
    assert_eq!(readiness.top() - title.bottom(), spacing.xs);
    assert_eq!(list.top() - readiness.bottom(), spacing.lg);
    let family = rendered(cx, "model-family-a-bundle");
    let card = rendered(cx, "model-card-bundle-asr");
    let header = rendered(cx, "model-header-bundle-asr");
    assert_eq!(card.left() - family.left(), spacing.lg);
    assert_eq!(family.right() - card.right(), spacing.lg);
    assert_eq!(
        (header.left() - card.left(), card.right() - header.right()),
        (spacing.lg, spacing.lg)
    );

    // Each family header ends with how many of its models are downloaded.
    for family in ["a-bundle", "b-native", "c-candidate"] {
        let header = rendered(cx, leak(format!("model-family-{family}")));
        let summary = rendered(cx, leak(format!("model-family-summary-{family}")));
        assert!(
            within(summary, header) && summary.left() > header.center().x,
            "{family}: the summary ends the header"
        );
    }
    let summaries = view.read_with(cx, |root, _| {
        ["a-bundle", "b-native"].map(|family| {
            let states = root
                .installation
                .catalog
                .iter()
                .filter(|entry| entry.model.model_family == family)
                .map(|entry| entry.install_state)
                .collect::<Vec<_>>();
            family_summary(root.ui.locale, &states)
        })
    });
    assert_eq!(
        summaries,
        [
            ("已下载 1 / 1".to_string(), true),
            ("已下载 0 / 1".to_string(), false)
        ]
    );
}

#[gpui::test]
fn a_passing_probe_record_shows_its_outcome_kind_time_and_backends(cx: &mut TestAppContext) {
    let mut installed = availability(bundle_asr(), ModelInstallState::Installed);
    installed.probe = Some(passing_download_probe());
    let state = published(vec![installed]);
    let (view, cx) = open_models(
        cx,
        FakeModels::new(catalog(vec![bundle_asr()])),
        &state,
        test_settings(),
    );

    let facts = facts(cx, &view, "bundle-asr");
    let probe = facts.probe.expect("the record is shown");
    assert_eq!(probe.summary, format!("下载后校验通过 · {PROBED_AT}"));
    assert!(within(
        rendered(cx, "model-probe-bundle-asr"),
        rendered(cx, "model-card-bundle-asr")
    ));
    assert_eq!(facts.failure, None);
    assert!(!shown(cx, "failure", "bundle-asr") && !shown(cx, "retry", "bundle-asr"));
    assert!(!shown(cx, "download", "bundle-asr"), "installed");
    // Installed models re-probe from the header.
    assert_in_header(cx, "reprobe", "bundle-asr");

    // Backend and checksum results are collapsed until expanded.
    assert!(!shown(cx, "backend", "bundle-asr-0"));
    click_selector(cx, "model-details-toggle-bundle-asr");
    assert_eq!(
        probe.backends,
        [
            row("sherpa-onnx", "通过"),
            row("whisper-rs", "未检查：没有注册后端检查"),
        ]
    );
    assert_eq!(
        probe.artifacts,
        [
            row(
                "model.int8.onnx",
                format!("归档一致 · 应为 {ARCHIVE_SHA256}")
            ),
            row("tokens.txt", format!("一致 · 应为 {ARCHIVE_SHA256}")),
        ]
    );
    for name in ["backend", "artifact"] {
        for index in 0..2 {
            assert!(
                shown(cx, name, &format!("bundle-asr-{index}")),
                "{name} {index} is shown"
            );
        }
    }
}

#[gpui::test]
fn a_failed_download_probe_shows_the_localized_reason_and_retry_starts_the_download_again(
    cx: &mut TestAppContext,
) {
    let models = FakeModels::new(catalog(vec![native_diarization()]));
    let state = published(vec![failed_download()]);
    let (view, cx) = open_models(cx, models.clone(), &state, test_settings());

    let facts = facts(cx, &view, "native-diarization");
    let failure = facts.failure.clone().expect("the failure is shown");
    assert_eq!(
        (
            failure.headline.as_str(),
            failure.explanation.as_str(),
            failure.time.as_str()
        ),
        (
            "加载检查未通过",
            "diarization-native 无法加载这个模型",
            PROBED_AT
        )
    );
    assert_eq!(
        failure.detail,
        "后端 diarization-native 未通过检查：plda_psi.npy is missing"
    );
    assert_eq!(
        facts.probe.map(|probe| probe.summary),
        Some(format!("下载后校验未通过 · {PROBED_AT}"))
    );
    let card = rendered(cx, "model-card-native-diarization");
    let failure = rendered(cx, "model-failure-native-diarization");
    assert!(within(failure, card));
    // The failure stands for the failed probe; retry takes the download's
    // place in the header.
    assert!(!shown(cx, "probe", "native-diarization"));
    assert_in_header(cx, "retry", "native-diarization");
    // Retry replaces the download button; nothing is installed to re-probe.
    assert!(!shown(cx, "download", "native-diarization"));
    assert!(!shown(cx, "reprobe", "native-diarization"));
    // The backend's own words wait in the details.
    assert!(!shown(cx, "failure-detail", "native-diarization"));
    click_selector(cx, "model-details-toggle-native-diarization");
    assert!(within(
        rendered(cx, "model-failure-detail-native-diarization"),
        rendered(cx, "model-card-native-diarization")
    ));

    click_selector(cx, "model-retry-native-diarization");
    wait_for(cx, &view, |_| !models.starts().is_empty());
    assert_eq!(models.starts(), ["native-diarization"]);
}

#[gpui::test]
fn an_unreadable_probe_record_shows_its_path_and_reason(cx: &mut TestAppContext) {
    let path = PathBuf::from("/models/asr/bundle-asr.probe.json");
    let reason = "unsupported schema version: meetnote.model-probe.v0";
    let mut unreadable = availability(bundle_asr(), ModelInstallState::Installed);
    unreadable.record_error = Some(ProbeRecordError {
        path: path.clone(),
        reason: reason.into(),
    });
    let state = published(vec![unreadable]);
    let (view, cx) = open_models(
        cx,
        FakeModels::new(catalog(vec![bundle_asr()])),
        &state,
        test_settings(),
    );

    assert_eq!(
        facts(cx, &view, "bundle-asr").record_error.as_deref(),
        Some(
            "无法读取校验记录 /models/asr/bundle-asr.probe.json：unsupported schema version: \
             meetnote.model-probe.v0。下次校验会覆盖它。"
        )
    );
    assert!(within(
        rendered(cx, "model-record-error-bundle-asr"),
        rendered(cx, "model-card-bundle-asr")
    ));
    // A re-probe writes a new record over it.
    assert!(shown(cx, "reprobe", "bundle-asr"));
}

#[gpui::test]
fn a_reprobe_runs_off_the_ui_thread_and_shows_progress_until_its_state_is_published(
    cx: &mut TestAppContext,
) {
    let _alone = hold_a_worker_alone();
    let settings = test_settings();
    let models = FakeModels::new(catalog(vec![bundle_asr()]));
    let state = published(vec![availability(
        bundle_asr(),
        ModelInstallState::Installed,
    )]);
    let (view, cx) = open_models(cx, models.clone(), &state, settings.clone());
    assert_eq!(facts(cx, &view, "bundle-asr").probe, None);
    assert!(!shown(cx, "reprobe-progress", "bundle-asr"));

    models.gate.arm();
    click_selector(cx, "model-reprobe-bundle-asr");
    wait_for(cx, &view, |_| models.gate.counts().0 >= 1);
    assert!(shown(cx, "reprobe-progress", "bundle-asr"));
    assert!(
        !shown(cx, "reprobe", "bundle-asr"),
        "one re-probe at a time"
    );
    // The blocked call holds a worker, not the UI thread.
    assert_ui_responsive_while_held(cx, &view, &models.gate, &settings, "dia-during-reprobe");
    view.update(cx, |root, cx| root.navigate(Route::Models, cx));
    wait_for(cx, &view, |_| models.finish.lock().unwrap().is_some());
    assert_eq!(models.reprobes(), ["bundle-asr"]);
    assert!(shown(cx, "reprobe-progress", "bundle-asr"), "still probing");

    // The record is written; the page waits for the state that carries it.
    let mut record = passing_download_probe();
    record.kind = ModelOperation::Reprobe;
    let sender = models.finish.lock().unwrap().take().unwrap();
    sender.send(Ok(record.clone())).unwrap();
    wait_for(cx, &view, |root| {
        root.model_reprobes.get("bundle-asr")
            == Some(&ReprobeWait::Written {
                probed_at_ms: PROBED_AT_MS,
            })
    });
    assert!(shown(cx, "reprobe-progress", "bundle-asr"));
    assert_eq!(facts(cx, &view, "bundle-asr").probe, None);

    // The service publishes it, as it does when a re-probe ends.
    state.lock().unwrap().catalog[0].probe = Some(record);
    view.read_with(cx, |root, _| root.refresh_installation());
    wait_for(cx, &view, |root| root.model_reprobes.is_empty());
    assert!(!shown(cx, "reprobe-progress", "bundle-asr"));
    assert!(shown(cx, "reprobe", "bundle-asr"));
    assert_eq!(
        facts(cx, &view, "bundle-asr")
            .probe
            .map(|probe| probe.summary),
        Some(format!("重新校验通过 · {PROBED_AT}"))
    );
}

#[test]
fn a_written_reprobe_settles_once_the_published_state_carries_its_record() {
    let written = ReprobeWait::Written {
        probed_at_ms: PROBED_AT_MS,
    };
    let mut entry = availability(bundle_asr(), ModelInstallState::Installed);
    assert!(!ReprobeWait::Running.settled_by(Some(&entry)));
    assert!(!written.settled_by(Some(&entry)), "no record yet");
    let mut older = passing_download_probe();
    older.probed_at_ms = PROBED_AT_MS - 1;
    entry.probe = Some(older);
    assert!(!written.settled_by(Some(&entry)), "an earlier record");
    entry.probe = Some(passing_download_probe());
    assert!(written.settled_by(Some(&entry)));
    entry.probe = None;
    entry.record_error = Some(ProbeRecordError {
        path: "/models/asr/bundle-asr.probe.json".into(),
        reason: "not json".into(),
    });
    assert!(written.settled_by(Some(&entry)), "the record is unreadable");
    assert!(written.settled_by(None), "the model left the catalog");
    assert!(!ReprobeWait::Running.settled_by(None));
}

#[gpui::test]
fn an_installation_update_enables_the_new_model_without_resetting_the_open_menu(
    cx: &mut TestAppContext,
) {
    let entries =
        ["menu-a", "menu-b", "menu-c"].map(|id| entry(id, "asr", "menu", &["whisper-rs"]));
    let models = FakeModels::new(catalog(entries.to_vec()));
    let state = published(
        entries
            .into_iter()
            .enumerate()
            .map(|(index, model)| {
                availability(
                    model,
                    if index == 2 {
                        ModelInstallState::Missing
                    } else {
                        ModelInstallState::Installed
                    },
                )
            })
            .collect(),
    );
    let (view, cx) = open_models(cx, models, &state, test_settings());
    click_selector(cx, "nav-Settings");
    click_selector(cx, "settings-section-3");
    let select = view.read_with(cx, |root, _| {
        root.settings_selectors.as_ref().unwrap().asr_model.clone()
    });
    cx.update(|window, cx| {
        gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx);
    });
    cx.simulate_keystrokes("enter down");
    cx.run_until_parked();
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("menu-b".into())
    );
    let focus = cx.update(|window, cx| window.focused(cx));
    // Publish availability through the service, as after a download. This
    // checks UI projection; FakeModels does not perform a model download.
    state.lock().unwrap().catalog[2].install_state = ModelInstallState::Installed;
    view.read_with(cx, |root, _| root.refresh_installation());
    wait_for(cx, &view, |root| {
        root.installation.catalog[2].install_state == ModelInstallState::Installed
    });
    assert!(
        cx.debug_bounds("select-menu").is_some(),
        "installation must leave the active menu open"
    );
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("menu-b".into())
    );
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("menu-c".into()),
        "the newly installed model must become available in the existing menu"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("select-menu").is_none());
}

/// Every candidate backend accepts the files.
struct AcceptingBackend;

impl models::BackendProbe for AcceptingBackend {
    fn probe(&self, _: &str, _: &ModelRef) -> Result<(), String> {
        Ok(())
    }
}

/// Hold the real manager after it checks the fixture's bytes, before its
/// backend outcome is written and published. No model inference is performed.
struct HeldBackend(Gate);

impl models::BackendProbe for HeldBackend {
    fn probe(&self, _: &str, _: &ModelRef) -> Result<(), String> {
        self.0.pass();
        Ok(())
    }
}

#[gpui::test]
fn a_real_model_probe_keeps_an_open_settings_menu_and_its_keyboard_choice(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let root = temp_directory();
    let model = ModelCatalogEntry {
        source: "local/fixture.bin".into(),
        sha256: Some(MODEL_SHA256.into()),
        ..entry("menu-asr", "asr", "a-real", &["fake-asr"])
    };
    let file = root.join("asr/menu-asr/fixture.bin");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, MODEL_BYTES).unwrap();
    let gate = Gate::default();
    let manager = ModelManager::with_backend_probe(
        catalog(vec![model]),
        &root,
        Some(Arc::new(HeldBackend(gate.clone()))),
    );
    let settings = test_settings();
    let mut services = desktop_services(
        Arc::new(FakeAssistantUiService::default()),
        settings.clone(),
    );
    services.installation = InstallationService::new(manager.clone());
    services.model_manager = Arc::new(manager);
    let (view, cx) = open_on_models(cx, services);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    cx.run_until_parked();
    gate.arm();
    click_selector(cx, "model-reprobe-menu-asr");
    wait_for(cx, &view, |_| gate.counts().1 == 1);
    click_selector(cx, "nav-Settings");
    click_selector(cx, "settings-section-6");
    let select = view.read_with(cx, |root, _| {
        root.settings_selectors
            .as_ref()
            .unwrap()
            .compute_device
            .clone()
    });
    cx.update(|window, cx| {
        gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx);
    });
    cx.simulate_keystrokes("enter down");
    cx.run_until_parked();
    assert!(cx.debug_bounds("select-menu").is_some());
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("CPU".into())
    );
    let focus = cx.update(|window, cx| window.focused(cx));
    let revision = settings.revision().unwrap();
    gate.release();
    wait_for(cx, &view, |root| {
        root.model_reprobes.is_empty()
            && root.installation.catalog[0]
                .probe
                .as_ref()
                .is_some_and(|probe| probe.kind == ModelOperation::Reprobe)
    });
    assert!(
        cx.debug_bounds("select-menu").is_some(),
        "a real probe result must not dismiss the menu being browsed"
    );
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        focus,
        "background model results must leave focus in the menu"
    );
    assert_eq!(
        settings.revision().unwrap(),
        revision,
        "browsing has not saved a choice"
    );
    assert_eq!(
        select.read_with(cx, |select, _| select
            .highlighted_value()
            .map(str::to_owned)),
        Some("CPU".into())
    );
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, |_| {
        settings.snapshot().unwrap().compute_device == contracts::ComputeDevice::Cpu
    });
    assert!(cx.debug_bounds("select-menu").is_none());
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(select.read(cx), cx).is_focused(window)
    ));
    assert_eq!(std::fs::read(&file).unwrap(), MODEL_BYTES);
    std::fs::remove_dir_all(root).ok();
}

#[gpui::test]
fn a_real_reprobe_of_changed_files_reports_the_mismatch_and_keeps_the_model_installed(
    cx: &mut TestAppContext,
) {
    let root = temp_directory();
    let model = ModelCatalogEntry {
        source: "local/fixture.bin".into(),
        sha256: Some(MODEL_SHA256.into()),
        ..entry("real-asr", "asr", "a-real", &["fake-asr"])
    };
    let file = root.join("asr/real-asr/fixture.bin");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, MODEL_BYTES).unwrap();
    let manager = ModelManager::with_backend_probe(
        catalog(vec![model]),
        &root,
        Some(Arc::new(AcceptingBackend)),
    );
    let mut services =
        desktop_services(Arc::new(FakeAssistantUiService::default()), test_settings());
    services.installation = InstallationService::new(manager.clone());
    services.model_manager = Arc::new(manager);
    let (view, cx) = open_on_models(cx, services);
    let reprobed = |cx: &mut VisualTestContext, view: &View| {
        click_selector(cx, "model-reprobe-real-asr");
        wait_for(cx, view, |root| {
            root.model_reprobes.is_empty()
                && root.installation.catalog[0]
                    .probe
                    .as_ref()
                    .is_some_and(|probe| probe.kind == ModelOperation::Reprobe)
        });
    };

    reprobed(cx, &view);
    let passed = facts(cx, &view, "real-asr");
    let probe = passed.probe.expect("the written record is shown");
    assert!(
        probe.summary.starts_with("重新校验通过 · "),
        "{}",
        probe.summary
    );
    assert_eq!(probe.backends, [row("fake-asr", "通过")]);
    assert_eq!(passed.failure, None);

    std::fs::write(&file, CHANGED_BYTES).unwrap();
    let before = view.read_with(cx, |root, _| {
        root.installation.catalog[0]
            .probe
            .clone()
            .unwrap()
            .probed_at_ms
    });
    click_selector(cx, "model-reprobe-real-asr");
    wait_for(cx, &view, |root| {
        root.model_reprobes.is_empty()
            && root.installation.catalog[0].failure.is_some()
            && root.installation.catalog[0]
                .probe
                .as_ref()
                .is_some_and(|probe| probe.probed_at_ms >= before)
    });
    let failed = facts(cx, &view, "real-asr");
    let failure = failed.failure.expect("the failure is shown");
    assert_eq!(
        (failure.headline.as_str(), failure.explanation.as_str()),
        ("校验未通过", "文件内容与目录记录不符")
    );
    assert_eq!(
        failure.detail,
        format!(
            "{} 的校验和不符：应为 731e82f72481…，实际为 13c12877ebfb…",
            file.display()
        )
    );
    assert_eq!(
        failed.probe.unwrap().artifacts,
        [row(
            "fixture.bin",
            format!("不一致，实际为 {CHANGED_SHA256} · 应为 {MODEL_SHA256}")
        )]
    );
    // The files stay; the model re-probes again instead of downloading.
    assert_eq!(
        view.read_with(cx, |root, _| root.installation.catalog[0].install_state),
        ModelInstallState::Installed
    );
    assert!(shown(cx, "failure", "real-asr") && shown(cx, "reprobe", "real-asr"));
    assert!(!shown(cx, "retry", "real-asr") && !shown(cx, "download", "real-asr"));
    std::fs::remove_dir_all(root).ok();
}

/// Model references are service fixtures here; the settings repository is a
/// real JSON file. These tests verify UI ordering, not native model loading.
fn model_selection_fixture<'a>(
    cx: &'a mut TestAppContext,
    initial_asr: Option<&str>,
) -> (
    View,
    &'a mut VisualTestContext,
    Arc<FakeModels>,
    settings::SettingsService,
) {
    let entries = [
        ("choice-a", "asr", "choice-asr"),
        ("choice-b", "asr", "choice-asr"),
        ("choice-failed", "asr", "choice-asr"),
        ("choice-old-error", "asr", "choice-asr"),
        ("choice-dia-base", "diarization", "choice-dia"),
        ("choice-dia-new", "diarization", "choice-dia"),
    ]
    .map(|(id, task, backend)| entry(id, task, "choices", &[backend]));
    let models = FakeModels::new(catalog(entries.to_vec()));
    for model in &entries {
        if model.id.contains("failed") || model.id.contains("error") {
            continue;
        }
        models.references.lock().unwrap().insert(
            model.id.clone(),
            ModelRef {
                backend_id: model.candidate_backends[0].clone(),
                model_id: model.id.clone(),
                revision: model.revision.clone(),
                artifact_path: format!("fixture/{}.bin", model.id),
                artifacts: Vec::new(),
            },
        );
    }
    let settings = test_settings();
    let mut value = settings.snapshot().unwrap();
    value.asr_backend_id = Some("choice-asr".into());
    value.asr_model = initial_asr.map(|id| models.references.lock().unwrap()[id].clone());
    value.diarization_backend_id = Some("choice-dia".into());
    value.diarization_model = Some(models.references.lock().unwrap()["choice-dia-base"].clone());
    settings.update(value).unwrap();
    let state = published(
        entries
            .into_iter()
            .map(|model| availability(model, ModelInstallState::Installed))
            .collect(),
    );
    let (view, cx) = open_models(cx, models.clone(), &state, settings.clone());
    view.update(cx, |root, cx| {
        let mut diarization = application::diarization_descriptor("choice-dia", &[]);
        diarization.configurable_diarization_stages =
            vec![contracts::DiarizationStageKind::Clustering];
        diarization.diarization_stage_options = ["cluster-default", "cluster-new"]
            .map(|id| contracts::DiarizationStageOption {
                stage: contracts::DiarizationStageKind::Clustering,
                id: id.into(),
                model_id: None,
                revision: Some("fixture-v1".into()),
                artifact_files: Vec::new(),
            })
            .to_vec();
        root.inventory = application::BackendInventory::new(
            vec![
                application::asr_descriptor("choice-asr", &[]),
                application::asr_descriptor("next-asr", &[]),
            ],
            vec![diarization],
        );
        root.settings_selectors = None;
        root.navigate(Route::Settings, cx);
    });
    cx.run_until_parked();
    click_selector(cx, "settings-section-3");
    (view, cx, models, settings)
}

fn choose_setting_value(
    cx: &mut VisualTestContext,
    select: &gpui::Entity<crate::controls::Dropdown>,
    value: &str,
) {
    cx.update(|window, cx| gpui::Focusable::focus_handle(select.read(cx), cx).focus(window, cx));
    cx.simulate_keystrokes("enter");
    for step in 0..12 {
        if select.read_with(cx, |select, _| select.highlighted_value() == Some(value)) {
            break;
        }
        assert!(step < 11, "{value} is an enabled option");
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
}

fn finish_model_selection(cx: &mut VisualTestContext, view: &View, task: gpui::Task<()>) {
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = finished.clone();
    cx.spawn(move |_| async move {
        task.await;
        done.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .detach();
    wait_for(cx, view, |_| {
        finished.load(std::sync::atomic::Ordering::SeqCst)
    });
}

fn settle_model_settings(
    cx: &mut VisualTestContext,
    view: &View,
    settings: &settings::SettingsService,
) {
    wait_for(cx, view, |root| {
        let saved = settings.snapshot().unwrap();
        saved.asr_backend_id == root.settings_draft.asr_backend_id
            && saved.asr_model == root.settings_draft.asr_model
            && saved.diarization_backend_id == root.settings_draft.diarization_backend_id
            && saved.diarization_model == root.settings_draft.diarization_model
            && saved.diarization_stages == root.settings_draft.diarization_stages
    });
}

#[derive(Clone, Copy)]
enum LaterModelChoice {
    Model,
    Backend,
    SameBackend,
    FailedModel,
    OtherTask,
}

#[gpui::test]
fn a_model_selection_keeps_a_newer_model_after_an_old_success(cx: &mut TestAppContext) {
    old_model_selection(cx, LaterModelChoice::Model, false);
}
#[gpui::test]
fn a_model_selection_keeps_a_newer_notice_after_an_old_failure(cx: &mut TestAppContext) {
    old_model_selection(cx, LaterModelChoice::Model, true);
}
#[gpui::test]
fn a_model_selection_keeps_a_later_backend_choice(cx: &mut TestAppContext) {
    old_model_selection(cx, LaterModelChoice::Backend, false);
}
#[gpui::test]
fn a_model_selection_keeps_a_repeated_backend_choice(cx: &mut TestAppContext) {
    old_model_selection(cx, LaterModelChoice::SameBackend, false);
}
#[gpui::test]
fn a_model_selection_does_not_fall_back_to_an_older_success_after_a_new_failure(
    cx: &mut TestAppContext,
) {
    old_model_selection(cx, LaterModelChoice::FailedModel, false);
}
#[gpui::test]
fn a_model_selection_keeps_the_other_tasks_pending_choice(cx: &mut TestAppContext) {
    old_model_selection(cx, LaterModelChoice::OtherTask, false);
}

fn old_model_selection(cx: &mut TestAppContext, later: LaterModelChoice, old_error: bool) {
    let _alone = hold_a_worker_alone();
    let (view, cx, models, settings) = model_selection_fixture(cx, None);
    let id = if old_error {
        "choice-old-error"
    } else {
        "choice-a"
    };
    *models.held_model.lock().unwrap() = Some(id.into());
    models.gate.arm();
    let old = view.update(cx, |root, cx| {
        root.use_model(id.into(), "asr".into(), "choice-asr".into(), cx)
    });
    wait_for(cx, &view, |_| models.gate.counts() == (1, 1));
    let (select, value) = view.read_with(cx, |root, _| {
        let selectors = root.settings_selectors.as_ref().unwrap();
        match later {
            LaterModelChoice::Model => (selectors.asr_model.clone(), "choice-b"),
            LaterModelChoice::Backend => (selectors.asr_backend.clone(), "next-asr"),
            LaterModelChoice::SameBackend => (selectors.asr_backend.clone(), "choice-asr"),
            LaterModelChoice::FailedModel => (selectors.asr_model.clone(), "choice-failed"),
            LaterModelChoice::OtherTask => (selectors.diarization_model.clone(), "choice-dia-new"),
        }
    });
    if matches!(later, LaterModelChoice::OtherTask) {
        click_selector(cx, "settings-section-4");
    }
    choose_setting_value(cx, &select, value);
    let failed = matches!(later, LaterModelChoice::FailedModel);
    let expected_notice = if failed {
        crate::i18n::message(
            crate::i18n::Locale::ZhCn,
            "model_not_ready",
            &ModelManagerError::ModelNotInstalled(value.into()).to_string(),
        )
    } else {
        crate::i18n::text(crate::i18n::Locale::ZhCn, "settings_saved").into()
    };
    wait_for(cx, &view, |root| root.ui.notice == expected_notice);
    settle_model_settings(cx, &view, &settings);
    let latest = settings.snapshot().unwrap();
    models.gate.release();
    finish_model_selection(cx, &view, old);
    settle_model_settings(cx, &view, &settings);
    let saved = settings.snapshot().unwrap();
    if matches!(later, LaterModelChoice::OtherTask) {
        assert_eq!(
            saved.asr_model.unwrap().model_id,
            "choice-a",
            "ASR selection remains independent"
        );
        assert_eq!(saved.diarization_model.unwrap().model_id, "choice-dia-new");
    } else {
        assert_eq!(
            saved.asr_backend_id, latest.asr_backend_id,
            "a late model reference cannot undo the latest backend choice"
        );
        assert_eq!(
            saved.asr_model, latest.asr_model,
            "a late reference cannot replace the latest model choice"
        );
        assert_eq!(
            view.read_with(cx, |root, _| root.ui.notice.clone()),
            expected_notice,
            "an obsolete result cannot replace the latest outcome"
        );
    }
}

#[gpui::test]
fn a_model_selection_failure_restores_the_model_that_is_still_configured(cx: &mut TestAppContext) {
    let (view, cx, _, settings) = model_selection_fixture(cx, Some("choice-b"));
    let select = view.read_with(cx, |root, _| {
        root.settings_selectors.as_ref().unwrap().asr_model.clone()
    });
    choose_setting_value(cx, &select, "choice-failed");
    let error = crate::i18n::message(
        crate::i18n::Locale::ZhCn,
        "model_not_ready",
        &ModelManagerError::ModelNotInstalled("choice-failed".into()).to_string(),
    );
    wait_for(cx, &view, |root| root.ui.notice == error);
    assert_eq!(
        settings.snapshot().unwrap().asr_model.unwrap().model_id,
        "choice-b"
    );
    assert_eq!(
        select.read_with(cx, |select, _| select.selected_value().map(str::to_owned)),
        Some("choice-b".into()),
        "the dropdown must show the model still used for processing"
    );
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(select.read(cx), cx).is_focused(window)
    ));
}

#[gpui::test]
fn a_model_selection_keeps_a_later_clustering_choice(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let (view, cx, models, settings) = model_selection_fixture(cx, None);
    *models.held_model.lock().unwrap() = Some("choice-dia-new".into());
    models.gate.arm();
    let old = view.update(cx, |root, cx| {
        root.use_model(
            "choice-dia-new".into(),
            "diarization".into(),
            "choice-dia".into(),
            cx,
        )
    });
    wait_for(cx, &view, |_| models.gate.counts() == (1, 1));
    click_selector(cx, "settings-section-4");
    let select = view.read_with(cx, |root, _| {
        root.settings_selectors
            .as_ref()
            .unwrap()
            .diarization_clustering
            .clone()
    });
    choose_setting_value(cx, &select, "cluster-new");
    wait_for(cx, &view, |_| {
        settings
            .snapshot()
            .unwrap()
            .diarization_stages
            .clustering_algorithm
            .is_some()
    });
    models.gate.release();
    finish_model_selection(cx, &view, old);
    settle_model_settings(cx, &view, &settings);
    let saved = settings.snapshot().unwrap();
    assert_eq!(
        saved
            .diarization_stages
            .clustering_algorithm
            .as_ref()
            .map(|a| a.id.as_str()),
        Some("cluster-new"),
        "a pending bundle must not clear a later stage choice"
    );
    assert_eq!(saved.diarization_model.unwrap().model_id, "choice-dia-base");
}

#[gpui::test]
fn a_model_selection_refreshes_the_stages_cleared_by_a_new_bundle(cx: &mut TestAppContext) {
    let (view, cx, _, settings) = model_selection_fixture(cx, None);
    click_selector(cx, "settings-section-4");
    let clustering = view.read_with(cx, |root, _| {
        root.settings_selectors
            .as_ref()
            .unwrap()
            .diarization_clustering
            .clone()
    });
    choose_setting_value(cx, &clustering, "cluster-new");
    wait_for(cx, &view, |_| {
        settings
            .snapshot()
            .unwrap()
            .diarization_stages
            .clustering_algorithm
            .is_some()
    });
    let model = view.read_with(cx, |root, _| {
        root.settings_selectors
            .as_ref()
            .unwrap()
            .diarization_model
            .clone()
    });
    choose_setting_value(cx, &model, "choice-dia-new");
    wait_for(cx, &view, |root| {
        let saved = settings.snapshot().unwrap();
        saved
            .diarization_model
            .as_ref()
            .is_some_and(|m| m.model_id == "choice-dia-new")
            && saved.diarization_stages.clustering_algorithm.is_none()
            && root
                .settings_draft
                .diarization_stages
                .clustering_algorithm
                .is_none()
    });
    assert_eq!(
        clustering.read_with(cx, |select, _| select.selected_value().map(str::to_owned)),
        Some("cluster-default".into()),
        "the stage control must show the default after its explicit override is cleared"
    );
}

/// Open a real document package while keeping the fixture's published model
/// facts and reference service. No audio inference is performed here.
fn model_setup_document(
    cx: &mut VisualTestContext,
    view: &View,
) -> crate::document_tests::DocumentsRoot {
    let documents = crate::document_tests::DocumentsRoot::new();
    let audio = documents.path().join("setup/recording.wav");
    documents.put(
        domain::DocumentState::new(
            domain::ProjectId::new("setup"),
            "Setup recording",
            audio.display().to_string(),
        ),
        "# Keep this document\n",
    );
    std::fs::write(&audio, b"fixture recording").unwrap();
    let workspace = application::WorkspaceService::new(
        application::ProcessingService::new(
            Arc::new(asr_whisper_rs::WhisperRsBackend::default()),
            Arc::new(diarization_sherpa_onnx::SherpaOnnxDiarizationBackend::default()),
        ),
        application::DocumentService::new(Arc::new(documents.repository())),
    );
    view.update(cx, |root, cx| {
        root.services.workspace = workspace;
        root.reload_projects(cx);
        root.navigate(Route::Meetings, cx);
    });
    wait_for(cx, view, |root| {
        root.ui
            .selected_project
            .as_ref()
            .is_some_and(|id| id == &domain::ProjectId::new("setup"))
            && !root.audio.pending()
    });
    documents
}

#[gpui::test]
fn model_setup_opens_the_installed_transcription_choice_and_keeps_the_document(
    cx: &mut TestAppContext,
) {
    model_setup_opens_installed_choice(cx, false, false);
}

#[gpui::test]
fn model_setup_opens_the_installed_diarization_choice_from_the_keyboard(cx: &mut TestAppContext) {
    model_setup_opens_installed_choice(cx, true, true);
}

fn model_setup_opens_installed_choice(cx: &mut TestAppContext, diarization: bool, keyboard: bool) {
    use crate::document_tests::{rendered, within};
    let (view, cx, _, settings) = model_selection_fixture(cx, diarization.then_some("choice-b"));
    if diarization {
        view.update(cx, |root, cx| {
            root.select_diarization_backend("next-dia".into(), cx)
        });
        wait_for(cx, &view, |_| {
            settings
                .snapshot()
                .unwrap()
                .diarization_backend_id
                .as_deref()
                == Some("next-dia")
        });
        view.update(cx, |root, cx| {
            root.select_diarization_backend("choice-dia".into(), cx)
        });
        wait_for(cx, &view, |_| {
            settings
                .snapshot()
                .unwrap()
                .diarization_backend_id
                .as_deref()
                == Some("choice-dia")
        });
        settle_model_settings(cx, &view, &settings);
    }
    let documents = model_setup_document(cx, &view);
    let before = documents.package_bytes("setup");
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    if diarization {
        view.update(cx, |root, cx| {
            root.set_locale(crate::i18n::Locale::EnUs, cx)
        });
        wait_for(cx, &view, |root| {
            root.ui.notice == crate::i18n::text(crate::i18n::Locale::EnUs, "language_changed")
        });
    }
    click_selector(cx, "open-project-detail");
    let action = if cx.debug_bounds("go-to-model-settings").is_some() {
        "go-to-model-settings"
    } else {
        "go-to-models"
    };
    assert!(within(
        rendered(cx, action),
        rendered(cx, "meeting-content")
    ));
    if keyboard {
        let bounds = rendered(cx, action);
        cx.update(|window, cx| {
            view.read(cx).window_focus.clone().focus(window, cx);
        });
        for step in 0..24 {
            cx.simulate_keystrokes("tab");
            if cx
                .debug_bounds("focus-ring")
                .is_some_and(|ring| within(ring, bounds))
            {
                break;
            }
            assert!(step < 23, "the setup action is keyboard reachable");
        }
        let keystroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    } else {
        click_selector(cx, action);
    }
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.route),
        Route::Settings,
        "an installed choice belongs in Settings, where it can be selected"
    );
    let section = if diarization { 4 } else { 3 };
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.settings_section),
        section
    );
    let label_key = if diarization {
        "diarization_model"
    } else {
        "asr_model"
    };
    let locale = if diarization {
        crate::i18n::Locale::EnUs
    } else {
        crate::i18n::Locale::ZhCn
    };
    let label = crate::document_tests::leak(format!(
        "setting-label-{}",
        crate::i18n::text(locale, label_key)
    ));
    assert!(
        within(rendered(cx, label), rendered(cx, "settings-scroll")),
        "the destination reveals the actual model control"
    );
    let model_id = if diarization {
        "choice-dia-new"
    } else {
        "choice-b"
    };
    let model = view.read_with(cx, |root, _| {
        let selectors = root.settings_selectors.as_ref().unwrap();
        if diarization {
            selectors.diarization_model.clone()
        } else {
            selectors.asr_model.clone()
        }
    });
    choose_setting_value(cx, &model, model_id);
    wait_for(cx, &view, |_| {
        let saved = settings.snapshot().unwrap();
        let model = if diarization {
            saved.diarization_model
        } else {
            saved.asr_model
        };
        model.is_some_and(|model| model.model_id == model_id)
    });
    settle_model_settings(cx, &view, &settings);
    click_selector(cx, "nav-Meetings");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.selected_project.clone()),
        Some(domain::ProjectId::new("setup"))
    );
    assert!(
        cx.debug_bounds("models-required").is_none(),
        "choosing the model clears the prerequisite guidance"
    );
    assert!(!view.read_with(cx, |root, _| root.ui.processing));
    assert_eq!(documents.package_bytes("setup"), before);
}

#[gpui::test]
fn model_setup_keeps_the_download_destination_when_no_compatible_model_is_installed(
    cx: &mut TestAppContext,
) {
    model_setup_needs_a_download(cx, false);
}

#[gpui::test]
fn model_setup_does_not_offer_an_installed_model_for_another_backend(cx: &mut TestAppContext) {
    model_setup_needs_a_download(cx, true);
}

fn model_setup_needs_a_download(cx: &mut TestAppContext, incompatible: bool) {
    let (view, cx, _, _) = model_selection_fixture(cx, None);
    let mut state = view.read_with(cx, |root, _| root.installation.clone());
    for entry in state
        .catalog
        .iter_mut()
        .filter(|entry| entry.model.task == "asr")
    {
        if incompatible {
            entry.model.candidate_backends = vec!["another-backend".into()];
        } else {
            entry.install_state = ModelInstallState::Missing;
        }
    }
    // Vary only the published installation facts; this case checks the
    // destination offered by the UI, not a filesystem probe.
    view.update(cx, |root, cx| {
        root.installation = state;
        cx.notify();
    });
    let documents = model_setup_document(cx, &view);
    let before = documents.package_bytes("setup");
    assert!(cx.debug_bounds("go-to-model-settings").is_none());
    click_selector(cx, "go-to-models");
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Models);
    assert_eq!(documents.package_bytes("setup"), before);
}
