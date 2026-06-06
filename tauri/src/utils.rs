/// Expand ~ to home directory
pub(crate) fn expand_tilde(path: &str) -> String {
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
/// In Tauri dev mode, the binary is at tauri/target/debug/<bin>,
/// so 4 levels up lands at the repo root.
/// Falls back to current_dir() if the walk fails.
pub(crate) fn find_repo_root() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut path = exe;
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
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("python/asr_cli.py").exists() {
            return Some(cwd);
        }
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
pub(crate) fn find_uv_binary() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from(format!("{}/.local/bin/uv", home)),
        std::path::PathBuf::from("/opt/homebrew/bin/uv"),
        std::path::PathBuf::from("/usr/local/bin/uv"),
        std::path::PathBuf::from(format!("{}/.cargo/bin/uv", home)),
    ];

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
