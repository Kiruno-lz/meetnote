#[tauri::command]
fn app_status() -> &'static str {
    "meetnote-tauri-shell-ready"
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![app_status])
        .run(tauri::generate_context!())
        .expect("error while running meetnote tauri application");
}
