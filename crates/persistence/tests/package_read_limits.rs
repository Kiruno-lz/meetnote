use application::{DocumentCommandService, DocumentRepository, DocumentService};
use contracts::DocumentPatch;
use domain::{DocumentState, ProjectId};
use persistence::FileDocumentRepository;
use persistence::{DocumentPackage, DocumentPackageError, MAX_DOCUMENT_PACKAGE_FILE_BYTES};
use std::sync::Arc;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("meetnote-write-limit-{}", ProjectId::new_uuid()));
        let package = DocumentPackage::new(
            root.join("meeting"),
            DocumentState::new(ProjectId::new("meeting"), "Review", "audio.wav"),
            "Original notes",
        )
        .unwrap();
        package.save().unwrap();
        Self(root)
    }

    fn bytes(&self) -> [Vec<u8>; 2] {
        ["state.json", "document.md"]
            .map(|name| std::fs::read(self.0.join("meeting").join(name)).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn oversized_update_preserves_files(state_overflow: bool) {
    let fixture = Fixture::new();
    let repository = Arc::new(FileDocumentRepository::open(&fixture.0).unwrap());
    let original = repository
        .read(&ProjectId::new("meeting"))
        .unwrap()
        .unwrap();
    let before = fixture.bytes();
    let patch = if state_overflow {
        DocumentPatch {
            summary: Some(Some("\n".repeat(MAX_DOCUMENT_PACKAGE_FILE_BYTES / 2 + 1))),
            ..Default::default()
        }
    } else {
        DocumentPatch {
            markdown: Some("x".repeat(MAX_DOCUMENT_PACKAGE_FILE_BYTES + 1)),
            ..Default::default()
        }
    };
    let result = DocumentCommandService::new(DocumentService::new(repository.clone()))
        .update_document(original.state.id(), original.state.revision(), patch);
    assert!(
        result.is_err(),
        "an oversized update must fail before committing"
    );
    assert!(
        fixture.bytes() == before,
        "a refused update must preserve state, revision and Markdown"
    );
    assert_eq!(
        repository.read(original.state.id()).unwrap().unwrap(),
        original
    );
}

#[test]
fn oversized_markdown_updates_preserve_both_original_files() {
    oversized_update_preserves_files(false);
}

#[test]
fn oversized_serialized_state_updates_preserve_both_original_files() {
    oversized_update_preserves_files(true);
}

#[test]
fn saving_over_an_unreadable_package_keeps_its_assets() {
    let fixture = Fixture::new();
    let repository = FileDocumentRepository::open(&fixture.0).unwrap();
    let document = repository
        .read(&ProjectId::new("meeting"))
        .unwrap()
        .unwrap()
        .state;
    std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.0.join("meeting/document.md"))
        .unwrap()
        .set_len(MAX_DOCUMENT_PACKAGE_FILE_BYTES as u64 + 1)
        .unwrap();
    let before = fixture.bytes();
    assert!(
        repository.save(document, None).is_err(),
        "an unreadable existing package is not a new document"
    );
    assert!(fixture.bytes() == before);
}

#[test]
fn markdown_exactly_at_the_input_limit_round_trips() {
    let fixture = Fixture::new();
    let mut package = DocumentPackage::load(fixture.0.join("meeting")).unwrap();
    package.markdown = "x".repeat(MAX_DOCUMENT_PACKAGE_FILE_BYTES);
    package.save().unwrap();
    let reopened = DocumentPackage::load(fixture.0.join("meeting")).unwrap();
    assert!(reopened.markdown == package.markdown);
}

#[test]
fn full_document_reads_reject_oversized_state_and_markdown_without_changing_assets() {
    let temporary =
        std::env::temp_dir().join(format!("meetnote-read-limit-{}", ProjectId::new_uuid()));
    let root = temporary.join("meeting");
    let package = DocumentPackage::new(
        &root,
        DocumentState::new(ProjectId::new("meeting"), "Review", "audio.wav"),
        "Notes",
    )
    .unwrap();
    package.save().unwrap();
    let state = std::fs::read(root.join("state.json")).unwrap();
    for file in ["state.json", "document.md"] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join(file))
            .unwrap()
            .set_len(MAX_DOCUMENT_PACKAGE_FILE_BYTES as u64 + 1)
            .unwrap();
        let result = DocumentPackage::load(&root);
        assert!(
            matches!(result, Err(DocumentPackageError::TooLarge { path, max_bytes }) if path == root.join(file) && max_bytes == MAX_DOCUMENT_PACKAGE_FILE_BYTES)
        );
        assert_eq!(
            std::fs::metadata(root.join(file)).unwrap().len(),
            MAX_DOCUMENT_PACKAGE_FILE_BYTES as u64 + 1
        );
        std::fs::write(root.join("state.json"), &state).unwrap();
        std::fs::write(root.join("document.md"), "Notes").unwrap();
    }
    assert_eq!(DocumentPackage::load(&root).unwrap().markdown, "Notes");
    std::fs::remove_dir_all(temporary).unwrap();
}
