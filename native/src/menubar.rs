// The Mac menu bar icon. Slint's own tray opens its menu on any click there and can't make a template icon, so on a Mac this replaces it:
// a left-click opens the mini player, a right-click the menu.
use objc2::MainThreadMarker;
use objc2::runtime::{AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;
use std::sync::OnceLock;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy)]
pub enum Event {
    Mini,
    Main,
    Toggle,
    Quit,
}

/// Call once the event loop runs. The icon goes away when the returned value is dropped. `on` runs from the event loop.
pub fn show((rgba, width, height): (Vec<u8>, u32, u32), on: fn(Event)) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();
    for (id, text) in [("main", "Show Lamplit"), ("toggle", "Play or Pause"), ("quit", "Quit")] {
        menu.append(&MenuItem::with_id(id, text, true, None))?;
    }
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let event = match e.id.0.as_str() {
            "main" => Event::Main,
            "toggle" => Event::Toggle,
            "quit" => Event::Quit,
            _ => return,
        };
        let _ = slint::invoke_from_event_loop(move || on(event));
    }));
    TrayIconEvent::set_event_handler(Some(move |e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            let _ = slint::invoke_from_event_loop(move || on(Event::Mini));
        }
    }));
    Ok(TrayIconBuilder::new()
        .with_icon(Icon::from_rgba(rgba, width, height)?)
        .with_icon_as_template(true) // macOS tints it to suit the menu bar
        .with_tooltip("Lamplit")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?)
}

static REOPEN: OnceLock<fn()> = OnceLock::new();

/// Runs `f` when Lamplit is opened again while it runs (from the Dock, Finder or Spotlight). macOS starts no second copy then, it asks
/// the app delegate, and winit's has no answer, so a closed window would stay closed.
pub fn on_reopen(f: fn()) {
    extern "C-unwind" fn reopen(_: &AnyObject, _: Sel, _: *mut AnyObject, _: Bool) -> Bool {
        if let Some(f) = REOPEN.get() {
            let _ = slint::invoke_from_event_loop(*f);
        }
        Bool::NO // handled
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else { return };
    if REOPEN.set(f).is_err() {
        return;
    }
    let class: *const _ = AsRef::<AnyObject>::as_ref(&*delegate).class();
    // SAFETY: the function has the selector's signature, and the class doesn't implement it yet (if it did, adding would do nothing)
    unsafe {
        let imp = std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, Bool) -> Bool, Imp>(reopen);
        objc2::ffi::class_addMethod(class.cast_mut(), sel!(applicationShouldHandleReopen:hasVisibleWindows:), imp, c"B@:@B".as_ptr());
    }
    app.setDelegate(Some(&delegate)); // AppKit looks up what the delegate can do when it is set
}
