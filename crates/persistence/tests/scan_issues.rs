//! The issues the file repository reports for paths under the documents root
//! that did not become tree documents: each names the affected path and reads
//! as a sentence, never as Debug output.

use application::DocumentRepository;
use domain::{DocumentId, DocumentState};
use persistence::{
    DocumentPackage, FileDocumentRepository, DOCUMENT_MARKDOWN_FILENAME, DOCUMENT_STATE_FILENAME,
};
use std::fs;
use std::path::{Path, PathBuf};

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "meetnote-scan-issues-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn document(id: &str, parent: Option<&str>, asset: Option<&str>, depth: u8) -> DocumentState {
    let mut state = DocumentState::new(DocumentId::new(id), id, "recording.m4a");
    state.hierarchy.parent_id = parent.map(DocumentId::new);
    state.source.audio_asset_id = asset.map(DocumentId::new);
    state.hierarchy.depth = depth;
    state
}

fn save(root: &Path, state: DocumentState) {
    let directory = root.join(state.id().as_str());
    DocumentPackage::new(directory, state, "# Fixture\n")
        .unwrap()
        .save()
        .unwrap();
}

/// Writes a package directory by hand, bypassing the package validation that
/// would refuse these contents.
fn write_raw_package(root: &Path, directory: &str, state: &DocumentState) {
    let directory = root.join(directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(DOCUMENT_STATE_FILENAME),
        serde_json::to_vec(state).unwrap(),
    )
    .unwrap();
    fs::write(directory.join(DOCUMENT_MARKDOWN_FILENAME), "# Fixture\n").unwrap();
}

/// A root with one entry for every issue the scanner can produce.
/// `DuplicateDocumentId` has no fixture: a package's directory name must
/// equal its id, so two packages with one id cannot exist side by side.
fn root_with_every_issue() -> PathBuf {
    let root = temp_root("every-kind");
    fs::create_dir_all(root.join("broken")).unwrap();
    let mut future = document("future", None, None, 0);
    future.schema.schema_version += 1;
    write_raw_package(&root, "future", &future);
    write_raw_package(&root, "renamed", &document("original", None, None, 0));
    save(&root, document("orphan", Some("missing"), None, 1));
    save(&root, document("loop-a", Some("loop-b"), None, 1));
    save(&root, document("loop-b", Some("loop-a"), None, 1));
    save(&root, document("top", None, None, 0));
    save(&root, document("middle", Some("top"), Some("top"), 1));
    save(&root, document("low", Some("middle"), Some("top"), 2));
    save(&root, document("deep", Some("low"), Some("top"), 3));
    save(&root, document("other-recording", None, None, 0));
    save(
        &root,
        document("foreign", Some("other-recording"), Some("top"), 1),
    );
    save(&root, document("lonely", None, None, 2));
    fs::write(root.join("notes.txt"), "loose notes\n").unwrap();
    root
}

#[test]
fn every_scanner_issue_reads_as_a_sentence_without_debug_text() {
    let root = root_with_every_issue();
    let snapshot = FileDocumentRepository::open(&root)
        .unwrap()
        .snapshot()
        .unwrap();

    assert!(!snapshot.issues.is_empty());
    for issue in &snapshot.issues {
        let message = &issue.message;
        assert!(message.starts_with(char::is_uppercase), "{message}");
        assert!(message.ends_with('.'), "{message}");
        for artefact in [
            "{",
            "}",
            "ProjectId(",
            "UnsupportedSchema {",
            "PathBuf",
            "Some(",
        ] {
            assert!(!message.contains(artefact), "{artefact} in {message}");
        }
    }
    let mut paths: Vec<_> = snapshot
        .issues
        .iter()
        .map(|issue| issue.path.clone())
        .collect();
    paths.sort();
    let mut expected: Vec<_> = [
        "broken",
        "deep",
        "foreign",
        "future",
        "lonely",
        "loop-a",
        "loop-b",
        "notes.txt",
        "orphan",
        "renamed",
    ]
    .iter()
    .map(|name| root.join(name).display().to_string())
    .collect();
    expected.sort();
    assert_eq!(paths, expected, "{:#?}", snapshot.issues);
    let message_for = |name: &str| {
        let path = root.join(name).display().to_string();
        snapshot
            .issues
            .iter()
            .find(|issue| issue.path == path)
            .map(|issue| issue.message.clone())
            .unwrap()
    };
    assert!(message_for("future").contains("unsupported document schema version"));
    assert!(message_for("renamed").contains("contains state for original"));
    assert!(message_for("orphan").contains("orphan"));
    assert!(message_for("orphan").contains("missing"));
    fs::remove_dir_all(root).unwrap();
}

fn snapshot_issue_paths(root: &Path) -> Vec<String> {
    FileDocumentRepository::open(root)
        .unwrap()
        .snapshot()
        .unwrap()
        .issues
        .into_iter()
        .map(|issue| issue.path)
        .collect()
}

#[test]
fn a_plain_root_file_is_reported_with_its_path() {
    let root = temp_root("plain-file");
    save(&root, document("kept", None, None, 0));
    fs::write(root.join("notes.txt"), "loose notes\n").unwrap();

    let snapshot = FileDocumentRepository::open(&root)
        .unwrap()
        .snapshot()
        .unwrap();

    assert_eq!(snapshot.documents.len(), 1);
    assert_eq!(snapshot.documents[0].id().as_str(), "kept");
    assert_eq!(
        snapshot_issue_paths(&root),
        [root.join("notes.txt").display().to_string()]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_hidden_root_file_is_not_reported() {
    let root = temp_root("hidden-file");
    save(&root, document("kept", None, None, 0));
    fs::write(root.join(".DS_Store"), [0_u8; 8]).unwrap();

    assert_eq!(snapshot_issue_paths(&root), Vec::<String>::new());
    fs::remove_dir_all(root).unwrap();
}
