//! The file repository's async change subscription: the watcher callback
//! pushes committed changes straight into the stream.

use application::{DocumentChange, DocumentChanges, DocumentService};
use futures::{FutureExt, StreamExt};
use persistence::FileDocumentRepository;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "meetnote-document-changes-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// The next item, or `None` if nothing arrives within `timeout`.
fn next_within(changes: &mut DocumentChanges, timeout: Duration) -> Option<DocumentChange> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(change) = changes.next().now_or_never() {
            return Some(change.expect("the change stream ended"));
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn subscribe(root: &PathBuf) -> DocumentChanges {
    let repository = FileDocumentRepository::open(root).unwrap();
    DocumentService::new(Arc::new(repository))
        .subscribe_changes()
        .unwrap()
}

#[test]
fn a_file_created_in_the_workspace_arrives_as_an_async_change() {
    let root = temp_root("created");
    let mut changes = subscribe(&root);

    fs::write(root.join("external-note.md"), "# External\n").unwrap();

    assert_eq!(
        next_within(&mut changes, Duration::from_secs(5)),
        Some(DocumentChange::Changed)
    );
    drop(changes);
    fs::remove_dir_all(root).unwrap();
}
