// The Mac menu bar icon. Slint's own tray opens its menu on any click there and can't make a template icon, so on a Mac this replaces it:
// a left-click opens the mini player, a right-click the menu.
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
