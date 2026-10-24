// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

fn main() {
    tauri::Builder::default()
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::scan_drives,
            commands::layout,
            commands::hover,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
