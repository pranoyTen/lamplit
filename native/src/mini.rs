// The tray mini player's window chores on Windows: open it beside the tray icon (bottom right, or wherever the taskbar is), keep it
// out of the taskbar, bring it to the front, and tell whether it still has the focus so a click elsewhere can close it.
// On a Mac: under the menu bar icon, with rounded corners. Elsewhere these do nothing and the window opens where the system puts it.

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    pub fn hwnd(w: &slint::Window) -> Option<HWND> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match w.window_handle().window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
            _ => None,
        }
    }

    /// Where the window goes, in physical pixels: centred on the cursor (which is on the tray icon), inside the work area, on the side
    /// of the screen the taskbar is on.
    pub fn place(w: &slint::Window, width: f32, height: f32) {
        let scale = w.scale_factor();
        let (ww, wh) = ((width * scale) as i32, (height * scale) as i32);
        let mut c = POINT { x: 0, y: 0 };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let ok = unsafe { GetCursorPos(&mut c) != 0 && GetMonitorInfoW(MonitorFromPoint(c, MONITOR_DEFAULTTONEAREST), &mut info) != 0 };
        if !ok {
            return;
        }
        let a: RECT = info.rcWork;
        let gap = (8.0 * scale) as i32;
        let x = (c.x - ww / 2).clamp(a.left + gap, (a.right - ww - gap).max(a.left));
        let y = if c.y > (a.top + a.bottom) / 2 { a.bottom - wh - gap } else { a.top + gap };
        w.set_position(slint::PhysicalPosition::new(x, y));
    }

    /// A tool window has no taskbar button.
    pub fn hide_from_taskbar(w: &slint::Window) {
        if let Some(h) = hwnd(w) {
            unsafe {
                let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
                SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW as isize);
            }
        }
    }

    pub fn bring_to_front(w: &slint::Window) {
        if let Some(h) = hwnd(w) {
            unsafe { SetForegroundWindow(h) };
        }
    }

    pub fn has_focus(w: &slint::Window) -> bool {
        hwnd(w).is_some_and(|h| unsafe { GetForegroundWindow() } == h)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSColor, NSEvent, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior};

    fn ns_window(w: &slint::Window) -> Option<Retained<NSWindow>> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match w.window_handle().window_handle().ok()?.as_raw() {
            // SAFETY: the handle is this live window's view, and all of this runs on the main thread
            RawWindowHandle::AppKit(h) => unsafe { h.ns_view.cast::<NSView>().as_ref() }.window(),
            _ => None,
        }
    }

    /// Centred on the cursor (which is on the menu bar icon), just under the menu bar of that screen. In points, with y going up.
    pub fn place(w: &slint::Window, width: f32, _: f32) {
        let (Some(win), Some(mtm)) = (ns_window(w), MainThreadMarker::new()) else { return };
        let width = width as f64;
        let mut p = NSEvent::mouseLocation();
        for s in NSScreen::screens(mtm) {
            let (f, a) = (s.frame(), s.visibleFrame());
            let inside = p.x >= f.origin.x && p.x <= f.origin.x + f.size.width && p.y >= f.origin.y && p.y <= f.origin.y + f.size.height;
            if inside {
                let left = a.origin.x + 8.0;
                p.x = (p.x - width / 2.0).clamp(left, (a.origin.x + a.size.width - width - 8.0).max(left));
                p.y = a.origin.y + a.size.height - 4.0;
                win.setFrameTopLeftPoint(p);
                return;
            }
        }
    }

    /// There is no taskbar to hide from; this is where the window gets its popover look.
    pub fn hide_from_taskbar(w: &slint::Window) {
        let Some(win) = ns_window(w) else { return };
        // opens on the Space in use, also over a full-screen app
        win.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace | NSWindowCollectionBehavior::FullScreenAuxiliary);
        win.setOpaque(false);
        win.setBackgroundColor(Some(&NSColor::clearColor()));
        if let Some(view) = win.contentView() {
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setCornerRadius(12.0);
                layer.setMasksToBounds(true);
            }
        }
        win.invalidateShadow();
    }

    pub fn bring_to_front(w: &slint::Window) {
        let (Some(win), Some(mtm)) = (ns_window(w), MainThreadMarker::new()) else { return };
        #[allow(deprecated)] // its replacement needs macOS 14
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        win.makeKeyAndOrderFront(None);
    }

    pub fn has_focus(w: &slint::Window) -> bool {
        ns_window(w).is_some_and(|win| win.isKeyWindow())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn place(_: &slint::Window, _: f32, _: f32) {}
    pub fn hide_from_taskbar(_: &slint::Window) {}
    pub fn bring_to_front(_: &slint::Window) {}
    pub fn has_focus(_: &slint::Window) -> bool {
        true // shortcut: it closes with Escape or the tray icon, not on focus loss; add the check if Linux becomes supported
    }
}

pub use imp::*;
