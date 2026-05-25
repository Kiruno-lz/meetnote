use std::process::Stdio;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AsrProgressEvent {
    pub stage: String,
    pub message: String,
}

/// Expand ~ to home directory
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return if path == "~" {
                home
            } else {
                format!("{}/{}", home, &path[2..])
            };
        }
    }
    path.to_string()
}

/// Find the repo root by walking up from the current executable.
/// In Tauri dev mode, the binary is at src-tauri/target/debug/<bin>,
/// so 4 levels up lands at the repo root.
/// Falls back to current_dir() if the walk fails.
fn find_repo_root() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut path = exe;
        // Walk up: binary → debug → target → src-tauri → repo_root
        for _ in 0..4 {
            path = match path.parent() {
                Some(p) => p.to_path_buf(),
                None => break,
            };
        }
        if path.join("python/asr_cli.py").exists() {
            return Some(path);
        }
    }
    // Fallback: try current working directory
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("python/asr_cli.py").exists() {
            return Some(cwd);
        }
        // Try parent of cwd (in case we're running from src-tauri/)
        if let Some(parent) = cwd.parent() {
            if parent.join("python/asr_cli.py").exists() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

/// Find the uv binary by trying common install locations.
/// macOS apps get a minimal PATH so ~/.local/bin is usually missing.
fn find_uv_binary() -> Result<std::path::PathBuf, String> {
    // Collect candidate paths
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<std::path::PathBuf> = vec![
        // Astral default install location
        std::path::PathBuf::from(format!("{}/.local/bin/uv", home)),
        // Homebrew (Apple Silicon)
        std::path::PathBuf::from("/opt/homebrew/bin/uv"),
        // Homebrew (Intel)
        std::path::PathBuf::from("/usr/local/bin/uv"),
        // Cargo install
        std::path::PathBuf::from(format!("{}/.cargo/bin/uv", home)),
    ];

    // Also try PATH-based lookup
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            candidates.push(std::path::PathBuf::from(dir).join("uv"));
        }
    }

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "'uv' not found. Checked: {}. Install from https://docs.astral.sh/uv/",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[tauri::command]
pub async fn run_asr(
    audio_path: String,
    model_id: String,
    language: String,
    device: Option<String>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let audio_path = expand_tilde(&audio_path);

    // Find python script
    let repo_root = find_repo_root()
        .ok_or_else(|| "Cannot find repository root (python/asr_cli.py not found)".to_string())?;
    let script_path = repo_root.join("python/asr_cli.py");
    if !script_path.exists() {
        return Err(format!("ASR script not found: {}", script_path.display()));
    }

    // Find uv binary (macOS apps get a minimal PATH, so ~/.local/bin is usually missing)
    let uv_path = find_uv_binary()?;

    // Build command: uv run python <script> --audio <path> --model <model> --language <lang>
    let mut cmd = Command::new(&uv_path);
    cmd.arg("run")
        .arg("python")
        .arg(script_path.to_str().unwrap_or("python/asr_cli.py"))
        .arg("--audio")
        .arg(&audio_path)
        .arg("--model")
        .arg(&model_id)
        .arg("--language")
        .arg(&language);
    if let Some(dev) = &device {
        cmd.arg("--device").arg(dev);
    }
    cmd.current_dir(repo_root.join("python"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn ASR process: {}", e))?;

    // Stream stderr for progress events
    let stderr = child.stderr.take().ok_or("No stderr handle")?;
    let app_clone = app.clone();
    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.starts_with("PROGRESS:") {
                let rest = &line["PROGRESS:".len()..];
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                let stage = parts.get(1).copied().unwrap_or("processing").to_string();
                let _ = app_clone.emit(
                    "asr-progress",
                    AsrProgressEvent {
                        stage: stage.clone(),
                        message: stage,
                    },
                );
            }
        }
    });

    // Read stdout (the JSON result)
    let stdout = child.stdout.take().ok_or("No stdout handle")?;
    let mut stdout_reader = BufReader::new(stdout);
    let mut output = String::new();
    stdout_reader
        .read_to_string(&mut output)
        .await
        .map_err(|e| format!("Failed to read ASR output: {}", e))?;

    let status = child
        .wait()
        .await
        .map_err(|e| format!("ASR process error: {}", e))?;
    let _ = stderr_task.await;

    if !status.success() {
        // Try to parse error JSON from stdout
        if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(&output) {
            if let Some(err_msg) = err_json.get("error").and_then(|v| v.as_str()) {
                return Err(err_msg.to_string());
            }
        }
        return Err(format!("ASR process exited with code {:?}", status.code()));
    }

    // Parse the output JSON to validate it
    let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
        let preview = &output[..output.len().min(200)];
        format!("Failed to parse ASR output: {} (output: {})", e, preview)
    })?;

    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }

    Ok(output)
}
