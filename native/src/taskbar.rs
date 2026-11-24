// The Back, Play/Pause and Forward buttons under Lamplit's preview when the mouse rests on its taskbar icon. Windows only.

#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Button {
    Back,
    Toggle,
    Forward,
}

/// What the buttons show. They are greyed out without a book.
#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Shown {
    pub has_book: bool,
    pub playing: bool,
    pub back: i32,
    pub fwd: i32,
}

#[cfg(windows)]
mod imp {
    use super::{Button, Shown};
    use std::cell::{Cell, OnceCell};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, SetWindowSubclass, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THUMBBUTTON, TaskbarList,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, GetSystemMetrics, HICON, RegisterWindowMessageW, SM_CXSMICON, WM_COMMAND};
    use windows::core::w;

    const THBN_CLICKED: usize = 0x1800;
    const FIRST_ID: u32 = 100;

    struct Bar {
        list: ITaskbarList3,
        hwnd: HWND,
        icons: [HICON; 4], // back, play, pause, forward
        created: u32,      // the message Windows sends when the taskbar button appears, also after the window was hidden
        clicked: fn(Button),
    }

    thread_local! {
        static BAR: OnceCell<Bar> = const { OnceCell::new() };
        static SHOWN: Cell<Shown> = const { Cell::new(Shown { has_book: false, playing: false, back: 0, fwd: 0 }) };
    }

    /// Call once the window is shown. `clicked` runs from the event loop.
    pub fn register(w: &slint::Window, clicked: fn(Button)) {
        let Some(hwnd) = crate::mini::hwnd(w).map(HWND) else { return };
        let bar = unsafe {
            let Ok(list) = CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) else { return };
            if list.HrInit().is_err() {
                return;
            }
            let n = GetSystemMetrics(SM_CXSMICON).max(16);
            let icon = |i| {
                let (mask, bgra) = (vec![0u8; (n * n) as usize], draw(i, n as usize));
                CreateIcon(None, n, n, 1, 32, mask.as_ptr(), bgra.as_ptr())
            };
            let (Ok(a), Ok(b), Ok(c), Ok(d)) = (icon(0), icon(1), icon(2), icon(3)) else { return };
            Bar { list, hwnd, icons: [a, b, c, d], created: RegisterWindowMessageW(w!("TaskbarButtonCreated")), clicked }
        };
        BAR.with(|b| {
            if b.set(bar).is_ok() {
                unsafe {
                    let _ = SetWindowSubclass(hwnd, Some(proc), 1, 0);
                }
            }
        });
        apply(true);
    }

    pub fn update(s: Shown) {
        if SHOWN.replace(s) != s {
            apply(false);
        }
    }

    /// Adding fails harmlessly when the buttons are there already, or when the taskbar button isn't yet (its message adds them).
    fn apply(add: bool) {
        BAR.with(|b| {
            let Some(b) = b.get() else { return };
            let s = SHOWN.get();
            let button = |i: u32, icon: usize, tip: String| {
                let mut t = THUMBBUTTON {
                    dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                    iId: FIRST_ID + i,
                    hIcon: b.icons[icon],
                    dwFlags: if s.has_book { THBF_ENABLED } else { THBF_DISABLED },
                    ..Default::default()
                };
                for (d, c) in t.szTip.iter_mut().zip(tip.encode_utf16().take(259)) {
                    *d = c;
                }
                t
            };
            let all = [
                button(0, 0, format!("Back {} seconds", s.back)),
                if s.playing { button(1, 2, "Pause".into()) } else { button(1, 1, "Play".into()) },
                button(2, 3, format!("Forward {} seconds", s.fwd)),
            ];
            unsafe {
                if add {
                    let _ = b.list.ThumbBarAddButtons(b.hwnd, &all);
                }
                let _ = b.list.ThumbBarUpdateButtons(b.hwnd, &all);
            }
        });
    }

    // the work goes through the event loop: this can run in the middle of a call into Windows
    unsafe extern "system" fn proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
        if let Some((created, clicked)) = BAR.with(|b| b.get().map(|b| (b.created, b.clicked))) {
            if msg == created {
                let _ = slint::invoke_from_event_loop(|| apply(true));
            } else if msg == WM_COMMAND && (wp.0 >> 16) & 0xffff == THBN_CLICKED {
                let button = match (wp.0 as u32 & 0xffff).wrapping_sub(FIRST_ID) {
                    0 => Some(Button::Back),
                    1 => Some(Button::Toggle),
                    2 => Some(Button::Forward),
                    _ => None,
                };
                if let Some(button) = button {
                    let _ = slint::invoke_from_event_loop(move || clicked(button));
                    return LRESULT(0);
                }
            }
        }
        unsafe { DefSubclassProc(h, msg, wp, lp) }
    }

    /// Icon `i` as `n` × `n` BGRA pixels, smoothed by sampling each pixel 4 × 4 times.
    fn draw(i: usize, n: usize) -> Vec<u8> {
        // a triangle pointing right from x0 to x1, full height at x0
        let tri = |x: f32, y: f32, x0: f32, x1: f32| x >= x0 && x <= x1 && (y - 0.5).abs() <= 0.36 * (x1 - x) / (x1 - x0);
        let inside = |x: f32, y: f32| match i {
            0 => tri(1.0 - x, y, 0.08, 0.5) || tri(1.0 - x, y, 0.5, 0.92),
            1 => tri(x, y, 0.22, 0.86),
            2 => (0.14..=0.86).contains(&y) && ((0.2..=0.4).contains(&x) || (0.6..=0.8).contains(&x)),
            _ => tri(x, y, 0.08, 0.5) || tri(x, y, 0.5, 0.92),
        };
        let mut px = Vec::with_capacity(n * n * 4);
        for p in 0..n * n {
            let hits = (0..16)
                .filter(|s| {
                    let (sx, sy) = ((s % 4) as f32 / 4.0 + 0.125, (s / 4) as f32 / 4.0 + 0.125);
                    inside(((p % n) as f32 + sx) / n as f32, ((p / n) as f32 + sy) / n as f32)
                })
                .count();
            // an amber that shows on both the light and the dark taskbar preview
            px.extend([0x29, 0x84, 0xc0, (hits * 255 / 16) as u8]);
        }
        px
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_back_icon_mirrors_the_forward_one() {
            let n = 16;
            let alpha = |i| super::draw(i, n).chunks(4).map(|p| p[3]).collect::<Vec<_>>();
            let (back, fwd) = (alpha(0), alpha(3));
            assert!(fwd.contains(&0) && fwd.iter().any(|a| *a > 200));
            assert!((0..n * n).all(|p| back[p].abs_diff(fwd[p / n * n + n - 1 - p % n]) <= 64)); // samples right on an edge may differ
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn register(_: &slint::Window, _: fn(super::Button)) {}
    pub fn update(_: super::Shown) {}
}

pub use imp::*;
