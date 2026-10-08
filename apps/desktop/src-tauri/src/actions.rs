//! Opening, creating and closing projects, from the menu, the UI, or Finder.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::document::Document;
use crate::files::{self, EXTENSION};
use crate::ipc::{changed, AppState, IpcError, Snapshot};
use crate::menu::MenuHandles;

pub const CLOSED_EVENT: &str = "document-closed";

fn main_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Makes `doc` the open project: backs up the previous one, updates the window title,
/// Open Recent and the Edit menu, and returns the snapshot for the UI.
pub fn install(app: &AppHandle, doc: Document) -> Snapshot {
    let state = app.state::<AppState>();
    let mut guard = state.doc.lock().expect("state lock");
    if let Some(previous) = guard.as_mut() {
        backup_logged(app, previous);
    }
    files::note_recent(app, &doc.path);
    if let Some(w) = main_window(app) {
        let _ = w.set_title(&files::display_name(&doc.path));
    }
    let snap = changed(app, &doc);
    *guard = Some(doc);
    drop(guard);
    refresh_menus(app);
    show_window(app);
    snap
}

pub fn open_path(app: &AppHandle, path: &Path) -> Result<Snapshot, IpcError> {
    let doc = Document::open(path).map_err(|e| IpcError { message: format!("“{}” could not be opened: {e}", files::display_name(path)) })?;
    Ok(install(app, doc))
}

/// Closes the project (after a backup) and shows the start screen.
pub fn close(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut guard = state.doc.lock().expect("state lock");
    if let Some(doc) = guard.as_mut() {
        backup_logged(app, doc);
    }
    *guard = None;
    drop(guard);
    if let Some(w) = main_window(app) {
        let _ = w.set_title("Super PM");
    }
    refresh_menus(app);
    let _ = app.emit(CLOSED_EVENT, ());
}

/// Backs up the open project if it changed (timer, quit).
pub fn backup_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut guard = state.doc.lock().expect("state lock");
    if let Some(doc) = guard.as_mut() {
        backup_logged(app, doc);
    }
}

/// A failed backup must never interrupt work, so it is only logged.
fn backup_logged(app: &AppHandle, doc: &mut Document) {
    if let Err(e) = files::backup_if_changed(app, doc) {
        eprintln!("backup failed: {e}");
    }
}

pub fn refresh_menus(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (open, undo, redo) = match state.doc.lock().expect("state lock").as_ref() {
        Some(d) => (true, d.undo_label(), d.redo_label()),
        None => (false, None, None),
    };
    if let Some(menu) = app.try_state::<MenuHandles>() {
        menu.update_undo(undo, redo);
        menu.set_document_open(open);
        menu.set_recent(app, &files::recent(app));
    }
}

fn documents_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().document_dir().ok()
}

// Dialogs. These block until the user answers, so they must not run on the main thread:
// Tauri runs async commands on a worker thread, and menu actions start one.

pub fn new_with_dialog(app: &AppHandle) -> Result<Option<Snapshot>, IpcError> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("New Project")
        .set_file_name(format!("Untitled.{EXTENSION}"))
        .add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = documents_dir(app) {
        dialog = dialog.set_directory(dir);
    }
    let Some(choice) = dialog.blocking_save_file() else { return Ok(None) };
    let path = files::with_extension(choice.into_path().map_err(|e| IpcError { message: e.to_string() })?);
    let doc = files::create(&path).map_err(|message| IpcError { message })?;
    Ok(Some(install(app, doc)))
}

pub fn open_with_dialog(app: &AppHandle, directory: Option<PathBuf>, title: &str) -> Result<Option<Snapshot>, IpcError> {
    let mut dialog = app.dialog().file().set_title(title).add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = directory.or_else(|| documents_dir(app)) {
        dialog = dialog.set_directory(dir);
    }
    let Some(choice) = dialog.blocking_pick_file() else { return Ok(None) };
    let path = choice.into_path().map_err(|e| IpcError { message: e.to_string() })?;
    open_path(app, &path).map(Some)
}

pub fn save_copy_with_dialog(app: &AppHandle) -> Result<(), IpcError> {
    let state = app.state::<AppState>();
    let name = match state.doc.lock().expect("state lock").as_ref() {
        Some(d) => files::display_name(&d.path),
        None => return Ok(()),
    };
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save a Copy")
        .set_file_name(format!("{name} copy.{EXTENSION}"))
        .add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = documents_dir(app) {
        dialog = dialog.set_directory(dir);
    }
    let Some(choice) = dialog.blocking_save_file() else { return Ok(()) };
    let path = files::with_extension(choice.into_path().map_err(|e| IpcError { message: e.to_string() })?);
    if path.exists() {
        // The panel asked before replacing.
        std::fs::remove_file(&path)?;
    }
    let guard = state.doc.lock().expect("state lock");
    if let Some(doc) = guard.as_ref() {
        doc.save_copy(&path)?;
    }
    Ok(())
}

/// Runs a dialog action off the main thread and reports the outcome to the UI.
pub fn spawn_dialog(app: &AppHandle, action: impl FnOnce(&AppHandle) -> Result<Option<Snapshot>, IpcError> + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || match action(&app) {
        Ok(Some(snapshot)) => {
            let _ = app.emit(crate::CHANGED_EVENT, snapshot);
        }
        Ok(None) => {}
        Err(e) => {
            let _ = app.emit("error", e);
        }
    });
}
