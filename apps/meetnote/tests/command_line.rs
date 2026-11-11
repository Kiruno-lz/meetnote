use std::process::Command;

#[test]
fn agent_command_is_handled_before_desktop_startup() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .current_dir(directory.path())
        .args(["agent", "unknown", "--help"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{error}");
    assert!(error.contains("usage: meetnote agent"), "{error}");
}

#[test]
fn desktop_executable_serves_the_workspace_capability_description() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .current_dir(directory.path())
        .args(["agent", "describe", "--json", "--all-documents"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let description: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(description.to_string().contains("read_document"));
    assert!(directory.path().join("data/documents").is_dir());
}

#[test]
fn desktop_executable_uses_the_configured_data_directory_from_another_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let storage = directory.path().join("meetnote-storage");
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .current_dir(directory.path())
        .env("MEETNOTE_DATA_DIR", &storage)
        .args(["agent", "describe", "--json", "--all-documents"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(storage.join("data/documents").is_dir());
    assert!(!directory.path().join("data").exists());
}

#[test]
fn desktop_executable_reports_missing_process_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .current_dir(directory.path())
        .arg("process")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: meetnote process <audio>"));
}
