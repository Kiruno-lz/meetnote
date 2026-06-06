use crate::utils::{expand_tilde, find_repo_root, find_uv_binary};
use std::process::Stdio;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiarizationProgressEvent {
    pub stage: String,
    pub message: String,
}

#[tauri::command]
pub async fn run_diarization(
    audio_path: String,
    hf_token: String,
    min_speakers: Option<u32>,
    max_speakers: Option<u32>,
    device: Option<String>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let audio_path = expand_tilde(&audio_path);

    // Find python script
    let repo_root = find_repo_root().ok_or_else(|| {
        "Cannot find repository root (python/diarization_cli.py not found)".to_string()
    })?;
    let script_path = repo_root.join("python/diarization_cli.py");
    if !script_path.exists() {
        return Err(format!(
            "Diarization script not found: {}",
            script_path.display()
        ));
    }

    // Find uv binary (macOS apps get a minimal PATH, so ~/.local/bin is usually missing)
    let uv_path = find_uv_binary()?;

    // Build command: uv run python <script> --audio <path> --hf-token <token> [--min-speakers N] [--max-speakers N] [--device <dev>]
    let mut cmd = Command::new(&uv_path);
    cmd.arg("run")
        .arg("python")
        .arg(script_path.to_str().unwrap_or("python/diarization_cli.py"))
        .arg("--audio")
        .arg(&audio_path)
        .arg("--hf-token")
        .arg(&hf_token);
    if let Some(min) = min_speakers {
        cmd.arg("--min-speakers").arg(min.to_string());
    }
    if let Some(max) = max_speakers {
        cmd.arg("--max-speakers").arg(max.to_string());
    }
    if let Some(dev) = &device {
        cmd.arg("--device").arg(dev);
    }
    cmd.current_dir(repo_root.join("python"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn diarization process: {}", e))?;

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
                    "diarization-progress",
                    DiarizationProgressEvent {
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
        .map_err(|e| format!("Failed to read diarization output: {}", e))?;

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Diarization process error: {}", e))?;
    let _ = stderr_task.await;

    if !status.success() {
        // Try to parse error JSON from stdout
        if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(&output) {
            if let Some(err_msg) = err_json.get("error").and_then(|v| v.as_str()) {
                return Err(err_msg.to_string());
            }
        }
        return Err(format!(
            "Diarization process exited with code {:?}",
            status.code()
        ));
    }

    // Parse the output JSON to validate it
    let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
        let preview = &output[..output.len().min(200)];
        format!(
            "Failed to parse diarization output: {} (output: {})",
            e, preview
        )
    })?;

    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }

    Ok(output)
}
