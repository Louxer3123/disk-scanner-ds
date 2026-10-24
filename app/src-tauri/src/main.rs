// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod commands;
mod export;
mod settings;
mod shell;
mod snapshots;
mod state;
mod watch;

/// The folder passed on the command line (e.g. by the Explorer context menu),
/// as a full path without the `\\?\` prefix.
fn startup_path() -> Option<String> {
    let arg = std::env::args_os().nth(1)?;
    let path = std::fs::canonicalize(arg).ok()?;
    path.is_dir()
        .then(|| scanner_core::display_path(&path).to_string_lossy().into_owned())
}

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(state::AppState::new(startup_path()))
        .manage(settings::Settings::load())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The window is made here rather than from the config, so that in
            // portable mode the WebView keeps its data beside the exe too.
            let config = app.config().app.windows.first().cloned().ok_or("no window in tauri.conf.json")?;
            let mut window = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
            if let Some(dir) = app.state::<settings::Settings>().webview_dir() {
                window = window.data_directory(dir);
            }
            window.build()?;
            watch::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::scan_drives,
            commands::layout,
            commands::hover,
            commands::children,
            commands::search,
            commands::file_types,
            commands::changes,
            commands::find_dupes,
            commands::cancel_dupes,
            export::export_view,
            snapshots::list_snapshots,
            snapshots::save_snapshot,
            snapshots::delete_snapshot,
            snapshots::compare,
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
            actions::fast_mode,
            actions::enable_fast_mode,
            settings::get_settings,
            settings::set_setting,
            settings::open_data_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
