// Lamplit's own title bar on Windows: the system caption goes, the app's toolbars take its place and draw their own minimise,
// maximise and close buttons. The window keeps its native frame otherwise, so the shadow, rounded corners, resizing from every
// edge, Snap and the Alt+Space menu all still work.

#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Command {
    Minimize,
    ToggleMaximized,
    Close,
}

#[cfg(windows)]
mod imp {
    use super::Command;
    use std::cell::Cell;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, HTCAPTION, HTCLIENT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, PostMessageW,
        SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SIZE_MAXIMIZED, SIZE_RESTORED, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_SIZE, WM_SYSCOMMAND,
    };

    thread_local! {
        static MAXIMIZED: Cell<Option<fn(bool)>> = const { Cell::new(None) };
        static BORDER: Cell<i32> = const { Cell::new(8) }; // the resize border's thickness in physical pixels, from the last layout
    }

    fn hwnd(w: &slint::Window) -> Option<HWND> {
        crate::mini::hwnd(w).map(HWND)
    }

    /// Call once the window is shown. Returns whether the caption was replaced; `maximized` runs from the event loop on each change.
    pub fn register(w: &slint::Window, maximized: fn(bool)) -> bool {
        let Some(h) = hwnd(w) else { return false };
        MAXIMIZED.set(Some(maximized));
        unsafe {
            if !SetWindowSubclass(h, Some(proc), 2, 0).as_bool() {
                return false;
            }
            // makes Windows lay the frame out again through the new WM_NCCALCSIZE
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            maximized(IsZoomed(h).as_bool());
        }
        true
    }

    /// Hands a press on the title bar to Windows, which moves the window (with Snap, and dragging down from maximised).
    pub fn drag(w: &slint::Window) {
        let Some(h) = hwnd(w) else { return };
        unsafe {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let _ = ReleaseCapture();
            let lp = ((p.y as u32 & 0xffff) << 16) | (p.x as u32 & 0xffff);
            let _ = PostMessageW(Some(h), WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(lp as isize));
        }
    }

    /// Through the system commands, so they animate and behave exactly like the system's own buttons (close asks the app first).
    pub fn command(w: &slint::Window, c: Command) {
        let Some(h) = hwnd(w) else { return };
        unsafe {
            let sc = match c {
                Command::Minimize => SC_MINIMIZE,
                Command::ToggleMaximized if IsZoomed(h).as_bool() => SC_RESTORE,
                Command::ToggleMaximized => SC_MAXIMIZE,
                Command::Close => SC_CLOSE,
            };
            let _ = PostMessageW(Some(h), WM_SYSCOMMAND, WPARAM(sc as usize), LPARAM(0));
        }
    }

    unsafe extern "system" fn proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
        unsafe {
            match msg {
                // the client area reaches the top of the window; the side and bottom borders stay the system's
                WM_NCCALCSIZE if wp.0 != 0 => {
                    let p = &mut *(lp.0 as *mut NCCALCSIZE_PARAMS);
                    let (top, left) = (p.rgrc[0].top, p.rgrc[0].left);
                    let r = DefSubclassProc(h, msg, wp, lp);
                    let border = p.rgrc[0].left - left;
                    BORDER.set(border.max(1));
                    // maximised, the frame hangs off the screen: without this the top of the toolbar would too.
                    // shortcut: an auto-hiding taskbar can't be revealed along the top edge of a maximised window, add it if asked
                    p.rgrc[0].top = if IsZoomed(h).as_bool() { top + border } else { top };
                    r
                }
                // the system has no top border any more, so the strip along the top resizes
                WM_NCHITTEST => {
                    let r = DefSubclassProc(h, msg, wp, lp);
                    if r.0 as u32 != HTCLIENT || IsZoomed(h).as_bool() {
                        return r;
                    }
                    let (x, y) = ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
                    let mut w = RECT::default();
                    let _ = GetWindowRect(h, &mut w);
                    let b = BORDER.get();
                    if y >= w.top + b {
                        return r;
                    }
                    LRESULT(if x < w.left + 2 * b {
                        HTTOPLEFT
                    } else if x >= w.right - 2 * b {
                        HTTOPRIGHT
                    } else {
                        HTTOP
                    } as isize)
                }
                WM_SIZE if wp.0 as u32 == SIZE_MAXIMIZED || wp.0 as u32 == SIZE_RESTORED => {
                    if let Some(f) = MAXIMIZED.get() {
                        let max = wp.0 as u32 == SIZE_MAXIMIZED;
                        // the work goes through the event loop: this can run in the middle of a call into Windows
                        let _ = slint::invoke_from_event_loop(move || f(max));
                    }
                    DefSubclassProc(h, msg, wp, lp)
                }
                _ => DefSubclassProc(h, msg, wp, lp),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn register(_: &slint::Window, _: fn(bool)) -> bool {
        false
    }
    pub fn drag(_: &slint::Window) {}
    pub fn command(_: &slint::Window, _: super::Command) {}
}

pub use imp::*;
