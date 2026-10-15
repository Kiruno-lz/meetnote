//! Processing admission holds one workspace from request construction until
//! its jobs are registered. Backends only probe; no inference is executed.

use application::{
    asr_descriptor, diarization_descriptor, AsrBackend, BackendFuture, DiarizationBackend,
    DocumentChange, DocumentRepository, DocumentService, DocumentSnapshot, DocumentsRoot,
    DocumentsRootError, ProcessingRequest, ProcessingService, WorkspaceService,
};
use contracts::{
    AsrRequest, AsrResult, BackendDescriptor, BackendError, DiarizationRequest, DiarizationResult,
    DocumentRecord, ModelRef,
};
use domain::{DocumentId, DocumentState};
use persistence::{DocumentPackage, FileDocumentRepository};
use runtime::JobContext;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

struct ProbeOnly;

impl AsrBackend for ProbeOnly {
    fn descriptor(&self) -> BackendDescriptor {
        asr_descriptor("probe-asr", &[])
    }
    fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
        Ok(())
    }
    fn transcribe(&self, _: AsrRequest, _: JobContext) -> BackendFuture<AsrResult> {
        panic!("the processing admission test must not execute inference")
    }
}

impl DiarizationBackend for ProbeOnly {
    fn descriptor(&self) -> BackendDescriptor {
        diarization_descriptor("probe-dia", &[])
    }
    fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
        Ok(())
    }
    fn diarize(&self, _: DiarizationRequest, _: JobContext) -> BackendFuture<DiarizationResult> {
        panic!("the processing admission test must not execute inference")
    }
}

struct GatedRepository {
    inner: FileDocumentRepository,
    reads: Arc<Mutex<Vec<String>>>,
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl DocumentRepository for GatedRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        self.inner.snapshot()
    }
    fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
        let result = self.inner.get(id);
        if let Ok(Some(state)) = &result {
            self.reads.lock().unwrap().push(state.audio_path().into());
        }
        let gate = self.gate.lock().unwrap().take();
        if let Some((entered, release)) = gate {
            entered.send(()).unwrap();
            // File IO has finished; this gate holds no repository file lock.
            release.recv().unwrap();
        }
        result
    }
    fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        self.inner.read(id)
    }
    fn save(&self, state: DocumentState, markdown: Option<String>) -> Result<(), String> {
        self.inner.save(state, markdown)
    }
    fn watch(&self) -> Result<mpsc::Receiver<DocumentChange>, String> {
        self.inner.watch()
    }
}

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn package(root: &Path, audio: &Path) {
    let state = DocumentState::new(
        DocumentId::new("meeting"),
        "Meeting",
        audio.to_str().unwrap(),
    );
    DocumentPackage::new(root.join("meeting"), state, "# User notes\n")
        .unwrap()
        .save()
        .unwrap();
    fs::write(root.join("meeting/attachment.bin"), b"user attachment").unwrap();
}

fn package_bytes(root: &Path) -> Vec<Vec<u8>> {
    ["state.json", "document.md", "attachment.bin"]
        .map(|name| fs::read(root.join("meeting").join(name)).unwrap())
        .into()
}

fn start_during_switch(explicit_request: bool) {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "meetnote-processing-start-{}-{}-{}",
        std::process::id(),
        explicit_request,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    let old_root = fixture.0.join("old");
    let new_root = fixture.0.join("new");
    let old_audio = fixture.0.join("old.wav");
    let new_audio = fixture.0.join("new.wav");
    fs::write(&old_audio, b"old audio").unwrap();
    fs::write(&new_audio, b"different audio").unwrap();
    package(&old_root, &old_audio);
    package(&new_root, &new_audio);
    let old_bytes = package_bytes(&old_root);
    let new_bytes = package_bytes(&new_root);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let (entered, reached) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let repository = GatedRepository {
        inner: FileDocumentRepository::open(&old_root).unwrap(),
        reads: reads.clone(),
        gate: Mutex::new(Some((entered, resume))),
    };
    let factory_reads = reads.clone();
    let workspace = WorkspaceService::new(
        ProcessingService::new(Arc::new(ProbeOnly), Arc::new(ProbeOnly)),
        DocumentService::new(Arc::new(repository)),
    )
    .with_documents_root(DocumentsRoot::new(
        &old_root,
        fixture.0.join("models"),
        move |root| {
            FileDocumentRepository::open(root)
                .map(|inner| {
                    Arc::new(GatedRepository {
                        inner,
                        reads: factory_reads.clone(),
                        gate: Mutex::new(None),
                    }) as Arc<dyn DocumentRepository>
                })
                .map_err(|error| error.to_string())
        },
    ));
    let model = |backend: &str| ModelRef {
        backend_id: backend.into(),
        model_id: "probe-only".into(),
        revision: None,
        artifact_path: "unused".into(),
        artifacts: Vec::new(),
    };
    let asr = model("probe-asr");
    let dia = model("probe-dia");
    let worker = workspace.clone();
    let audio_path = old_audio.to_str().unwrap().to_string();
    let start = std::thread::spawn(move || {
        if explicit_request {
            worker.start_project_processing_with_request(
                DocumentId::new("meeting"),
                ProcessingRequest {
                    audio_path,
                    asr_model: asr,
                    diarization_model: dia,
                    language: None,
                    expected_speakers: None,
                    device: contracts::ComputeDevice::Cpu,
                    speaker_library_path: None,
                    stage_configuration: Default::default(),
                },
            )
        } else {
            worker.start_project_processing_with_speaker_library(
                DocumentId::new("meeting"),
                asr,
                dia,
                None,
            )
        }
    });
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let switcher = workspace.clone();
    let target = new_root.clone();
    let (attempted, attempt) = mpsc::channel();
    let (completed, completion) = mpsc::channel();
    let switch = std::thread::spawn(move || {
        attempted.send(()).unwrap();
        let result = switcher.switch_documents_root(target);
        completed.send(result.clone()).unwrap();
        result
    });
    attempt.recv_timeout(Duration::from_secs(10)).unwrap();
    // Give the public switch a chance to finish on the baseline. A timeout is
    // only an observation; correctness is asserted against the final result.
    let early_result = completion.recv_timeout(Duration::from_millis(250));
    release.send(()).unwrap();
    let job = start.join().unwrap().unwrap();
    let switch_result = switch.join().unwrap();
    assert!(
        matches!(switch_result, Err(DocumentsRootError::WorkspaceBusy { .. })),
        "the switch must wait for admission and then see the registered jobs; early={early_result:?}, final={switch_result:?}, document reads={:?}",
        reads.lock().unwrap(),
    );
    assert_eq!(workspace.documents_root(), Some(old_root.clone()));
    let expected_reads = if explicit_request { 1 } else { 2 };
    assert_eq!(
        *reads.lock().unwrap(),
        vec![old_audio.to_str().unwrap(); expected_reads]
    );
    assert!(package_bytes(&old_root) == old_bytes);
    assert!(package_bytes(&new_root) == new_bytes);
    workspace.discard_processing_job(&job);
    drop(job);
    workspace.switch_documents_root(&new_root).unwrap();
    assert_eq!(workspace.documents_root(), Some(new_root));
    assert_eq!(fs::read(old_audio).unwrap(), b"old audio");
    assert_eq!(fs::read(new_audio).unwrap(), b"different audio");
}

#[test]
fn processing_request_and_job_admission_keep_the_same_workspace() {
    start_during_switch(false);
}

#[test]
fn explicit_processing_request_keeps_the_workspace_during_job_admission() {
    start_during_switch(true);
}
