use application::{
    DocumentChange, DocumentIssue, DocumentMetadata, DocumentMutationBatch, DocumentMutationCommit,
    DocumentMutationRevision, DocumentMutationStoreError, DocumentRepository, DocumentSnapshot,
};
use contracts::{AgentMutationProvenance, DocumentRecord};
use domain::{
    AudioSource, DocumentContent, DocumentHierarchy, DocumentId, DocumentIdentity, DocumentSchema,
    DocumentState, DocumentStatus,
};
use fs2::FileExt;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, RwLock};

pub const DOCUMENT_MARKDOWN_FILENAME: &str = "document.md";
pub const DOCUMENT_STATE_FILENAME: &str = "state.json";
/// Bound one metadata input file without constructing a complete document.
pub const MAX_DOCUMENT_PACKAGE_FILE_BYTES: usize = 16 * 1024 * 1024;
const AGENT_MUTATION_DIRECTORY: &str = ".agent-mutations";
const AGENT_MUTATION_JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentPackage {
    directory: PathBuf,
    pub state: DocumentState,
    pub markdown: String,
}

impl DocumentPackage {
    pub fn new(
        directory: impl Into<PathBuf>,
        state: DocumentState,
        markdown: impl Into<String>,
    ) -> Result<Self, DocumentPackageError> {
        let package = Self {
            directory: directory.into(),
            state,
            markdown: markdown.into(),
        };
        package.validate()?;
        Ok(package)
    }

    pub fn load(directory: impl AsRef<Path>) -> Result<Self, DocumentPackageError> {
        let directory = directory.as_ref().to_path_buf();
        if !directory.is_dir() {
            return Err(DocumentPackageError::NotDirectory(directory));
        }
        let state_path = directory.join(DOCUMENT_STATE_FILENAME);
        let markdown_path = directory.join(DOCUMENT_MARKDOWN_FILENAME);
        let state_bytes = fs::read(&state_path).map_err(|source| DocumentPackageError::Read {
            path: state_path.clone(),
            source,
        })?;
        let legacy = serde_json::from_slice::<serde_json::Value>(&state_bytes)
            .map_err(|source| DocumentPackageError::InvalidState {
                path: state_path.clone(),
                source,
            })?
            .get("schema")
            .is_none();
        let state = serde_json::from_slice::<DocumentState>(&state_bytes).map_err(|source| {
            DocumentPackageError::InvalidState {
                path: state_path.clone(),
                source,
            }
        })?;
        let markdown =
            fs::read_to_string(&markdown_path).map_err(|source| DocumentPackageError::Read {
                path: markdown_path,
                source,
            })?;
        let package = Self {
            directory,
            state,
            markdown,
        };
        package.validate()?;
        if legacy {
            package.save()?;
        }
        Ok(package)
    }

    pub fn load_metadata(
        directory: impl AsRef<Path>,
    ) -> Result<DocumentMetadata, DocumentPackageError> {
        let directory = directory.as_ref().to_path_buf();
        if !directory.is_dir() {
            return Err(DocumentPackageError::NotDirectory(directory));
        }
        let state_path = directory.join(DOCUMENT_STATE_FILENAME);
        let file = File::open(&state_path).map_err(|source| DocumentPackageError::Read {
            path: state_path.clone(),
            source,
        })?;
        if file
            .metadata()
            .map_err(|source| DocumentPackageError::Read {
                path: state_path.clone(),
                source,
            })?
            .len()
            > MAX_DOCUMENT_PACKAGE_FILE_BYTES as u64
        {
            return Err(DocumentPackageError::TooLarge {
                path: state_path,
                max_bytes: MAX_DOCUMENT_PACKAGE_FILE_BYTES,
            });
        }
        let wire: MetadataState =
            serde_json::from_reader(BufReader::new(file)).map_err(|source| {
                DocumentPackageError::InvalidState {
                    path: state_path.clone(),
                    source,
                }
            })?;
        let metadata = wire
            .into_metadata()
            .map_err(|error| DocumentPackageError::InvalidStateValue { error })?;
        let directory_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DocumentPackageError::InvalidDirectory(directory.clone()))?;
        if directory_name != metadata.document_id.as_str() {
            return Err(DocumentPackageError::DirectoryIdMismatch {
                directory,
                document_id: metadata.document_id,
            });
        }
        Ok(metadata)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn document_id(&self) -> &DocumentId {
        self.state.id()
    }

    pub fn state_path(&self) -> PathBuf {
        self.directory.join(DOCUMENT_STATE_FILENAME)
    }

    pub fn markdown_path(&self) -> PathBuf {
        self.directory.join(DOCUMENT_MARKDOWN_FILENAME)
    }

    pub fn save(&self) -> Result<(), DocumentPackageError> {
        self.validate()?;
        fs::create_dir_all(&self.directory).map_err(|source| DocumentPackageError::Write {
            path: self.directory.clone(),
            source,
        })?;
        let state = serde_json::to_vec_pretty(&self.state)
            .map_err(|source| DocumentPackageError::SerializeState { source })?;
        write_atomic(&self.state_path(), &state)?;
        write_atomic(&self.markdown_path(), self.markdown.as_bytes())?;
        Ok(())
    }

    fn validate(&self) -> Result<(), DocumentPackageError> {
        self.state
            .validate_schema()
            .map_err(|error| DocumentPackageError::InvalidStateValue { error })?;
        let directory_name = self
            .directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DocumentPackageError::InvalidDirectory(self.directory.clone()))?;
        if directory_name != self.state.id().as_str() {
            return Err(DocumentPackageError::DirectoryIdMismatch {
                directory: self.directory.clone(),
                document_id: self.state.id().clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct MetadataState {
    schema: DocumentSchema,
    identity: DocumentIdentity,
    source: AudioSource,
    hierarchy: DocumentHierarchy,
    document_status: DocumentStatus,
    content: DocumentContent,
}

impl MetadataState {
    fn into_metadata(self) -> Result<DocumentMetadata, domain::DocumentStateError> {
        let identity = self.identity;
        let schema = self.schema;
        let source = self.source;
        let hierarchy = self.hierarchy;
        let document_status = self.document_status;
        let content = self.content;
        if schema.schema_version != domain::DOCUMENT_STATE_SCHEMA_VERSION {
            return Err(domain::DocumentStateError::UnsupportedSchema {
                actual: schema.schema_version,
                expected: domain::DOCUMENT_STATE_SCHEMA_VERSION,
            });
        }
        Ok(DocumentMetadata {
            document_id: identity.id,
            revision: schema.revision,
            title: content.title,
            tags: content.tags,
            summary: content.summary,
            document_status: document_status.value,
            parent_id: hierarchy.parent_id,
            depth: hierarchy.depth,
            sibling_order: hierarchy.sibling_order,
            audio_asset_id: source.audio_asset_id,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentPackageError {
    #[error("document package directory does not exist: {0}")]
    NotDirectory(PathBuf),
    #[error("cannot read document package file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid document state JSON at {path}: {source}")]
    InvalidState {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid document state: {error}")]
    InvalidStateValue { error: domain::DocumentStateError },
    #[error("cannot serialize document state: {source}")]
    SerializeState {
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot write document package file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("document package path is not a directory: {0}")]
    InvalidDirectory(PathBuf),
    #[error("document directory {directory} contains state for {document_id}")]
    DirectoryIdMismatch {
        directory: PathBuf,
        document_id: DocumentId,
    },
    #[error("document package file {path} exceeds the {max_bytes} byte limit")]
    TooLarge { path: PathBuf, max_bytes: usize },
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), DocumentPackageError> {
    let temporary = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&temporary, bytes).map_err(|source| DocumentPackageError::Write {
        path: temporary.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(DocumentPackageError::Write {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentScan {
    pub packages: Vec<DocumentPackage>,
    pub errors: Vec<DocumentScanError>,
}

impl DocumentScan {
    pub fn documents(&self) -> Vec<DocumentState> {
        self.packages
            .iter()
            .map(|package| package.state.clone())
            .collect()
    }
}

/// Why a root entry did not become a tree document; the variants live in
/// `application` so issue messages and paths are built in one place.
pub use application::DocumentIssueKind as DocumentScanError;

#[derive(Debug, Clone)]
pub struct DocumentScanner {
    root: PathBuf,
}

impl DocumentScanner {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn scan(&self) -> Result<DocumentScan, DocumentScannerError> {
        if !self.root.exists() {
            return Ok(DocumentScan {
                packages: Vec::new(),
                errors: Vec::new(),
            });
        }
        if !self.root.is_dir() {
            return Err(DocumentScannerError::RootNotDirectory(self.root.clone()));
        }

        let mut entries = fs::read_dir(&self.root)
            .map_err(|source| DocumentScannerError::ReadRoot {
                path: self.root.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| DocumentScannerError::ReadRoot {
                path: self.root.clone(),
                source,
            })?;
        entries.sort_by_key(|entry| entry.file_name());

        let mut packages = Vec::new();
        let mut errors = Vec::new();
        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                errors.extend(DocumentScanError::unexpected_root_entry(&path));
                continue;
            }
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            match DocumentPackage::load(&path) {
                Ok(package) => packages.push(package),
                Err(error) => errors.push(DocumentScanError::InvalidPackage {
                    path,
                    message: error.to_string(),
                }),
            }
        }
        packages.sort_by_key(|package| package.state.hierarchy.sibling_order);
        errors.extend(validate_relations(&packages));
        Ok(DocumentScan { packages, errors })
    }

    pub fn scan_metadata(
        &self,
        visitor: &mut dyn FnMut(DocumentMetadata) -> Result<bool, String>,
    ) -> Result<(), String> {
        if !self.root.exists() {
            return Ok(());
        }
        if !self.root.is_dir() {
            return Err(DocumentScannerError::RootNotDirectory(self.root.clone()).to_string());
        }
        for entry in fs::read_dir(&self.root).map_err(|source| {
            DocumentScannerError::ReadRoot {
                path: self.root.clone(),
                source,
            }
            .to_string()
        })? {
            let entry = entry.map_err(|source| {
                DocumentScannerError::ReadRoot {
                    path: self.root.clone(),
                    source,
                }
                .to_string()
            })?;
            let path = entry.path();
            if !path.is_dir()
                || entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            let metadata =
                DocumentPackage::load_metadata(&path).map_err(|error| error.to_string())?;
            if !visitor(metadata)? {
                break;
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentScannerError {
    #[error("document root is not a directory: {0}")]
    RootNotDirectory(PathBuf),
    #[error("cannot read document root {path}: {source}")]
    ReadRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn validate_relations(packages: &[DocumentPackage]) -> Vec<DocumentScanError> {
    let mut errors = Vec::new();
    let mut by_id = BTreeMap::new();
    for package in packages {
        if by_id.insert(package.state.id().clone(), package).is_some() {
            errors.push(DocumentScanError::DuplicateDocumentId {
                document_id: package.state.id().clone(),
                path: package.directory().to_path_buf(),
            });
        }
    }

    for package in packages {
        let state = &package.state;
        let Some(parent_id) = state.hierarchy.parent_id.as_ref() else {
            if state.hierarchy.depth != 0 {
                errors.push(DocumentScanError::DepthMismatch {
                    document_id: state.id().clone(),
                    declared: state.hierarchy.depth,
                    actual: 0,
                });
            }
            continue;
        };
        let Some(parent) = by_id.get(parent_id) else {
            errors.push(DocumentScanError::MissingParent {
                document_id: state.id().clone(),
                parent_id: parent_id.clone(),
            });
            continue;
        };
        if parent.state.id() == state.id() || reaches(parent, state.id(), &by_id) {
            errors.push(DocumentScanError::Cycle {
                document_id: state.id().clone(),
            });
            continue;
        }
        let state_asset = state.source.audio_asset_id.as_ref().unwrap_or(state.id());
        let parent_asset = parent
            .state
            .source
            .audio_asset_id
            .as_ref()
            .unwrap_or(parent.state.id());
        if state_asset != parent_asset {
            errors.push(DocumentScanError::CrossAudioAsset {
                document_id: state.id().clone(),
            });
        }
        let actual_depth = parent.state.hierarchy.depth.saturating_add(1);
        if actual_depth > 2 {
            errors.push(DocumentScanError::MaximumDepth {
                document_id: state.id().clone(),
            });
        }
        if state.hierarchy.depth != actual_depth {
            errors.push(DocumentScanError::DepthMismatch {
                document_id: state.id().clone(),
                declared: state.hierarchy.depth,
                actual: actual_depth,
            });
        }
    }
    errors
}

fn reaches(
    start: &DocumentPackage,
    target: &DocumentId,
    by_id: &BTreeMap<DocumentId, &DocumentPackage>,
) -> bool {
    let mut current = start.state.hierarchy.parent_id.as_ref();
    while let Some(id) = current {
        if id == target {
            return true;
        }
        current = by_id
            .get(id)
            .and_then(|package| package.state.hierarchy.parent_id.as_ref());
    }
    false
}

#[derive(Default)]
struct InMemoryMutationState {
    by_mutation_id: BTreeMap<String, (String, DocumentMutationCommit)>,
    undo: BTreeMap<String, Vec<(String, Option<DocumentRecord>)>>,
    undo_commits: BTreeMap<String, DocumentMutationCommit>,
    undone_mutation_ids: std::collections::BTreeSet<String>,
    pending: BTreeMap<String, application::PendingDocumentMutation>,
}

#[derive(Clone, Default)]
pub struct InMemoryDocumentRepository {
    transaction_lock: Arc<RwLock<()>>,
    documents: Arc<RwLock<BTreeMap<String, DocumentState>>>,
    markdown: Arc<RwLock<BTreeMap<String, String>>>,
    mutations: Arc<Mutex<InMemoryMutationState>>,
}

impl DocumentRepository for InMemoryDocumentRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        Ok(DocumentSnapshot {
            documents: self
                .documents
                .read()
                .map_err(|_| "document repository lock poisoned".to_string())?
                .values()
                .cloned()
                .collect(),
            issues: Vec::new(),
        })
    }

    fn scan_metadata(
        &self,
        visitor: &mut dyn FnMut(DocumentMetadata) -> Result<bool, String>,
    ) -> Result<(), String> {
        let documents = self
            .documents
            .read()
            .map_err(|_| "document repository lock poisoned".to_string())?;
        for document in documents.values() {
            if !visitor(DocumentMetadata::from(document))? {
                break;
            }
        }
        Ok(())
    }

    fn get(&self, document_id: &DocumentId) -> Result<Option<DocumentState>, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        Ok(self
            .documents
            .read()
            .map_err(|_| "document repository lock poisoned".to_string())?
            .get(document_id.as_str())
            .cloned())
    }

    fn read(&self, document_id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        let Some(state) = self
            .documents
            .read()
            .map_err(|_| "document repository lock poisoned".to_string())?
            .get(document_id.as_str())
            .cloned()
        else {
            return Ok(None);
        };
        let markdown = self
            .markdown
            .read()
            .map_err(|_| "document markdown lock poisoned".to_string())?
            .get(document_id.as_str())
            .cloned()
            .unwrap_or_else(|| format!("# {}\n\n", state.title()));
        Ok(Some(DocumentRecord { markdown, state }))
    }

    fn watch(&self) -> Result<Receiver<DocumentChange>, String> {
        let (_sender, receiver) = mpsc::channel();
        Ok(receiver)
    }

    fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
        let _guard = self
            .transaction_lock
            .write()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        let document_id = document.id().as_str().to_string();
        self.documents
            .write()
            .map_err(|_| "document repository lock poisoned".to_string())?
            .insert(document_id.clone(), document);
        if let Some(markdown) = markdown {
            self.markdown
                .write()
                .map_err(|_| "document markdown lock poisoned".to_string())?
                .insert(document_id, markdown);
        }
        Ok(())
    }

    fn commit_agent_mutation(
        &self,
        mut batch: DocumentMutationBatch,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        let _transaction_guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let mut mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        if let Some((fingerprint, commit)) = mutations.by_mutation_id.get(&batch.mutation_id) {
            if fingerprint != &batch.fingerprint {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            if mutations.undone_mutation_ids.contains(&batch.mutation_id) {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            let mut commit = commit.clone();
            commit.replayed = true;
            return Ok(commit);
        }
        let mut documents = self.documents.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("document repository lock poisoned".into())
        })?;
        let mut markdown = self.markdown.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("document markdown lock poisoned".into())
        })?;
        validate_mutation_batch(&batch, |id| {
            Ok(documents.get(id).cloned().map(|state| DocumentRecord {
                markdown: markdown.get(id).cloned().unwrap_or_default(),
                state,
            }))
        })?;
        let mut post_states = documents.values().cloned().collect::<Vec<_>>();
        apply_writes_to_states(&mut post_states, &batch);
        validate_state_relations(&post_states)?;
        let before: Vec<_> = batch
            .writes
            .iter()
            .map(|write| {
                let id = write.record.state.id().as_str().to_string();
                let record = documents.get(&id).cloned().map(|state| DocumentRecord {
                    markdown: markdown.get(&id).cloned().unwrap_or_default(),
                    state,
                });
                (id, record)
            })
            .collect();
        batch.provenance.applied_at_ms = now_ms();
        let revisions = batch
            .writes
            .iter()
            .map(|write| DocumentMutationRevision {
                document_id: write.record.state.id().as_str().into(),
                previous_revision: write.expected_revision,
                revision: Some(write.record.state.revision()),
            })
            .collect();
        let records: Vec<_> = batch
            .writes
            .iter()
            .map(|write| write.record.clone())
            .collect();
        for write in &batch.writes {
            let id = write.record.state.id().as_str().to_string();
            documents.insert(id.clone(), write.record.state.clone());
            markdown.insert(id, write.record.markdown.clone());
        }
        let commit = DocumentMutationCommit {
            mutation_id: batch.mutation_id.clone(),
            undo_token: batch.undo_token.clone(),
            records,
            revisions,
            provenance: batch.provenance,
            replayed: false,
        };
        mutations.undo.insert(batch.undo_token, before);
        mutations
            .by_mutation_id
            .insert(batch.mutation_id, (batch.fingerprint, commit.clone()));
        Ok(commit)
    }

    fn find_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
        let mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let Some((stored_fingerprint, commit)) = mutations.by_mutation_id.get(mutation_id) else {
            return Ok(None);
        };
        if stored_fingerprint != fingerprint || mutations.undone_mutation_ids.contains(mutation_id)
        {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        let mut commit = commit.clone();
        commit.replayed = true;
        Ok(Some(commit))
    }

    fn agent_mutation_document_ids(
        &self,
        undo_token: &str,
    ) -> Result<Vec<String>, DocumentMutationStoreError> {
        let mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let commit = mutations
            .by_mutation_id
            .values()
            .map(|(_, commit)| commit)
            .find(|commit| commit.undo_token == undo_token)
            .ok_or(DocumentMutationStoreError::UndoNotFound)?;
        Ok(commit
            .revisions
            .iter()
            .filter(|revision| revision.previous_revision.is_some())
            .map(|revision| revision.document_id.clone())
            .collect())
    }

    fn undo_agent_mutation(
        &self,
        undo_token: &str,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        let _transaction_guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let mut mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        if let Some(commit) = mutations.undo_commits.get(undo_token) {
            let mut commit = commit.clone();
            commit.replayed = true;
            return Ok(commit);
        }
        let before = mutations
            .undo
            .get(undo_token)
            .cloned()
            .ok_or(DocumentMutationStoreError::UndoNotFound)?;
        let original_commit = mutations
            .by_mutation_id
            .values()
            .find(|(_, commit)| commit.undo_token == undo_token)
            .map(|(_, commit)| commit.clone())
            .ok_or(DocumentMutationStoreError::UndoNotFound)?;
        let mut documents = self.documents.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("document repository lock poisoned".into())
        })?;
        let mut markdown = self.markdown.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("document markdown lock poisoned".into())
        })?;
        let mut records = Vec::new();
        let mut revisions = Vec::new();
        let current_records = before
            .iter()
            .map(|(id, _)| {
                let state = documents.get(id).cloned().ok_or_else(|| {
                    DocumentMutationStoreError::NotFound {
                        document_id: id.clone(),
                    }
                })?;
                let markdown = markdown.get(id).cloned().unwrap_or_default();
                Ok((id.clone(), DocumentRecord { state, markdown }))
            })
            .collect::<Result<BTreeMap<_, _>, DocumentMutationStoreError>>()?;
        for (id, current) in &current_records {
            let committed = original_commit
                .records
                .iter()
                .find(|record| record.state.id().as_str() == id)
                .ok_or_else(|| {
                    DocumentMutationStoreError::Transaction(format!(
                        "mutation commit is missing document {id}"
                    ))
                })?;
            let committed_revision = committed.state.revision();
            let expected_revision = expected_memory_undo_revision(
                &mutations,
                &original_commit,
                id,
                committed_revision,
            )?;
            if current.state.revision() != expected_revision
                || !records_match_except_revision(current, committed)
            {
                return Err(DocumentMutationStoreError::RevisionConflict {
                    document_id: id.clone(),
                    expected: expected_revision,
                    actual: current.state.revision(),
                });
            }
        }
        for (id, record) in before {
            let current_revision = current_records[&id].state.revision();
            match record {
                Some(mut record) => {
                    record.state.schema.revision = current_revision.saturating_add(1);
                    record.state.schema.updated_at_ms = now_ms();
                    documents.insert(id.clone(), record.state.clone());
                    markdown.insert(id.clone(), record.markdown.clone());
                    revisions.push(DocumentMutationRevision {
                        document_id: id,
                        previous_revision: Some(current_revision),
                        revision: Some(record.state.revision()),
                    });
                    records.push(record);
                }
                None => {
                    documents.remove(&id);
                    markdown.remove(&id);
                    revisions.push(DocumentMutationRevision {
                        document_id: id,
                        previous_revision: Some(current_revision),
                        revision: None,
                    });
                }
            }
        }
        let commit = DocumentMutationCommit {
            mutation_id: format!("undo:{undo_token}"),
            undo_token: undo_token.into(),
            records,
            revisions,
            provenance: AgentMutationProvenance {
                actor: contracts::AgentActor::User,
                mutation_id: format!("undo:{undo_token}"),
                skill: Some(contracts::AgentSkillSelector {
                    id: "undo".into(),
                    version: 1,
                }),
                connection_id: None,
                protocol: None,
                requested_model: None,
                resolved_model: None,
                source_revisions: Vec::new(),
                applied_at_ms: now_ms(),
            },
            replayed: false,
        };
        let undone_mutation_id = mutations
            .by_mutation_id
            .iter()
            .find(|(_, (_, commit))| commit.undo_token == undo_token)
            .map(|(_, (_, original_commit))| original_commit.mutation_id.clone());
        if let Some(mutation_id) = undone_mutation_id {
            mutations.undone_mutation_ids.insert(mutation_id);
        }
        mutations.undo.remove(undo_token);
        mutations
            .undo_commits
            .insert(undo_token.to_string(), commit.clone());
        Ok(commit)
    }

    fn save_pending_agent_mutation(
        &self,
        pending: application::PendingDocumentMutation,
    ) -> Result<(), DocumentMutationStoreError> {
        let _transaction_guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        validate_opaque_token(&pending.confirmation_id)?;
        let mut mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        if mutations.pending.contains_key(&pending.confirmation_id) {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        mutations
            .pending
            .insert(pending.confirmation_id.clone(), pending);
        Ok(())
    }

    fn take_pending_agent_mutation(
        &self,
        confirmation_id: &str,
    ) -> Result<Option<application::PendingDocumentMutation>, DocumentMutationStoreError> {
        let _transaction_guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        validate_opaque_token(confirmation_id)?;
        self.mutations
            .lock()
            .map_err(|_| DocumentMutationStoreError::Transaction("mutation lock poisoned".into()))
            .map(|mut mutations| mutations.pending.remove(confirmation_id))
    }

    fn find_pending_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<application::PendingDocumentMutation>, DocumentMutationStoreError> {
        let mut mutations = self.mutations.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let now = now_ms();
        mutations
            .pending
            .retain(|_, pending| pending.expires_at_ms > now);
        let Some(pending) = mutations
            .pending
            .values()
            .find(|pending| pending.batch.mutation_id == mutation_id)
        else {
            return Ok(None);
        };
        if pending.batch.fingerprint != fingerprint {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        Ok(Some(pending.clone()))
    }
}

#[derive(Clone)]
pub struct FileDocumentRepository {
    scanner: DocumentScanner,
    transaction_lock: Arc<RwLock<()>>,
    #[cfg(test)]
    failure_point: Arc<Mutex<Option<MutationWritePoint>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MutationWritePoint {
    PreparedJournal,
    StagedState(String),
    StagedMarkdown(String),
    BackedUp(String),
    Published(String),
    AppliedJournal,
    UndoPreparedJournal,
    UndoAppliedJournal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MutationJournalStatus {
    Prepared,
    Applied,
    UndoPrepared,
    Undone,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MutationBeforeRecord {
    document_id: String,
    record: Option<DocumentRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentMutationJournal {
    version: u32,
    status: MutationJournalStatus,
    batch: DocumentMutationBatch,
    before: Vec<MutationBeforeRecord>,
    commit: Option<DocumentMutationCommit>,
    undo_before: Vec<MutationBeforeRecord>,
    undo_commit: Option<DocumentMutationCommit>,
}

struct RepositoryLock(File);

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl FileDocumentRepository {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let repository = Self {
            scanner: DocumentScanner::new(root),
            transaction_lock: Arc::new(RwLock::new(())),
            #[cfg(test)]
            failure_point: Arc::new(Mutex::new(None)),
        };
        let _lock = repository.acquire_repository_lock()?;
        repository.recover_agent_mutations()?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        self.scanner.root()
    }

    pub fn create_watcher(&self) -> Result<DocumentWatcher, String> {
        DocumentWatcher::new(self.root(), self.transaction_lock.clone())
    }

    pub fn list_agent_trash(&self) -> Result<Vec<String>, String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let root = self.mutation_root().join("trash");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut names = fs::read_dir(root)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    pub fn restore_agent_trash(&self, trash_name: &str, document_id: &str) -> Result<(), String> {
        validate_opaque_token(trash_name).map_err(|error| error.to_string())?;
        validate_document_id(document_id).map_err(|error| error.to_string())?;
        let _lock = self.acquire_repository_lock()?;
        let source = self.mutation_root().join("trash").join(trash_name);
        let destination = self.root().join(document_id);
        if !source.is_dir() {
            return Err("mutation trash entry was not found".into());
        }
        if destination.exists() {
            return Err("cannot restore trash over an existing document package".into());
        }
        fs::rename(source, destination).map_err(|error| error.to_string())
    }

    pub fn purge_agent_trash(&self, trash_name: &str) -> Result<(), String> {
        validate_opaque_token(trash_name).map_err(|error| error.to_string())?;
        let _lock = self.acquire_repository_lock()?;
        let target = self.mutation_root().join("trash").join(trash_name);
        if !target.is_dir() {
            return Err("mutation trash entry was not found".into());
        }
        fs::remove_dir_all(target).map_err(|error| error.to_string())
    }

    fn snapshot_unlocked(&self) -> Result<DocumentSnapshot, String> {
        let scan = self.scanner.scan().map_err(|error| error.to_string())?;
        let documents = scan.documents();
        let issues = scan
            .errors
            .into_iter()
            .map(|error| DocumentIssue::from_kind(self.root(), &error))
            .collect();
        Ok(DocumentSnapshot { documents, issues })
    }

    fn acquire_repository_lock(&self) -> Result<RepositoryLock, String> {
        acquire_repository_lock_at(self.root())
    }

    fn read_unlocked(&self, document_id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        validate_document_id(document_id.as_str()).map_err(|error| error.to_string())?;
        let directory = self.root().join(document_id.as_str());
        DocumentPackage::load(directory)
            .map(Some)
            .or_else(|error| {
                if matches!(error, DocumentPackageError::NotDirectory(_)) {
                    Ok(None)
                } else {
                    Err(error.to_string())
                }
            })
            .map(|package| {
                package.map(|package| DocumentRecord {
                    state: package.state,
                    markdown: package.markdown,
                })
            })
    }

    fn save_unlocked(
        &self,
        mut document: DocumentState,
        markdown: Option<String>,
    ) -> Result<(), String> {
        validate_document_id(document.id().as_str()).map_err(|error| error.to_string())?;
        let directory = self.root().join(document.id().as_str());
        let existing = DocumentPackage::load(&directory).ok();
        if let Some(existing) = &existing {
            document.schema.revision = existing.state.revision().saturating_add(1);
        }
        document.schema.updated_at_ms = now_ms();
        let markdown = markdown
            .or_else(|| existing.map(|package| package.markdown))
            .unwrap_or_else(|| format!("# {}\n\n", document.title()));
        DocumentPackage::new(directory, document, markdown)
            .map_err(|error| error.to_string())?
            .save()
            .map_err(|error| error.to_string())
    }

    fn mutation_root(&self) -> PathBuf {
        self.root().join(AGENT_MUTATION_DIRECTORY)
    }

    fn journal_path(&self, undo_token: &str) -> PathBuf {
        self.mutation_root().join(format!("{undo_token}.json"))
    }

    fn pending_root(&self) -> PathBuf {
        self.mutation_root().join("pending")
    }

    fn pending_path(&self, confirmation_id: &str) -> PathBuf {
        self.pending_root().join(format!("{confirmation_id}.json"))
    }

    fn load_journals(&self) -> Result<Vec<(PathBuf, AgentMutationJournal)>, String> {
        let root = self.mutation_root();
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(&root)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).map_err(|error| error.to_string())?;
                let journal = serde_json::from_slice(&bytes).map_err(|error| {
                    format!("invalid agent mutation journal {}: {error}", path.display())
                })?;
                Ok((path, journal))
            })
            .collect()
    }

    fn write_journal(
        &self,
        path: &Path,
        journal: &AgentMutationJournal,
    ) -> Result<(), DocumentMutationStoreError> {
        ensure_private_directory(&self.mutation_root()).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot create mutation journal directory: {error}"
            ))
        })?;
        let bytes = serde_json::to_vec_pretty(journal).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot serialize mutation journal: {error}"
            ))
        })?;
        write_atomic_bytes(path, &bytes).map_err(DocumentMutationStoreError::Transaction)
    }

    fn remove_expired_pending(&self, current_time_ms: u64) -> Result<(), String> {
        let root = self.pending_root();
        if !root.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            let pending: application::PendingDocumentMutation = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid pending mutation {}: {error}", path.display()))?;
            if pending.expires_at_ms <= current_time_ms {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn inject_failure_once(&self, point: MutationWritePoint) {
        *self.failure_point.lock().unwrap() = Some(point);
    }

    #[cfg(test)]
    fn fail_at(&self, point: MutationWritePoint) -> Result<(), DocumentMutationStoreError> {
        if std::env::var("MEETNOTE_TEST_MUTATION_FAILURE_POINT")
            .ok()
            .as_deref()
            == Some(mutation_write_point_name(&point))
        {
            return Err(DocumentMutationStoreError::Transaction(format!(
                "process-injected mutation failure at {point:?}"
            )));
        }
        let mut configured = self.failure_point.lock().map_err(|_| {
            DocumentMutationStoreError::Transaction("failure injection lock poisoned".into())
        })?;
        if configured.as_ref() == Some(&point) {
            *configured = None;
            return Err(DocumentMutationStoreError::Transaction(format!(
                "injected mutation failure at {point:?}"
            )));
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn fail_at(&self, _point: MutationWritePoint) -> Result<(), DocumentMutationStoreError> {
        Ok(())
    }

    fn recover_agent_mutations(&self) -> Result<(), String> {
        for (path, mut journal) in self.load_journals()? {
            match journal.status {
                MutationJournalStatus::Prepared => {
                    self.restore_records(&journal.before)
                        .map_err(|error| error.to_string())?;
                    journal.status = MutationJournalStatus::RolledBack;
                    self.write_journal(&path, &journal)
                        .map_err(|error| error.to_string())?;
                }
                MutationJournalStatus::UndoPrepared => {
                    self.restore_records(&journal.undo_before)
                        .map_err(|error| error.to_string())?;
                    journal.status = MutationJournalStatus::Applied;
                    self.write_journal(&path, &journal)
                        .map_err(|error| error.to_string())?;
                }
                MutationJournalStatus::Applied
                | MutationJournalStatus::Undone
                | MutationJournalStatus::RolledBack => {}
            }
        }
        self.remove_abandoned_transaction_directories()
    }

    fn remove_abandoned_transaction_directories(&self) -> Result<(), String> {
        let root = self.mutation_root();
        if !root.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("transaction-"))
            {
                fs::remove_dir_all(path).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn restore_records(
        &self,
        records: &[MutationBeforeRecord],
    ) -> Result<(), DocumentMutationStoreError> {
        let replacements = records
            .iter()
            .map(|before| (before.document_id.clone(), before.record.clone()))
            .collect::<Vec<_>>();
        self.replace_records_atomically("recovery", &replacements)
    }

    fn replace_records_atomically(
        &self,
        operation_id: &str,
        replacements: &[(String, Option<DocumentRecord>)],
    ) -> Result<(), DocumentMutationStoreError> {
        let workspace = self.mutation_root().join(format!(
            "transaction-{}-{}",
            sanitize_transaction_component(operation_id),
            std::process::id()
        ));
        if workspace.exists() {
            fs::remove_dir_all(&workspace).map_err(|error| {
                DocumentMutationStoreError::Transaction(format!(
                    "cannot clean transaction workspace: {error}"
                ))
            })?;
        }
        let staged = workspace.join("staged");
        let backup = workspace.join("backup");
        fs::create_dir_all(&staged).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot create transaction staging directory: {error}"
            ))
        })?;
        let stage_result = (|| {
            for (id, record) in replacements {
                validate_document_id(id)?;
                if let Some(record) = record {
                    self.stage_record(&staged, id, record)?;
                }
            }
            fs::create_dir_all(&backup).map_err(|error| {
                DocumentMutationStoreError::Transaction(format!(
                    "cannot create transaction backup directory: {error}"
                ))
            })?;
            Ok(())
        })();
        if let Err(error) = stage_result {
            let _ = fs::remove_dir_all(&workspace);
            return Err(error);
        }
        let mut swapped = Vec::new();
        let result = (|| {
            for (id, record) in replacements {
                let target = self.root().join(id);
                let backup_target = backup.join(id);
                if target.exists() {
                    fs::rename(&target, &backup_target).map_err(|error| {
                        DocumentMutationStoreError::Transaction(format!(
                            "cannot back up document {id}: {error}"
                        ))
                    })?;
                    swapped.push(id.clone());
                    self.fail_at(MutationWritePoint::BackedUp(id.clone()))?;
                } else {
                    swapped.push(id.clone());
                }
                if record.is_some() {
                    if backup_target.exists() {
                        move_unmanaged_entries(&backup_target, &staged.join(id))?;
                    }
                    fs::rename(staged.join(id), &target).map_err(|error| {
                        DocumentMutationStoreError::Transaction(format!(
                            "cannot publish document {id}: {error}"
                        ))
                    })?;
                    self.fail_at(MutationWritePoint::Published(id.clone()))?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            for id in swapped.iter().rev() {
                let target = self.root().join(id);
                let backup_target = backup.join(id);
                if target.exists() {
                    if backup_target.exists() {
                        let _ = move_unmanaged_entries(&target, &backup_target);
                    }
                    let _ = fs::remove_dir_all(&target);
                }
                if backup_target.exists() {
                    let _ = fs::rename(&backup_target, &target);
                }
            }
            let _ = fs::remove_dir_all(&workspace);
            return Err(error);
        }
        for (id, record) in replacements {
            if record.is_none() {
                let backup_target = backup.join(id);
                if backup_target.exists() {
                    let trash_root = self.mutation_root().join("trash");
                    fs::create_dir_all(&trash_root).map_err(|error| {
                        DocumentMutationStoreError::Transaction(format!(
                            "cannot create mutation trash directory: {error}"
                        ))
                    })?;
                    let trash_target = trash_root.join(format!(
                        "{}-{}-{}",
                        sanitize_transaction_component(operation_id),
                        sanitize_transaction_component(id),
                        std::process::id()
                    ));
                    fs::rename(&backup_target, &trash_target).map_err(|error| {
                        DocumentMutationStoreError::Transaction(format!(
                            "cannot preserve undone document {id}: {error}"
                        ))
                    })?;
                }
            }
        }
        let _ = fs::remove_dir_all(&workspace);
        Ok(())
    }

    fn stage_record(
        &self,
        staged: &Path,
        id: &str,
        record: &DocumentRecord,
    ) -> Result<(), DocumentMutationStoreError> {
        if record.state.id().as_str() != id {
            return Err(DocumentMutationStoreError::Transaction(format!(
                "mutation record ID {} does not match target {id}",
                record.state.id().as_str()
            )));
        }
        let package = DocumentPackage::new(
            staged.join(id),
            record.state.clone(),
            record.markdown.clone(),
        )
        .map_err(|error| DocumentMutationStoreError::Transaction(error.to_string()))?;
        fs::create_dir_all(package.directory()).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot create staged document {id}: {error}"
            ))
        })?;
        let state = serde_json::to_vec_pretty(&package.state).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot serialize staged document {id}: {error}"
            ))
        })?;
        write_atomic(&package.state_path(), &state)
            .map_err(|error| DocumentMutationStoreError::Transaction(error.to_string()))?;
        self.fail_at(MutationWritePoint::StagedState(id.into()))?;
        write_atomic(&package.markdown_path(), package.markdown.as_bytes())
            .map_err(|error| DocumentMutationStoreError::Transaction(error.to_string()))?;
        self.fail_at(MutationWritePoint::StagedMarkdown(id.into()))?;
        Ok(())
    }
}

#[cfg(test)]
fn mutation_write_point_name(point: &MutationWritePoint) -> &'static str {
    match point {
        MutationWritePoint::PreparedJournal => "prepared_journal",
        MutationWritePoint::StagedState(_) => "staged_state",
        MutationWritePoint::StagedMarkdown(_) => "staged_markdown",
        MutationWritePoint::BackedUp(_) => "backed_up",
        MutationWritePoint::Published(_) => "published",
        MutationWritePoint::AppliedJournal => "applied_journal",
        MutationWritePoint::UndoPreparedJournal => "undo_prepared_journal",
        MutationWritePoint::UndoAppliedJournal => "undo_applied_journal",
    }
}

fn move_unmanaged_entries(from: &Path, to: &Path) -> Result<(), DocumentMutationStoreError> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|error| {
        DocumentMutationStoreError::Transaction(format!(
            "cannot create attachment preservation directory: {error}"
        ))
    })?;
    for entry in fs::read_dir(from).map_err(|error| {
        DocumentMutationStoreError::Transaction(format!(
            "cannot inspect document attachments: {error}"
        ))
    })? {
        let entry = entry.map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot inspect document attachment entry: {error}"
            ))
        })?;
        let name = entry.file_name();
        if name == "state.json" || name == "document.md" {
            continue;
        }
        fs::rename(entry.path(), to.join(name)).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot preserve document attachment: {error}"
            ))
        })?;
    }
    Ok(())
}

pub struct DocumentWatcher {
    receiver: Receiver<Result<DocumentWatchEvent, String>>,
    root: PathBuf,
    transaction_lock: Arc<RwLock<()>>,
    _watcher: RecommendedWatcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentWatchEvent {
    Changed,
}

impl DocumentWatcher {
    fn new(root: &Path, transaction_lock: Arc<RwLock<()>>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let mutation_root = root.join(AGENT_MUTATION_DIRECTORY);
        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                let event = match result {
                    Ok(event)
                        if !event.paths.is_empty()
                            && event
                                .paths
                                .iter()
                                .all(|path| path.starts_with(&mutation_root)) =>
                    {
                        return;
                    }
                    Ok(_) => Ok(DocumentWatchEvent::Changed),
                    Err(error) => Err(error.to_string()),
                };
                let _ = sender.send(event);
            },
            Config::default(),
        )
        .map_err(|error| error.to_string())?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            receiver,
            root: root.to_path_buf(),
            transaction_lock,
            _watcher: watcher,
        })
    }

    fn wait_for_committed_view(&self) -> Result<(), String> {
        let _repository_lock = acquire_repository_lock_at(&self.root)?;
        drop(
            self.transaction_lock
                .read()
                .map_err(|_| "mutation lock poisoned".to_string())?,
        );
        Ok(())
    }

    pub fn try_recv(&self) -> Option<Result<DocumentWatchEvent, String>> {
        let event = self.receiver.try_recv().ok()?;
        Some(event.and_then(|event| {
            self.wait_for_committed_view()?;
            Ok(event)
        }))
    }

    pub fn recv(&self) -> Result<DocumentWatchEvent, String> {
        let event = self.receiver.recv().map_err(|error| error.to_string())??;
        self.wait_for_committed_view()?;
        Ok(event)
    }

    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<DocumentWatchEvent>, String> {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => {
                let event = event?;
                self.wait_for_committed_view()?;
                Ok(Some(event))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("document watcher disconnected".into())
            }
        }
    }
}

/// Watches `root` and pushes each change into the returned stream from the
/// notify callback itself, once the writer's transaction is visible as in
/// `DocumentWatcher::recv`. A watcher error ends the stream.
fn subscribe_document_changes(
    root: &Path,
    transaction_lock: Arc<RwLock<()>>,
) -> Result<application::DocumentChanges, String> {
    let (sender, changes) = futures::channel::mpsc::unbounded();
    let repository_root = root.to_path_buf();
    let mutation_root = root.join(AGENT_MUTATION_DIRECTORY);
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| match result {
            Ok(event)
                if !event.paths.is_empty()
                    && event
                        .paths
                        .iter()
                        .all(|path| path.starts_with(&mutation_root)) => {}
            Ok(_) => {
                let committed = acquire_repository_lock_at(&repository_root).and_then(|_lock| {
                    transaction_lock
                        .read()
                        .map(drop)
                        .map_err(|_| "mutation lock poisoned".to_string())
                });
                match committed {
                    Ok(()) => {
                        let _ = sender.unbounded_send(DocumentChange::Changed);
                    }
                    Err(_) => sender.close_channel(),
                }
            }
            Err(_) => sender.close_channel(),
        },
        Config::default(),
    )
    .map_err(|error| error.to_string())?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;
    Ok(application::DocumentChanges::new(changes, watcher))
}

impl DocumentRepository for FileDocumentRepository {
    fn agent_mutation_document_ids(
        &self,
        undo_token: &str,
    ) -> Result<Vec<String>, DocumentMutationStoreError> {
        validate_opaque_token(undo_token)?;
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.read().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let bytes = fs::read(self.journal_path(undo_token)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DocumentMutationStoreError::UndoNotFound
            } else {
                DocumentMutationStoreError::Transaction(error.to_string())
            }
        })?;
        let journal: AgentMutationJournal = serde_json::from_slice(&bytes)
            .map_err(|error| DocumentMutationStoreError::Transaction(error.to_string()))?;
        let commit = journal
            .commit
            .ok_or(DocumentMutationStoreError::UndoNotFound)?;
        Ok(commit
            .revisions
            .iter()
            .filter(|revision| revision.previous_revision.is_some())
            .map(|revision| revision.document_id.clone())
            .collect())
    }

    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        self.snapshot_unlocked()
    }

    fn scan_metadata(
        &self,
        visitor: &mut dyn FnMut(DocumentMetadata) -> Result<bool, String>,
    ) -> Result<(), String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        self.scanner.scan_metadata(visitor)
    }

    fn get(&self, document_id: &DocumentId) -> Result<Option<DocumentState>, String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        Ok(self
            .snapshot_unlocked()?
            .documents
            .into_iter()
            .find(|document| document.id() == document_id))
    }

    fn read(&self, document_id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        self.read_unlocked(document_id)
    }

    fn watch(&self) -> Result<Receiver<DocumentChange>, String> {
        let watcher = self.create_watcher()?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while watcher.recv().is_ok() {
                if sender.send(DocumentChange::Changed).is_err() {
                    break;
                }
            }
        });
        Ok(receiver)
    }

    fn subscribe_changes(&self) -> Result<application::DocumentChanges, String> {
        subscribe_document_changes(self.root(), self.transaction_lock.clone())
    }

    fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
        let _repository_lock = self.acquire_repository_lock()?;
        let _guard = self
            .transaction_lock
            .write()
            .map_err(|_| "mutation lock poisoned".to_string())?;
        self.save_unlocked(document, markdown)
    }

    fn commit_agent_mutation(
        &self,
        mut batch: DocumentMutationBatch,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let journals = self
            .load_journals()
            .map_err(DocumentMutationStoreError::Transaction)?;
        if let Some((_, journal)) = journals
            .iter()
            .find(|(_, journal)| journal.batch.mutation_id == batch.mutation_id)
        {
            if journal.batch.fingerprint != batch.fingerprint {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            if journal.status == MutationJournalStatus::Applied {
                let mut commit = journal.commit.clone().ok_or_else(|| {
                    DocumentMutationStoreError::Transaction(
                        "applied mutation journal has no commit".into(),
                    )
                })?;
                commit.replayed = true;
                return Ok(commit);
            }
            if journal.status == MutationJournalStatus::Undone {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
        }

        validate_mutation_batch(&batch, |id| {
            self.read_unlocked(&DocumentId::new(id))
                .map_err(DocumentMutationStoreError::Transaction)
        })?;
        let scan = self
            .scanner
            .scan()
            .map_err(|error| DocumentMutationStoreError::Transaction(error.to_string()))?;
        // A stray file beside the packages is reported but is not part of
        // the tree, so it does not block tree mutations.
        if scan
            .errors
            .iter()
            .any(|error| !matches!(error, DocumentScanError::UnexpectedFile { .. }))
        {
            return Err(DocumentMutationStoreError::Transaction(
                "cannot mutate a document tree that already has relation errors".into(),
            ));
        }
        let mut post_states = scan.documents();
        apply_writes_to_states(&mut post_states, &batch);
        validate_state_relations(&post_states)?;
        let before = batch
            .writes
            .iter()
            .map(|write| {
                let document_id = write.record.state.id().as_str().to_string();
                let record = self
                    .read_unlocked(&DocumentId::new(document_id.clone()))
                    .map_err(DocumentMutationStoreError::Transaction)?;
                Ok(MutationBeforeRecord {
                    document_id,
                    record,
                })
            })
            .collect::<Result<Vec<_>, DocumentMutationStoreError>>()?;
        batch.provenance.applied_at_ms = now_ms();
        let commit = DocumentMutationCommit {
            mutation_id: batch.mutation_id.clone(),
            undo_token: batch.undo_token.clone(),
            records: batch
                .writes
                .iter()
                .map(|write| write.record.clone())
                .collect(),
            revisions: batch
                .writes
                .iter()
                .map(|write| DocumentMutationRevision {
                    document_id: write.record.state.id().as_str().into(),
                    previous_revision: write.expected_revision,
                    revision: Some(write.record.state.revision()),
                })
                .collect(),
            provenance: batch.provenance.clone(),
            replayed: false,
        };
        let path = self.journal_path(&batch.undo_token);
        let mut journal = AgentMutationJournal {
            version: AGENT_MUTATION_JOURNAL_VERSION,
            status: MutationJournalStatus::Prepared,
            batch,
            before,
            commit: None,
            undo_before: Vec::new(),
            undo_commit: None,
        };
        self.fail_at(MutationWritePoint::PreparedJournal)?;
        self.write_journal(&path, &journal)?;
        let replacements = journal
            .batch
            .writes
            .iter()
            .map(|write| {
                (
                    write.record.state.id().as_str().to_string(),
                    Some(write.record.clone()),
                )
            })
            .collect::<Vec<_>>();
        if let Err(error) =
            self.replace_records_atomically(&journal.batch.undo_token, &replacements)
        {
            journal.status = MutationJournalStatus::RolledBack;
            let _ = self.write_journal(&path, &journal);
            return Err(error);
        }
        journal.status = MutationJournalStatus::Applied;
        journal.commit = Some(commit.clone());
        if let Err(error) = self.fail_at(MutationWritePoint::AppliedJournal) {
            let rollback = self.restore_records(&journal.before);
            journal.status = MutationJournalStatus::RolledBack;
            let _ = self.write_journal(&path, &journal);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(DocumentMutationStoreError::Transaction(format!(
                    "{error}; rollback also failed: {rollback_error}"
                ))),
            };
        }
        if let Err(error) = self.write_journal(&path, &journal) {
            let rollback = self.restore_records(&journal.before);
            journal.status = MutationJournalStatus::RolledBack;
            let _ = self.write_journal(&path, &journal);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(DocumentMutationStoreError::Transaction(format!(
                    "{error}; rollback also failed: {rollback_error}"
                ))),
            };
        }
        Ok(commit)
    }

    fn find_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.read().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let journals = self
            .load_journals()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let Some((_, journal)) = journals
            .iter()
            .find(|(_, journal)| journal.batch.mutation_id == mutation_id)
        else {
            return Ok(None);
        };
        if journal.batch.fingerprint != fingerprint {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        if journal.status == MutationJournalStatus::Undone {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        if journal.status != MutationJournalStatus::Applied {
            return Ok(None);
        }
        let mut commit = journal.commit.clone().ok_or_else(|| {
            DocumentMutationStoreError::Transaction("applied mutation journal has no commit".into())
        })?;
        commit.replayed = true;
        Ok(Some(commit))
    }

    fn undo_agent_mutation(
        &self,
        undo_token: &str,
    ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        validate_opaque_token(undo_token)?;
        let _guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        let path = self.journal_path(undo_token);
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DocumentMutationStoreError::UndoNotFound
            } else {
                DocumentMutationStoreError::Transaction(error.to_string())
            }
        })?;
        let mut journal: AgentMutationJournal =
            serde_json::from_slice(&bytes).map_err(|error| {
                DocumentMutationStoreError::Transaction(format!(
                    "invalid agent mutation journal: {error}"
                ))
            })?;
        if journal.status == MutationJournalStatus::Undone {
            let mut commit = journal
                .undo_commit
                .ok_or(DocumentMutationStoreError::UndoNotFound)?;
            commit.replayed = true;
            return Ok(commit);
        }
        if journal.status != MutationJournalStatus::Applied {
            return Err(DocumentMutationStoreError::UndoNotFound);
        }
        let journals = self
            .load_journals()
            .map_err(DocumentMutationStoreError::Transaction)?;

        let mut undo_before = Vec::with_capacity(journal.before.len());
        let mut replacements = Vec::with_capacity(journal.before.len());
        let mut revisions = Vec::with_capacity(journal.before.len());
        let mut records = Vec::new();
        for before in &journal.before {
            let current = self
                .read_unlocked(&DocumentId::new(before.document_id.clone()))
                .map_err(DocumentMutationStoreError::Transaction)?
                .ok_or_else(|| DocumentMutationStoreError::NotFound {
                    document_id: before.document_id.clone(),
                })?;
            let committed_revision = journal
                .commit
                .as_ref()
                .and_then(|commit| {
                    commit
                        .revisions
                        .iter()
                        .find(|revision| revision.document_id == before.document_id)
                })
                .and_then(|revision| revision.revision)
                .ok_or_else(|| {
                    DocumentMutationStoreError::Transaction(
                        "mutation journal is missing a committed revision".into(),
                    )
                })?;
            let committed = journal
                .commit
                .as_ref()
                .and_then(|commit| {
                    commit
                        .records
                        .iter()
                        .find(|record| record.state.id().as_str() == before.document_id)
                })
                .ok_or_else(|| {
                    DocumentMutationStoreError::Transaction(
                        "mutation journal is missing a committed record".into(),
                    )
                })?;
            let expected_revision = expected_file_undo_revision(
                &journal,
                &journals,
                &before.document_id,
                committed_revision,
            )?;
            if current.state.revision() != expected_revision
                || !records_match_except_revision(&current, committed)
            {
                return Err(DocumentMutationStoreError::RevisionConflict {
                    document_id: before.document_id.clone(),
                    expected: expected_revision,
                    actual: current.state.revision(),
                });
            }
            undo_before.push(MutationBeforeRecord {
                document_id: before.document_id.clone(),
                record: Some(current.clone()),
            });
            match &before.record {
                Some(original) => {
                    let mut restored = original.clone();
                    restored.state.schema.revision = expected_revision.saturating_add(1);
                    restored.state.schema.updated_at_ms = now_ms();
                    revisions.push(DocumentMutationRevision {
                        document_id: before.document_id.clone(),
                        previous_revision: Some(expected_revision),
                        revision: Some(restored.state.revision()),
                    });
                    records.push(restored.clone());
                    replacements.push((before.document_id.clone(), Some(restored)));
                }
                None => {
                    revisions.push(DocumentMutationRevision {
                        document_id: before.document_id.clone(),
                        previous_revision: Some(expected_revision),
                        revision: None,
                    });
                    replacements.push((before.document_id.clone(), None));
                }
            }
        }
        let mutation_id = format!("undo:{undo_token}");
        let commit = DocumentMutationCommit {
            mutation_id: mutation_id.clone(),
            undo_token: undo_token.into(),
            records,
            revisions,
            provenance: AgentMutationProvenance {
                actor: contracts::AgentActor::User,
                mutation_id,
                skill: Some(contracts::AgentSkillSelector {
                    id: "undo".into(),
                    version: 1,
                }),
                connection_id: None,
                protocol: None,
                requested_model: None,
                resolved_model: None,
                source_revisions: Vec::new(),
                applied_at_ms: now_ms(),
            },
            replayed: false,
        };
        journal.status = MutationJournalStatus::UndoPrepared;
        journal.undo_before = undo_before;
        self.fail_at(MutationWritePoint::UndoPreparedJournal)?;
        self.write_journal(&path, &journal)?;
        if let Err(error) = self.replace_records_atomically(undo_token, &replacements) {
            journal.status = MutationJournalStatus::Applied;
            journal.undo_before.clear();
            let _ = self.write_journal(&path, &journal);
            return Err(error);
        }
        journal.status = MutationJournalStatus::Undone;
        journal.undo_commit = Some(commit.clone());
        if let Err(error) = self.fail_at(MutationWritePoint::UndoAppliedJournal) {
            let rollback = self.restore_records(&journal.undo_before);
            journal.status = MutationJournalStatus::Applied;
            journal.undo_before.clear();
            let _ = self.write_journal(&path, &journal);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(DocumentMutationStoreError::Transaction(format!(
                    "{error}; undo rollback also failed: {rollback_error}"
                ))),
            };
        }
        if let Err(error) = self.write_journal(&path, &journal) {
            let rollback = self.restore_records(&journal.undo_before);
            journal.status = MutationJournalStatus::Applied;
            journal.undo_before.clear();
            let _ = self.write_journal(&path, &journal);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(DocumentMutationStoreError::Transaction(format!(
                    "{error}; undo rollback also failed: {rollback_error}"
                ))),
            };
        }
        Ok(commit)
    }

    fn save_pending_agent_mutation(
        &self,
        pending: application::PendingDocumentMutation,
    ) -> Result<(), DocumentMutationStoreError> {
        validate_opaque_token(&pending.confirmation_id)?;
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        ensure_private_directory(&self.mutation_root()).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot create mutation directory: {error}"
            ))
        })?;
        ensure_private_directory(&self.pending_root()).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot create pending mutation directory: {error}"
            ))
        })?;
        self.remove_expired_pending(now_ms())
            .map_err(DocumentMutationStoreError::Transaction)?;
        let path = self.pending_path(&pending.confirmation_id);
        if path.exists() {
            return Err(DocumentMutationStoreError::IdempotencyConflict);
        }
        let bytes = serde_json::to_vec_pretty(&pending).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot serialize pending mutation: {error}"
            ))
        })?;
        write_atomic_bytes(&path, &bytes).map_err(DocumentMutationStoreError::Transaction)
    }

    fn take_pending_agent_mutation(
        &self,
        confirmation_id: &str,
    ) -> Result<Option<application::PendingDocumentMutation>, DocumentMutationStoreError> {
        validate_opaque_token(confirmation_id)?;
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        self.remove_expired_pending(now_ms())
            .map_err(DocumentMutationStoreError::Transaction)?;
        let path = self.pending_path(confirmation_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(DocumentMutationStoreError::Transaction(format!(
                    "cannot read pending mutation: {error}"
                )));
            }
        };
        let pending = serde_json::from_slice(&bytes).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!("invalid pending mutation: {error}"))
        })?;
        fs::remove_file(&path).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot consume pending mutation: {error}"
            ))
        })?;
        Ok(Some(pending))
    }

    fn find_pending_agent_mutation(
        &self,
        mutation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<application::PendingDocumentMutation>, DocumentMutationStoreError> {
        let _repository_lock = self
            .acquire_repository_lock()
            .map_err(DocumentMutationStoreError::Transaction)?;
        let _guard = self.transaction_lock.write().map_err(|_| {
            DocumentMutationStoreError::Transaction("mutation lock poisoned".into())
        })?;
        self.remove_expired_pending(now_ms())
            .map_err(DocumentMutationStoreError::Transaction)?;
        let root = self.pending_root();
        if !root.is_dir() {
            return Ok(None);
        }
        for entry in fs::read_dir(root).map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "cannot read pending mutations: {error}"
            ))
        })? {
            let entry = entry.map_err(|error| {
                DocumentMutationStoreError::Transaction(format!(
                    "cannot read pending mutation: {error}"
                ))
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| {
                DocumentMutationStoreError::Transaction(format!(
                    "cannot read pending mutation: {error}"
                ))
            })?;
            let pending: application::PendingDocumentMutation = serde_json::from_slice(&bytes)
                .map_err(|error| {
                    DocumentMutationStoreError::Transaction(format!(
                        "invalid pending mutation: {error}"
                    ))
                })?;
            if pending.batch.mutation_id != mutation_id {
                continue;
            }
            if pending.batch.fingerprint != fingerprint {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            return Ok(Some(pending));
        }
        Ok(None)
    }
}

fn records_match_except_revision(current: &DocumentRecord, committed: &DocumentRecord) -> bool {
    if current.markdown != committed.markdown {
        return false;
    }
    let mut current_state = current.state.clone();
    current_state.schema.revision = committed.state.schema.revision;
    current_state.schema.updated_at_ms = committed.state.schema.updated_at_ms;
    current_state == committed.state
}

fn expected_memory_undo_revision(
    mutations: &InMemoryMutationState,
    original_commit: &DocumentMutationCommit,
    document_id: &str,
    committed_revision: u64,
) -> Result<u64, DocumentMutationStoreError> {
    let mut expected = committed_revision;
    loop {
        let next = mutations
            .by_mutation_id
            .values()
            .filter(|(_, commit)| commit.mutation_id != original_commit.mutation_id)
            .filter_map(|(_, commit)| {
                let applied = commit
                    .revisions
                    .iter()
                    .find(|revision| revision.document_id == document_id)?;
                (applied.previous_revision == Some(expected)).then_some((commit, applied))
            })
            .next();
        let Some((commit, applied)) = next else {
            break;
        };
        if !mutations.undone_mutation_ids.contains(&commit.mutation_id) {
            break;
        }
        let applied_revision = applied.revision.ok_or_else(|| {
            DocumentMutationStoreError::Transaction(
                "later mutation commit unexpectedly deleted a document".into(),
            )
        })?;
        let undo = mutations
            .undo_commits
            .get(&commit.undo_token)
            .and_then(|undo| {
                undo.revisions
                    .iter()
                    .find(|revision| revision.document_id == document_id)
            })
            .ok_or_else(|| {
                DocumentMutationStoreError::Transaction(
                    "undone mutation is missing its undo revision".into(),
                )
            })?;
        if undo.previous_revision != Some(applied_revision) {
            return Err(DocumentMutationStoreError::Transaction(
                "undo revision chain is inconsistent".into(),
            ));
        }
        expected = undo.revision.ok_or_else(|| {
            DocumentMutationStoreError::Transaction(
                "later undo unexpectedly deleted an existing document".into(),
            )
        })?;
    }
    Ok(expected)
}

fn expected_file_undo_revision(
    original: &AgentMutationJournal,
    journals: &[(PathBuf, AgentMutationJournal)],
    document_id: &str,
    committed_revision: u64,
) -> Result<u64, DocumentMutationStoreError> {
    let mut expected = committed_revision;
    loop {
        let next = journals
            .iter()
            .map(|(_, journal)| journal)
            .filter(|journal| journal.batch.mutation_id != original.batch.mutation_id)
            .filter_map(|journal| {
                let applied = journal
                    .commit
                    .as_ref()?
                    .revisions
                    .iter()
                    .find(|revision| revision.document_id == document_id)?;
                (applied.previous_revision == Some(expected)).then_some((journal, applied))
            })
            .next();
        let Some((journal, applied)) = next else {
            break;
        };
        if journal.status != MutationJournalStatus::Undone {
            break;
        }
        let applied_revision = applied.revision.ok_or_else(|| {
            DocumentMutationStoreError::Transaction(
                "later mutation journal unexpectedly deleted a document".into(),
            )
        })?;
        let undo = journal
            .undo_commit
            .as_ref()
            .and_then(|undo| {
                undo.revisions
                    .iter()
                    .find(|revision| revision.document_id == document_id)
            })
            .ok_or_else(|| {
                DocumentMutationStoreError::Transaction(
                    "undone mutation journal is missing its undo revision".into(),
                )
            })?;
        if undo.previous_revision != Some(applied_revision) {
            return Err(DocumentMutationStoreError::Transaction(
                "undo journal revision chain is inconsistent".into(),
            ));
        }
        expected = undo.revision.ok_or_else(|| {
            DocumentMutationStoreError::Transaction(
                "later undo journal unexpectedly deleted an existing document".into(),
            )
        })?;
    }
    Ok(expected)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn validate_mutation_batch(
    batch: &DocumentMutationBatch,
    mut read: impl FnMut(&str) -> Result<Option<DocumentRecord>, DocumentMutationStoreError>,
) -> Result<(), DocumentMutationStoreError> {
    if batch.mutation_id.trim().is_empty()
        || batch.fingerprint.trim().is_empty()
        || batch.undo_token.trim().is_empty()
        || batch.writes.is_empty()
    {
        return Err(DocumentMutationStoreError::Transaction(
            "mutation identifiers and writes must not be empty".into(),
        ));
    }
    validate_opaque_token(&batch.undo_token)?;
    let mut seen = std::collections::BTreeSet::new();
    for write in &batch.writes {
        let id = write.record.state.id().as_str();
        validate_document_id(id)?;
        if !seen.insert(id.to_string()) {
            return Err(DocumentMutationStoreError::Transaction(format!(
                "mutation contains duplicate document {id}"
            )));
        }
        write.record.state.validate_schema().map_err(|error| {
            DocumentMutationStoreError::Transaction(format!(
                "invalid document state for {id}: {error:?}"
            ))
        })?;
        match (write.expected_revision, read(id)?) {
            (Some(expected), Some(current)) => {
                if current.state.revision() != expected {
                    return Err(DocumentMutationStoreError::RevisionConflict {
                        document_id: id.into(),
                        expected,
                        actual: current.state.revision(),
                    });
                }
                let required_revision = expected.saturating_add(1);
                if write.record.state.revision() != required_revision {
                    return Err(DocumentMutationStoreError::Transaction(format!(
                        "document {id} must advance revision from {expected} to {required_revision}"
                    )));
                }
            }
            (Some(_), None) => {
                return Err(DocumentMutationStoreError::NotFound {
                    document_id: id.into(),
                });
            }
            (None, Some(_)) => {
                return Err(DocumentMutationStoreError::AlreadyExists {
                    document_id: id.into(),
                });
            }
            (None, None) => {
                if write.record.state.revision() != 0 {
                    return Err(DocumentMutationStoreError::Transaction(format!(
                        "new document {id} must start at revision 0"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn apply_writes_to_states(states: &mut Vec<DocumentState>, batch: &DocumentMutationBatch) {
    for write in &batch.writes {
        if let Some(existing) = states
            .iter_mut()
            .find(|state| state.id() == write.record.state.id())
        {
            *existing = write.record.state.clone();
        } else {
            states.push(write.record.state.clone());
        }
    }
}

fn validate_state_relations(states: &[DocumentState]) -> Result<(), DocumentMutationStoreError> {
    let packages = states
        .iter()
        .cloned()
        .map(|state| DocumentPackage {
            directory: PathBuf::from(state.id().as_str()),
            markdown: String::new(),
            state,
        })
        .collect::<Vec<_>>();
    let errors = validate_relations(&packages);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DocumentMutationStoreError::Transaction(format!(
            "mutation would produce an invalid document tree: {:?}",
            errors[0]
        )))
    }
}

fn validate_document_id(id: &str) -> Result<(), DocumentMutationStoreError> {
    let path = Path::new(id);
    if id.is_empty()
        || id == "."
        || id == ".."
        || path.is_absolute()
        || path.components().count() != 1
        || id.starts_with('.')
    {
        return Err(DocumentMutationStoreError::Transaction(
            "document ID is not a safe package directory name".into(),
        ));
    }
    Ok(())
}

fn validate_opaque_token(token: &str) -> Result<(), DocumentMutationStoreError> {
    if token.is_empty()
        || token.starts_with('.')
        || token
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err(DocumentMutationStoreError::Transaction(
            "mutation token contains unsupported characters".into(),
        ));
    }
    Ok(())
}

fn sanitize_transaction_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .take(96)
        .collect()
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    set_private_file_permissions(&temporary)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(temporary);
        return Err(error.to_string());
    }
    Ok(())
}

fn acquire_repository_lock_at(root: &Path) -> Result<RepositoryLock, String> {
    let mutation_root = root.join(AGENT_MUTATION_DIRECTORY);
    fs::create_dir_all(&mutation_root).map_err(|error| error.to_string())?;
    let path = mutation_root.join("repository.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.lock_exclusive().map_err(|error| error.to_string())?;
    Ok(RepositoryLock(file))
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::{DocumentMutationBatch, DocumentMutationWrite};
    use contracts::{AgentActor, AgentMutationProvenance, AgentSkillSelector};
    use std::process::Command;
    use std::sync::mpsc;

    fn package_directory(root: &Path, id: &str) -> PathBuf {
        root.join(id)
    }

    fn mutation_batch(
        mutation_id: &str,
        fingerprint: &str,
        undo_token: &str,
        writes: Vec<DocumentMutationWrite>,
    ) -> DocumentMutationBatch {
        DocumentMutationBatch {
            mutation_id: mutation_id.into(),
            fingerprint: fingerprint.into(),
            undo_token: undo_token.into(),
            tool_name: "update_document".into(),
            provenance: AgentMutationProvenance {
                actor: AgentActor::ExternalAgent,
                mutation_id: mutation_id.into(),
                skill: Some(AgentSkillSelector {
                    id: "organize".into(),
                    version: 1,
                }),
                connection_id: None,
                protocol: None,
                requested_model: None,
                resolved_model: None,
                source_revisions: Vec::new(),
                applied_at_ms: 0,
            },
            writes,
        }
    }

    fn updated_record(original: &DocumentState, title: &str) -> DocumentRecord {
        let mut state = original.clone();
        state.content.title = title.into();
        state.schema.revision = original.revision() + 1;
        DocumentRecord {
            state,
            markdown: format!("# {title}\n"),
        }
    }

    #[test]
    fn subprocess_mutation_fault_worker() {
        let Ok(root) = std::env::var("MEETNOTE_TEST_MUTATION_ROOT") else {
            return;
        };
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = repository
            .read(&DocumentId::new("process-doc"))
            .unwrap()
            .unwrap();
        let batch = mutation_batch(
            "process-fault",
            "process-fault-fingerprint",
            "process-fault-undo",
            vec![DocumentMutationWrite {
                record: updated_record(&original.state, "Child process update"),
                expected_revision: Some(original.state.revision()),
                source_markdown_sha256: None,
            }],
        );
        repository
            .commit_agent_mutation(batch)
            .expect("the child process must fail at the injected journal stage");
    }

    #[test]
    fn subprocess_trash_lifecycle_worker() {
        let Ok(root) = std::env::var("MEETNOTE_TEST_TRASH_ROOT") else {
            return;
        };
        let trash_name = std::env::var("MEETNOTE_TEST_TRASH_NAME").unwrap();
        let operation = std::env::var("MEETNOTE_TEST_TRASH_OPERATION").unwrap();
        let repository = FileDocumentRepository::open(root).unwrap();
        match operation.as_str() {
            "restore" => repository
                .restore_agent_trash(&trash_name, "child")
                .unwrap(),
            "purge" => repository.purge_agent_trash(&trash_name).unwrap(),
            other => panic!("unknown trash operation {other}"),
        }
    }

    #[test]
    fn subprocess_repository_lock_holder_worker() {
        let Ok(root) = std::env::var("MEETNOTE_TEST_LOCK_ROOT") else {
            return;
        };
        let repository = FileDocumentRepository::open(&root).unwrap();
        let lock = repository.acquire_repository_lock().unwrap();
        fs::write(Path::new(&root).join("lock-held"), b"ready").unwrap();
        while !Path::new(&root).join("release-lock").is_file() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(lock);
    }

    #[test]
    fn subprocess_ordinary_save_worker() {
        let Ok(root) = std::env::var("MEETNOTE_TEST_SAVE_ROOT") else {
            return;
        };
        let repository = FileDocumentRepository::open(&root).unwrap();
        let current = repository
            .read(&DocumentId::new("process-save"))
            .unwrap()
            .unwrap();
        let mut updated = current.state;
        updated.content.title = "Child ordinary save".into();
        repository
            .save(updated, Some("# Child ordinary save\n".into()))
            .unwrap();
    }

    #[test]
    fn subprocess_open_and_read_worker() {
        let Ok(root) = std::env::var("MEETNOTE_TEST_RECOVERY_ROOT") else {
            return;
        };
        let repository = FileDocumentRepository::open(&root).unwrap();
        let document = repository
            .read(&DocumentId::new("process-doc"))
            .unwrap()
            .unwrap();
        assert_eq!(document.state.title(), "Original");
        assert_eq!(document.markdown, "# Original\n");
    }

    #[test]
    fn file_repository_recovers_a_journal_after_a_real_child_process_fault() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-process-fault-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        let original =
            DocumentState::new(DocumentId::new("process-doc"), "Original", "recording.wav");
        DocumentPackage::new(root.join("process-doc"), original.clone(), "# Original\n")
            .unwrap()
            .save()
            .unwrap();
        let repository = FileDocumentRepository::open(&root).unwrap();

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_mutation_fault_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_MUTATION_ROOT", &root)
            .env("MEETNOTE_TEST_MUTATION_FAILURE_POINT", "staged_state")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "child unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );

        let held = repository.acquire_repository_lock().unwrap();
        let mut recovery_reader = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_open_and_read_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_RECOVERY_ROOT", &root)
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(recovery_reader.try_wait().unwrap().is_none());
        drop(held);
        assert!(recovery_reader.wait().unwrap().success());

        let reopened = FileDocumentRepository::open(&root).unwrap();
        let restored = reopened
            .read(&DocumentId::new("process-doc"))
            .unwrap()
            .unwrap();
        assert_eq!(restored.state.content.title, "Original");
        assert_eq!(restored.state.revision(), original.revision());
        assert_eq!(restored.markdown, "# Original\n");
        assert!(reopened.snapshot().unwrap().issues.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_restore_and_purge_work_across_process_boundaries() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-process-trash-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let parent = DocumentState::new(DocumentId::new("parent"), "Parent", "recording.wav");
        repository
            .save(parent.clone(), Some("# Parent\n".into()))
            .unwrap();
        let child = parent.semantic_child(DocumentId::new("child"), "Child", 0, 1_000, Vec::new());
        repository
            .commit_agent_mutation(mutation_batch(
                "process-trash-create",
                "process-trash-create-fingerprint",
                "process-trash-create-undo",
                vec![DocumentMutationWrite {
                    record: DocumentRecord {
                        state: child,
                        markdown: "# Child\n".into(),
                    },
                    expected_revision: None,
                    source_markdown_sha256: None,
                }],
            ))
            .unwrap();
        repository
            .undo_agent_mutation("process-trash-create-undo")
            .unwrap();
        let restore_name = repository
            .list_agent_trash()
            .unwrap()
            .into_iter()
            .find(|name| name.contains("child"))
            .unwrap();
        drop(repository);

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_trash_lifecycle_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_TRASH_ROOT", &root)
            .env("MEETNOTE_TEST_TRASH_NAME", &restore_name)
            .env("MEETNOTE_TEST_TRASH_OPERATION", "restore")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "trash restore child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let repository = FileDocumentRepository::open(&root).unwrap();
        assert!(repository
            .read(&DocumentId::new("child"))
            .unwrap()
            .is_some());
        assert!(!repository
            .list_agent_trash()
            .unwrap()
            .contains(&restore_name));

        let child_two = parent.semantic_child(
            DocumentId::new("child-two"),
            "Child two",
            1_000,
            2_000,
            Vec::new(),
        );
        repository
            .commit_agent_mutation(mutation_batch(
                "process-trash-purge",
                "process-trash-purge-fingerprint",
                "process-trash-purge-undo",
                vec![DocumentMutationWrite {
                    record: DocumentRecord {
                        state: child_two,
                        markdown: "# Child two\n".into(),
                    },
                    expected_revision: None,
                    source_markdown_sha256: None,
                }],
            ))
            .unwrap();
        repository
            .undo_agent_mutation("process-trash-purge-undo")
            .unwrap();
        let purge_name = repository
            .list_agent_trash()
            .unwrap()
            .into_iter()
            .find(|name| name.contains("child-two"))
            .unwrap();
        drop(repository);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_trash_lifecycle_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_TRASH_ROOT", &root)
            .env("MEETNOTE_TEST_TRASH_NAME", &purge_name)
            .env("MEETNOTE_TEST_TRASH_OPERATION", "purge")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "trash purge child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let repository = FileDocumentRepository::open(&root).unwrap();
        assert!(!repository.list_agent_trash().unwrap().contains(&purge_name));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stores_and_lists_documents_without_a_database() {
        let repository = InMemoryDocumentRepository::default();
        let project = DocumentState::new(
            DocumentId::new("project-1"),
            "Fixture meeting",
            "fixture.wav",
        );

        repository.save(project.clone(), None).unwrap();

        assert_eq!(repository.get(project.id()).unwrap(), Some(project.clone()));
        assert_eq!(repository.snapshot().unwrap().documents, vec![project]);
    }

    #[test]
    fn in_memory_agent_mutation_is_atomic_idempotent_and_undoable() {
        let repository = InMemoryDocumentRepository::default();
        let original = DocumentState::new(DocumentId::new("memory-doc"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let batch = mutation_batch(
            "memory-mutation",
            "fingerprint-a",
            "memory-undo",
            vec![DocumentMutationWrite {
                record: updated_record(&original, "Updated"),
                expected_revision: Some(0),
                source_markdown_sha256: None,
            }],
        );

        let committed = repository.commit_agent_mutation(batch.clone()).unwrap();
        assert!(!committed.replayed);
        assert_eq!(
            repository
                .read(&DocumentId::new("memory-doc"))
                .unwrap()
                .unwrap()
                .state
                .title(),
            "Updated"
        );
        let replay = repository.commit_agent_mutation(batch.clone()).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.revisions, committed.revisions);

        let mut conflicting = batch.clone();
        conflicting.fingerprint = "fingerprint-b".into();
        assert_eq!(
            repository.commit_agent_mutation(conflicting).unwrap_err(),
            DocumentMutationStoreError::IdempotencyConflict
        );

        let undone = repository.undo_agent_mutation("memory-undo").unwrap();
        assert!(!undone.replayed);
        let restored = repository
            .read(&DocumentId::new("memory-doc"))
            .unwrap()
            .unwrap();
        assert_eq!(restored.state.title(), "Original");
        assert_eq!(restored.state.revision(), 2);
        assert_eq!(restored.markdown, "# Original\n");
        assert!(
            repository
                .undo_agent_mutation("memory-undo")
                .unwrap()
                .replayed
        );
        assert_eq!(
            repository.commit_agent_mutation(batch).unwrap_err(),
            DocumentMutationStoreError::IdempotencyConflict
        );
    }

    #[test]
    fn saves_and_loads_a_document_package() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-documents-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let id = DocumentId::new("document-1");
        let state = DocumentState::new(id, "Daily recording", "recording.m4a");
        let directory = package_directory(&root, "document-1");
        let package =
            DocumentPackage::new(&directory, state.clone(), "# Daily recording\n").unwrap();

        package.save().unwrap();
        let loaded = DocumentPackage::load(&directory).unwrap();

        assert_eq!(loaded.state, state);
        assert_eq!(loaded.markdown, "# Daily recording\n");
        assert_eq!(loaded.document_id().as_str(), "document-1");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_protocol_blocks_cross_instance_reads_until_save_lock_releases() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-protocol-read-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        repository
            .save(
                DocumentState::new(DocumentId::new("visible"), "Original", "recording.wav"),
                Some("# Original\n".into()),
            )
            .unwrap();
        let other = FileDocumentRepository::open(&root).unwrap();
        let held = repository.acquire_repository_lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            sender
                .send(other.read(&DocumentId::new("visible")))
                .unwrap();
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(held);
        let result = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.state.title(), "Original");
        assert_eq!(result.markdown, "# Original\n");
        reader.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_save_uses_the_process_protocol() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-protocol-save-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        repository
            .save(
                DocumentState::new(DocumentId::new("process-save"), "Original", "recording.wav"),
                Some("# Original\n".into()),
            )
            .unwrap();
        let held = repository.acquire_repository_lock().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_ordinary_save_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_SAVE_ROOT", &root)
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(child.try_wait().unwrap().is_none());
        drop(held);
        assert!(child.wait().unwrap().success());
        let saved = repository
            .read(&DocumentId::new("process-save"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.state.title(), "Child ordinary save");
        assert_eq!(saved.state.revision(), 1);
        assert_eq!(saved.markdown, "# Child ordinary save\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scanner_returns_valid_packages_and_relation_errors_together() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-scan-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let first = DocumentState::new(DocumentId::new("root"), "Root", "recording.m4a");
        let mut child = DocumentState::new(DocumentId::new("child"), "Child", "recording.m4a");
        child.hierarchy.parent_id = Some(DocumentId::new("root"));
        child.source.audio_asset_id = Some(DocumentId::new("root"));
        child.hierarchy.depth = 1;
        let mut invalid =
            DocumentState::new(DocumentId::new("invalid"), "Invalid", "recording.m4a");
        invalid.hierarchy.parent_id = Some(DocumentId::new("missing"));
        invalid.hierarchy.depth = 1;

        DocumentPackage::new(package_directory(&root, "root"), first, "# Root\n")
            .unwrap()
            .save()
            .unwrap();
        DocumentPackage::new(package_directory(&root, "child"), child, "# Child\n")
            .unwrap()
            .save()
            .unwrap();
        DocumentPackage::new(package_directory(&root, "invalid"), invalid, "# Invalid\n")
            .unwrap()
            .save()
            .unwrap();

        let scan = DocumentScanner::new(&root).scan().unwrap();

        assert_eq!(scan.packages.len(), 3);
        assert!(scan.errors.iter().any(|error| matches!(
            error,
            DocumentScanError::MissingParent { document_id, parent_id }
                if document_id.as_str() == "invalid" && parent_id.as_str() == "missing"
        )));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scanner_rejects_a_package_with_a_future_schema() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-schema-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let directory = package_directory(&root, "future");
        std::fs::create_dir_all(&directory).unwrap();
        let mut state = DocumentState::new(DocumentId::new("future"), "Future", "recording.m4a");
        state.schema.schema_version += 1;
        std::fs::write(
            directory.join(DOCUMENT_STATE_FILENAME),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join(DOCUMENT_MARKDOWN_FILENAME), "# Future\n").unwrap();

        let scan = DocumentScanner::new(&root).scan().unwrap();

        assert!(matches!(
            &scan.errors[0],
            DocumentScanError::InvalidPackage { path, .. } if path == &directory
        ));
        assert!(scan.packages.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_repository_migrates_a_legacy_flat_state_when_loaded() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-migration-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let directory = package_directory(&root, "legacy");
        std::fs::create_dir_all(&directory).unwrap();
        let legacy = serde_json::json!({
            "schema_version": 1,
            "revision": 0,
            "updated_at_ms": 20,
            "id": "legacy",
            "title": "Legacy",
            "audio_path": "recording.m4a",
            "created_at_ms": 10,
            "recorded_at_ms": null,
            "audio_start_ms": 0,
            "audio_end_ms": null,
            "parent_id": null,
            "audio_asset_id": null,
            "depth": 0,
            "sibling_order": 0,
            "tags": [],
            "summary": null,
            "processing_error": null,
            "status": "Imported",
            "transcript": [],
            "diarization": [],
            "speaker_segments": [],
            "markdown_path": null
        });
        std::fs::write(
            directory.join(DOCUMENT_STATE_FILENAME),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join(DOCUMENT_MARKDOWN_FILENAME), "# Legacy\n").unwrap();

        let repository = FileDocumentRepository::open(&root).unwrap();
        let migrated = repository.get(&DocumentId::new("legacy")).unwrap().unwrap();
        let stored: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join(DOCUMENT_STATE_FILENAME)).unwrap(),
        )
        .unwrap();

        assert_eq!(migrated.title(), "Legacy");
        assert!(stored.get("identity").is_some());
        assert!(stored.get("title").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_repository_reopens_documents_and_preserves_markdown() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-repository-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let document = DocumentState::new(
            DocumentId::new("persistent"),
            "Persistent document",
            "recording.m4a",
        );
        repository
            .save(document.clone(), Some("# Persistent document\n".into()))
            .unwrap();

        let reopened = FileDocumentRepository::open(&root).unwrap();
        let loaded = reopened
            .get(&DocumentId::new("persistent"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id(), document.id());
        assert_eq!(loaded.title(), document.title());
        assert_eq!(loaded.revision(), 0);
        reopened
            .save(
                DocumentState {
                    content: domain::DocumentContent {
                        title: "Renamed document".into(),
                        ..loaded.content.clone()
                    },
                    ..loaded.clone()
                },
                None,
            )
            .unwrap();
        let final_scan = reopened.scanner.scan().unwrap();
        assert_eq!(final_scan.packages[0].markdown, "# Persistent document\n");
        assert_eq!(final_scan.packages[0].state.revision(), 1);
        assert_eq!(final_scan.packages[0].state.title(), "Renamed document");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_agent_mutation_replays_after_reopen_and_undo_persists() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-mutation-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("file-doc"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let batch = mutation_batch(
            "file-mutation",
            "fingerprint-a",
            "file-undo",
            vec![DocumentMutationWrite {
                record: updated_record(&original, "Updated"),
                expected_revision: Some(0),
                source_markdown_sha256: None,
            }],
        );

        let committed = repository.commit_agent_mutation(batch.clone()).unwrap();
        assert!(!committed.replayed);
        drop(repository);

        let reopened = FileDocumentRepository::open(&root).unwrap();
        assert!(
            reopened
                .commit_agent_mutation(batch.clone())
                .unwrap()
                .replayed
        );
        let updated = reopened
            .read(&DocumentId::new("file-doc"))
            .unwrap()
            .unwrap();
        assert_eq!(updated.state.title(), "Updated");
        assert_eq!(updated.state.revision(), 1);
        assert_eq!(updated.markdown, "# Updated\n");

        let undone = reopened.undo_agent_mutation("file-undo").unwrap();
        assert!(!undone.replayed);
        assert!(reopened.undo_agent_mutation("file-undo").unwrap().replayed);
        drop(reopened);

        let reopened = FileDocumentRepository::open(&root).unwrap();
        let restored = reopened
            .read(&DocumentId::new("file-doc"))
            .unwrap()
            .unwrap();
        assert_eq!(restored.state.title(), "Original");
        assert_eq!(restored.state.revision(), 2);
        assert_eq!(restored.markdown, "# Original\n");
        assert_eq!(
            reopened.commit_agent_mutation(batch).unwrap_err(),
            DocumentMutationStoreError::IdempotencyConflict
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_agent_mutation_preserves_unmanaged_package_attachments() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-attachment-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("file-doc"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let attachment = root.join("file-doc").join("notes.txt");
        std::fs::write(&attachment, "keep this user file").unwrap();
        let hidden_attachment = root.join("file-doc").join(".private-notes");
        std::fs::write(&hidden_attachment, "keep hidden data").unwrap();
        let unicode_attachment = root.join("file-doc").join("用户 notes.txt");
        std::fs::write(&unicode_attachment, "keep unicode data").unwrap();
        let lookalike_attachment = root.join("file-doc").join("state.json.bak");
        std::fs::write(&lookalike_attachment, "keep lookalike data").unwrap();
        let nested_attachment = root.join("file-doc").join("nested folder").join("clip.bin");
        std::fs::create_dir_all(nested_attachment.parent().unwrap()).unwrap();
        std::fs::write(&nested_attachment, [0_u8, 1, 2, 3]).unwrap();
        #[cfg(unix)]
        let symlink_target = {
            let target = root.join("outside-user-file.txt");
            std::fs::write(&target, "outside remains untouched").unwrap();
            std::os::unix::fs::symlink(&target, root.join("file-doc").join("user-link")).unwrap();
            target
        };
        let batch = mutation_batch(
            "file-attachment",
            "fingerprint-attachment",
            "undo-attachment",
            vec![DocumentMutationWrite {
                record: updated_record(&original, "Updated"),
                expected_revision: Some(0),
                source_markdown_sha256: None,
            }],
        );
        repository.commit_agent_mutation(batch).unwrap();
        assert_eq!(
            std::fs::read_to_string(&attachment).unwrap(),
            "keep this user file"
        );
        assert_eq!(
            std::fs::read_to_string(&hidden_attachment).unwrap(),
            "keep hidden data"
        );
        assert_eq!(
            std::fs::read_to_string(&unicode_attachment).unwrap(),
            "keep unicode data"
        );
        assert_eq!(
            std::fs::read_to_string(&lookalike_attachment).unwrap(),
            "keep lookalike data"
        );
        assert_eq!(std::fs::read(&nested_attachment).unwrap(), [0_u8, 1, 2, 3]);
        repository.undo_agent_mutation("undo-attachment").unwrap();
        assert_eq!(
            std::fs::read_to_string(&attachment).unwrap(),
            "keep this user file"
        );
        assert_eq!(
            std::fs::read_to_string(&hidden_attachment).unwrap(),
            "keep hidden data"
        );
        assert_eq!(
            std::fs::read_to_string(&unicode_attachment).unwrap(),
            "keep unicode data"
        );
        assert_eq!(
            std::fs::read_to_string(&lookalike_attachment).unwrap(),
            "keep lookalike data"
        );
        assert_eq!(std::fs::read(&nested_attachment).unwrap(), [0_u8, 1, 2, 3]);
        #[cfg(unix)]
        {
            let link = root.join("file-doc").join("user-link");
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                std::fs::read_to_string(&symlink_target).unwrap(),
                "outside remains untouched"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_agent_mutation_is_blocked_by_tree_issues_but_not_by_plain_root_files() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-root-file-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("file-doc"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        std::fs::write(root.join("notes.txt"), "loose user notes").unwrap();
        let write = |state: &DocumentState, title: &str| DocumentMutationWrite {
            record: updated_record(state, title),
            expected_revision: Some(state.revision()),
            source_markdown_sha256: None,
        };

        repository
            .commit_agent_mutation(mutation_batch(
                "root-file",
                "fingerprint-root-file",
                "undo-root-file",
                vec![write(&original, "Updated")],
            ))
            .unwrap();
        std::fs::create_dir_all(root.join("broken")).unwrap();
        let updated = repository
            .read(&DocumentId::new("file-doc"))
            .unwrap()
            .unwrap()
            .state;
        let error = repository
            .commit_agent_mutation(mutation_batch(
                "broken-tree",
                "fingerprint-broken-tree",
                "undo-broken-tree",
                vec![write(&updated, "Blocked")],
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            DocumentMutationStoreError::Transaction(message) if message.contains("relation errors")
        ));
        assert_eq!(updated.title(), "Updated");
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "loose user notes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_lock_serializes_independent_open_and_recovery() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-lock-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let held = repository.acquire_repository_lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let other_root = root.clone();
        let thread = std::thread::spawn(move || {
            let result = FileDocumentRepository::open(other_root);
            sender.send(result.is_ok()).unwrap();
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(held);
        assert!(receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap());
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_lock_serializes_a_real_child_process_owner() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-process-lock-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::subprocess_repository_lock_holder_worker",
                "--nocapture",
            ])
            .env("MEETNOTE_TEST_LOCK_ROOT", &root)
            .spawn()
            .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !root.join("lock-held").is_file() {
            assert!(
                std::time::Instant::now() < deadline,
                "child process did not publish its lock ownership"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let (sender, receiver) = mpsc::channel();
        let other_root = root.clone();
        let thread = std::thread::spawn(move || {
            let result = FileDocumentRepository::open(other_root);
            sender.send(result.is_ok()).unwrap();
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        fs::write(root.join("release-lock"), b"release").unwrap();
        assert!(receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap());
        thread.join().unwrap();
        assert!(child.wait().unwrap().success());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undoing_document_creation_moves_the_package_to_recoverable_trash() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-trash-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let parent = DocumentState::new(DocumentId::new("parent"), "Parent", "a.wav");
        repository
            .save(parent.clone(), Some("# Parent\n".into()))
            .unwrap();
        let child = parent.semantic_child(DocumentId::new("child"), "Child", 0, 1_000, Vec::new());
        let batch = mutation_batch(
            "create-child",
            "create-child-fingerprint",
            "create-child-undo",
            vec![DocumentMutationWrite {
                record: DocumentRecord {
                    state: child,
                    markdown: "# Child\n".into(),
                },
                expected_revision: None,
                source_markdown_sha256: None,
            }],
        );
        repository.commit_agent_mutation(batch).unwrap();
        assert!(root.join("child").is_dir());
        repository.undo_agent_mutation("create-child-undo").unwrap();
        assert!(!root.join("child").exists());
        let trash = root.join(AGENT_MUTATION_DIRECTORY).join("trash");
        let entries = repository.list_agent_trash().unwrap();
        let trash_name = entries
            .iter()
            .find(|name| name.contains("child"))
            .unwrap()
            .clone();
        assert!(trash.join(&trash_name).is_dir());
        repository
            .restore_agent_trash(&trash_name, "child")
            .unwrap();
        assert!(root.join("child").is_dir());
        let child2 = parent.semantic_child(
            DocumentId::new("child2"),
            "Child two",
            1_000,
            2_000,
            Vec::new(),
        );
        repository
            .commit_agent_mutation(mutation_batch(
                "create-child2",
                "create-child2-fingerprint",
                "create-child2-undo",
                vec![DocumentMutationWrite {
                    record: DocumentRecord {
                        state: child2,
                        markdown: "# Child two\n".into(),
                    },
                    expected_revision: None,
                    source_markdown_sha256: None,
                }],
            ))
            .unwrap();
        repository
            .undo_agent_mutation("create-child2-undo")
            .unwrap();
        let child2_trash = repository
            .list_agent_trash()
            .unwrap()
            .into_iter()
            .find(|name| name.contains("child2"))
            .unwrap();
        repository.purge_agent_trash(&child2_trash).unwrap();
        assert!(!repository
            .list_agent_trash()
            .unwrap()
            .iter()
            .any(|name| name == &child2_trash));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_agent_commit_rolls_back_every_injected_write_point() {
        let points = [
            MutationWritePoint::PreparedJournal,
            MutationWritePoint::StagedState("parent".into()),
            MutationWritePoint::StagedMarkdown("parent".into()),
            MutationWritePoint::BackedUp("parent".into()),
            MutationWritePoint::Published("parent".into()),
            MutationWritePoint::Published("child".into()),
            MutationWritePoint::AppliedJournal,
        ];
        for (index, point) in points.into_iter().enumerate() {
            let root = std::env::temp_dir().join(format!(
                "meetnote-file-agent-commit-failure-{index}-{}-{}",
                std::process::id(),
                unique_suffix()
            ));
            let repository = FileDocumentRepository::open(&root).unwrap();
            let original = DocumentState::new(DocumentId::new("parent"), "Original", "a.wav");
            repository
                .save(original.clone(), Some("# Original\n".into()))
                .unwrap();
            let mut child =
                original.semantic_child(DocumentId::new("child"), "Child", 0, 1_000, Vec::new());
            child.hierarchy.sibling_order = 0;
            let batch = mutation_batch(
                &format!("commit-failure-{index}"),
                &format!("commit-fingerprint-{index}"),
                &format!("commit-undo-{index}"),
                vec![
                    DocumentMutationWrite {
                        record: updated_record(&original, "Updated"),
                        expected_revision: Some(0),
                        source_markdown_sha256: None,
                    },
                    DocumentMutationWrite {
                        record: DocumentRecord {
                            state: child,
                            markdown: "# Child\n".into(),
                        },
                        expected_revision: None,
                        source_markdown_sha256: None,
                    },
                ],
            );
            repository.inject_failure_once(point.clone());
            let error = repository.commit_agent_mutation(batch).unwrap_err();
            assert!(
                matches!(error, DocumentMutationStoreError::Transaction(_)),
                "failure point {point:?} returned {error:?}"
            );
            assert_original_without_child(&repository);
            assert_no_transaction_workspace(&repository);
            drop(repository);

            let reopened = FileDocumentRepository::open(&root).unwrap();
            assert_original_without_child(&reopened);
            assert_no_transaction_workspace(&reopened);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn file_agent_undo_rolls_back_every_injected_write_point() {
        let points = [
            MutationWritePoint::UndoPreparedJournal,
            MutationWritePoint::StagedState("document".into()),
            MutationWritePoint::StagedMarkdown("document".into()),
            MutationWritePoint::BackedUp("document".into()),
            MutationWritePoint::Published("document".into()),
            MutationWritePoint::UndoAppliedJournal,
        ];
        for (index, point) in points.into_iter().enumerate() {
            let root = std::env::temp_dir().join(format!(
                "meetnote-file-agent-undo-failure-{index}-{}-{}",
                std::process::id(),
                unique_suffix()
            ));
            let repository = FileDocumentRepository::open(&root).unwrap();
            let original = DocumentState::new(DocumentId::new("document"), "Original", "a.wav");
            repository
                .save(original.clone(), Some("# Original\n".into()))
                .unwrap();
            let undo_token = format!("undo-failure-{index}");
            repository
                .commit_agent_mutation(mutation_batch(
                    &format!("undo-mutation-{index}"),
                    &format!("undo-fingerprint-{index}"),
                    &undo_token,
                    vec![DocumentMutationWrite {
                        record: updated_record(&original, "Updated"),
                        expected_revision: Some(0),
                        source_markdown_sha256: None,
                    }],
                ))
                .unwrap();

            repository.inject_failure_once(point.clone());
            let error = repository.undo_agent_mutation(&undo_token).unwrap_err();
            assert!(
                matches!(error, DocumentMutationStoreError::Transaction(_)),
                "failure point {point:?} returned {error:?}"
            );
            assert_updated_document(&repository);
            assert_no_transaction_workspace(&repository);
            drop(repository);

            let reopened = FileDocumentRepository::open(&root).unwrap();
            assert_updated_document(&reopened);
            assert_no_transaction_workspace(&reopened);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn file_pending_mutations_are_private_ephemeral_and_expiry_cleaned() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-pending-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("pending-doc"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let pending =
            |confirmation_id: &str, expires_at_ms: u64| application::PendingDocumentMutation {
                confirmation_id: confirmation_id.into(),
                expires_at_ms,
                batch: mutation_batch(
                    &format!("mutation-{confirmation_id}"),
                    &format!("fingerprint-{confirmation_id}"),
                    &format!("undo-{confirmation_id}"),
                    vec![DocumentMutationWrite {
                        record: updated_record(&original, "Pending"),
                        expected_revision: Some(0),
                        source_markdown_sha256: None,
                    }],
                ),
                uncovered_segment_ids: Vec::new(),
                ranges: Vec::new(),
            };
        repository
            .save_pending_agent_mutation(pending("expired", 0))
            .unwrap();
        assert!(repository.pending_path("expired").is_file());
        repository
            .save_pending_agent_mutation(pending("active", u64::MAX))
            .unwrap();
        assert!(!repository.pending_path("expired").exists());
        assert!(repository.pending_path("active").is_file());
        assert!(repository
            .take_pending_agent_mutation("expired")
            .unwrap()
            .is_none());
        assert_eq!(
            repository
                .take_pending_agent_mutation("active")
                .unwrap()
                .unwrap()
                .confirmation_id,
            "active"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(repository.pending_root())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            let journal_root = repository.mutation_root();
            assert_eq!(
                fs::metadata(journal_root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_original_without_child(repository: &FileDocumentRepository) {
        let parent = repository
            .read(&DocumentId::new("parent"))
            .unwrap()
            .unwrap();
        assert_eq!(parent.state.title(), "Original");
        assert_eq!(parent.state.revision(), 0);
        assert_eq!(parent.markdown, "# Original\n");
        assert!(repository
            .read(&DocumentId::new("child"))
            .unwrap()
            .is_none());
        assert!(repository.snapshot().unwrap().issues.is_empty());
    }

    fn assert_updated_document(repository: &FileDocumentRepository) {
        let document = repository
            .read(&DocumentId::new("document"))
            .unwrap()
            .unwrap();
        assert_eq!(document.state.title(), "Updated");
        assert_eq!(document.state.revision(), 1);
        assert_eq!(document.markdown, "# Updated\n");
        assert!(repository.snapshot().unwrap().issues.is_empty());
    }

    fn assert_no_transaction_workspace(repository: &FileDocumentRepository) {
        let mutation_root = repository.mutation_root();
        if !mutation_root.is_dir() {
            return;
        }
        assert!(fs::read_dir(mutation_root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("transaction-")
        }));
    }

    #[test]
    fn prepared_file_batch_recovers_to_the_complete_before_state() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-recovery-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("parent"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let mut child =
            original.semantic_child(DocumentId::new("child"), "Child", 0, 1_000, Vec::new());
        child.hierarchy.sibling_order = 0;
        let batch = mutation_batch(
            "recover-mutation",
            "recover-fingerprint",
            "recover-undo",
            vec![
                DocumentMutationWrite {
                    record: updated_record(&original, "Partially updated"),
                    expected_revision: Some(0),
                    source_markdown_sha256: None,
                },
                DocumentMutationWrite {
                    record: DocumentRecord {
                        state: child.clone(),
                        markdown: "# Child\n".into(),
                    },
                    expected_revision: None,
                    source_markdown_sha256: None,
                },
            ],
        );
        let journal = AgentMutationJournal {
            version: AGENT_MUTATION_JOURNAL_VERSION,
            status: MutationJournalStatus::Prepared,
            batch,
            before: vec![
                MutationBeforeRecord {
                    document_id: "parent".into(),
                    record: Some(DocumentRecord {
                        state: original.clone(),
                        markdown: "# Original\n".into(),
                    }),
                },
                MutationBeforeRecord {
                    document_id: "child".into(),
                    record: None,
                },
            ],
            commit: None,
            undo_before: Vec::new(),
            undo_commit: None,
        };
        let journal_path = repository.journal_path("recover-undo");
        repository.write_journal(&journal_path, &journal).unwrap();
        DocumentPackage::new(
            root.join("parent"),
            updated_record(&original, "Partially updated").state,
            "# Partially updated\n",
        )
        .unwrap()
        .save()
        .unwrap();
        DocumentPackage::new(root.join("child"), child, "# Child\n")
            .unwrap()
            .save()
            .unwrap();
        drop(repository);

        let recovered = FileDocumentRepository::open(&root).unwrap();
        let parent = recovered.read(&DocumentId::new("parent")).unwrap().unwrap();
        assert_eq!(parent.state, original);
        assert_eq!(parent.markdown, "# Original\n");
        assert!(recovered.read(&DocumentId::new("child")).unwrap().is_none());
        let bytes = fs::read(journal_path).unwrap();
        let recovered_journal: AgentMutationJournal = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(recovered_journal.status, MutationJournalStatus::RolledBack);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_and_watchers_wait_until_a_file_batch_is_fully_visible() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-file-agent-visibility-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let original = DocumentState::new(DocumentId::new("visible"), "Original", "a.wav");
        repository
            .save(original.clone(), Some("# Original\n".into()))
            .unwrap();
        let watcher = repository.create_watcher().unwrap();
        while watcher.try_recv().is_some() {}

        let transaction_guard = repository.transaction_lock.write().unwrap();
        let updated = updated_record(&original, "Committed");
        DocumentPackage::new(
            root.join("visible"),
            updated.state.clone(),
            updated.markdown.clone(),
        )
        .unwrap()
        .save()
        .unwrap();

        let (read_sender, read_receiver) = mpsc::channel();
        let reader = repository.clone();
        std::thread::spawn(move || {
            read_sender
                .send(reader.read(&DocumentId::new("visible")))
                .unwrap();
        });
        let (watch_sender, watch_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            watch_sender
                .send(watcher.recv_timeout(std::time::Duration::from_secs(2)))
                .unwrap();
        });
        assert!(read_receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        assert!(watch_receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(transaction_guard);

        let visible = read_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(visible.state.title(), "Committed");
        assert_eq!(visible.markdown, "# Committed\n");
        assert!(watch_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_repository_can_watch_its_document_root() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-watcher-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let watcher = repository.create_watcher().unwrap();

        repository
            .save(
                DocumentState::new(
                    DocumentId::new("watched"),
                    "Watched document",
                    "recording.m4a",
                ),
                None,
            )
            .unwrap();

        assert!(watcher
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_repository_does_not_reprocess_a_completed_document_on_open() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-recovery-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let directory = package_directory(&root, "interrupted");
        let mut state = DocumentState::new(
            DocumentId::new("interrupted"),
            "Interrupted document",
            "recording.m4a",
        );
        state.processing.last_error = Some("processing interrupted before completion".into());
        DocumentPackage::new(directory, state, "# Interrupted document\n")
            .unwrap()
            .save()
            .unwrap();

        let repository = FileDocumentRepository::open(&root).unwrap();
        let recovered = repository
            .get(&DocumentId::new("interrupted"))
            .unwrap()
            .unwrap();

        assert_eq!(recovered.processing_status(), domain::ProjectStatus::Failed);
        assert_eq!(
            recovered.processing.last_error.as_deref(),
            Some("processing interrupted before completion")
        );
        assert_eq!(recovered.revision(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_scan_reads_one_bounded_projection_and_stops_without_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-metadata-scan-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        for id in ["first", "second", "third"] {
            repository
                .save(
                    DocumentState::new(DocumentId::new(id), format!("{id} title"), "recording.m4a"),
                    None,
                )
                .unwrap();
        }

        let mut visited = Vec::new();
        repository
            .scan_metadata(&mut |metadata| {
                visited.push(metadata);
                Ok(false)
            })
            .unwrap();

        assert_eq!(visited.len(), 1);
        assert!(visited[0].title.ends_with("title"));
        assert!(visited[0].audio_asset_id.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_scan_rejects_a_future_schema_with_its_actual_version() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-metadata-schema-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = FileDocumentRepository::open(&root).unwrap();
        let document = DocumentState::new(DocumentId::new("document"), "Title", "audio.wav");
        repository.save(document.clone(), None).unwrap();
        let mut future = document;
        future.schema.schema_version = domain::DOCUMENT_STATE_SCHEMA_VERSION + 1;
        let directory = root.join("document");
        fs::write(
            directory.join(DOCUMENT_STATE_FILENAME),
            serde_json::to_vec(&future).unwrap(),
        )
        .unwrap();
        let error = DocumentPackage::load_metadata(&directory).unwrap_err();
        assert!(
            matches!(&error, DocumentPackageError::InvalidStateValue {
            error: domain::DocumentStateError::UnsupportedSchema { actual, expected }
        } if *actual == domain::DOCUMENT_STATE_SCHEMA_VERSION + 1 && *expected == domain::DOCUMENT_STATE_SCHEMA_VERSION),
            "{error:?}"
        );
        let mut visited = false;
        assert!(repository
            .scan_metadata(&mut |_| {
                visited = true;
                Ok(true)
            })
            .is_err());
        assert!(!visited);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_scan_reports_missing_fields_as_invalid_json_state() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-metadata-fields-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let directory = root.join("document");
        fs::create_dir_all(&directory).unwrap();
        let mut state = serde_json::to_value(DocumentState::new(
            DocumentId::new("document"),
            "Title",
            "audio.wav",
        ))
        .unwrap();
        state.as_object_mut().unwrap().remove("content");
        fs::write(
            directory.join(DOCUMENT_STATE_FILENAME),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let error = DocumentPackage::load_metadata(&directory).unwrap_err();
        assert!(
            matches!(error, DocumentPackageError::InvalidState { .. }),
            "{error:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
