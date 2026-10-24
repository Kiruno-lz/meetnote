use domain::{DocumentState, ProjectId};
use persistence::{DocumentPackage, DocumentPackageError, MAX_DOCUMENT_PACKAGE_FILE_BYTES};

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
