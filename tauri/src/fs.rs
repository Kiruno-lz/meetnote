use std::fs;
use std::io::Write;
use std::path::Path;

#[tauri::command]
pub async fn write_text_file(path: String, content: String) -> Result<(), String> {
    let p = Path::new(&path);

    // Create parent directories
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    }

    // Write the file atomically: write to .tmp then rename
    let tmp_path = format!("{}.tmp", path);
    let mut file = fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create file {}: {}", tmp_path, e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write file {}: {}", path, e))?;

    file.flush()
        .map_err(|e| format!("Failed to flush file {}: {}", path, e))?;

    fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename {} to {}: {}", tmp_path, path, e))?;

    Ok(())
}
