use application::{DocumentIssue, DocumentRepository, DocumentSnapshot};
use domain::{DocumentId, DocumentState};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, RwLock};

pub const DOCUMENT_MARKDOWN_FILENAME: &str = "document.md";
pub const DOCUMENT_STATE_FILENAME: &str = "state.json";

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
        let state =
            serde_json::from_slice::<DocumentState>(&fs::read(&state_path).map_err(|source| {
                DocumentPackageError::Read {
                    path: state_path.clone(),
                    source,
                }
            })?)
            .map_err(|source| DocumentPackageError::InvalidState {
                path: state_path.clone(),
                source,
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
        Ok(package)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.state.id
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
        if directory_name != self.state.id.as_str() {
            return Err(DocumentPackageError::DirectoryIdMismatch {
                directory: self.directory.clone(),
                document_id: self.state.id.clone(),
            });
        }
        Ok(())
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
    #[error("invalid document state: {error:?}")]
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
    #[error("document directory {directory} contains state for {document_id:?}")]
    DirectoryIdMismatch {
        directory: PathBuf,
        document_id: DocumentId,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentScanError {
    InvalidPackage {
        path: PathBuf,
        message: String,
    },
    DuplicateDocumentId {
        document_id: DocumentId,
        path: PathBuf,
    },
    MissingParent {
        document_id: DocumentId,
        parent_id: DocumentId,
    },
    Cycle {
        document_id: DocumentId,
    },
    MaximumDepth {
        document_id: DocumentId,
    },
    CrossAudioAsset {
        document_id: DocumentId,
    },
    DepthMismatch {
        document_id: DocumentId,
        declared: u8,
        actual: u8,
    },
}

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
        packages.sort_by_key(|package| package.state.sibling_order);
        errors.extend(validate_relations(&packages));
        Ok(DocumentScan { packages, errors })
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
        if by_id.insert(package.state.id.clone(), package).is_some() {
            errors.push(DocumentScanError::DuplicateDocumentId {
                document_id: package.state.id.clone(),
                path: package.directory().to_path_buf(),
            });
        }
    }

    for package in packages {
        let state = &package.state;
        let Some(parent_id) = state.parent_id.as_ref() else {
            if state.depth != 0 {
                errors.push(DocumentScanError::DepthMismatch {
                    document_id: state.id.clone(),
                    declared: state.depth,
                    actual: 0,
                });
            }
            continue;
        };
        let Some(parent) = by_id.get(parent_id) else {
            errors.push(DocumentScanError::MissingParent {
                document_id: state.id.clone(),
                parent_id: parent_id.clone(),
            });
            continue;
        };
        if parent.state.id == state.id || reaches(parent, &state.id, &by_id) {
            errors.push(DocumentScanError::Cycle {
                document_id: state.id.clone(),
            });
            continue;
        }
        let state_asset = state.audio_asset_id.as_ref().unwrap_or(&state.id);
        let parent_asset = parent
            .state
            .audio_asset_id
            .as_ref()
            .unwrap_or(&parent.state.id);
        if state_asset != parent_asset {
            errors.push(DocumentScanError::CrossAudioAsset {
                document_id: state.id.clone(),
            });
        }
        let actual_depth = parent.state.depth.saturating_add(1);
        if actual_depth > 2 {
            errors.push(DocumentScanError::MaximumDepth {
                document_id: state.id.clone(),
            });
        }
        if state.depth != actual_depth {
            errors.push(DocumentScanError::DepthMismatch {
                document_id: state.id.clone(),
                declared: state.depth,
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
    let mut current = start.state.parent_id.as_ref();
    while let Some(id) = current {
        if id == target {
            return true;
        }
        current = by_id
            .get(id)
            .and_then(|package| package.state.parent_id.as_ref());
    }
    false
}

#[derive(Clone, Default)]
pub struct InMemoryDocumentRepository {
    documents: Arc<RwLock<BTreeMap<String, DocumentState>>>,
}

impl DocumentRepository for InMemoryDocumentRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
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

    fn get(&self, document_id: &DocumentId) -> Result<Option<DocumentState>, String> {
        Ok(self
            .documents
            .read()
            .map_err(|_| "document repository lock poisoned".to_string())?
            .get(document_id.as_str())
            .cloned())
    }

    fn save(&self, document: DocumentState, _markdown: Option<String>) -> Result<(), String> {
        self.documents
            .write()
            .map_err(|_| "document repository lock poisoned".to_string())?
            .insert(document.id.as_str().to_string(), document);
        Ok(())
    }
}

#[derive(Clone)]
pub struct FileDocumentRepository {
    scanner: DocumentScanner,
}

impl FileDocumentRepository {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(Self {
            scanner: DocumentScanner::new(root),
        })
    }

    pub fn root(&self) -> &Path {
        self.scanner.root()
    }

    pub fn watch(&self) -> Result<DocumentWatcher, String> {
        DocumentWatcher::new(self.root())
    }
}

pub struct DocumentWatcher {
    receiver: Receiver<Result<DocumentWatchEvent, String>>,
    _watcher: RecommendedWatcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentWatchEvent {
    Changed,
}

impl DocumentWatcher {
    fn new(root: &Path) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                let event = result
                    .map(|_| DocumentWatchEvent::Changed)
                    .map_err(|error| error.to_string());
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
            _watcher: watcher,
        })
    }

    pub fn try_recv(&self) -> Option<Result<DocumentWatchEvent, String>> {
        self.receiver.try_recv().ok()
    }

    pub fn recv(&self) -> Result<DocumentWatchEvent, String> {
        self.receiver.recv().map_err(|error| error.to_string())?
    }

    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<DocumentWatchEvent>, String> {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => event.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("document watcher disconnected".into())
            }
        }
    }
}

impl DocumentRepository for FileDocumentRepository {
    fn snapshot(&self) -> Result<DocumentSnapshot, String> {
        let scan = self.scanner.scan().map_err(|error| error.to_string())?;
        let documents = scan.documents();
        let issues = scan
            .errors
            .into_iter()
            .map(|error| DocumentIssue {
                path: self.root().display().to_string(),
                message: format!("{error:?}"),
            })
            .collect();
        Ok(DocumentSnapshot { documents, issues })
    }

    fn get(&self, document_id: &DocumentId) -> Result<Option<DocumentState>, String> {
        Ok(self
            .snapshot()?
            .documents
            .into_iter()
            .find(|document| &document.id == document_id))
    }

    fn save(&self, mut document: DocumentState, markdown: Option<String>) -> Result<(), String> {
        let directory = self.root().join(document.id.as_str());
        let existing = DocumentPackage::load(&directory).ok();
        if let Some(existing) = &existing {
            document.revision = existing.state.revision.saturating_add(1);
        }
        document.updated_at_ms = now_ms();
        let markdown = markdown
            .or_else(|| existing.map(|package| package.markdown))
            .unwrap_or_else(|| format!("# {}\n\n", document.title));
        DocumentPackage::new(directory, document, markdown)
            .map_err(|error| error.to_string())?
            .save()
            .map_err(|error| error.to_string())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_directory(root: &Path, id: &str) -> PathBuf {
        root.join(id)
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

        assert_eq!(repository.get(&project.id).unwrap(), Some(project.clone()));
        assert_eq!(repository.snapshot().unwrap().documents, vec![project]);
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
    fn scanner_returns_valid_packages_and_relation_errors_together() {
        let root = std::env::temp_dir().join(format!(
            "meetnote-document-scan-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let first = DocumentState::new(DocumentId::new("root"), "Root", "recording.m4a");
        let mut child = DocumentState::new(DocumentId::new("child"), "Child", "recording.m4a");
        child.parent_id = Some(DocumentId::new("root"));
        child.audio_asset_id = Some(DocumentId::new("root"));
        child.depth = 1;
        let mut invalid =
            DocumentState::new(DocumentId::new("invalid"), "Invalid", "recording.m4a");
        invalid.parent_id = Some(DocumentId::new("missing"));
        invalid.depth = 1;

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
        state.schema_version += 1;
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
        assert_eq!(loaded.id, document.id);
        assert_eq!(loaded.title, document.title);
        assert_eq!(loaded.revision, 0);
        reopened
            .save(
                DocumentState {
                    title: "Renamed document".into(),
                    ..loaded.clone()
                },
                None,
            )
            .unwrap();
        let final_scan = reopened.scanner.scan().unwrap();
        assert_eq!(final_scan.packages[0].markdown, "# Persistent document\n");
        assert_eq!(final_scan.packages[0].state.revision, 1);
        assert_eq!(final_scan.packages[0].state.title, "Renamed document");
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
        let watcher = repository.watch().unwrap();

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

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
