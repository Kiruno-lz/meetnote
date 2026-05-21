mod download;

use std::sync::Arc;
use download::DownloadManager;

#[tauri::command]
fn app_status() -> &'static str {
    "meetnote-tauri-shell-ready"
}

pub fn run() {
    let manager = Arc::new(DownloadManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(manager)
        .invoke_handler(tauri::generate_handler![
            app_status,
            download::list_cached_models,
            download::delete_model_file,
            download::get_active_model_id,
            download::set_active_model_id,
            download::start_model_download,
            download::pause_model_download,
            download::cancel_model_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running meetnote tauri application");
}
