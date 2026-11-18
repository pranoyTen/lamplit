#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod taskbar;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
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

static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);

fn show_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// With "keep playing when closed" on, closing the window hides it and a tray icon stays; the icon's menu shows the window, toggles
// play/pause (as a "taskbar" event, the same one the Windows thumbnail buttons send) or quits. Off removes the icon again.
#[tauri::command]
fn close_to_tray(app: tauri::AppHandle, on: bool) -> Result<(), String> {
    CLOSE_TO_TRAY.store(on, Ordering::Relaxed);
    if !on {
        app.remove_tray_by_id("main");
        return Ok(());
    }
    if app.tray_by_id("main").is_some() {
        return Ok(());
    }
    let item = |id, text| MenuItem::with_id(&app, id, text, true, None::<&str>).map_err(|e| e.to_string());
    let menu = Menu::with_items(&app, &[&item("show", "Show Lectern")?, &item("toggle", "Play or pause")?, &item("quit", "Quit")?]).map_err(|e| e.to_string())?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("Lectern").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, e| match e.id.as_ref() {
        "show" => show_window(app),
        "toggle" => {
            let _ = app.emit("taskbar", "toggle");
        }
        _ => app.exit(0),
    })
    .on_tray_icon_event(|tray, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_window(tray.app_handle());
        }
    })
    .build(&app)
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                if CLOSE_TO_TRAY.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        })
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
        .invoke_handler(tauri::generate_handler![allow_dir, taskbar_playing, close_to_tray])
        .run(tauri::generate_context!())
        .expect("error while running Lectern");
}
