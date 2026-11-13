// Windows taskbar thumbnail toolbar: skip back, play/pause, skip forward under the preview that shows when you hover the app.
// A click is sent to the page as a "taskbar" event ("back", "toggle" or "fwd"); the page reports play state through `set_playing`.
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, WebviewWindow};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_ENABLED, THB_FLAGS, THB_ICON, THB_TOOLTIP,
    THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, RegisterWindowMessageW, HICON, ICONINFO, WM_COMMAND,
};

const THBN_CLICKED: usize = 0x1800;
const BACK: u32 = 0;
const TOGGLE: u32 = 1;
const FWD: u32 = 2;
const N: i32 = 32;

static APP: OnceLock<AppHandle> = OnceLock::new();
static CREATED_MSG: OnceLock<u32> = OnceLock::new();

struct Ui {
    list: ITaskbarList3,
    hwnd: HWND,
    back: HICON,
    play: HICON,
    pause: HICON,
    fwd: HICON,
    added: bool,
}

// the shell objects belong to the window's thread, so they live in a thread-local of the main thread
thread_local! { static UI: RefCell<Option<Ui>> = const { RefCell::new(None) }; }

fn tri(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let s = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1);
    let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}
fn rect(p: (f32, f32), x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
}
fn glyph_play(p: (f32, f32)) -> bool {
    tri(p, (11.0, 7.0), (11.0, 25.0), (26.0, 16.0))
}
fn glyph_pause(p: (f32, f32)) -> bool {
    rect(p, 9.0, 7.0, 14.0, 25.0) || rect(p, 18.0, 7.0, 23.0, 25.0)
}
fn glyph_back(p: (f32, f32)) -> bool {
    tri(p, (5.0, 16.0), (16.0, 7.0), (16.0, 25.0)) || tri(p, (16.0, 16.0), (27.0, 7.0), (27.0, 25.0))
}
fn glyph_fwd(p: (f32, f32)) -> bool {
    tri(p, (27.0, 16.0), (16.0, 7.0), (16.0, 25.0)) || tri(p, (16.0, 16.0), (5.0, 7.0), (5.0, 25.0))
}

// a white glyph with a thin dark halo, so it reads on both light and dark taskbar themes; drawn 4x4 supersampled
fn make_icon(glyph: fn((f32, f32)) -> bool) -> HICON {
    let mut px = vec![0u32; (N * N) as usize];
    let halo = |x: f32, y: f32| {
        [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.1, -1.1), (1.1, -1.1), (-1.1, 1.1), (1.1, 1.1)]
            .iter()
            .any(|(dx, dy)| glyph((x + dx, y + dy)))
    };
    for y in 0..N {
        for x in 0..N {
            let (mut a, mut c) = (0.0f32, 0.0f32); // alpha and premultiplied colour, averaged over the samples
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                    if glyph(p) {
                        a += 1.0;
                        c += 255.0;
                    } else if halo(p.0, p.1) {
                        a += 1.0;
                        c += 32.0;
                    }
                }
            }
            let (a, c) = ((a / 16.0 * 255.0) as u32, (c / 16.0) as u32);
            px[(y * N + x) as usize] = (a << 24) | (c << 16) | (c << 8) | c;
        }
    }
    let mask = vec![0u8; (N / 8 * N) as usize];
    unsafe {
        let color = CreateBitmap(N, N, 1, 32, Some(px.as_ptr() as *const c_void));
        let hmask = CreateBitmap(N, N, 1, 1, Some(mask.as_ptr() as *const c_void));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: hmask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(hmask.0));
        icon
    }
}

fn button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut b = THUMBBUTTON { dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS, iId: id, hIcon: icon, dwFlags: THBF_ENABLED, ..Default::default() };
    for (i, c) in tip.encode_utf16().take(259).enumerate() {
        b.szTip[i] = c;
    }
    b
}

fn add_buttons() {
    UI.with(|u| {
        if let Some(ui) = u.borrow_mut().as_mut() {
            if ui.added {
                return;
            }
            let buttons = [button(BACK, ui.back, "Skip back"), button(TOGGLE, ui.play, "Play"), button(FWD, ui.fwd, "Skip forward")];
            // fails while the taskbar button does not exist yet; the TaskbarButtonCreated message tries again
            ui.added = unsafe { ui.list.ThumbBarAddButtons(ui.hwnd, &buttons) }.is_ok();
        }
    });
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if CREATED_MSG.get() == Some(&msg) {
        add_buttons();
    } else if msg == WM_COMMAND && (wp.0 >> 16) & 0xFFFF == THBN_CLICKED {
        let name = match (wp.0 & 0xFFFF) as u32 {
            BACK => "back",
            TOGGLE => "toggle",
            _ => "fwd",
        };
        if let Some(app) = APP.get() {
            let _ = app.emit("taskbar", name);
        }
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

pub fn init(app: AppHandle, win: &WebviewWindow) {
    let Ok(hwnd) = win.hwnd() else { return };
    let _ = APP.set(app);
    let list: ITaskbarList3 = match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
        Ok(l) => l,
        Err(_) => return,
    };
    if unsafe { list.HrInit() }.is_err() {
        return;
    }
    let ui = Ui { list, hwnd, back: make_icon(glyph_back), play: make_icon(glyph_play), pause: make_icon(glyph_pause), fwd: make_icon(glyph_fwd), added: false };
    UI.with(|u| *u.borrow_mut() = Some(ui));
    unsafe {
        let _ = CREATED_MSG.set(RegisterWindowMessageW(w!("TaskbarButtonCreated")));
        let _ = SetWindowSubclass(hwnd, Some(proc), 1, 0);
    }
    add_buttons();
}

// swap the middle button between its play and pause face
pub fn set_playing(app: &AppHandle, playing: bool) {
    let _ = app.run_on_main_thread(move || {
        UI.with(|u| {
            if let Some(ui) = u.borrow().as_ref() {
                if ui.added {
                    let b = button(TOGGLE, if playing { ui.pause } else { ui.play }, if playing { "Pause" } else { "Play" });
                    let _ = unsafe { ui.list.ThumbBarUpdateButtons(ui.hwnd, &[b]) };
                }
            }
        });
    });
}
