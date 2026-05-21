use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::task::AbortHandle;

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
pub struct DownloadProgressEvent {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
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
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete {}: {}", path, e))
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

#[tauri::command]
pub async fn start_model_download(
    model_id: String,
    url: String,
    dest_path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
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
