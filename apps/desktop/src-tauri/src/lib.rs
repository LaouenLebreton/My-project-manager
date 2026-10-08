mod menu;
mod platform;

use tauri::Emitter;

/// Event sent to the web UI when a custom menu item is chosen. Payload is the item id.
pub const MENU_EVENT: &str = "menu";

pub fn run() {
    tauri::Builder::default()
        .menu(menu::build)
        .on_menu_event(|app, event| {
            let _ = app.emit(MENU_EVENT, event.id().as_ref());
        })
        .run(tauri::generate_context!())
        .expect("error while running Super PM");
}
