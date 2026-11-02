mod actions;
mod document;
mod edits;
mod files;
mod hosting;
mod ipc;
mod menu;
mod platform;
mod sample;
mod team;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

/// Event sent to a window when a custom menu item is chosen. Payload is the item id.
pub const MENU_EVENT: &str = "menu";
/// Event sent to a window when its project changed without it asking (menu, Finder).
/// Payload: Snapshot.
pub const CHANGED_EVENT: &str = "document-changed";

/// Menu actions apply to the front window.
fn on_menu(app: &tauri::AppHandle, id: &str) {
    let front = actions::focused_label(app);
    let label = front.clone();
    let to_front = |event: &str, payload: &str| match &front {
        Some(l) => {
            let _ = app.emit_to(l.as_str(), event, payload);
        }
        None => {
            let _ = app.emit(event, payload);
        }
    };
    let typing = front.as_ref().is_some_and(|l| app.state::<actions::AppState>().editing_text.lock().expect("lock").contains(l));
    let result = match id {
        menu::id::NEW => return actions::spawn_dialog(app, label, |a, l| actions::new_with_dialog(a, l).map(|_| ())),
        menu::id::OPEN => return actions::spawn_dialog(app, label, |a, l| actions::open_with_dialog(a, l, None, "Open Project").map(|_| ())),
        menu::id::OPEN_BACKUP => {
            return actions::spawn_dialog(app, label, |a, l| actions::open_with_dialog(a, l, files::backups_root(a), "Open Backup").map(|_| ()));
        }
        menu::id::IMPORT => return actions::spawn_dialog(app, label, |a, l| actions::import_with_dialog(a, l).map(|_| ())),
        menu::id::EXPORT => {
            return actions::spawn_dialog(app, label, |a, l| l.map_or(Ok(()), |l| actions::export_with_dialog(a, l)));
        }
        menu::id::SAVE_COPY => {
            return actions::spawn_dialog(app, label, |a, l| l.map_or(Ok(()), |l| actions::save_copy_with_dialog(a, l)));
        }
        menu::id::CLEAR_RECENT => {
            files::clear_recent(app);
            return actions::refresh_menus(app);
        }
        recent if recent.starts_with(menu::id::RECENT_PREFIX) => {
            actions::open_path(app, label.as_deref(), std::path::Path::new(&recent[menu::id::RECENT_PREFIX.len()..])).map(|_| ())
        }
        menu::id::OPEN_5K => ipc::open_generated(app, label.as_deref(), 5_000).map(|_| ()),
        menu::id::SECOND_USER => team::open_second_user(),
        menu::id::OPEN_20K => ipc::open_generated(app, label.as_deref(), 20_000).map(|_| ()),
        // While a cell editor is open, Undo and Redo belong to the text being typed.
        menu::id::UNDO | menu::id::REDO if typing => return to_front(MENU_EVENT, id),
        menu::id::UNDO | menu::id::REDO => {
            let Some(l) = label else { return };
            let done = if id == menu::id::UNDO { ipc::undo_in(app, &l) } else { ipc::redo_in(app, &l) };
            done.map(|snapshot| {
                let _ = app.emit_to(l.as_str(), CHANGED_EVENT, snapshot);
            })
        }
        _ => return to_front(MENU_EVENT, id),
    };
    if let Err(e) = result {
        match &front {
            Some(l) => {
                let _ = app.emit_to(l.as_str(), "error", e);
            }
            None => {
                let _ = app.emit("error", e);
            }
        }
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(actions::AppState::default())
        .setup(|app| {
            let (menu, handles) = menu::build(app.handle())?;
            app.set_menu(menu)?;
            app.manage(handles);
            actions::set_focused(app.handle(), actions::START_WINDOW);
            team::on_launch(app.handle());
            // Local backups on a schedule (spec 4.7); only projects that changed are copied.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(files::BACKUP_INTERVAL);
                actions::backup_all(&handle);
            });
            Ok(())
        })
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::Focused(true) => actions::set_focused(app, window.label()),
                // Closing a window closes its project. The last window stays (hidden) so the
                // app keeps running in the Dock with a start window, like Mac document apps.
                WindowEvent::CloseRequested { api, .. } => {
                    if actions::close_window(app, window.label()) {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                WindowEvent::Destroyed => actions::forget_window(app, window.label()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::take_import_report,
            ipc::open_sample,
            ipc::apply_edit,
            ipc::undo,
            ipc::redo,
            ipc::set_text_editing,
            ipc::new_project,
            ipc::open_project,
            ipc::open_recent,
            ipc::recent_files,
            ipc::import_project,
            ipc::export_project,
            team::team_status,
            team::team_sign_in,
            team::team_sign_out,
            team::team_create_account,
            team::team_create_project,
            team::team_invite,
            team::team_join,
            team::team_history,
            team::team_revert,
            team::team_members,
            team::team_set_member,
            team::chat_me,
            team::chat_teams,
            team::chat_posts,
            team::chat_send,
            team::chat_read,
            team::chat_create_channel,
            team::chat_edit,
            team::chat_delete,
            team::chat_react,
            team::open_second_user,
            team::team_teams,
            team::team_create_team,
            team::team_add_member,
            team::team_projects,
            team::team_share,
            team::team_open,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Super PM");

    app.run(|app, event| match event {
        // Double-clicking a .pmproj in Finder (or dropping it on the Dock icon).
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            for path in urls.iter().filter_map(|u| u.to_file_path().ok()) {
                if let Err(e) = actions::open_path(app, None, &path) {
                    actions::show_any_window(app);
                    let _ = app.emit("error", e);
                }
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => actions::show_any_window(app),
        RunEvent::ExitRequested { .. } => actions::backup_all(app),
        _ => {}
    });
}
