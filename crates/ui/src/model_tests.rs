//! L2: the Models route's diagnosis of each catalog model — its name, status
//! and one action in the header row; task, size, VRAM, backends, license,
//! source and checksum; the latest probe record; a failed download or probe
//! with its localized reason and retry; an unreadable probe record — and the
//! re-probe of an installed model.
//!
//! Most tests render installation states they build, as
//! `InstallationService` would publish them, through a probe that returns
//! the state the test holds, with a fake `ModelService` that records the
//! calls the page makes. The last test re-probes through a real
//! `ModelManager` on files in a temporary model root, so the record it shows
//! is the one the manager wrote and the service published.

use crate::controller::{ModelService, ReprobeWait};
use crate::document_tests::{leak, rendered, within, View};
use crate::pages::models::{model_facts, FactRow, ModelFacts};
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
/// `gate` while it is armed, then returns a task the test finishes.
#[derive(Default)]
struct FakeModels {
    catalog: Option<ModelCatalog>,
    starts: Mutex<Vec<String>>,
    reprobes: Mutex<Vec<String>>,
    gate: Gate,
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
}

impl ModelService for FakeModels {
    fn catalog(&self) -> &ModelCatalog {
        self.catalog.as_ref().expect("the fake has a catalog")
    }

    fn list(&self) -> Result<Vec<DownloadProgress>, ModelManagerError> {
        Ok(Vec::new())
    }

    fn model_ref(&self, model_id: &str, _: &str) -> Result<ModelRef, ModelManagerError> {
        Err(ModelManagerError::ModelNotInstalled(model_id.into()))
    }

    fn start(&self, model_id: &str) -> Result<DownloadTask, ModelManagerError> {
        self.starts.lock().unwrap().push(model_id.into());
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

    fn pause(&self, _: DownloadId) -> Result<(), ModelManagerError> {
        Ok(())
    }

    fn resume(&self, _: DownloadId) -> Result<(), ModelManagerError> {
        Ok(())
    }

    fn cancel(&self, _: DownloadId) -> Result<(), ModelManagerError> {
        Ok(())
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
fn every_card_leads_with_its_name_status_and_action_over_one_quiet_line_of_facts(
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
            "checksum",
            "details-toggle",
        ] {
            let fact = rendered(cx, selector(name, &model.id));
            assert!(within(fact, card), "{name} of {} is on its card", model.id);
        }
        let facts = facts(cx, &view, &model.id);
        assert_eq!(facts.license, model.license);
        assert_eq!(facts.source, model.source, "the full source, faded");
        assert!(
            !facts.summary.contains(':') && !facts.summary.contains('：'),
            "facts read without labels: {}",
            facts.summary
        );
    }
    // The long source fades inside the card instead of widening it.
    let card = rendered(cx, "model-card-bundle-asr");
    assert!(within(rendered(cx, "model-source-bundle-asr"), card));

    let bundle = facts(cx, &view, "bundle-asr");
    assert_eq!(
        bundle.summary,
        "语音识别 · 78 MiB · 显存约 1.0 GiB · sherpa-onnx, whisper-rs"
    );
    assert_eq!(bundle.unavailable, None, "a ready model downloads");
    assert_eq!(bundle.checksum, "da92b3db5218…", "one archive checksum");
    let native = facts(cx, &view, "native-diarization");
    assert_eq!(
        native.summary,
        "说话人分离 · 78 MiB · 显存约 1.0 GiB · diarization-native"
    );
    assert_eq!(native.checksum, "038b971741ed… 等 3 个");
    let candidate = facts(cx, &view, "candidate-asr");
    assert_eq!(
        candidate.summary, "语音识别 · 显存约 1.0 GiB · sherpa-onnx",
        "an unknown size is left out"
    );
    assert_eq!(
        candidate.unavailable.as_deref(),
        Some("来源尚未验证，暂不可下载")
    );
    assert_eq!(candidate.checksum, "未声明");
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

    // The full checksums are one expansion away, collapsed at first.
    assert!(!shown(cx, "declared", "native-diarization-0"));
    click_selector(cx, "model-details-toggle-native-diarization");
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
        !shown(cx, "declared", "native-diarization-0"),
        "collapsed again"
    );
    assert_eq!(candidate.declared, [row("model.bin", "未声明")]);
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
    assert_eq!(
        facts.failure.as_deref(),
        Some(
            "下载失败（2026-08-29 10:40 UTC）：后端 diarization-native 未通过检查：\
             plda_psi.npy is missing"
        )
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

/// Every candidate backend accepts the files.
struct AcceptingBackend;

impl models::BackendProbe for AcceptingBackend {
    fn probe(&self, _: &str, _: &ModelRef) -> Result<(), String> {
        Ok(())
    }
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
    assert!(failure.starts_with("重新校验失败（"), "{failure}");
    assert!(
        failure.ends_with(&format!(
            "）：{} 的校验和不符：应为 731e82f72481…，实际为 13c12877ebfb…",
            file.display()
        )),
        "{failure}"
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
