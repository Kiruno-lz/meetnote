//! A command must keep its original file repository across a root switch,
//! including the reads used to return its result. Gates run after real file
//! operations release their locks; the switch uses the public workspace API.

use application::{
    AsrBackend, AsrBackendProvider, DiarizationBackend, DiarizationBackendProvider, DocumentChange,
    DocumentCommandError, DocumentCommandService, DocumentRepository, DocumentService,
    DocumentSnapshot, DocumentsRoot, ProcessingService, WorkspaceService,
};
use contracts::{
    BackendDescriptor, BackendError, DocumentCommand, DocumentCommandEnvelope,
    DocumentCommandResult, DocumentPatch, DocumentRecord,
};
use domain::{DiarizationSegment, DocumentId, DocumentState};
use persistence::{DocumentPackage, FileDocumentRepository};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Operation {
    Read,
    Save,
    Snapshot,
}

struct Gate {
    operation: Operation,
    remaining: usize,
    reached: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

struct GatedRepository {
    files: FileDocumentRepository,
    gate: Mutex<Option<Gate>>,
}

impl GatedRepository {
    fn after(&self, operation: Operation) {
        let gate = {
            let mut pending = self.gate.lock().unwrap();
            match pending.as_mut() {
                Some(gate) if gate.operation == operation => {
                    gate.remaining -= 1;
                    (gate.remaining == 0).then(|| pending.take().unwrap())
                }
                _ => None,
            }
        };
        if let Some(gate) = gate {
            gate.reached.send(()).unwrap();
            gate.release.recv().unwrap();
        }
    }
}

impl DocumentRepository for GatedRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        let result = self.files.snapshot();
        self.after(Operation::Snapshot);
        result
    }

    fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
        self.files.get(id)
    }

    fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        let result = self.files.read(id);
        self.after(Operation::Read);
        result
    }

    fn save(&self, state: DocumentState, markdown: Option<String>) -> Result<(), String> {
        let result = self.files.save(state, markdown);
        self.after(Operation::Save);
        result
    }

    fn watch(&self) -> Result<mpsc::Receiver<DocumentChange>, String> {
        self.files.watch()
    }
}

struct NoAudio;

impl AsrBackendProvider for NoAudio {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }

    fn backend(&self, _: &str) -> Result<Arc<dyn AsrBackend>, BackendError> {
        Err(BackendError::not_implemented(
            "audio is not used in this test",
        ))
    }
}

impl DiarizationBackendProvider for NoAudio {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }

    fn backend(&self, _: &str) -> Result<Arc<dyn DiarizationBackend>, BackendError> {
        Err(BackendError::not_implemented(
            "audio is not used in this test",
        ))
    }
}

struct Fixture {
    base: PathBuf,
    old: PathBuf,
    new: PathBuf,
    repository: Arc<GatedRepository>,
    workspace: WorkspaceService,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "meetnote-command-workspace-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        let old = base.join("old");
        let new = base.join("new");
        for (root, name) in [(&old, "Old"), (&new, "New")] {
            for (id, parent, depth, order) in [
                ("a", None, 0, 0),
                ("b", None, 0, 1),
                ("c", Some("a"), 1, 0),
                ("d", Some("c"), 2, 0),
            ] {
                let mut state =
                    DocumentState::new(DocumentId::new(id), format!("{name} {id}"), "audio.wav");
                state.schema.revision = 7;
                state.hierarchy.parent_id = parent.map(DocumentId::new);
                state.hierarchy.depth = depth;
                state.hierarchy.sibling_order = order;
                state.source.audio_asset_id = parent.map(|_| DocumentId::new("a"));
                state.results.diarization = (0..2)
                    .map(|index| DiarizationSegment {
                        id: format!("segment-{index}"),
                        speaker_id: format!("speaker-{index}"),
                        speaker_profile_id: None,
                        speaker_label: None,
                        start_ms: index * 1_000,
                        end_ms: (index + 1) * 1_000,
                        confidence: None,
                    })
                    .collect();
                DocumentPackage::new(root.join(id), state, format!("# {name} {id}\n"))
                    .unwrap()
                    .save()
                    .unwrap();
                fs::write(root.join(id).join("attachment.txt"), name).unwrap();
            }
        }
        let repository = Arc::new(GatedRepository {
            files: FileDocumentRepository::open(&old).unwrap(),
            gate: Mutex::new(None),
        });
        let workspace = WorkspaceService::new(
            ProcessingService::new_with_providers(Arc::new(NoAudio), Arc::new(NoAudio)),
            DocumentService::new(repository.clone()),
        )
        .with_documents_root(DocumentsRoot::new(&old, base.join("models"), |root| {
            FileDocumentRepository::open(root)
                .map(|repository| Arc::new(repository) as Arc<dyn DocumentRepository>)
                .map_err(|error| error.to_string())
        }));
        Self {
            base,
            old,
            new,
            repository,
            workspace,
        }
    }

    fn commands(&self) -> DocumentCommandService {
        // The public constructor must also bind each invocation, even when
        // retained for multiple commands across root switches.
        DocumentCommandService::new(self.workspace.documents())
    }

    fn switch_during<T: Send + 'static>(
        &self,
        operation: Operation,
        occurrence: usize,
        run: impl FnOnce(DocumentCommandService) -> Result<T, DocumentCommandError> + Send + 'static,
    ) -> T {
        let (reached, receiver) = mpsc::channel();
        let (release, released) = mpsc::channel();
        *self.repository.gate.lock().unwrap() = Some(Gate {
            operation,
            remaining: occurrence,
            reached,
            release: released,
        });
        let commands = self.commands();
        let worker = std::thread::spawn(move || run(commands));
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("command did not reach its IO gate");
        let before = package_bytes(&self.new);
        self.workspace.switch_documents_root(&self.new).unwrap();
        release.send(()).unwrap();
        let result = worker.join().unwrap();
        assert!(
            package_bytes(&self.new) == before,
            "command changed packages in the new workspace"
        );
        for id in ["a", "b", "c", "d"] {
            assert_eq!(
                fs::read(self.old.join(id).join("attachment.txt")).unwrap(),
                b"Old"
            );
        }
        result.unwrap()
    }

    fn old_record(&self, id: &str) -> DocumentRecord {
        self.repository
            .files
            .read(&DocumentId::new(id))
            .unwrap()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn package_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn update() -> DocumentCommand {
    DocumentCommand::Update {
        document_id: DocumentId::new("a"),
        expected_revision: 7,
        patch: DocumentPatch {
            title: Some("Saved in old workspace".into()),
            ..DocumentPatch::default()
        },
    }
}

fn execute(
    commands: DocumentCommandService,
    command: DocumentCommand,
) -> Result<DocumentCommandResult, DocumentCommandError> {
    commands.execute(DocumentCommandEnvelope::new("workspace-context", command))
}

#[test]
fn update_keeps_the_repository_between_its_first_read_and_save() {
    let fixture = Fixture::new();
    let result = fixture.switch_during(Operation::Read, 1, |commands| {
        commands.update_document(
            &DocumentId::new("a"),
            7,
            DocumentPatch {
                title: Some("Saved in old workspace".into()),
                ..DocumentPatch::default()
            },
        )
    });
    assert_eq!(result, fixture.old_record("a"));
    assert_eq!(result.state.title(), "Saved in old workspace");
    assert_eq!(result.markdown, "# Old a\n");
}

#[test]
fn update_reads_its_result_from_the_repository_it_wrote() {
    let fixture = Fixture::new();
    let result = fixture.switch_during(Operation::Save, 1, |commands| execute(commands, update()));
    assert_eq!(
        result,
        DocumentCommandResult::Updated(fixture.old_record("a"))
    );
}

#[test]
fn reparent_keeps_the_repository_of_its_tree_snapshot() {
    let fixture = Fixture::new();
    let changed = fixture.switch_during(Operation::Snapshot, 1, |commands| {
        commands.reparent_document(&DocumentId::new("c"), 7, None, 2)
    });
    assert_eq!(changed.len(), 2);
    assert_eq!(fixture.old_record("c").state.hierarchy.depth, 0);
    assert_eq!(fixture.old_record("d").state.hierarchy.depth, 1);
}

#[test]
fn reparent_keeps_all_descendant_writes_in_the_original_workspace() {
    let fixture = Fixture::new();
    let result = fixture.switch_during(Operation::Save, 1, |commands| {
        execute(
            commands,
            DocumentCommand::Reparent {
                document_id: DocumentId::new("c"),
                expected_revision: 7,
                new_parent_id: None,
                sibling_order: 2,
            },
        )
    });
    let DocumentCommandResult::Snapshot { documents, issues } = result else {
        panic!("expected snapshot")
    };
    assert!(issues.is_empty());
    assert!(documents
        .iter()
        .all(|state| state.title().starts_with("Old")));
    assert_eq!(fixture.old_record("c").state.hierarchy.depth, 0);
    assert_eq!(fixture.old_record("d").state.hierarchy.depth, 1);
}

#[test]
fn reorder_keeps_the_repository_of_its_tree_snapshot() {
    let fixture = Fixture::new();
    let changed = fixture.switch_during(Operation::Snapshot, 1, |commands| {
        commands.reorder_document(&DocumentId::new("b"), 7, &DocumentId::new("a"), true)
    });
    assert_eq!(changed.len(), 2);
    assert_eq!(fixture.old_record("b").state.hierarchy.sibling_order, 0);
}

#[test]
fn speaker_merge_keeps_the_repository_it_read() {
    let fixture = Fixture::new();
    let record = fixture.switch_during(Operation::Read, 1, |commands| {
        commands.merge_speaker(&DocumentId::new("a"), 7, "speaker-0", "speaker-1")
    });
    assert_eq!(record, fixture.old_record("a"));
    assert!(record
        .state
        .results
        .diarization
        .iter()
        .all(|segment| segment.speaker_id == "speaker-1"));
    assert_eq!(record.markdown, "# Old a\n");
}

#[test]
fn speaker_label_keeps_the_repository_it_read() {
    let fixture = Fixture::new();
    let record = fixture.switch_during(Operation::Read, 1, |commands| {
        commands.set_speaker_label(&DocumentId::new("a"), 7, "speaker-0", Some("Alice".into()))
    });
    assert_eq!(record, fixture.old_record("a"));
    assert_eq!(
        record.state.results.diarization[0].speaker_label.as_deref(),
        Some("Alice")
    );
}

#[test]
fn create_reads_back_from_the_repository_it_wrote() {
    let fixture = Fixture::new();
    let result = fixture.switch_during(Operation::Save, 1, |commands| {
        execute(
            commands,
            DocumentCommand::Create {
                title: "Created in old workspace".into(),
                audio_path: "recording.wav".into(),
                markdown: "# Created\n".into(),
                tags: Vec::new(),
            },
        )
    });
    let DocumentCommandResult::Document(Some(record)) = result else {
        panic!("created document was lost")
    };
    assert_eq!(record, fixture.old_record(record.state.id().as_str()));
    assert_eq!(record.markdown, "# Created\n");
}

#[test]
fn semantic_child_keeps_the_repository_of_its_parent() {
    let fixture = Fixture::new();
    let result = fixture.switch_during(Operation::Read, 1, |commands| {
        execute(
            commands,
            DocumentCommand::CreateSemanticChild {
                parent_id: DocumentId::new("a"),
                expected_parent_revision: 7,
                title: "Old child".into(),
                start_ms: 0,
                end_ms: 1_000,
                tags: Vec::new(),
                markdown: "# Old child\n".into(),
            },
        )
    });
    let DocumentCommandResult::Document(Some(record)) = result else {
        panic!("created child was lost")
    };
    assert_eq!(record, fixture.old_record(record.state.id().as_str()));
    assert_eq!(record.state.hierarchy.parent_id, Some(DocumentId::new("a")));
}

#[test]
fn a_queued_workspace_command_keeps_the_root_that_created_its_handle() {
    let fixture = Fixture::new();
    let commands = fixture.workspace.document_commands();
    let before = package_bytes(&fixture.new);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    let result = execute(commands, update()).unwrap();
    assert!(
        package_bytes(&fixture.new) == before,
        "queued command wrote to the new workspace"
    );
    assert_eq!(
        result,
        DocumentCommandResult::Updated(fixture.old_record("a"))
    );
}

#[test]
fn a_directly_constructed_dynamic_service_uses_the_root_at_each_command_start() {
    let fixture = Fixture::new();
    let commands = fixture.commands();
    let before_switch = execute(
        commands.clone(),
        DocumentCommand::Read {
            document_id: DocumentId::new("a"),
        },
    )
    .unwrap();
    assert_eq!(
        before_switch,
        DocumentCommandResult::Document(Some(fixture.old_record("a")))
    );
    let old = package_bytes(&fixture.old);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    let result = execute(commands, update()).unwrap();
    let DocumentCommandResult::Updated(record) = result else {
        panic!("expected update")
    };
    assert_eq!(record.state.title(), "Saved in old workspace");
    assert_eq!(record.markdown, "# New a\n");
    assert!(
        package_bytes(&fixture.old) == old,
        "a new command wrote to the old workspace"
    );
}
