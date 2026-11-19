// Media keys and the system "now playing" overlay: Windows' media flyout (SMTC), the macOS Now Playing widget, Linux MPRIS.
// The OS calls back on its own thread; the handler passes each event to the UI thread.
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
use std::time::Duration;

/// Registers with the OS. On Windows this needs the window's handle, so call it after the window is shown.
pub fn register(window: &slint::Window, on_event: impl Fn(MediaControlEvent) + Send + 'static) -> Option<MediaControls> {
    #[cfg(windows)]
    let hwnd = {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
            _ => None,
        }
    };
    #[cfg(not(windows))]
    let hwnd = {
        let _ = window;
        None
    };
    let config = PlatformConfig { display_name: "Lectern", dbus_name: "lectern", hwnd };
    let mut c = MediaControls::new(config).map_err(|e| eprintln!("media keys unavailable: {e:?}")).ok()?;
    c.attach(on_event).map_err(|e| eprintln!("media keys unavailable: {e:?}")).ok()?;
    Some(c)
}

pub fn show_book(c: &mut MediaControls, title: &str, author: &str, cover: &str, duration: f64) {
    let _ = c.set_metadata(MediaMetadata { title: Some(title), artist: Some(author), album: Some(title), cover_url: Some(cover), duration: Some(Duration::from_secs_f64(duration.max(0.0))) });
}

pub fn show_state(c: &mut MediaControls, playing: Option<bool>, pos: f64) {
    let progress = Some(MediaPosition(Duration::from_secs_f64(pos.max(0.0))));
    let _ = c.set_playback(match playing {
        None => MediaPlayback::Stopped,
        Some(true) => MediaPlayback::Playing { progress },
        Some(false) => MediaPlayback::Paused { progress },
    });
}
