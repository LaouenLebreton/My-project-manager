mod document;
mod edits;
mod ipc;
mod menu;
mod platform;
mod sample;

use tauri::{Emitter, Manager};

/// Event sent to the web UI when a custom menu item is chosen. Payload is the item id.
pub const MENU_EVENT: &str = "menu";
/// Event sent when the project changed without the UI asking (menu undo). Payload: Snapshot.
pub const CHANGED_EVENT: &str = "document-changed";

pub fn run() {
    tauri::Builder::default()
        .manage(ipc::AppState::default())
        .setup(|app| {
            let (menu, handles) = menu::build(app.handle())?;
            app.set_menu(menu)?;
            app.manage(handles);
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            let state = app.state::<ipc::AppState>();
            // While a cell editor is open, Undo and Redo belong to the text being typed.
            let typing = state.editing_text.load(std::sync::atomic::Ordering::Relaxed);
            let result = match id {
                menu::id::UNDO | menu::id::REDO if typing => {
                    let _ = app.emit(MENU_EVENT, id);
                    return;
                }
                menu::id::UNDO => ipc::undo(app.clone(), state),
                menu::id::REDO => ipc::redo(app.clone(), state),
                _ => {
                    let _ = app.emit(MENU_EVENT, id);
                    return;
                }
            };
            match result {
                Ok(snapshot) => {
                    let _ = app.emit(CHANGED_EVENT, snapshot);
                }
                Err(e) => {
                    let _ = app.emit("error", e);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::open_sample,
            ipc::apply_edit,
            ipc::undo,
            ipc::redo,
            ipc::set_text_editing,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Super PM");
}
