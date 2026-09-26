//! Why a path under the documents root did not become a tree document.
//!
//! The variants and fields match the persistence scanner's
//! `DocumentScanError`, so the scanner can use this type directly and turn
//! each one into a [`DocumentIssue`] with [`DocumentIssue::from_kind`].

use crate::DocumentIssue;
use domain::DocumentId;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentIssueKind {
    /// A directory whose `state.json`/`document.md` cannot be loaded;
    /// `message` is the load error's display text.
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
    /// A non-hidden entry of the documents root that is not a directory.
    UnexpectedFile {
        path: PathBuf,
    },
}

impl DocumentIssueKind {
    /// The issue for a root entry that is not a directory, or `None` for a
    /// hidden entry (a name starting with `.`, such as `.DS_Store` or an
    /// editor swap file), which the scanner skips silently.
    pub fn unexpected_root_entry(path: &Path) -> Option<Self> {
        let hidden = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        (!hidden).then(|| Self::UnexpectedFile {
            path: path.to_path_buf(),
        })
    }

    /// The package directory or file the issue is about. Relation issues
    /// point at the package directory named by the document id.
    pub fn path(&self, root: &Path) -> PathBuf {
        match self {
            Self::InvalidPackage { path, .. }
            | Self::DuplicateDocumentId { path, .. }
            | Self::UnexpectedFile { path } => path.clone(),
            Self::MissingParent { document_id, .. }
            | Self::Cycle { document_id }
            | Self::MaximumDepth { document_id }
            | Self::CrossAudioAsset { document_id }
            | Self::DepthMismatch { document_id, .. } => root.join(document_id.as_str()),
        }
    }
}

impl std::fmt::Display for DocumentIssueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPackage { message, .. } => {
                write!(f, "The document package cannot be read: {message}.")
            }
            Self::DuplicateDocumentId { document_id, .. } => write!(
                f,
                "Another document package already uses the document id {document_id}."
            ),
            Self::MissingParent {
                document_id,
                parent_id,
            } => write!(
                f,
                "Document {document_id} refers to parent {parent_id}, which does not exist."
            ),
            Self::Cycle { document_id } => write!(
                f,
                "Document {document_id} is its own ancestor through its parent links."
            ),
            Self::MaximumDepth { document_id } => write!(
                f,
                "Document {document_id} is deeper than the three-level tree limit."
            ),
            Self::CrossAudioAsset { document_id } => write!(
                f,
                "Document {document_id} has a parent from a different recording."
            ),
            Self::DepthMismatch {
                document_id,
                declared,
                actual,
            } => write!(
                f,
                "Document {document_id} declares depth {declared}, but its parent places it at depth {actual}."
            ),
            Self::UnexpectedFile { .. } => f.write_str(
                "This file is not a document package; each document is a folder with document.md and state.json.",
            ),
        }
    }
}

impl DocumentIssue {
    /// The issue the UI lists: the affected path, a readable English
    /// sentence and the kind a client localizes.
    pub fn from_kind(root: &Path, kind: &DocumentIssueKind) -> Self {
        Self {
            path: kind.path(root).display().to_string(),
            message: kind.to_string(),
            kind: Some(kind.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> DocumentId {
        DocumentId::new(value)
    }

    fn every_kind() -> Vec<DocumentIssueKind> {
        vec![
            DocumentIssueKind::InvalidPackage {
                path: PathBuf::from("/work/documents/broken"),
                message: "cannot read document package file /work/documents/broken/state.json: \
                          No such file or directory (os error 2)"
                    .into(),
            },
            DocumentIssueKind::DuplicateDocumentId {
                document_id: id("doc-1"),
                path: PathBuf::from("/work/documents/copy"),
            },
            DocumentIssueKind::MissingParent {
                document_id: id("doc-2"),
                parent_id: id("gone"),
            },
            DocumentIssueKind::Cycle {
                document_id: id("doc-3"),
            },
            DocumentIssueKind::MaximumDepth {
                document_id: id("doc-4"),
            },
            DocumentIssueKind::CrossAudioAsset {
                document_id: id("doc-5"),
            },
            DocumentIssueKind::DepthMismatch {
                document_id: id("doc-6"),
                declared: 2,
                actual: 1,
            },
            DocumentIssueKind::UnexpectedFile {
                path: PathBuf::from("/work/documents/notes.txt"),
            },
        ]
    }

    #[test]
    fn every_issue_kind_reads_as_a_sentence_without_debug_text() {
        for kind in every_kind() {
            let message = kind.to_string();
            assert!(message.starts_with(char::is_uppercase), "{message}");
            assert!(message.ends_with('.'), "{message}");
            for artefact in ["{", "}", "ProjectId", "PathBuf", "Some(", "\\\""] {
                assert!(!message.contains(artefact), "{artefact} in {message}");
            }
            assert_ne!(message, format!("{kind:?}"));
        }
        assert_eq!(
            DocumentIssueKind::DepthMismatch {
                document_id: id("doc-6"),
                declared: 2,
                actual: 1,
            }
            .to_string(),
            "Document doc-6 declares depth 2, but its parent places it at depth 1."
        );
    }

    #[test]
    fn issues_point_at_the_package_or_file_they_describe() {
        let root = Path::new("/work/documents");
        let paths: Vec<_> = every_kind()
            .iter()
            .map(|kind| DocumentIssue::from_kind(root, kind).path)
            .collect();

        assert_eq!(
            paths,
            [
                "/work/documents/broken",
                "/work/documents/copy",
                "/work/documents/doc-2",
                "/work/documents/doc-3",
                "/work/documents/doc-4",
                "/work/documents/doc-5",
                "/work/documents/doc-6",
                "/work/documents/notes.txt",
            ]
        );
        let issue = DocumentIssue::from_kind(
            root,
            &DocumentIssueKind::Cycle {
                document_id: id("doc-3"),
            },
        );
        assert_eq!(
            issue.message,
            "Document doc-3 is its own ancestor through its parent links."
        );
    }

    #[test]
    fn issues_built_from_a_kind_carry_it_beside_the_english_sentence() {
        let root = Path::new("/work/documents");
        for kind in every_kind() {
            let issue = DocumentIssue::from_kind(root, &kind);
            assert_eq!(issue.kind.as_ref(), Some(&kind));
            assert_eq!(issue.message, kind.to_string());
        }
    }

    #[test]
    fn plain_root_files_are_issues_unless_hidden() {
        let root = Path::new("/work/documents");

        assert_eq!(
            DocumentIssueKind::unexpected_root_entry(&root.join("notes.txt")),
            Some(DocumentIssueKind::UnexpectedFile {
                path: root.join("notes.txt")
            })
        );
        assert_eq!(
            DocumentIssueKind::unexpected_root_entry(&root.join(".DS_Store")),
            None
        );
        assert_eq!(
            DocumentIssueKind::unexpected_root_entry(&root.join(".state.json.swp")),
            None
        );
    }
}
