//! Capability grants retain their real file repository across workspace switches.
use application::{
    AsrBackend, AsrBackendProvider, DiarizationBackend, DiarizationBackendProvider, DocumentChange,
    DocumentMutationBatch, DocumentMutationCommit, DocumentMutationStoreError, DocumentRepository,
    DocumentService, DocumentSnapshot, DocumentsRoot, PendingDocumentMutation, ProcessingService,
    WorkspaceAccessPolicy, WorkspaceCapabilityPort, WorkspaceService,
};
use contracts::{
    AgentScopeSelector, AgentToolResult, AgentWriteMode, BackendDescriptor, BackendError,
    CapabilityRequest, CapabilityResponse, DocumentRecord,
};
use domain::{DocumentId, DocumentState};
use persistence::{DocumentPackage, FileDocumentRepository};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

struct NoAudio;
impl AsrBackendProvider for NoAudio {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }
    fn backend(&self, _: &str) -> Result<Arc<dyn AsrBackend>, BackendError> {
        panic!("no ASR during a document mutation")
    }
}
impl DiarizationBackendProvider for NoAudio {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        Vec::new()
    }
    fn backend(&self, _: &str) -> Result<Arc<dyn DiarizationBackend>, BackendError> {
        panic!("no diarization during a document mutation")
    }
}
struct Files {
    inner: FileDocumentRepository,
    after_take: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}
impl DocumentRepository for Files {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        self.inner.snapshot()
    }
    fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
        self.inner.get(id)
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
    fn commit_agent_mutation(
        &self,
        batch: DocumentMutationBatch,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        self.inner.commit_agent_mutation(batch)
    }
    fn find_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
        self.inner.find_agent_mutation(mutation_id, fingerprint)
    }
    fn agent_mutation_document_ids(
        &self,
        undo_token: &str,
    ) -> Result<Vec<String>, DocumentMutationStoreError> {
        self.inner.agent_mutation_document_ids(undo_token)
    }
    fn undo_agent_mutation(
        &self,
        undo_token: &str,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        self.inner.undo_agent_mutation(undo_token)
    }
    fn save_pending_agent_mutation(
        &self,
        pending: PendingDocumentMutation,
    ) -> Result<(), DocumentMutationStoreError> {
        self.inner.save_pending_agent_mutation(pending)
    }
    fn find_pending_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        self.inner
            .find_pending_agent_mutation(mutation_id, fingerprint)
    }
    fn take_pending_agent_mutation(
        &self,
        id: &str,
    ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
        let result = self.inner.take_pending_agent_mutation(id);
        let gate = self.after_take.lock().unwrap().take();
        if let Some((reached, release)) = gate {
            reached.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        result
    }
}
struct Fixture {
    base: PathBuf,
    old: PathBuf,
    new: PathBuf,
    files: Arc<Files>,
    workspace: WorkspaceService,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "meetnote-capability-context-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let old = base.join("old");
        let new = base.join("new");
        let mut state = DocumentState::new(DocumentId::new("shared"), "Shared", "recording.wav");
        state.schema.revision = 3;
        for (root, attachment) in [(&old, "old attachment"), (&new, "new attachment")] {
            DocumentPackage::new(root.join("shared"), state.clone(), "# Shared notes\n")
                .unwrap()
                .save()
                .unwrap();
            fs::write(root.join("shared/attachment.txt"), attachment).unwrap();
        }
        let files = Arc::new(Files {
            inner: FileDocumentRepository::open(&old).unwrap(),
            after_take: Mutex::new(None),
        });
        let workspace = WorkspaceService::new(
            ProcessingService::new_with_providers(Arc::new(NoAudio), Arc::new(NoAudio)),
            DocumentService::new(files.clone()),
        )
        .with_documents_root(DocumentsRoot::new(&old, base.join("models"), |root| {
            FileDocumentRepository::open(root)
                .map(|repo| Arc::new(repo) as Arc<dyn DocumentRepository>)
                .map_err(|error| error.to_string())
        }));
        Self {
            base,
            old,
            new,
            files,
            workspace,
        }
    }
    fn port(&self) -> WorkspaceCapabilityPort {
        WorkspaceCapabilityPort::new(
            self.workspace.documents(),
            WorkspaceAccessPolicy::documents([DocumentId::new("shared")]),
        )
    }
    fn bytes(root: &Path) -> [Vec<u8>; 3] {
        ["state.json", "document.md", "attachment.txt"]
            .map(|file| fs::read(root.join("shared").join(file)).unwrap())
    }
    fn title(root: &Path) -> String {
        FileDocumentRepository::open(root)
            .unwrap()
            .get(&DocumentId::new("shared"))
            .unwrap()
            .unwrap()
            .title()
            .into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn update(port: &WorkspaceCapabilityPort, request: &str) -> CapabilityResponse {
    let read = port.execute(
        "read_document",
        CapabilityRequest::new(
            format!("{request}-read"),
            "read",
            AgentScopeSelector::default(),
            serde_json::json!({"documentId": "shared", "projection": "metadata"}),
        ),
    );
    let hash = match read.result.as_ref() {
        Some(AgentToolResult::Document(document)) => document.metadata.markdown_sha256.clone(),
        _ => panic!("expected a real read projection: {read:?}"),
    };
    port.execute("update_document", CapabilityRequest::new(request, request, AgentScopeSelector::default(), serde_json::json!({
        "documentId": "shared", "expectedRevision": 3, "sourceMarkdownSha256": hash, "patch": {"title": "Confirmed in original workspace"}
    })))
}
fn confirmation(port: &WorkspaceCapabilityPort) -> String {
    let response = update(port, "pending");
    match response.result.as_ref() {
        Some(AgentToolResult::Pending(pending)) => pending.confirmation_id.clone(),
        _ => panic!("expected a real pending write: {response:?}"),
    }
}
fn receipt(response: CapabilityResponse) -> contracts::AgentMutationReceipt {
    match response.result.as_ref() {
        Some(AgentToolResult::Mutation(receipt)) => receipt.clone(),
        _ => panic!("expected a committed mutation: {response:?}"),
    }
}
#[test]
fn a_granted_port_does_not_redirect_automatic_writes_after_a_root_switch() {
    let fixture = Fixture::new();
    let port = fixture.port();
    let untouched = Fixture::bytes(&fixture.new);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    receipt(update(&port, "automatic"));
    assert_eq!(
        Fixture::bytes(&fixture.new),
        untouched,
        "an existing grant must not write matching documents in a replacement workspace"
    );
    assert_eq!(
        Fixture::title(&fixture.old),
        "Confirmed in original workspace"
    );
}
#[test]
fn a_granted_port_keeps_pending_confirmation_after_a_root_switch() {
    let fixture = Fixture::new();
    let port = fixture.port().with_write_mode(AgentWriteMode::ConfirmEach);
    let id = confirmation(&port);
    let untouched = Fixture::bytes(&fixture.new);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    receipt(port.confirm(&id, "confirm"));
    assert_eq!(Fixture::bytes(&fixture.new), untouched);
    assert_eq!(
        Fixture::title(&fixture.old),
        "Confirmed in original workspace"
    );
}
#[test]
fn confirming_a_taken_pending_write_cannot_cross_a_root_switch() {
    let fixture = Fixture::new();
    let port = fixture.port().with_write_mode(AgentWriteMode::ConfirmEach);
    let id = confirmation(&port);
    let untouched = Fixture::bytes(&fixture.new);
    let (entered, reached) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    *fixture.files.after_take.lock().unwrap() = Some((entered, resume));
    let confirm = std::thread::spawn(move || port.confirm(&id, "confirm-held"));
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let switched = fixture.workspace.switch_documents_root(&fixture.new);
    release.send(()).unwrap();
    let response = confirm.join().unwrap();
    switched.unwrap();
    receipt(response);
    assert_eq!(
        Fixture::bytes(&fixture.new),
        untouched,
        "a consumed old pending write must not commit into the replacement repository"
    );
    assert_eq!(
        Fixture::title(&fixture.old),
        "Confirmed in original workspace"
    );
}

#[test]
fn a_granted_port_can_undo_its_original_workspace_after_a_switch() {
    let fixture = Fixture::new();
    let port = fixture.port();
    let undo = receipt(update(&port, "original-write")).undo_token;
    let untouched = Fixture::bytes(&fixture.new);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    receipt(port.undo(&undo, "original-undo"));
    assert_eq!(Fixture::title(&fixture.old), "Shared");
    assert_eq!(Fixture::bytes(&fixture.new), untouched);
}

#[test]
fn a_granted_port_can_reject_its_original_pending_write_after_a_switch() {
    let fixture = Fixture::new();
    let port = fixture.port().with_write_mode(AgentWriteMode::ConfirmEach);
    let id = confirmation(&port);
    let old = Fixture::bytes(&fixture.old);
    let new = Fixture::bytes(&fixture.new);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    let rejected = port.reject(&id, "reject-original");
    assert_eq!(
        rejected.error.as_ref().map(|error| error.code),
        Some(contracts::AgentErrorCode::WritePolicyRejected)
    );
    assert!(fixture
        .files
        .inner
        .take_pending_agent_mutation(&id)
        .unwrap()
        .is_none());
    assert_eq!(Fixture::bytes(&fixture.old), old);
    assert_eq!(Fixture::bytes(&fixture.new), new);
}

#[test]
fn a_new_grant_uses_the_new_root_while_an_existing_grant_keeps_its_reads() {
    let fixture = Fixture::new();
    let original = fixture.port();
    let untouched = Fixture::bytes(&fixture.old);
    fixture
        .workspace
        .switch_documents_root(&fixture.new)
        .unwrap();
    let current = fixture.port();
    let undo = receipt(update(&current, "new-root-write")).undo_token;
    assert_eq!(
        Fixture::title(&fixture.new),
        "Confirmed in original workspace"
    );
    assert_eq!(Fixture::bytes(&fixture.old), untouched);
    let read = original.execute(
        "read_document",
        CapabilityRequest::new(
            "original-read",
            "read",
            AgentScopeSelector::default(),
            serde_json::json!({"documentId": "shared", "projection": "metadata"}),
        ),
    );
    let Some(AgentToolResult::Document(document)) = read.result.as_ref() else {
        panic!("{read:?}")
    };
    assert_eq!(
        document.metadata.title, "Shared",
        "an existing grant keeps reading its original root"
    );
    receipt(current.undo(&undo, "undo-new-root-write"));
    assert_eq!(Fixture::title(&fixture.new), "Shared");
    assert_eq!(Fixture::bytes(&fixture.old), untouched);
}
