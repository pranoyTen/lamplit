#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod taskbar;

use tauri::Manager;
use tauri_plugin_fs::FsExt;

// Lets the app read and write one more folder (the user's chosen download location) and play audio from it.
#[tauri::command]
fn allow_dir(app: tauri::AppHandle, path: String) -> Result<(), String> {
    app.fs_scope().allow_directory(&path, true).map_err(|e| e.to_string())?;
    app.asset_protocol_scope().allow_directory(&path, true).map_err(|e| e.to_string())
}

// the page tells the Windows taskbar thumbnail buttons whether audio is playing; other platforms ignore it
#[tauri::command]
fn taskbar_playing(app: tauri::AppHandle, playing: bool) {
    #[cfg(windows)]
    taskbar::set_playing(&app, playing);
    #[cfg(not(windows))]
    let _ = (app, playing);
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(windows)]
            if let Some(win) = app.get_webview_window("main") {
                taskbar::init(app.handle().clone(), &win);
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![allow_dir, taskbar_playing])
        .run(tauri::generate_context!())
        .expect("error while running Lectern");
}
