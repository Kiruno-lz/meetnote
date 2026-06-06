use crate::utils::{expand_tilde, find_repo_root, find_uv_binary};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::task::AbortHandle;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
    pub monitor_handles: Mutex<HashMap<String, AbortHandle>>,
    pub active_model_id: Mutex<String>,
    /// Maps model_id → child PID for library-managed downloads, so cancel can kill the process.
    pub child_pids: Mutex<HashMap<String, u32>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            abort_handles: Mutex::new(HashMap::new()),
            monitor_handles: Mutex::new(HashMap::new()),
            active_model_id: Mutex::new(String::new()),
            child_pids: Mutex::new(HashMap::new()),
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
        let metadata = entry.metadata().ok();
        let size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

        // Whisper .pt files — extract model_id from filename.
        // list_cached_models returns manifest-style ids (e.g. "whisper-base")
        // so the frontend can compare directly against MODEL_MATRIX ids.
        if path.is_file() && path.extension().map_or(false, |ex| ex == "pt") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let model_id = if stem.starts_with("whisper-") {
                stem
            } else {
                format!("whisper-{}", stem)
            };
            let path_str = path.to_string_lossy().to_string();
            let is_active = model_id == active_id;
            results.push(ModelFileInfo {
                model_id,
                path: path_str,
                size_bytes,
                is_active,
            });
        }

        // Library-managed model subdirectories — use dir name as model_id
        if path.is_dir() {
            let dir_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Only include known library-managed model dirs
            let is_known_lib = matches!(
                dir_name.as_str(),
                "paraformer-zh" | "qwen3-asr-1.7b" | "mimo-v2.5-asr"
            );
            if is_known_lib {
                let path_str = path.to_string_lossy().to_string();
                let dir_size = compute_dir_size(&path);
                let is_active = dir_name == active_id;
                results.push(ModelFileInfo {
                    model_id: dir_name,
                    path: path_str,
                    size_bytes: dir_size,
                    is_active,
                });
            }
        }
    }

    Ok(results)
}

/// Recursively compute total file size of a directory.
fn compute_dir_size(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    fn walk(dir: &std::path::Path, total: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, total);
                } else if let Ok(m) = entry.metadata() {
                    *total += m.len();
                }
            }
        }
    }
    walk(path, &mut total);
    total
}

/// File-size cache for directory monitoring.
///
/// Instead of stat'ing every file on every tick (O(files) per tick),
/// this struct remembers each file's (size, mtime) and only re-stats a
/// file when its mtime changes or it is newly discovered. Files that
/// disappear are pruned from the cache. The result is O(new + changed
/// files) per tick — effectively zero overhead once all files are on
/// disk and stable.
struct FileSizeCache {
    files: std::collections::HashMap<std::path::PathBuf, (u64, std::time::SystemTime)>,
    total: u64,
}

impl FileSizeCache {
    fn new() -> Self {
        Self {
            files: std::collections::HashMap::new(),
            total: 0,
        }
    }

    /// Walk a directory tree, updating the cache. Returns the total sum
    /// of all discovered file sizes.
    fn refresh(&mut self, root: &std::path::Path) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.walk_dir(root, &mut seen);
        // Prune entries whose files no longer exist on disk
        self.files.retain(|p, _| seen.contains(p));
        self.total
    }

    fn walk_dir(
        &mut self,
        dir: &std::path::Path,
        seen: &mut std::collections::HashSet<std::path::PathBuf>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            seen.insert(path.clone());
            if path.is_dir() {
                self.walk_dir(&path, seen);
            } else if let Ok(meta) = entry.metadata() {
                let mtime = meta
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let size = meta.len();
                if let Some(&(cached_size, cached_mtime)) = self.files.get(&path) {
                    if cached_mtime == mtime {
                        // Unchanged since last tick — keep cached value
                        continue;
                    }
                    // mtime changed — update
                    self.total = self.total.wrapping_sub(cached_size).wrapping_add(size);
                    self.files.insert(path, (size, mtime));
                } else {
                    // Newly discovered file
                    self.total = self.total.wrapping_add(size);
                    self.files.insert(path, (size, mtime));
                }
            }
        }
    }
}

#[tauri::command]
pub async fn delete_model_file(
    path: String,
    model_id: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    let path = expand_tilde(&path);
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete {}: {}", path, e))?;
    // Clear any cached download state so re-download doesn't see "Already completed"
    state.states.lock().unwrap().remove(&model_id);
    Ok(())
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

/// Map a HuggingFace repo id to the local directory name used in the unified
/// model cache. Must match FROM_REPO_KEY in asr_cli.py.
fn repo_to_dir_name(repo: &str) -> &str {
    match repo {
        "funasr/paraformer-zh" => "paraformer-zh",
        "Qwen/Qwen3-ASR-1.7B" => "qwen3-asr-1.7b",
        "XiaomiMiMo/MiMo-V2.5-ASR" => "mimo-v2.5-asr",
        _ => "unknown",
    }
}

/// Compute the target download directory for a library-managed model.
fn compute_library_target_dir(model_source: &str, cache_dir: &Option<String>) -> std::path::PathBuf {
    let dir_name = repo_to_dir_name(model_source);
    if let Some(ref cdir) = cache_dir {
        let expanded = expand_tilde(cdir);
        std::path::PathBuf::from(&expanded).join(dir_name)
    } else {
        // Fallback: HuggingFace hub cache path
        let home = std::env::var("HOME").unwrap_or_default();
        let hf_dir_name = format!("models--{}", model_source.replace('/', "--"));
        std::path::PathBuf::from(&home)
            .join(".cache/huggingface/hub")
            .join(&hf_dir_name)
    }
}

/// Check whether a library-managed model (Paraformer / Qwen3-ASR / MiMo) has
/// already been downloaded into the unified model cache directory.
///
/// `model_source` is the repo id, e.g. "funasr/paraformer-zh" or "Qwen/Qwen3-ASR-1.7B".
/// `cache_dir` is the unified model cache path.
#[tauri::command]
pub async fn check_library_model_installed(
    model_source: String,
    cache_dir: String,
) -> Result<bool, String> {
    let cache_dir = expand_tilde(&cache_dir);
    let cache_path = std::path::PathBuf::from(&cache_dir);

    let dir_name = repo_to_dir_name(&model_source);
    if dir_name == "unknown" {
        return Ok(false);
    }

    let model_dir = cache_path.join(dir_name);
    let installed = cache_dir_has_content(&model_dir);
    Ok(installed)
}

/// Remove a library-managed model from the unified model cache directory.
///
/// `model_source` is the repo id, e.g. "funasr/paraformer-zh" or "Qwen/Qwen3-ASR-1.7B".
/// `cache_dir` is the unified model cache path.
#[tauri::command]
pub async fn remove_library_model(
    model_source: String,
    cache_dir: String,
    model_id: String,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    let cache_dir = expand_tilde(&cache_dir);
    let cache_path = std::path::PathBuf::from(&cache_dir);

    let dir_name = repo_to_dir_name(&model_source);
    if dir_name == "unknown" {
        return Err(format!("Unknown library model source: {}", model_source));
    }

    let model_dir = cache_path.join(dir_name);
    if model_dir.exists() && model_dir.is_dir() {
        std::fs::remove_dir_all(&model_dir)
            .map_err(|e| format!("Failed to remove {}: {}", model_dir.display(), e))?;
    }
    // Clear any cached download state so re-download doesn't see "Already completed"
    state.states.lock().unwrap().remove(&model_id);

    Ok(())
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
    model_source: Option<String>,
    hf_token: Option<String>,
    cache_dir: Option<String>,
    total_size_hint: Option<u64>,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<DownloadManager>>,
) -> Result<(), String> {
    // Route to library-managed download if specified
    if let Some(method) = download_method {
        if method == "library-managed" {
            let backend = backend.ok_or("backend is required for library-managed download")?;
            let model_source_str = model_source.unwrap_or_else(|| model_id.clone());
            return start_library_download(
                model_id,
                backend,
                model_source_str,
                hf_token,
                cache_dir,
                total_size_hint,
                app,
                state,
            )
            .await;
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
    model_source: String,
    hf_token: Option<String>,
    cache_dir: Option<String>,
    total_size_hint: Option<u64>,
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
    let uv_path = find_uv_binary()?;

    let project_root = find_repo_root()
        .ok_or_else(|| "Cannot find repository root (python/asr_cli.py not found)".to_string())?;
    let downloader_script = project_root.join("python").join("model_downloader.py");

    if !downloader_script.exists() {
        return Err(format!(
            "model_downloader.py not found at {}",
            downloader_script.display()
        ));
    }

    // Compute the target directory for this library download (used for progress
    // monitoring and partial-file cleanup on cancel).
    let target_dir = compute_library_target_dir(&model_source, &cache_dir);
    let dest_path = target_dir.to_string_lossy().to_string();

    // 3. Set state to Downloading, carrying forward downloaded_bytes from
    //    a Paused state (so the UI resumes at the correct progress).
    let resume_bytes = {
        state
            .states
            .lock()
            .unwrap()
            .get(&model_id)
            .and_then(|s| {
                if s.status == DownloadStatus::Paused {
                    Some(s.downloaded_bytes)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    };

    {
        state.states.lock().unwrap().insert(
            model_id.clone(),
            DownloadState {
                model_id: model_id.clone(),
                downloaded_bytes: resume_bytes,
                total_bytes: total_size_hint.unwrap_or(0),
                status: DownloadStatus::Downloading,
                dest_path: dest_path.clone(),
            },
        );
    }

    // 4. Build command with process group support on unix (so kill -9 -<pgid>
    // kills uv AND the Python child process tree).
    let mut std_cmd = std::process::Command::new(&uv_path);
    #[cfg(unix)]
    {
        std_cmd.process_group(0);
    }
    std_cmd
        .arg("run")
        .arg("python")
        .arg(&downloader_script)
        .arg("--backend")
        .arg(&backend)
        .arg("--model-id")
        .arg(&model_source);

    if let Some(ref cdir) = cache_dir {
        std_cmd.arg("--cache-dir").arg(cdir);
    }

    std_cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .current_dir(project_root.join("python"));

    let mut cmd = Command::from(std_cmd);

    // 5. Spawn process
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn model_downloader.py: {}", e))?;

    // Track child PID for cancel support
    if let Some(pid) = child.id() {
        state
            .child_pids
            .lock()
            .unwrap()
            .insert(model_id.clone(), pid);
    }

    // 6. Write HF token to stdin if provided, then close stdin
    {
        let mut stdin = child.stdin.take().ok_or("Failed to capture stdin")?;
        if let Some(ref token) = hf_token {
            stdin
                .write_all(format!("{}\n", token).as_bytes())
                .await
                .map_err(|e| format!("Failed to write token to stdin: {}", e))?;
        }
        // Drop closes stdin, signaling EOF to Python
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

    // 7. Spawn a directory-monitoring task that emits real byte progress based
    // on the growing download directory, replacing the fake fraction×totalSizeHint
    // progress that Python emits.
    let monitor_model_id = model_id.clone();
    let monitor_target_dir = target_dir.clone();
    let monitor_total = total_size_hint.unwrap_or(0);
    let app_monitor = app.clone();
    let state_monitor = state.inner().clone();

    let monitor_handle = tokio::spawn(async move {
        // File-size cache avoids recursive stat() walk on every tick:
        // only changed or new files trigger a syscall.
        let mut cache = FileSizeCache::new();
        let mut last_size: u64 = cache.refresh(&monitor_target_dir);
        let mut last_time = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let current_size = cache.refresh(&monitor_target_dir);
            let now = std::time::Instant::now();
            let delta_ms = now.duration_since(last_time).as_millis();
            let speed_bps = if delta_ms > 0 && current_size > last_size {
                (current_size - last_size) as f64 / (delta_ms as f64 / 1000.0)
            } else {
                0.0
            };
            let total_bytes = if monitor_total > 0 {
                monitor_total
            } else {
                current_size.max(100)
            };
            let event = DownloadProgressEvent {
                model_id: monitor_model_id.clone(),
                downloaded_bytes: current_size,
                total_bytes,
                speed_bps,
                status: "downloading".to_string(),
            };
            let _ = app_monitor.emit("model-download-progress", event);

            // Update download state
            if let Ok(mut states) = state_monitor.states.lock() {
                if let Some(s) = states.get_mut(&monitor_model_id) {
                    s.downloaded_bytes = current_size;
                    s.total_bytes = total_bytes;
                }
            }

            last_size = current_size;
            last_time = now;
        }
    });

    // Store monitor abort handle so cancel/pause can stop it
    state
        .monitor_handles
        .lock()
        .unwrap()
        .insert(model_id.clone(), monitor_handle.abort_handle());

    // 8. Spawn async task to read Python stdout for completion/failure events.
    // Progress events are ignored — real progress comes from the dir monitor above.
    let model_id_clone = model_id.clone();
    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let target_dir_clone = target_dir.clone();

    let handle = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<PythonEvent>(&line) {
                Ok(event) => match event {
                    // Progress is now tracked via directory size monitoring;
                    // ignore Python-emitted progress fractions.
                    PythonEvent::Progress { .. } => {}
                    PythonEvent::Result {
                        success,
                        cache_path: _cache_path,
                        error,
                        code,
                    } => {
                        if success {
                            // Compute real directory size for the completed event
                            let actual_size = compute_dir_size(&target_dir_clone);
                            let total_bytes = if actual_size > 0 {
                                actual_size
                            } else {
                                total_size_hint.unwrap_or(100)
                            };
                            let event = DownloadProgressEvent {
                                model_id: model_id_clone.clone(),
                                downloaded_bytes: actual_size,
                                total_bytes,
                                speed_bps: 0.0,
                                status: "completed".to_string(),
                            };
                            let _ = app_clone.emit("model-download-progress", event);

                            if let Ok(mut states) = state_clone.states.lock() {
                                if let Some(s) = states.get_mut(&model_id_clone) {
                                    s.status = DownloadStatus::Completed;
                                    s.downloaded_bytes = actual_size;
                                    s.total_bytes = total_bytes;
                                    s.dest_path = target_dir_clone.to_string_lossy().to_string();
                                }
                            }
                        } else {
                            let error_msg =
                                error.unwrap_or_else(|| "Unknown error".to_string());
                            let error_code =
                                code.unwrap_or_else(|| "UNKNOWN_ERROR".to_string());
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

                        // Abort the directory monitor now that download is done
                        state_clone
                            .monitor_handles
                            .lock()
                            .unwrap()
                            .remove(&model_id_clone)
                            .map(|h| h.abort());

                        break;
                    }
                },
                Err(e) => {
                    eprintln!("Failed to parse JSON event: {} | Line: {}", e, line);
                }
            }
        }

        // Wait for process to complete
        let _ = child.wait().await;

        // Clean up PID tracking
        state_clone
            .child_pids
            .lock()
            .unwrap()
            .remove(&model_id_clone);

        // Ensure monitor is aborted (belt-and-suspenders)
        state_clone
            .monitor_handles
            .lock()
            .unwrap()
            .remove(&model_id_clone)
            .map(|h| h.abort());
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
    // Kill child process for library-managed downloads (same as cancel)
    {
        let mut pids = state.child_pids.lock().unwrap();
        if let Some(pid) = pids.remove(&model_id) {
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &format!("-{}", pid)])
                    .output();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output();
            }
        }
    }

    // Abort the download task
    {
        let mut handles = state.abort_handles.lock().unwrap();
        if let Some(handle) = handles.remove(&model_id) {
            handle.abort();
        }
    }

    // Abort the directory monitor (for library-managed downloads)
    {
        let mut mon_handles = state.monitor_handles.lock().unwrap();
        if let Some(mh) = mon_handles.remove(&model_id) {
            mh.abort();
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
    // Kill child process for library-managed downloads
    {
        let mut pids = state.child_pids.lock().unwrap();
        if let Some(pid) = pids.remove(&model_id) {
            #[cfg(unix)]
            {
                // Negative PID = kill process group (kills uv + python + any threads)
                let _ = std::process::Command::new("kill")
                    .args(["-9", &format!("-{}", pid)])
                    .output();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output();
            }
        }
    }

    // Abort the download task
    let dest_path = {
        let mut handles = state.abort_handles.lock().unwrap();
        if let Some(handle) = handles.remove(&model_id) {
            handle.abort();
        }

        // Abort the directory monitor
        let mut mon_handles = state.monitor_handles.lock().unwrap();
        if let Some(mh) = mon_handles.remove(&model_id) {
            mh.abort();
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

    // Delete partial download (file or directory)
    if let Some(path) = dest_path {
        let p = std::path::Path::new(&path);
        if p.exists() {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(p);
            } else {
                let _ = std::fs::remove_file(p);
            }
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

    #[test]
    fn repo_to_dir_name_maps_correctly() {
        assert_eq!(repo_to_dir_name("funasr/paraformer-zh"), "paraformer-zh");
        assert_eq!(repo_to_dir_name("Qwen/Qwen3-ASR-1.7B"), "qwen3-asr-1.7b");
        assert_eq!(
            repo_to_dir_name("XiaomiMiMo/MiMo-V2.5-ASR"),
            "mimo-v2.5-asr"
        );
        assert_eq!(repo_to_dir_name("unknown/repo"), "unknown");
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
