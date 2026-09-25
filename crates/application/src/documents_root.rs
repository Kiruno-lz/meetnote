//! The documents root a workspace reads and writes, and the rules for
//! switching it at runtime.

use crate::DocumentRepository;
use runtime::JobId;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Opens the document repository stored under a documents root. The desktop
/// passes `FileDocumentRepository::open`.
pub type DocumentRepositoryFactory =
    Arc<dyn Fn(&Path) -> Result<Arc<dyn DocumentRepository>, String> + Send + Sync>;

/// The documents root a workspace's `DocumentService` was opened on, the
/// model directory that can never become one, and how to open another root.
#[derive(Clone)]
pub struct DocumentsRoot {
    pub(crate) root: PathBuf,
    pub(crate) model_directory: PathBuf,
    pub(crate) factory: DocumentRepositoryFactory,
}

impl DocumentsRoot {
    pub fn new(
        root: impl Into<PathBuf>,
        model_directory: impl Into<PathBuf>,
        factory: impl Fn(&Path) -> Result<Arc<dyn DocumentRepository>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            root: root.into(),
            model_directory: model_directory.into(),
            factory: Arc::new(factory),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// A location that holds other workspace assets and cannot also hold the
/// document packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedLocation {
    SpeakerLibrary,
    ModelDirectory,
}

impl fmt::Display for ReservedLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SpeakerLibrary => "Speaker Library",
            Self::ModelDirectory => "model directory",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentsRootError {
    /// The workspace was built without a repository factory.
    #[error("this workspace cannot switch its documents root")]
    Unsupported,
    #[error("the documents root must not be empty")]
    Empty,
    #[error("the documents root {} is not a directory", path.display())]
    NotADirectory { path: PathBuf },
    /// The root is, contains or lies inside `reserved`.
    #[error(
        "the documents root {} overlaps the {location} at {}",
        path.display(),
        reserved.display()
    )]
    Reserved {
        path: PathBuf,
        location: ReservedLocation,
        reserved: PathBuf,
    },
    /// Processing or Assistant jobs are queued or running on the shared job
    /// runtime; switching now could let them write into the old root.
    #[error("the workspace is busy with {} running job(s)", job_ids.len())]
    WorkspaceBusy { job_ids: Vec<JobId> },
    #[error("cannot create the documents root {}: {message}", path.display())]
    Create { path: PathBuf, message: String },
    #[error("cannot open the documents root {}: {message}", path.display())]
    Open { path: PathBuf, message: String },
    #[error("cannot read the job runtime: {0}")]
    Jobs(String),
}

/// Checks that `root` can hold the document packages: it is non-empty, is a
/// directory if it exists, and neither is, contains nor lies inside the
/// Speaker Library or the model directory. Relative paths are resolved
/// against the current directory and symbolic links of existing ancestors
/// are followed before comparing.
pub fn validate_documents_root(
    root: &Path,
    speaker_library_path: Option<&Path>,
    model_directory: &Path,
) -> Result<(), DocumentsRootError> {
    if root.to_str().is_some_and(|value| value.trim().is_empty()) {
        return Err(DocumentsRootError::Empty);
    }
    let resolved = resolve(root);
    let reserved = speaker_library_path
        .map(|path| (ReservedLocation::SpeakerLibrary, path))
        .into_iter()
        .chain([(ReservedLocation::ModelDirectory, model_directory)]);
    for (location, reserved) in reserved {
        let reserved_resolved = resolve(reserved);
        if resolved.starts_with(&reserved_resolved) || reserved_resolved.starts_with(&resolved) {
            return Err(DocumentsRootError::Reserved {
                path: root.to_path_buf(),
                location,
                reserved: reserved.to_path_buf(),
            });
        }
    }
    match std::fs::metadata(root) {
        Ok(metadata) if !metadata.is_dir() => Err(DocumentsRootError::NotADirectory {
            path: root.to_path_buf(),
        }),
        _ => Ok(()),
    }
}

/// Whether `a` and `b` name the same location once resolved.
pub(crate) fn same_location(a: &Path, b: &Path) -> bool {
    resolve(a) == resolve(b)
}

/// `path` made absolute and lexically normalized, with its longest existing
/// ancestor canonicalized so that symbolic links compare equal.
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(canonical, |path, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => return normalized,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "meetnote-documents-root-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn resolving_normalizes_dots_and_follows_existing_ancestors() {
        let root = temp_root("resolve");
        let plain = root.join("a/b");
        let dotted = root.join("a/./c/../b");
        assert!(same_location(&plain, &dotted));
        assert!(!same_location(&plain, &root.join("a/c")));
        let canonical = root.canonicalize().unwrap();
        assert_eq!(resolve(&root.join("missing")), canonical.join("missing"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
