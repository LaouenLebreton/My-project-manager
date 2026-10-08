mod actions;
mod document;
mod edits;
mod files;
mod ipc;
mod menu;
mod platform;
mod sample;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

/// Event sent to the web UI when a custom menu item is chosen. Payload is the item id.
pub const MENU_EVENT: &str = "menu";
/// Event sent when the project changed without the UI asking (menu, Finder). Payload: Snapshot.
pub const CHANGED_EVENT: &str = "document-changed";

fn on_menu(app: &tauri::AppHandle, id: &str) {
    let state = app.state::<ipc::AppState>();
    // While a cell editor is open, Undo and Redo belong to the text being typed.
    let typing = state.editing_text.load(std::sync::atomic::Ordering::Relaxed);
    let result = match id {
        menu::id::NEW => return actions::spawn_dialog(app, actions::new_with_dialog),
        menu::id::OPEN => return actions::spawn_dialog(app, |a| actions::open_with_dialog(a, None, "Open Project")),
        menu::id::OPEN_BACKUP => {
            return actions::spawn_dialog(app, |a| actions::open_with_dialog(a, files::backups_root(a), "Open Backup"));
        }
        menu::id::IMPORT => return actions::spawn_dialog(app, actions::import_with_dialog),
        menu::id::EXPORT => return actions::spawn_dialog(app, actions::export_with_dialog),
        menu::id::SAVE_COPY => return actions::spawn_dialog(app, |a| actions::save_copy_with_dialog(a).map(|()| None)),
        menu::id::CLEAR_RECENT => {
            files::clear_recent(app);
            return actions::refresh_menus(app);
        }
        recent if recent.starts_with(menu::id::RECENT_PREFIX) => {
            actions::open_path(app, std::path::Path::new(&recent[menu::id::RECENT_PREFIX.len()..]))
        }
        menu::id::OPEN_5K => ipc::open_generated(5_000, app.clone()),
        menu::id::OPEN_20K => ipc::open_generated(20_000, app.clone()),
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
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ipc::AppState::default())
        .setup(|app| {
            let (menu, handles) = menu::build(app.handle())?;
            app.set_menu(menu)?;
            app.manage(handles);
            actions::refresh_menus(app.handle());
            // Local backups on a schedule (spec 4.7); only projects that changed are copied.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(files::BACKUP_INTERVAL);
                actions::backup_now(&handle);
            });
            Ok(())
        })
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            // Closing the window closes the project; the app stays in the Dock, like other
            // Mac document apps, and the window comes back from the Dock or File menu.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                api.prevent_close();
                actions::close(window.app_handle());
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::open_sample,
            ipc::apply_edit,
            ipc::undo,
            ipc::redo,
            ipc::set_text_editing,
            ipc::open_generated,
            ipc::new_project,
            ipc::open_project,
            ipc::open_recent,
            ipc::recent_files,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Super PM");

    app.run(|app, event| match event {
        // Double-clicking a .pmproj in Finder (or dropping it on the Dock icon).
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            if let Some(path) = urls.iter().find_map(|u| u.to_file_path().ok()) {
                match actions::open_path(app, &path) {
                    Ok(snapshot) => {
                        let _ = app.emit(CHANGED_EVENT, snapshot);
                    }
                    Err(e) => {
                        let _ = app.emit("error", e);
                        actions::show_window(app);
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => actions::show_window(app),
        RunEvent::ExitRequested { .. } => actions::backup_now(app),
        _ => {}
    });
}
