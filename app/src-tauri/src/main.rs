// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod commands;
mod shell;
mod state;

/// The folder passed on the command line (e.g. by the Explorer context menu),
/// as a full path without the `\\?\` prefix.
fn startup_path() -> Option<String> {
    let arg = std::env::args_os().nth(1)?;
    let path = std::fs::canonicalize(arg).ok()?;
    path.is_dir()
        .then(|| scanner_core::display_path(&path).to_string_lossy().into_owned())
}

fn main() {
    tauri::Builder::default()
        .manage(state::AppState::new(startup_path()))
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::scan_drives,
            commands::layout,
            commands::hover,
            actions::open_node,
            actions::reveal_node,
            actions::remove_node,
            actions::rescan_folder,
            actions::recycle_bin_info,
            actions::empty_recycle_bin,
            actions::open_apps_settings,
            actions::explorer_menu_installed,
            actions::set_explorer_menu,
            actions::startup_target,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
