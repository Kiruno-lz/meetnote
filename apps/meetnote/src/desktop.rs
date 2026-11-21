use crate::audio_services::{AudioBackends, model_catalog, processing_options};
use application::{
    DocumentRepository, DocumentService, DocumentsRoot, WorkspaceService, validate_documents_root,
};
use contracts::ModelRef;
use models::ModelManager;
use persistence::FileDocumentRepository;
use runtime::JobRuntime;
use settings::{AppSettings, JsonSettingsRepository, SettingsService};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Downloaded models; it can never hold the document packages.
const MODEL_DIRECTORY: &str = "models";

fn open_documents_repository(root: &Path) -> Result<Arc<dyn DocumentRepository>, String> {
    Ok(Arc::new(FileDocumentRepository::open(root)?))
}

struct Headless {
    audio: AudioBackends,
    documents_root: PathBuf,
    snapshot: AppSettings,
    workspace: WorkspaceService,
}

impl Headless {
    fn open() -> Result<Self, String> {
        let settings = SettingsService::open(Arc::new(JsonSettingsRepository::open(
            "data/settings/settings.json",
        )))
        .map_err(|error| format!("cannot load settings: {error}"))?;
        let snapshot = settings
            .snapshot()
            .map_err(|error| format!("cannot read settings: {error}"))?;
        let audio = AudioBackends::registered(Path::new(&snapshot.speaker_library_path));
        let documents_root = PathBuf::from(&snapshot.documents_root);
        validate_documents_root(
            &documents_root,
            Some(Path::new(&snapshot.speaker_library_path)),
            Path::new(MODEL_DIRECTORY),
        )
        .map_err(|error| format!("invalid documents root: {error}"))?;
        let repository = open_documents_repository(&documents_root)
            .map_err(|error| format!("cannot open document repository: {error}"))?;
        let documents = DocumentService::new(repository);
        let processing = audio.processing(Arc::new(JobRuntime::default()));
        let workspace =
            WorkspaceService::with_options(processing, documents, processing_options(&snapshot))
                .with_documents_root(DocumentsRoot::new(
                    &documents_root,
                    MODEL_DIRECTORY,
                    open_documents_repository,
                ));
        Ok(Self {
            audio,
            documents_root,
            snapshot,
            workspace,
        })
    }

    fn load_models(&self) -> (ModelManager, ModelRef, ModelRef) {
        let asr_backend = self
            .snapshot
            .asr_backend_id
            .as_deref()
            .unwrap_or("whisper-rs");
        let diarization_backend = self
            .snapshot
            .diarization_backend_id
            .as_deref()
            .unwrap_or("diarization-native");
        let catalog =
            model_catalog().unwrap_or_else(|error| panic!("failed to open model catalog: {error}"));
        let manager = self.audio.models(catalog, Path::new(MODEL_DIRECTORY));
        let asr = manager
            .configured_model_ref("asr", asr_backend, self.snapshot.asr_model.as_ref())
            .unwrap_or_else(|_| unconfigured_model(asr_backend));
        let diarization = manager
            .configured_model_ref(
                "diarization",
                diarization_backend,
                self.snapshot.diarization_model.as_ref(),
            )
            .unwrap_or_else(|_| unconfigured_model(diarization_backend));
        (manager, asr, diarization)
    }
}

/// Handles the desktop application's headless commands before a UI starts.
pub fn run_command_line() -> bool {
    let args: Vec<String> = env::args().collect();
    let launch_dir = prepare_data_directory(&args).unwrap_or_else(|error| {
        let message = format!("cannot open MeetNote's data directory: {error}");
        match args.get(1).map(String::as_str) {
            Some("process") => fail_process(message),
            _ => panic!("{message}"),
        }
    });
    match args.get(1).map(String::as_str) {
        Some("devices") => {
            if args.get(2).map(String::as_str) != Some("probe") {
                eprintln!("usage: meetnote devices probe [seconds]");
                std::process::exit(2);
            }
            let seconds = args
                .get(3)
                .and_then(|value| value.parse().ok())
                .unwrap_or(15);
            probe_devices(std::time::Duration::from_secs(seconds));
            true
        }
        Some("process") => {
            if args.len() != 3 {
                eprintln!("usage: meetnote process <audio>");
                std::process::exit(2);
            }
            let host = Headless::open().unwrap_or_else(|error| fail_process(error));
            let (_, asr, diarization) = host.load_models();
            let audio_path = if let Some(launch_dir) = launch_dir {
                launch_dir
                    .join(&args[2])
                    .into_os_string()
                    .into_string()
                    .unwrap_or_else(|_| fail_process("the audio path is not valid Unicode".into()))
            } else {
                args[2].clone()
            };
            let project = host
                .workspace
                .import_audio(audio_path)
                .unwrap_or_else(|error| fail_process(error));
            let result = futures::executor::block_on(host.workspace.process_project(
                project.id().clone(),
                asr,
                diarization,
            ))
            .unwrap_or_else(|error| fail_process(error.message));
            println!("processed document {}", result.id().as_str());
            println!(
                "document package: {}",
                host.documents_root.join(result.id().as_str()).display()
            );
            true
        }
        _ => false,
    }
}

/// Prints what MeetNote sees of recording devices: every USB device with
/// the DJI ones and their volumes marked, then each Bluetooth advertisement
/// for `listen`. Used to calibrate recognition against new hardware.
fn probe_devices(listen: std::time::Duration) {
    use devices::{BleEvent, ble::Sightings, ble::Verification};
    match devices::usb::usb_devices() {
        Ok(usb) => {
            println!("usb devices: {}", usb.len());
            for device in &usb {
                println!(
                    "  {:04x}:{:04x} {:?} {:?} serial={:?} bcd={:?} location={:?} disks={:?}",
                    device.vendor_id,
                    device.product_id,
                    device.vendor_name,
                    device.product_name,
                    device.serial,
                    device.bcd_device,
                    device.location_id,
                    device.bsd_names
                );
            }
            use devices::DeviceDiscovery;
            match devices::SystemDiscovery.attached() {
                Ok(attached) => {
                    for attached in attached {
                        println!(
                            "recognised: key={} identity={:?} volumes={:?}",
                            attached.key(),
                            attached.identity,
                            attached.volumes
                        );
                    }
                }
                Err(error) => println!("recognition: {error}"),
            }
        }
        Err(error) => println!("usb: {error}"),
    }
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let _scan = match devices::ble::start_scan(sender) {
        Ok(scan) => scan,
        Err(block) => {
            println!("bluetooth: {block}");
            return;
        }
    };
    println!("bluetooth: listening for {}s", listen.as_secs());
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let deadline = std::time::Instant::now() + listen;
    let mut sightings = Sightings::default();
    while std::time::Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            match event {
                BleEvent::State(state) => println!("bluetooth state: {state:?}"),
                BleEvent::Seen(advertisement) => {
                    println!(
                        "  {} rssi={} name={:?} mfr={} services={:?} data={:?}",
                        advertisement.peripheral,
                        advertisement.rssi,
                        advertisement.local_name,
                        advertisement
                            .manufacturer_data
                            .as_deref()
                            .map(hex)
                            .unwrap_or_default(),
                        advertisement.service_uuids,
                        advertisement
                            .service_data
                            .iter()
                            .map(|(uuid, data)| format!("{uuid}={}", hex(data)))
                            .collect::<Vec<_>>()
                    );
                    sightings.observe(advertisement);
                }
                BleEvent::Identifying(peripheral) => {
                    println!("identifying {peripheral} over LE GATT");
                    sightings.verify(&peripheral, Verification::Reading);
                }
                BleEvent::Identified {
                    peripheral,
                    outcome,
                } => {
                    println!("identified {peripheral}: {outcome:?}");
                    sightings.verify(
                        &peripheral,
                        match outcome {
                            Ok(device) => Verification::Verified(device),
                            Err(reason) => Verification::Rejected(reason),
                        },
                    );
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for candidate in sightings.candidates() {
        println!(
            "candidate: {:?} rssi={} verification={:?}",
            candidate.identity, candidate.rssi, candidate.verification
        );
    }
}

fn prepare_data_directory(args: &[String]) -> std::io::Result<Option<PathBuf>> {
    let requested = env::var_os("MEETNOTE_DATA_DIR").map(PathBuf::from);
    let directory = match requested {
        Some(directory) => directory,
        None if cfg!(target_os = "macos")
            && !args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--sample" | "--scene" | "--help")) =>
        {
            let executable = env::current_exe()?;
            let Some(macos) = executable
                .parent()
                .filter(|path| path.ends_with("Contents/MacOS"))
            else {
                return Ok(None);
            };
            if !macos
                .parent()
                .and_then(Path::parent)
                .and_then(Path::extension)
                .is_some_and(|extension| extension == "app")
            {
                return Ok(None);
            }
            let home = env::var_os("HOME")
                .ok_or_else(|| std::io::Error::other("the home directory is unavailable"))?;
            PathBuf::from(home).join("Library/Application Support/MeetNote")
        }
        None => return Ok(None),
    };
    let launch_directory = env::current_dir()?;
    std::fs::create_dir_all(&directory)?;
    env::set_current_dir(directory)?;
    Ok(Some(launch_directory))
}

fn unconfigured_model(backend_id: &str) -> ModelRef {
    ModelRef {
        backend_id: backend_id.into(),
        model_id: "unconfigured".into(),
        revision: None,
        artifact_path: String::new(),
        artifacts: Vec::new(),
    }
}

fn fail_process(error: String) -> ! {
    eprintln!("processing failed: {error}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ProcessingService;
    use asr::{AsrBackendManager, AsrBackendRegistry};
    use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};

    #[test]
    fn a_documents_root_switch_moves_the_file_repository_to_the_new_directory() {
        let base = std::env::temp_dir().join(format!(
            "meetnote-documents-root-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let old_root = base.join("documents");
        let new_root = base.join("elsewhere/documents");
        let processing = ProcessingService::new_with_providers(
            Arc::new(AsrBackendManager::new(AsrBackendRegistry::default())),
            Arc::new(DiarizationBackendManager::new(
                DiarizationBackendRegistry::default(),
            )),
        );
        let workspace = WorkspaceService::new(
            processing,
            DocumentService::new(open_documents_repository(&old_root).unwrap()),
        )
        .with_documents_root(DocumentsRoot::new(
            &old_root,
            base.join(MODEL_DIRECTORY),
            open_documents_repository,
        ));
        let first = workspace.import_audio("first.wav").unwrap();

        workspace.switch_documents_root(&new_root).unwrap();
        let second = workspace.import_audio("second.wav").unwrap();

        let state = |root: &Path, document: &domain::DocumentState| {
            root.join(document.id().as_str())
                .join(persistence::DOCUMENT_STATE_FILENAME)
                .is_file()
        };
        assert!(state(&old_root, &first));
        assert!(!state(&old_root, &second));
        assert!(state(&new_root, &second));
        let listed: Vec<_> = workspace
            .list_projects()
            .unwrap()
            .into_iter()
            .map(|document| document.id().clone())
            .collect();
        assert_eq!(listed, vec![second.id().clone()]);
        assert_eq!(
            workspace.switch_documents_root(base.join(MODEL_DIRECTORY)),
            Err(application::DocumentsRootError::Reserved {
                path: base.join(MODEL_DIRECTORY),
                location: application::ReservedLocation::ModelDirectory,
                reserved: base.join(MODEL_DIRECTORY),
            })
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}
