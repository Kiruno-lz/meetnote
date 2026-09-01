use domain::{DocumentId, DocumentState};
use persistence::{DocumentPackage, DocumentScanner};
use std::time::Instant;

fn main() {
    let root = std::env::temp_dir().join(format!(
        "meetnote-document-scan-bench-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for index in 0..128 {
        let id = format!("document-{index}");
        let state = DocumentState::new(
            DocumentId::new(id.clone()),
            format!("Document {index}"),
            "recording.m4a",
        );
        DocumentPackage::new(root.join(id), state, "# Benchmark document\n")
            .expect("benchmark package should be valid")
            .save()
            .expect("benchmark package should save");
    }

    let scanner = DocumentScanner::new(&root);
    let start = Instant::now();
    let mut document_count = 0;
    for _ in 0..100 {
        document_count = scanner
            .scan()
            .expect("benchmark scan should succeed")
            .packages
            .len();
    }
    let elapsed = start.elapsed();
    println!(
        "document_scan documents={} iterations=100 elapsed_ms={} avg_us={}",
        document_count,
        elapsed.as_millis(),
        elapsed.as_micros() / 100
    );
    assert_eq!(document_count, 128);
    std::fs::remove_dir_all(root).expect("benchmark directory should be removable");
}
