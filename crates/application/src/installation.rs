//! Model installation facts the UI shows: catalog install states, whether the
//! configured default models exist, and which diarization stage files are
//! present. Probing reads the file system, so the service runs it on its own
//! worker and publishes the result.

use contracts::{DiarizationStageOption, ModelRef};
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::future::ready;
use futures::{FutureExt, StreamExt};
use models::manager::{DownloadEvent, ModelManager, ReprobeFinished};
use models::ModelAvailability;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// What an installation probe looks at.
#[derive(Debug, Clone)]
pub struct InstallationQuery {
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    /// The selected diarization model's directory.
    pub stage_root: Option<String>,
    /// Every artifact file a diarization stage option may need.
    pub stage_files: BTreeSet<String>,
}

/// Installation facts for one `InstallationQuery`; rendering only reads this
/// snapshot.
#[derive(Debug, Clone, Default)]
pub struct InstallationState {
    pub catalog: Vec<ModelAvailability>,
    pub default_asr_ready: bool,
    pub default_diarization_ready: bool,
    /// The diarization model directory `stage_files` were probed in.
    pub stage_root: Option<String>,
    /// Diarization stage artifact files present in `stage_root`.
    pub stage_files: BTreeSet<String>,
}

impl InstallationState {
    /// The catalog entries for `task` that support `backend_id`, as
    /// `ModelManager::browse` filters them.
    pub fn models<'a>(
        &'a self,
        task: &'a str,
        backend_id: Option<&'a str>,
    ) -> impl Iterator<Item = &'a ModelAvailability> + 'a {
        self.catalog.iter().filter(move |entry| {
            entry.model.task == task
                && backend_id.is_none_or(|backend| entry.model.supports_backend(backend))
        })
    }

    pub fn stage_installed(&self, model: &ModelRef, option: &DiarizationStageOption) -> bool {
        self.stage_root.as_deref() == Some(model.artifact_path.as_str())
            && option
                .artifact_files
                .iter()
                .all(|file| self.stage_files.contains(file))
    }
}

/// Equal when every fact the pages show is equal.
impl PartialEq for InstallationState {
    fn eq(&self, other: &Self) -> bool {
        self.default_asr_ready == other.default_asr_ready
            && self.default_diarization_ready == other.default_diarization_ready
            && self.stage_root == other.stage_root
            && self.stage_files == other.stage_files
            && self.catalog.len() == other.catalog.len()
            && self
                .catalog
                .iter()
                .zip(&other.catalog)
                .all(|(left, right)| {
                    left.model.id == right.model.id
                        && left.install_state == right.install_state
                        && left.probe == right.probe
                        && left.failure == right.failure
                        && left.record_error == right.record_error
                })
    }
}

/// Produces `InstallationState`; tests replace it to observe or block probing.
pub type InstallationProbe = Arc<dyn Fn(&InstallationQuery) -> InstallationState + Send + Sync>;

type Subscribers = Arc<Mutex<Vec<UnboundedSender<InstallationState>>>>;

/// Publishes the installation state of the last requested query after every
/// `refresh` and whenever a `ModelManager` download or re-probe ends, so the
/// catalog entries carry the latest probe records and failures. One worker thread
/// probes; it stops when the last clone of the service is dropped.
#[derive(Clone)]
pub struct InstallationService {
    probe: InstallationProbe,
    queries: UnboundedSender<InstallationQuery>,
    subscribers: Subscribers,
}

impl InstallationService {
    pub fn new(model_manager: ModelManager) -> Self {
        let probe_manager = model_manager.clone();
        Self::with_probe(
            model_manager,
            Arc::new(move |query| probe_installation(&probe_manager, query)),
        )
    }

    /// Uses `probe` instead of the file-system probe; download endings still
    /// come from `model_manager`.
    pub fn with_probe(model_manager: ModelManager, probe: InstallationProbe) -> Self {
        let (queries, requested) = mpsc::unbounded();
        let subscribers = Subscribers::default();
        let downloads = model_manager.subscribe_all_async();
        let reprobes = model_manager.subscribe_reprobes_async();
        let worker_probe = probe.clone();
        let worker_subscribers = subscribers.clone();
        std::thread::Builder::new()
            .name("meetnote-installation".into())
            .spawn(move || {
                futures::executor::block_on(publish_installation(
                    worker_probe,
                    worker_subscribers,
                    requested,
                    downloads,
                    reprobes,
                ))
            })
            .expect("failed to spawn the installation worker");
        Self {
            probe,
            queries,
            subscribers,
        }
    }

    /// Probes on the calling thread, which must not be the UI thread.
    pub fn probe(&self, query: &InstallationQuery) -> InstallationState {
        (self.probe)(query)
    }

    /// Every state published from now on.
    pub fn subscribe(&self) -> UnboundedReceiver<InstallationState> {
        let (sender, receiver) = mpsc::unbounded();
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sender);
        receiver
    }

    /// Makes `query` the current query and has the worker publish its state;
    /// returns without probing.
    pub fn refresh(&self, query: InstallationQuery) {
        let _ = self.queries.unbounded_send(query);
    }
}

enum Wake {
    Refresh(InstallationQuery),
    ModelChanged,
    Stopped,
}

/// The worker loop: every refresh, download ending or re-probe ending probes
/// the current query once; wakes that are already queued are coalesced into
/// that probe.
async fn publish_installation(
    probe: InstallationProbe,
    subscribers: Subscribers,
    requested: UnboundedReceiver<InstallationQuery>,
    downloads: UnboundedReceiver<DownloadEvent>,
    reprobes: UnboundedReceiver<ReprobeFinished>,
) {
    let refreshes = requested
        .map(Wake::Refresh)
        .chain(futures::stream::once(ready(Wake::Stopped)));
    let download_endings = downloads.filter_map(|DownloadEvent::Updated(progress)| {
        ready(progress.state.is_terminal().then_some(Wake::ModelChanged))
    });
    let reprobe_endings = reprobes.map(|_| Wake::ModelChanged);
    let mut wakes = futures::stream::select(
        refreshes,
        futures::stream::select(download_endings, reprobe_endings),
    );
    let mut current = None;
    while let Some(wake) = wakes.next().await {
        let mut queued = Some(wake);
        while let Some(wake) = queued {
            match wake {
                Wake::Refresh(query) => current = Some(query),
                Wake::ModelChanged => {}
                Wake::Stopped => return,
            }
            queued = wakes.next().now_or_never().flatten();
        }
        if let Some(query) = &current {
            publish(&subscribers, probe(query));
        }
    }
}

fn publish(subscribers: &Subscribers, state: InstallationState) {
    subscribers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|sender| sender.unbounded_send(state.clone()).is_ok());
}

/// The file-system probe behind `InstallationState`.
fn probe_installation(
    model_manager: &ModelManager,
    query: &InstallationQuery,
) -> InstallationState {
    let stage_files = query
        .stage_root
        .as_deref()
        .map(|root| {
            query
                .stage_files
                .iter()
                .filter(|file| Path::new(root).join(file).is_file())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    InstallationState {
        catalog: model_manager.browse(None, None),
        default_asr_ready: Path::new(&query.asr_model.artifact_path).is_file(),
        default_diarization_ready: Path::new(&query.diarization_model.artifact_path).is_dir(),
        stage_root: query.stage_root.clone(),
        stage_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::TryRecvError;
    use models::{
        ModelCatalog, ModelCatalogEntry, ModelFailure, ModelFailureRecord, ModelInstallState,
        ModelOperation, ProbeOutcome, ProbeRecord,
    };
    use std::path::PathBuf;
    use std::sync::mpsc as std_mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn catalog_entry(id: &str, task: &str, backend: &str, source: String) -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: id.into(),
            task: task.into(),
            model_family: "fixture".into(),
            candidate_backends: vec![backend.into()],
            revision: Some("fixture-v1".into()),
            source_format: "bin".into(),
            source,
            size_bytes: Some(7),
            estimated_vram_bytes: None,
            sha256: None,
            license: "test".into(),
            requires_auth: false,
            status: "ready".into(),
            artifact_files: vec![],
            artifacts: vec![],
        }
    }

    fn catalog(models: Vec<ModelCatalogEntry>) -> ModelCatalog {
        ModelCatalog {
            schema_version: "meetnote.model-catalog.v1".into(),
            models,
        }
    }

    fn model_ref(backend_id: &str, artifact_path: &Path) -> ModelRef {
        ModelRef {
            backend_id: backend_id.into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: artifact_path.display().to_string(),
            artifacts: vec![],
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "meetnote-installation-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Serves `body` to a single request and returns its URL.
    fn serve_once(body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request);
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        });
        format!("http://{address}/fixture.bin")
    }

    fn next_within(states: &mut UnboundedReceiver<InstallationState>) -> InstallationState {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match states.try_recv() {
                Ok(state) => return state,
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("no installation state was published: {error:?}"),
            }
        }
    }

    #[test]
    fn probes_catalog_default_models_and_stage_files() {
        let root = temp_root("probe");
        let asr = root.join("asr.bin");
        let diarization = root.join("diarization");
        fs_write(&asr);
        fs_write(&diarization.join("segmentation.onnx"));
        let manager = ModelManager::new(
            catalog(vec![catalog_entry(
                "fixture",
                "asr",
                "fake",
                "local/fixture.bin".into(),
            )]),
            root.join("models"),
        );
        let query = InstallationQuery {
            asr_model: model_ref("fake", &asr),
            diarization_model: model_ref("fake-diarization", &diarization),
            stage_root: Some(diarization.display().to_string()),
            stage_files: [
                "segmentation.onnx".to_string(),
                "embedding.onnx".to_string(),
            ]
            .into_iter()
            .collect(),
        };

        let state = InstallationService::new(manager).probe(&query);

        assert!(state.default_asr_ready);
        assert!(state.default_diarization_ready);
        assert_eq!(state.catalog.len(), 1);
        assert_eq!(state.catalog[0].install_state, ModelInstallState::Missing);
        assert_eq!(
            state.stage_files,
            ["segmentation.onnx".to_string()].into_iter().collect()
        );
        let segmentation = DiarizationStageOption {
            stage: contracts::DiarizationStageKind::Segmentation,
            id: "segmentation".into(),
            model_id: None,
            revision: None,
            artifact_files: vec!["segmentation.onnx".into()],
        };
        let embedding = DiarizationStageOption {
            artifact_files: vec!["embedding.onnx".into()],
            ..segmentation.clone()
        };
        assert!(state.stage_installed(&query.diarization_model, &segmentation));
        assert!(!state.stage_installed(&query.diarization_model, &embedding));
        assert!(!state.stage_installed(&query.asr_model, &segmentation));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fs_write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fixture").unwrap();
    }

    #[test]
    fn filters_models_by_task_and_backend_and_compares_shown_facts() {
        let root = temp_root("filter");
        let manager = ModelManager::new(
            catalog(vec![
                catalog_entry("whisper", "asr", "whisper-rs", "local/a.bin".into()),
                catalog_entry("sherpa", "asr", "sherpa-onnx", "local/b.bin".into()),
                catalog_entry("native", "diarization", "native", "local/c.bin".into()),
            ]),
            &root,
        );
        let state = InstallationState {
            catalog: manager.browse(None, None),
            ..InstallationState::default()
        };

        let ids = |task, backend| {
            state
                .models(task, backend)
                .map(|entry| entry.model.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("asr", None), ["whisper", "sherpa"]);
        assert_eq!(ids("asr", Some("sherpa-onnx")), ["sherpa"]);
        assert_eq!(ids("diarization", Some("whisper-rs")), Vec::<String>::new());

        let mut moved = state.clone();
        moved.catalog[0].artifact_paths.clear();
        assert_eq!(moved, state, "artifact paths are not shown");
        moved.catalog[0].install_state = ModelInstallState::Installed;
        assert_ne!(moved, state);
        let mut failed = state.clone();
        failed.catalog[0].failure = Some(ModelFailureRecord {
            operation: ModelOperation::Download,
            failed_at_ms: 1,
            revision: None,
            reason: ModelFailure::Network {
                message: "offline".into(),
            },
        });
        assert_ne!(failed, state, "failures are shown");
        let mut probed = state.clone();
        probed.catalog[0].probe = Some(ProbeRecord {
            kind: ModelOperation::Reprobe,
            probed_at_ms: 1,
            revision: None,
            checksums: vec![],
            backends: vec![],
            outcome: ProbeOutcome::Passed,
        });
        assert_ne!(probed, state, "probe records are shown");
        assert_ne!(
            InstallationState {
                default_asr_ready: true,
                ..state.clone()
            },
            state
        );
    }

    #[test]
    fn publishes_after_refresh_and_after_a_download_completes() {
        let root = temp_root("download");
        let manager = ModelManager::new(
            catalog(vec![catalog_entry(
                "fixture",
                "asr",
                "fake",
                serve_once(b"fixture"),
            )]),
            &root,
        );
        let service = InstallationService::new(manager.clone());
        let mut states = service.subscribe();
        let query = InstallationQuery {
            asr_model: manager.configured_model_ref("asr", "fake", None).unwrap(),
            diarization_model: model_ref("fake-diarization", &root.join("missing")),
            stage_root: None,
            stage_files: BTreeSet::new(),
        };

        service.refresh(query);
        let before = next_within(&mut states);
        manager.start("fixture").unwrap();
        let after = next_within(&mut states);

        assert!(!before.default_asr_ready);
        assert_eq!(before.catalog[0].install_state, ModelInstallState::Missing);
        assert!(after.default_asr_ready);
        assert_eq!(after.catalog[0].install_state, ModelInstallState::Installed);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publishes_after_a_reprobe_finishes() {
        let root = temp_root("reprobe");
        let manager = ModelManager::new(
            catalog(vec![catalog_entry(
                "fixture",
                "asr",
                "fake",
                "local/fixture.bin".into(),
            )]),
            &root,
        );
        let service = InstallationService::new(manager.clone());
        let mut states = service.subscribe();
        let asr_model = manager.configured_model_ref("asr", "fake", None).unwrap();
        fs_write(Path::new(&asr_model.artifact_path));
        service.refresh(InstallationQuery {
            asr_model,
            diarization_model: model_ref("fake-diarization", &root.join("missing")),
            stage_root: None,
            stage_files: BTreeSet::new(),
        });
        let before = next_within(&mut states);

        manager.reprobe("fixture").unwrap();
        let after = next_within(&mut states);

        assert_eq!(before.catalog[0].probe, None);
        let record = after.catalog[0].probe.as_ref().expect("no probe record");
        assert_eq!(record.kind, ModelOperation::Reprobe);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_worker_stops_when_the_last_service_clone_is_dropped() {
        let root = temp_root("stop");
        let probe: InstallationProbe = Arc::new(|_| InstallationState::default());
        // Kept alive, as the UI keeps it, so its download stream stays open.
        let manager = ModelManager::new(catalog(vec![]), &root);
        let service = InstallationService::with_probe(manager.clone(), probe.clone());
        let clone = service.clone();

        drop(service);
        drop(clone);

        // The worker owns the only other handle to the probe.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&probe) > 1 {
            assert!(Instant::now() < deadline, "the worker is still running");
            thread::sleep(Duration::from_millis(1));
        }
        drop(manager);
    }

    #[test]
    fn refresh_returns_while_a_blocking_probe_runs_on_the_service_worker() {
        let root = temp_root("blocking");
        let (entered, probe_thread) = std_mpsc::channel();
        let (release, released) = std_mpsc::channel::<()>();
        let entered = Mutex::new(entered);
        let released = Mutex::new(released);
        let service = InstallationService::with_probe(
            ModelManager::new(catalog(vec![]), &root),
            Arc::new(move |_| {
                entered
                    .lock()
                    .unwrap()
                    .send(thread::current().id())
                    .unwrap();
                // Bounded, so a probe run on the caller cannot hang the test.
                let _ = released
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
                InstallationState {
                    default_asr_ready: true,
                    ..InstallationState::default()
                }
            }),
        );
        let mut states = service.subscribe();
        let query = InstallationQuery {
            asr_model: model_ref("fake", &root.join("asr.bin")),
            diarization_model: model_ref("fake-diarization", &root.join("diarization")),
            stage_root: None,
            stage_files: BTreeSet::new(),
        };

        let (returned, caller_thread) = std_mpsc::channel();
        let caller = service.clone();
        thread::spawn(move || {
            caller.refresh(query);
            let _ = returned.send(thread::current().id());
        });
        let caller_thread = caller_thread
            .recv_timeout(Duration::from_secs(1))
            .expect("refresh waited for the probe");
        let probe_thread = probe_thread
            .recv_timeout(Duration::from_secs(5))
            .expect("the probe did not start");
        release.send(()).unwrap();

        assert_ne!(probe_thread, caller_thread);
        assert!(next_within(&mut states).default_asr_ready);
    }
}
