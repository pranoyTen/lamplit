#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod taskbar;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::WindowEffectsConfig;
use tauri::window::{Effect, EffectState};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
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

// The mini player: a small window shown under (or, with the tray at the bottom of the screen, above) the tray icon. It is made each time it
// opens and destroyed when it closes, because a hidden web view still holds about 170 MB. It closes when it loses focus; a click on the icon
// that caused that is ignored for a moment so it does not pop straight back open.
static MINI_HIDDEN_AT: AtomicU64 = AtomicU64::new(0);
const MINI_W: f64 = 340.0;
const MINI_H: f64 = 268.0;
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn mini(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window("mini") {
        return Some(w);
    }
    let mut b = WebviewWindowBuilder::new(app, "mini", WebviewUrl::App("mini.html".into()))
        .title("Lectern")
        .inner_size(MINI_W, MINI_H)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false);
    // on a Mac the popover is native glass: a transparent window with the system popover blur and rounded corners (the page paints no background)
    if cfg!(target_os = "macos") {
        b = b.transparent(true).effects(WindowEffectsConfig { effects: vec![Effect::Popover], state: Some(EffectState::Active), radius: Some(12.0), color: None, interactive: false });
    }
    b.build().map_err(|e| eprintln!("mini player: {e}")).ok()
}

fn toggle_mini(app: &tauri::AppHandle, icon: tauri::Rect) {
    if let Some(w) = app.get_webview_window("mini") {
        let _ = w.destroy();
        return;
    }
    if now_ms().saturating_sub(MINI_HIDDEN_AT.load(Ordering::Relaxed)) < 300 {
        return;
    }
    let Some(w) = mini(app) else { return };
    let scale = w.scale_factor().unwrap_or(1.0);
    let (p, s) = (icon.position.to_physical::<f64>(scale), icon.size.to_physical::<f64>(scale));
    let (ww, wh) = (MINI_W * scale, MINI_H * scale);
    let mon = app.monitor_from_point(p.x, p.y).ok().flatten().or_else(|| app.primary_monitor().ok().flatten());
    let mut x = p.x + s.width / 2.0 - ww / 2.0;
    let mut y = if mon.as_ref().map_or(true, |m| p.y < m.position().y as f64 + m.size().height as f64 / 2.0) { p.y + s.height + 4.0 * scale } else { p.y - wh - 4.0 * scale };
    if let Some(m) = &mon {
        let (mx, my, mw, mh) = (m.position().x as f64, m.position().y as f64, m.size().width as f64, m.size().height as f64);
        x = x.clamp(mx + 8.0 * scale, (mx + mw - ww - 8.0 * scale).max(mx));
        y = y.clamp(my, (my + mh - wh).max(my));
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
}

// the mini player's Open button: bring the full window back and put the mini player away
#[tauri::command]
fn show_main(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("mini") {
        let _ = w.destroy();
    }
    show_window(&app);
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
    // a Mac menu bar icon is a plain black shape that macOS tints to suit the bar (a "template" image); elsewhere the colour app icon is used
    if cfg!(target_os = "macos") {
        if let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")) {
            tray = tray.icon(icon).icon_as_template(true);
        }
    } else if let Some(icon) = app.default_window_icon() {
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
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
            toggle_mini(tray.app_handle(), rect);
        }
    })
    .build(&app)
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .on_window_event(|w, e| match e {
            WindowEvent::CloseRequested { api, .. } if CLOSE_TO_TRAY.load(Ordering::Relaxed) => {
                api.prevent_close();
                let _ = w.hide();
            }
            // while its web view starts, Windows reports the new window losing focus and getting it back in the same millisecond,
            // so close only when it is still unfocused a moment later
            WindowEvent::Focused(false) if w.label() == "mini" => {
                let w = w.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if !w.is_focused().unwrap_or(false) {
                        let _ = w.destroy();
                    }
                });
            }
            WindowEvent::Destroyed if w.label() == "mini" => MINI_HIDDEN_AT.store(now_ms(), Ordering::Relaxed),
            _ => {}
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
        .invoke_handler(tauri::generate_handler![allow_dir, taskbar_playing, close_to_tray, show_main])
        .run(tauri::generate_context!())
        .expect("error while running Lectern");
}
