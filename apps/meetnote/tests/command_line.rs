use std::process::Command;

#[test]
fn agent_command_is_handled_before_rune_window_options() {
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
fn rune_executable_serves_the_workspace_capability_description() {
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
fn rune_executable_uses_the_configured_data_directory_from_another_working_directory() {
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
fn rune_executable_reports_missing_process_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meetnote"))
        .current_dir(directory.path())
        .arg("process")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: meetnote process <audio>"));
}

#[test]
fn first_launch_opens_a_real_workspace_and_sample_mode_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let tree = directory.path().join("tree.json");
    let binary = env!("CARGO_BIN_EXE_meetnote");
    let run = |sample: bool| {
        let mut command = Command::new(binary);
        command
            .current_dir(directory.path())
            .env_remove("MEETNOTE_SETTINGS_PATH")
            .env_remove("MEETNOTE_DOCUMENTS_ROOT")
            .env_remove("MEETNOTE_MODEL_ROOT")
            .args([
                "--tree",
                tree.to_str().unwrap(),
                "--size",
                "900x600",
                "--realtime",
                "--at",
                "0.75",
            ]);
        if sample {
            command.arg("--sample");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::read_to_string(&tree).unwrap()
    };
    assert!(run(true).contains("Product sync"));
    assert!(!directory.path().join("data/documents").exists());
    let live = run(false);
    assert!(directory.path().join("data/documents").is_dir());
    assert!(live.contains("还没有录音"));
    assert!(!live.contains("Product sync"));
}
