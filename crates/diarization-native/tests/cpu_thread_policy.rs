// Compile the pinned vendor's pure device policy in workspace tests without
// requiring its separate model-fixture suite or exporting a test-only API.
#[path = "../../../third_party/speakrs/src/inference/embedding/threading.rs"]
mod cpu_thread_policy;
