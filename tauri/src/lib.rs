mod asr;
mod diarization;
mod download;
mod utils;

use download::DownloadManager;
use std::sync::Arc;

#[tauri::command]
fn app_status() -> &'static str {
    "meetnote-tauri-shell-ready"
}

pub fn run() {
    let manager = Arc::new(DownloadManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(manager)
        .invoke_handler(tauri::generate_handler![
            app_status,
            asr::run_asr,
            diarization::run_diarization,
            download::list_cached_models,
            download::check_library_model_installed,
            download::remove_library_model,
            download::delete_model_file,
            download::get_active_model_id,
            download::set_active_model_id,
            download::start_model_download,
            download::pause_model_download,
            download::cancel_model_download,
            download::get_file_size,
        ])
        .run(tauri::generate_context!())
        .expect("error while running meetnote tauri application");
}
