use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::task::AbortHandle;

// ── Path utilities ────────────────────────────────────────────────────────────

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

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum DownloadStatus {
    Idle,
    Queued,
    Downloading,
    Paused,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DownloadState {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: DownloadStatus,
    pub dest_path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressEvent {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileInfo {
    pub model_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_active: bool,
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct DownloadManager {
    pub states: Mutex<HashMap<String, DownloadState>>,
    pub abort_handles: Mutex<HashMap<String, AbortHandle>>,
    pub active_model_id: Mutex<String>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            abort_handles: Mutex::new(HashMap::new()),
            active_model_id: Mutex::new(String::new()),
        }
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_cached_models(
    cache_dir: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<Vec<ModelFileInfo>, String> {
    let cache_dir = expand_tilde(&cache_dir);
    let active_id = state.active_model_id.lock().unwrap().clone();
    let mut results = Vec::new();

    let read_dir = std::fs::read_dir(&cache_dir)
        .map_err(|e| format!("Cannot read cache dir {}: {}", cache_dir, e))?;

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_file() {
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let model_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let path_str = path.to_string_lossy().to_string();
            let is_active = model_id == active_id;
            results.push(ModelFileInfo {
                model_id,
                path: path_str,
                size_bytes,
                is_active,
            });
        }
    }

    Ok(results)
}

#[tauri::command]
pub async fn delete_model_file(path: String) -> Result<(), String> {
    let path = expand_tilde(&path);
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete {}: {}", path, e))
}

/// Returns true if `path` is a directory containing at least one entry.
fn cache_dir_has_content(path: &std::path::Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Check whether a library-managed model (Paraformer / Qwen3-ASR / MiMo) has
/// already been downloaded into the HuggingFace or ModelScope cache.
///
/// `model_source` is the repo id used by the Python libraries, e.g.
/// "funasr/paraformer-zh" or "Qwen/Qwen3-ASR-1.7B".
#[tauri::command]
pub async fn check_library_model_installed(model_source: String) -> Result<bool, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = std::path::PathBuf::from(&home);

    // HuggingFace cache: ~/.cache/huggingface/hub/models--{org}--{name}
    let hf_dir_name = format!("models--{}", model_source.replace('/', "--"));
    let hf_path = home_path.join(".cache/huggingface/hub").join(&hf_dir_name);

    // ModelScope cache (legacy layout): ~/.cache/modelscope/hub/{org}/{name}
    let ms_path = home_path.join(".cache/modelscope/hub").join(&model_source);

    // ModelScope cache (newer layout): ~/.cache/modelscope/hub/models/{org}/{name}
    let ms_path_new = home_path
        .join(".cache/modelscope/hub/models")
        .join(&model_source);

    let installed = cache_dir_has_content(&hf_path)
        || cache_dir_has_content(&ms_path)
        || cache_dir_has_content(&ms_path_new);

    Ok(installed)
}

#[tauri::command]
pub fn get_active_model_id(state: tauri::State<'_, Arc<DownloadManager>>) -> String {
    state.active_model_id.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_active_model_id(
    model_id: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    let mut active = state.active_model_id.lock().unwrap();
    *active = model_id;
    Ok(())
}

// Tauri commands take their inputs as flat parameters (plus the injected `app`
// and `state`), so this naturally exceeds clippy's 7-arg threshold. Grouping them
// into a struct would only obscure the command's IPC surface.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_model_download(
    model_id: String,
    url: String,
    dest_path: String,
    download_method: Option<String>,
    backend: Option<String>,
    hf_token: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    // Route to library-managed download if specified
    if let Some(method) = download_method {
        if method == "library-managed" {
            let backend = backend.ok_or("backend is required for library-managed download")?;
            return start_library_download(model_id, backend, hf_token, app, state).await;
        }
    }

    // Otherwise, use HTTP download (existing logic)
    let dest_path = expand_tilde(&dest_path);

    // 1. Check existing state
    let resume_from = {
        let states = state.states.lock().unwrap();
        if let Some(s) = states.get(&model_id) {
            match &s.status {
                DownloadStatus::Downloading => return Err("Already downloading".into()),
                DownloadStatus::Completed => return Err("Already completed".into()),
                DownloadStatus::Paused => s.downloaded_bytes,
                _ => 0,
            }
        } else {
            0
        }
    };

    // 2. Build reqwest client
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
    }

    // 3. Set state to Downloading
    {
        state.states.lock().unwrap().insert(
            model_id.clone(),
            DownloadState {
                model_id: model_id.clone(),
                downloaded_bytes: resume_from,
                total_bytes: 0,
                status: DownloadStatus::Downloading,
                dest_path: dest_path.clone(),
            },
        );
    }

    // 4. Spawn async task
    let model_id_clone = model_id.clone();
    let state_clone = state.inner().clone();
    let app_clone = app.clone();

    let handle = tokio::spawn(async move {
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = app_clone.emit(
                    "model-download-progress",
                    DownloadProgressEvent {
                        model_id: model_id_clone.clone(),
                        downloaded_bytes: resume_from,
                        total_bytes: 0,
                        speed_bps: 0.0,
                        status: format!("failed: {}", e),
                    },
                );
                if let Ok(mut states) = state_clone.states.lock() {
                    if let Some(s) = states.get_mut(&model_id_clone) {
                        s.status = DownloadStatus::Failed(e.to_string());
                    }
                }
                return;
            }
        };

        let total = resp.content_length().unwrap_or(0) + resume_from;

        // Update total in state
        if let Ok(mut states) = state_clone.states.lock() {
            if let Some(s) = states.get_mut(&model_id_clone) {
                s.total_bytes = total;
            }
        }

        let mut stream = resp.bytes_stream();
        let mut downloaded = resume_from;
        let mut last_emit = std::time::Instant::now();
        let mut last_bytes = resume_from;

        // Open dest file (append for resume, create for fresh)
        use std::io::Write;
        let open_result = if resume_from > 0 {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&dest_path)
                // Fallback: if file missing, start fresh
                .or_else(|_| {
                    if let Some(parent) = std::path::Path::new(&dest_path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .open(&dest_path)
                })
        } else {
            if let Some(parent) = std::path::Path::new(&dest_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .open(&dest_path)
        };

        let mut file = match open_result {
            Ok(f) => f,
            Err(e) => {
                let _ = app_clone.emit(
                    "model-download-progress",
                    DownloadProgressEvent {
                        model_id: model_id_clone.clone(),
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                        speed_bps: 0.0,
                        status: format!("failed: {}", e),
                    },
                );
                if let Ok(mut states) = state_clone.states.lock() {
                    if let Some(s) = states.get_mut(&model_id_clone) {
                        s.status = DownloadStatus::Failed(e.to_string());
                    }
                }
                return;
            }
        };

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if let Err(e) = file.write_all(&bytes) {
                        let _ = app_clone.emit(
                            "model-download-progress",
                            DownloadProgressEvent {
                                model_id: model_id_clone.clone(),
                                downloaded_bytes: downloaded,
                                total_bytes: total,
                                speed_bps: 0.0,
                                status: format!("failed: {}", e),
                            },
                        );
                        if let Ok(mut states) = state_clone.states.lock() {
                            if let Some(s) = states.get_mut(&model_id_clone) {
                                s.status = DownloadStatus::Failed(e.to_string());
                            }
                        }
                        return;
                    }
                    downloaded += bytes.len() as u64;

                    let now = std::time::Instant::now();
                    let delta_ms = now.duration_since(last_emit).as_millis();
                    if delta_ms >= 200 {
                        let speed = if delta_ms > 0 {
                            (downloaded - last_bytes) as f64 / (delta_ms as f64 / 1000.0)
                        } else {
                            0.0
                        };
                        let event = DownloadProgressEvent {
                            model_id: model_id_clone.clone(),
                            downloaded_bytes: downloaded,
                            total_bytes: total,
                            speed_bps: speed,
                            status: "downloading".into(),
                        };
                        let _ = app_clone.emit("model-download-progress", event);
                        last_emit = now;
                        last_bytes = downloaded;

                        if let Ok(mut states) = state_clone.states.lock() {
                            if let Some(s) = states.get_mut(&model_id_clone) {
                                s.downloaded_bytes = downloaded;
                                s.total_bytes = total;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = app_clone.emit(
                        "model-download-progress",
                        DownloadProgressEvent {
                            model_id: model_id_clone.clone(),
                            downloaded_bytes: downloaded,
                            total_bytes: total,
                            speed_bps: 0.0,
                            status: format!("failed: {}", e),
                        },
                    );
                    if let Ok(mut states) = state_clone.states.lock() {
                        if let Some(s) = states.get_mut(&model_id_clone) {
                            s.status = DownloadStatus::Failed(e.to_string());
                        }
                    }
                    return;
                }
            }
        }

        // Emit completed
        let _ = app_clone.emit(
            "model-download-progress",
            DownloadProgressEvent {
                model_id: model_id_clone.clone(),
                downloaded_bytes: downloaded,
                total_bytes: total,
                speed_bps: 0.0,
                status: "completed".into(),
            },
        );
        if let Ok(mut states) = state_clone.states.lock() {
            if let Some(s) = states.get_mut(&model_id_clone) {
                s.status = DownloadStatus::Completed;
                s.downloaded_bytes = downloaded;
            }
        }
    });

    // Store abort handle
    state
        .abort_handles
        .lock()
        .unwrap()
        .insert(model_id.clone(), handle.abort_handle());

    Ok(())
}

// ── Library-managed download ──────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum PythonEvent {
    Progress {
        stage: String,
        progress: f64,
    },
    Result {
        success: bool,
        cache_path: Option<String>,
        error: Option<String>,
        code: Option<String>,
    },
}

async fn start_library_download(
    model_id: String,
    backend: String,
    hf_token: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    // 1. Check existing state
    {
        let states = state.states.lock().unwrap();
        if let Some(s) = states.get(&model_id) {
            match &s.status {
                DownloadStatus::Downloading => return Err("Already downloading".into()),
                DownloadStatus::Completed => return Err("Already completed".into()),
                _ => {}
            }
        }
    }

    // 2. Find uv and model_downloader.py. Reuse asr.rs helpers so the lookup is
    // robust in bundled apps (current_dir() is not the project root there) and the
    // uv binary is found even when the app gets a minimal PATH (macOS .app bundles).
    let uv_path = crate::asr::find_uv_binary()?;

    let project_root = crate::asr::find_repo_root()
        .ok_or_else(|| "Cannot find repository root (python/asr_cli.py not found)".to_string())?;
    let downloader_script = project_root.join("python").join("model_downloader.py");

    if !downloader_script.exists() {
        return Err(format!(
            "model_downloader.py not found at {}",
            downloader_script.display()
        ));
    }

    // 3. Set state to Downloading
    {
        state.states.lock().unwrap().insert(
            model_id.clone(),
            DownloadState {
                model_id: model_id.clone(),
                downloaded_bytes: 0,
                total_bytes: 0,
                status: DownloadStatus::Downloading,
                dest_path: String::new(), // Will be set from result
            },
        );
    }

    // 4. Build command
    let mut cmd = Command::new(uv_path);
    cmd.arg("run")
        .arg("python")
        .arg(&downloader_script)
        .arg("--backend")
        .arg(&backend)
        .arg("--model-id")
        .arg(&model_id);

    if let Some(token) = hf_token {
        cmd.arg("--hf-token").arg(token);
    }

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(project_root.join("python"));

    // 5. Spawn process
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn model_downloader.py: {}", e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

    // 6. Spawn async task to monitor output
    let model_id_clone = model_id.clone();
    let state_clone = state.inner().clone();
    let app_clone = app.clone();

    let handle = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            // Parse JSON event
            match serde_json::from_str::<PythonEvent>(&line) {
                Ok(event) => match event {
                    PythonEvent::Progress { stage, progress } => {
                        // Map progress (0.0-1.0) to bytes for UI consistency
                        // Use arbitrary total (100MB) for progress bar
                        let total_bytes = 100_000_000u64;
                        let downloaded_bytes = (progress * total_bytes as f64) as u64;

                        let status_str = match stage.as_str() {
                            "starting" => "starting",
                            "downloading" => "downloading",
                            "loading" => "loading",
                            "completed" => "completed",
                            _ => "downloading",
                        };

                        let event = DownloadProgressEvent {
                            model_id: model_id_clone.clone(),
                            downloaded_bytes,
                            total_bytes,
                            speed_bps: 0.0, // Library download doesn't provide speed
                            status: status_str.to_string(),
                        };

                        let _ = app_clone.emit("model-download-progress", event);

                        // Update state
                        if let Ok(mut states) = state_clone.states.lock() {
                            if let Some(s) = states.get_mut(&model_id_clone) {
                                s.downloaded_bytes = downloaded_bytes;
                                s.total_bytes = total_bytes;
                            }
                        }
                    }
                    PythonEvent::Result {
                        success,
                        cache_path,
                        error,
                        code,
                    } => {
                        if success {
                            let total_bytes = 100_000_000u64;
                            let event = DownloadProgressEvent {
                                model_id: model_id_clone.clone(),
                                downloaded_bytes: total_bytes,
                                total_bytes,
                                speed_bps: 0.0,
                                status: "completed".to_string(),
                            };
                            let _ = app_clone.emit("model-download-progress", event);

                            if let Ok(mut states) = state_clone.states.lock() {
                                if let Some(s) = states.get_mut(&model_id_clone) {
                                    s.status = DownloadStatus::Completed;
                                    s.downloaded_bytes = total_bytes;
                                    if let Some(path) = cache_path {
                                        s.dest_path = path;
                                    }
                                }
                            }
                        } else {
                            let error_msg = error.unwrap_or_else(|| "Unknown error".to_string());
                            let error_code = code.unwrap_or_else(|| "UNKNOWN_ERROR".to_string());
                            let status_msg = format!("failed: {} ({})", error_msg, error_code);

                            let event = DownloadProgressEvent {
                                model_id: model_id_clone.clone(),
                                downloaded_bytes: 0,
                                total_bytes: 0,
                                speed_bps: 0.0,
                                status: status_msg.clone(),
                            };
                            let _ = app_clone.emit("model-download-progress", event);

                            if let Ok(mut states) = state_clone.states.lock() {
                                if let Some(s) = states.get_mut(&model_id_clone) {
                                    s.status = DownloadStatus::Failed(status_msg);
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Failed to parse JSON event: {} | Line: {}", e, line);
                }
            }
        }

        // Wait for process to complete
        let _ = child.wait().await;
    });

    // Store abort handle
    state
        .abort_handles
        .lock()
        .unwrap()
        .insert(model_id.clone(), handle.abort_handle());

    Ok(())
}

#[tauri::command]
pub fn pause_model_download(
    model_id: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    // Abort the download task
    {
        let mut handles = state.abort_handles.lock().unwrap();
        if let Some(handle) = handles.remove(&model_id) {
            handle.abort();
        }
    }

    // Update state to Paused, keeping downloaded_bytes for resume
    {
        let mut states = state.states.lock().unwrap();
        if let Some(s) = states.get_mut(&model_id) {
            s.status = DownloadStatus::Paused;
        } else {
            return Err(format!("No download found for model_id: {}", model_id));
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn get_file_size(path: String) -> Result<u64, String> {
    let expanded = expand_tilde(&path);
    std::fs::metadata(&expanded)
        .map(|m| m.len())
        .map_err(|e| format!("Cannot stat {}: {}", expanded, e))
}

#[tauri::command]
pub fn cancel_model_download(
    model_id: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    // Abort the download task
    let dest_path = {
        let mut handles = state.abort_handles.lock().unwrap();
        if let Some(handle) = handles.remove(&model_id) {
            handle.abort();
        }

        // Get dest_path before clearing state
        let states = state.states.lock().unwrap();
        states.get(&model_id).map(|s| s.dest_path.clone())
    };

    // Clear state
    {
        let mut states = state.states.lock().unwrap();
        states.remove(&model_id);
    }

    // Delete partial file if it exists
    if let Some(path) = dest_path {
        if std::path::Path::new(&path).exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_progress_event_serializes_camel_case() {
        let event = DownloadProgressEvent {
            model_id: "whisper-base".into(),
            downloaded_bytes: 1024,
            total_bytes: 2048,
            speed_bps: 512.0,
            status: "downloading".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["modelId"], "whisper-base");
        assert_eq!(json["downloadedBytes"], 1024);
        assert_eq!(json["totalBytes"], 2048);
        assert_eq!(json["speedBps"], 512.0);
        assert!(
            json.get("model_id").is_none(),
            "snake_case keys must not appear"
        );
    }

    #[test]
    fn model_file_info_serializes_camel_case() {
        let info = ModelFileInfo {
            model_id: "whisper-base".into(),
            path: "/tmp/whisper-base.bin".into(),
            size_bytes: 148_000_000,
            is_active: true,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["modelId"], "whisper-base");
        assert_eq!(json["sizeBytes"], 148_000_000u64);
        assert_eq!(json["isActive"], true);
        assert!(
            json.get("model_id").is_none(),
            "snake_case keys must not appear"
        );
        assert!(
            json.get("size_bytes").is_none(),
            "snake_case keys must not appear"
        );
        assert!(
            json.get("is_active").is_none(),
            "snake_case keys must not appear"
        );
    }

    // ── cache_dir_has_content tests ───────────────────────────────────────────

    #[test]
    fn cache_dir_has_content_nonexistent_path_returns_false() {
        let path = std::path::PathBuf::from("/nonexistent/path/that/does/not/exist/12345");
        assert!(!cache_dir_has_content(&path));
    }

    #[test]
    fn cache_dir_has_content_empty_dir_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "meetnote_test_empty_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let result = cache_dir_has_content(&tmp);
        std::fs::remove_dir(&tmp).ok();
        assert!(!result, "empty directory should return false");
    }

    #[test]
    fn cache_dir_has_content_dir_with_file_returns_true() {
        let tmp = std::env::temp_dir().join(format!(
            "meetnote_test_nonempty_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let file_path = tmp.join("model.bin");
        std::fs::write(&file_path, b"fake model data").expect("write temp file");
        let result = cache_dir_has_content(&tmp);
        std::fs::remove_file(&file_path).ok();
        std::fs::remove_dir(&tmp).ok();
        assert!(result, "directory containing a file should return true");
    }

    #[test]
    fn cache_dir_has_content_file_path_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "meetnote_test_file_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::write(&tmp, b"i am a file, not a dir").expect("write temp file");
        let result = cache_dir_has_content(&tmp);
        std::fs::remove_file(&tmp).ok();
        assert!(!result, "a file path (not a directory) should return false");
    }
}
