//! Recordings copied from a device become ordinary workspace documents: by
//! default the audio lives inside each document's package, and the device's
//! ledger keeps a second import from repeating the first.

use application::device_import::{DeviceImport, DeviceImportProgress, STAGING_DIRECTORY};
use application::DocumentService;
use devices::{DeviceIdentity, PairedDevice, RegistryStore};
use persistence::FileDocumentRepository;
use runtime::CancellationToken;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    volume: PathBuf,
    registry: RegistryStore,
    documents: DocumentService,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        let volume = directory.path().join("DJI MIC3");
        fs::create_dir_all(&root).unwrap();
        let registry = RegistryStore::new(directory.path().join("settings/devices.json"));
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        Self {
            root,
            volume,
            registry,
            documents,
            _directory: directory,
        }
    }

    fn record(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.volume.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn pair(&self, change: impl FnOnce(&mut PairedDevice)) -> PairedDevice {
        let mut device = PairedDevice::new(
            "sn:9SFXN8N024XU81".into(),
            DeviceIdentity {
                name: "DJI Mic 3 TX".into(),
                serial: Some("9SFXN8N024XU81".into()),
                ..Default::default()
            },
            1,
        );
        change(&mut device);
        self.registry
            .update(|registry| registry.pair(device.clone()))
            .unwrap();
        device
    }

    fn import(
        &self,
        device: &PairedDevice,
    ) -> (
        application::device_import::DeviceImportReport,
        Vec<DeviceImportProgress>,
    ) {
        let mut events = Vec::new();
        let report = DeviceImport {
            documents: &self.documents,
            workspace_root: &self.root,
            registry: &self.registry,
            device,
            volumes: std::slice::from_ref(&self.volume),
        }
        .run(&mut |event| events.push(event), &CancellationToken::new())
        .unwrap();
        (report, events)
    }
}

#[test]
fn default_import_keeps_audio_inside_each_package_and_resumes_from_the_ledger() {
    let fixture = Fixture::new();
    fixture.record("DJI_Audio/DJI_01_20251120_100000.WAV", b"first");
    fixture.record("DJI_Audio/DJI_02_20251120_110000.WAV", b"second");
    fixture.record("DJI_Audio/.DJI_hidden.WAV", b"hidden");
    // A partial copy left by an interrupted import is discarded first.
    let stale = fixture.root.join(STAGING_DIRECTORY).join("stale.part");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, b"partial").unwrap();
    let device = fixture.pair(|_| {});

    let (report, events) = fixture.import(&device);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.imported.len(), 2);
    assert_eq!(
        events.first(),
        Some(&DeviceImportProgress::Planned { total: 2 })
    );
    assert!(!stale.exists());
    for (document, bytes) in report.imported.iter().zip([b"first".as_slice(), b"second"]) {
        let audio = Path::new(document.audio_path());
        assert_eq!(
            audio.parent().unwrap(),
            fixture.root.join(document.id().as_str())
        );
        assert_eq!(fs::read(audio).unwrap(), bytes);
    }
    let snapshot = fixture.documents.snapshot().unwrap();
    assert_eq!(snapshot.documents.len(), 2);
    assert!(snapshot.issues.is_empty(), "{:?}", snapshot.issues);
    assert!(
        fixture
            .volume
            .join("DJI_Audio/DJI_01_20251120_100000.WAV")
            .exists(),
        "sources stay on the device by default"
    );

    let (again, _) = fixture.import(&device);
    assert!(
        again.imported.is_empty(),
        "the ledger skips imported recordings"
    );
    fixture.record("DJI_Audio/DJI_03_20251120_120000.WAV", b"third");
    let (third, _) = fixture.import(&device);
    assert_eq!(third.imported.len(), 1);
    let registry = fixture.registry.load().unwrap();
    assert_eq!(registry.ledger(&device.key).count(), 3);
    assert_eq!(
        registry
            .device(&device.key)
            .unwrap()
            .last_import
            .unwrap()
            .imported,
        1
    );
}

#[test]
fn custom_directory_keeps_both_same_named_recordings_and_can_delete_sources() {
    let fixture = Fixture::new();
    let target = fixture.root.parent().unwrap().join("Recordings");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("DJI_01.WAV"), b"older recording").unwrap();
    let source = fixture.record("DJI_01.WAV", b"new recording");
    let device = fixture.pair(|device| {
        device.import_dir = Some(target.clone());
        device.delete_after_import = true;
    });

    let (report, _) = fixture.import(&device);
    assert_eq!(report.imported.len(), 1, "{:?}", report.failed);
    assert_eq!(report.deleted, 1);
    assert!(!source.exists());
    let audio = PathBuf::from(report.imported[0].audio_path());
    assert_eq!(audio, target.join("DJI_01 (2).WAV"));
    assert_eq!(fs::read(&audio).unwrap(), b"new recording");
    assert_eq!(
        fs::read(target.join("DJI_01.WAV")).unwrap(),
        b"older recording"
    );
    assert!(fs::read_dir(&target)
        .unwrap()
        .all(|entry| entry.unwrap().path().extension().unwrap() != "part"));
}

#[test]
fn package_import_rejects_names_that_would_replace_package_files() {
    let fixture = Fixture::new();
    let staged = fixture.root.join(STAGING_DIRECTORY).join("x.part");
    fs::create_dir_all(staged.parent().unwrap()).unwrap();
    fs::write(&staged, b"x").unwrap();
    for name in ["state.json", "document.md", ".hidden.wav", "a/b.wav", ""] {
        assert!(
            fixture
                .documents
                .import_packaged_audio(&fixture.root, &staged, name)
                .is_err(),
            "{name}"
        );
    }
    assert!(fixture.documents.snapshot().unwrap().documents.is_empty());
}
