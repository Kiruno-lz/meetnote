//! A relink keeps the repository it read, even if the workspace switches roots.

use application::{
    AsrBackend, AsrBackendProvider, DiarizationBackend, DiarizationBackendProvider, DocumentChange,
    DocumentRepository, DocumentService, DocumentSnapshot, DocumentsRoot, ProcessingService,
    WorkspaceService,
};
use contracts::{BackendDescriptor, BackendError, DocumentRecord};
use domain::{DocumentId, DocumentState};
use persistence::{DocumentPackage, FileDocumentRepository};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

struct UnusedProviders;

impl AsrBackendProvider for UnusedProviders {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }
    fn backend(&self, _: &str) -> Result<Arc<dyn AsrBackend>, BackendError> {
        panic!("relinking must not load an ASR backend")
    }
}

impl DiarizationBackendProvider for UnusedProviders {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }
    fn backend(&self, _: &str) -> Result<Arc<dyn DiarizationBackend>, BackendError> {
        panic!("relinking must not load a diarization backend")
    }
}

#[derive(Clone, Copy)]
enum PauseAfter {
    Snapshot,
    Read,
}

struct GatedRepository {
    inner: FileDocumentRepository,
    pause_after: PauseAfter,
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl GatedRepository {
    fn pause(&self) {
        let gate = self.gate.lock().unwrap().take();
        if let Some((entered, release)) = gate {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
    }
}

impl DocumentRepository for GatedRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        let result = self.inner.snapshot();
        if matches!(self.pause_after, PauseAfter::Snapshot) {
            self.pause();
        }
        result
    }

    fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
        self.inner.get(id)
    }

    fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        let result = self.inner.read(id);
        if matches!(self.pause_after, PauseAfter::Read) {
            self.pause();
        }
        result
    }

    fn watch(&self) -> Result<mpsc::Receiver<DocumentChange>, String> {
        self.inner.watch()
    }

    fn save(&self, state: DocumentState, markdown: Option<String>) -> Result<(), String> {
        self.inner.save(state, markdown)
    }
}

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn package(root: &Path, id: &str, audio: &Path) {
    let mut state = DocumentState::new(DocumentId::new(id), id, audio.to_str().unwrap());
    state.schema.revision = 3;
    if id == "child" {
        state.hierarchy.parent_id = Some(DocumentId::new("root"));
        state.hierarchy.depth = 1;
        state.source.audio_asset_id = Some(DocumentId::new("root"));
    }
    let directory = root.join(id);
    DocumentPackage::new(&directory, state, format!("# {id} user notes\n"))
        .unwrap()
        .save()
        .unwrap();
    fs::write(directory.join("attachment.bin"), b"ordinary attachment\0").unwrap();
}

fn package_bytes(root: &Path) -> Vec<Vec<u8>> {
    ["root", "child"]
        .into_iter()
        .flat_map(|id| {
            ["state.json", "document.md", "attachment.bin"]
                .map(|file| fs::read(root.join(id).join(file)).unwrap())
        })
        .collect()
}

fn relink_during_switch(pause_after: PauseAfter) {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "meetnote-relink-context-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    )));
    let old_root = fixture.0.join("old");
    let new_root = fixture.0.join("new");
    fs::create_dir_all(&fixture.0).unwrap();
    let old_audio = fixture.0.join("old.wav");
    let new_audio = fixture.0.join("new.wav");
    let replacement = fixture.0.join("replacement.wav");
    fs::write(&old_audio, b"old unprocessed audio").unwrap();
    fs::write(&new_audio, b"different unprocessed audio").unwrap();
    fs::write(&replacement, b"replacement unprocessed audio").unwrap();
    for id in ["root", "child"] {
        package(&old_root, id, &old_audio);
        package(&new_root, id, &new_audio);
    }
    let before_new = package_bytes(&new_root);
    let (entered, reached) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let repository = GatedRepository {
        inner: FileDocumentRepository::open(&old_root).unwrap(),
        pause_after,
        gate: Mutex::new(Some((entered, resume))),
    };
    let workspace = WorkspaceService::new(
        ProcessingService::new_with_providers(Arc::new(UnusedProviders), Arc::new(UnusedProviders)),
        DocumentService::new(Arc::new(repository)),
    )
    .with_documents_root(DocumentsRoot::new(
        &old_root,
        fixture.0.join("models"),
        |root| {
            FileDocumentRepository::open(root)
                .map(|repository| Arc::new(repository) as Arc<dyn DocumentRepository>)
                .map_err(|error| error.to_string())
        },
    ));
    let worker = workspace.clone();
    let path = replacement.to_str().unwrap().to_string();
    let relink = std::thread::spawn(move || worker.relink_audio(&DocumentId::new("root"), 3, path));
    // The real old-root snapshot/read has finished and released its file locks.
    reached.recv().unwrap();
    workspace.switch_documents_root(&new_root).unwrap();
    release.send(()).unwrap();
    let result = relink.join().unwrap().unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(workspace.documents_root(), Some(new_root.clone()));
    assert!(
        package_bytes(&new_root) == before_new,
        "the new workspace's same-ID packages and attachments must remain untouched"
    );
    for id in ["root", "child"] {
        let record = DocumentPackage::load(old_root.join(id)).unwrap();
        assert_eq!(record.state.audio_path(), replacement.to_str().unwrap());
        assert_eq!(record.state.revision(), 4);
        assert_eq!(record.markdown, format!("# {id} user notes\n"));
        assert_eq!(
            fs::read(old_root.join(id).join("attachment.bin")).unwrap(),
            b"ordinary attachment\0"
        );
    }
    assert_eq!(fs::read(old_audio).unwrap(), b"old unprocessed audio");
    assert_eq!(fs::read(new_audio).unwrap(), b"different unprocessed audio");
    assert_eq!(
        fs::read(replacement).unwrap(),
        b"replacement unprocessed audio"
    );
}

#[test]
fn relink_keeps_its_workspace_after_listing_the_audio_asset() {
    relink_during_switch(PauseAfter::Snapshot);
}

#[test]
fn relink_keeps_its_workspace_after_reading_an_asset_document() {
    relink_during_switch(PauseAfter::Read);
}
