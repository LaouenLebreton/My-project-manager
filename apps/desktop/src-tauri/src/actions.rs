//! Opening, creating and closing projects, one window per project (owner decision
//! 2026-10-09), from the menu, the UI, or Finder.
//!
//! Locking rule: never hold the documents lock while creating, showing or focusing a window.
//! Those calls can deliver window events (focus) whose handlers take the same lock.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

use crate::document::{new_id, Document};
use crate::files::{self, EXTENSION};
use crate::ipc::{snapshot, IpcError, ReportView, Snapshot};
use crate::menu::MenuHandles;

pub const CLOSED_EVENT: &str = "document-closed";
/// Sent after an import; the report also waits in [`AppState::pending_reports`] for a window
/// that is still loading.
pub const IMPORT_REPORT_EVENT: &str = "import-report";
/// The window that shows the start screen when no project is open.
pub const START_WINDOW: &str = "main";

#[derive(Default)]
pub struct AppState {
    /// Open projects, by window label.
    pub docs: Mutex<HashMap<String, Document>>,
    /// Windows whose cell editor is open: ⌘Z there undoes typing, not project edits.
    pub editing_text: Mutex<HashSet<String>>,
    /// The window the menus act on: the one most recently brought to the front.
    pub focused: Mutex<Option<String>>,
    /// Import reports for windows that had not loaded yet when the import finished.
    pub pending_reports: Mutex<HashMap<String, ReportView>>,
    /// Tasks copied with Edit > Copy, to paste in any project window.
    pub clipboard: Mutex<Option<crate::edits::TaskClip>>,
}

fn err(message: impl Into<String>) -> IpcError {
    IpcError { message: message.into() }
}

pub fn focused_label(app: &AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let remembered = state.focused.lock().expect("lock").clone();
    remembered.filter(|l| app.get_webview_window(l).is_some()).or_else(|| {
        app.webview_windows().into_iter().find(|(_, w)| w.is_visible().unwrap_or(false)).map(|(l, _)| l)
    })
}

pub fn is_focused(app: &AppHandle, label: &str) -> bool {
    focused_label(app).as_deref() == Some(label)
}

pub fn set_focused(app: &AppHandle, label: &str) {
    *app.state::<AppState>().focused.lock().expect("lock") = Some(label.to_owned());
    refresh_menus(app);
}

pub fn show(window: &WebviewWindow) {
    let _ = window.show();
    crate::platform::show_tab_bar(window);
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Brings a window to the front: the start window if nothing is visible (Dock click).
pub fn show_any_window(app: &AppHandle) {
    let windows = app.webview_windows();
    if windows.values().any(|w| w.is_visible().unwrap_or(false)) {
        return;
    }
    match windows.get(START_WINDOW).or_else(|| windows.iter().find(|(l, _)| l.as_str() != crate::portfolio::WINDOW).map(|(_, w)| w)) {
        Some(w) => show(w),
        None => {
            if let Ok(w) = create_window(app, START_WINDOW) {
                show(&w);
            }
        }
    }
}

fn create_window(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Super PM")
        .inner_size(1200.0, 800.0)
        .min_inner_size(720.0, 480.0)
        .visible(false);
    #[cfg(target_os = "macos")]
    {
        builder = builder.tabbing_identifier(crate::platform::TAB_GROUP);
    }
    // Cascade from the front window, as document apps do.
    if let Some(front) = focused_label(app).and_then(|l| app.get_webview_window(&l))
        && let (Ok(pos), Ok(scale)) = (front.outer_position(), front.scale_factor())
    {
        let p = pos.to_logical::<f64>(scale);
        builder = builder.position(p.x + 28.0, p.y + 28.0);
    }
    builder.build()
}

/// Makes `window` a tab of the front project or Home window, unless it already is that window
/// or is already showing (then it stays where it is).
pub fn tab_into_front(app: &AppHandle, window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        return;
    }
    let front = focused_label(app)
        .filter(|l| l != window.label())
        .and_then(|l| app.get_webview_window(&l))
        .filter(|w| w.is_visible().unwrap_or(false));
    if let Some(front) = front {
        crate::platform::tab_into(app, front, window.clone());
    }
}

/// The start page (new, open, recent, portfolio, team). `new_tab` (the + and File > New Tab):
/// always a new tab, as Safari does. Otherwise (Home): go to the start page tab already open,
/// making one only if there is none. A hidden empty window (left after closing projects) is
/// reused rather than making another.
pub fn show_home(app: &AppHandle, new_tab: bool) {
    let docs = app.state::<AppState>();
    let empty: Vec<String> = {
        let docs = docs.docs.lock().expect("lock");
        let mut labels: Vec<String> = app
            .webview_windows()
            .into_iter()
            .filter(|(l, w)| l != crate::portfolio::WINDOW && !docs.contains_key(l) && (!new_tab || !w.is_visible().unwrap_or(false)))
            .map(|(l, _)| l)
            .collect();
        // Home prefers a start page that is already showing.
        labels.sort_by_key(|l| (!app.get_webview_window(l).is_some_and(|w| w.is_visible().unwrap_or(false)), l.clone()));
        labels
    };
    let window = match empty.first().and_then(|l| app.get_webview_window(l)) {
        Some(w) => Some(w),
        None => create_window(app, &format!("doc-{}", new_id().simple())).map_err(|e| eprintln!("could not open a tab: {e}")).ok(),
    };
    if let Some(w) = window {
        let _ = w.set_title("Super PM");
        tab_into_front(app, &w);
        show(&w);
        set_focused(app, w.label());
    }
}

/// The window a newly opened project should use: the asking window if it is empty, else any
/// empty window (such as the hidden start window), else a new one.
fn window_for_new_project(app: &AppHandle, asking: Option<&str>) -> String {
    let docs = app.state::<AppState>();
    let docs = docs.docs.lock().expect("lock");
    // The portfolio window never holds a project.
    let empty = |label: &str| label != crate::portfolio::WINDOW && !docs.contains_key(label) && app.get_webview_window(label).is_some();
    if let Some(label) = asking.filter(|l| empty(l)) {
        return label.to_owned();
    }
    let mut windows: Vec<String> = app.webview_windows().into_keys().collect();
    windows.sort();
    windows.into_iter().find(|l| empty(l)).unwrap_or_else(|| format!("doc-{}", new_id().simple()))
}

/// Opens `doc` in a window (see [`window_for_new_project`]), or brings forward the window
/// that already has this file. Returns the snapshot if it went into `asking`.
pub fn install(app: &AppHandle, asking: Option<&str>, doc: Document) -> Option<Snapshot> {
    let state = app.state::<AppState>();
    let existing = state.docs.lock().expect("lock").iter().find(|(_, d)| d.path == doc.path).map(|(l, _)| l.clone());
    if let Some(label) = existing {
        drop(doc); // the file is already open there
        if let Some(w) = app.get_webview_window(&label) {
            show(&w);
        }
        set_focused(app, &label);
        return None;
    }

    let label = window_for_new_project(app, asking);
    files::note_recent(app, &doc.path);
    // Team projects live in files named by id; their tab shows the project's name.
    let title = if doc.team.is_some() { doc.project.name.clone() } else { files::display_name(&doc.path) };
    let snap = snapshot(&doc);
    // Registered before the window exists, so a new window finds it when its page loads.
    state.docs.lock().expect("lock").insert(label.clone(), doc);
    crate::team::start_sync(app, &label);

    let window = match app.get_webview_window(&label) {
        Some(w) => Some(w),
        None => create_window(app, &label).map_err(|e| eprintln!("could not open a window: {e}")).ok(),
    };
    if let Some(w) = &window {
        let _ = w.set_title(&title);
        // A project opened from another window becomes a tab there, not a window of its own.
        tab_into_front(app, w);
        show(w);
    }
    let _ = app.emit_to(label.as_str(), crate::CHANGED_EVENT, snap.clone());
    set_focused(app, &label);
    (asking == Some(label.as_str())).then_some(snap)
}

pub fn open_path(app: &AppHandle, asking: Option<&str>, path: &Path) -> Result<Option<Snapshot>, IpcError> {
    // Already open: bring its window forward without opening the file twice.
    let open_in = app.state::<AppState>().docs.lock().expect("lock").iter().find(|(_, d)| d.path == path).map(|(l, _)| l.clone());
    if let Some(label) = open_in {
        if let Some(w) = app.get_webview_window(&label) {
            show(&w);
        }
        set_focused(app, &label);
        return Ok(None);
    }
    let doc = Document::open(path).map_err(|e| err(format!("“{}” could not be opened: {e}", files::display_name(path))))?;
    Ok(install(app, asking, doc))
}

/// The window is closing: back up and close its project. Returns true if the window should
/// stay open (hidden) because it is the last one, so the app keeps a start window.
pub fn close_window(app: &AppHandle, label: &str) -> bool {
    let state = app.state::<AppState>();
    if let Some(mut doc) = state.docs.lock().expect("lock").remove(label) {
        backup_logged(app, &mut doc);
    }
    state.editing_text.lock().expect("lock").remove(label);
    let others = app.webview_windows().into_iter().any(|(l, w)| l != label && w.is_visible().unwrap_or(false));
    if others {
        return false;
    }
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.set_title("Super PM");
    }
    let _ = app.emit_to(label, CLOSED_EVENT, ());
    refresh_menus(app);
    true
}

pub fn forget_window(app: &AppHandle, label: &str) {
    let state = app.state::<AppState>();
    state.docs.lock().expect("lock").remove(label);
    state.editing_text.lock().expect("lock").remove(label);
    state.pending_reports.lock().expect("lock").remove(label);
    let mut focused = state.focused.lock().expect("lock");
    if focused.as_deref() == Some(label) {
        *focused = None;
    }
    drop(focused);
    refresh_menus(app);
}

/// Backs up every open project that changed (timer, quit).
pub fn backup_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    for doc in state.docs.lock().expect("lock").values_mut() {
        backup_logged(app, doc);
    }
}

/// A failed backup must never interrupt work, so it is only logged.
fn backup_logged(app: &AppHandle, doc: &mut Document) {
    if let Err(e) = files::backup_if_changed(app, doc) {
        eprintln!("backup failed: {e}");
    }
}

/// Brings the menus up to date with the front window's project.
pub fn refresh_menus(app: &AppHandle) {
    let label = focused_label(app);
    let state = app.state::<AppState>();
    let (open, undo, redo) = match label.as_deref().and_then(|l| state.docs.lock().ok().map(|d| d.get(l).map(|d| (d.undo_label(), d.redo_label())))) {
        Some(Some((u, r))) => (true, u, r),
        _ => (false, None, None),
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

/// A dialog attached to its window as a sheet, as Mac apps do.
pub(crate) fn dialog(app: &AppHandle, parent: Option<&str>) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let mut d = app.dialog().file();
    if let Some(w) = parent.and_then(|l| app.get_webview_window(l)) {
        d = d.set_parent(&w);
    }
    d
}

// Dialogs. These block until the user answers, so they must not run on the main thread:
// Tauri runs async commands on a worker thread, and menu actions start one.

pub fn new_with_dialog(app: &AppHandle, asking: Option<&str>) -> Result<Option<Snapshot>, IpcError> {
    let mut d = dialog(app, asking)
        .set_title("New Project")
        .set_file_name(format!("Untitled.{EXTENSION}"))
        .add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = documents_dir(app) {
        d = d.set_directory(dir);
    }
    let Some(choice) = d.blocking_save_file() else { return Ok(None) };
    let path = files::with_extension(choice.into_path().map_err(|e| err(e.to_string()))?);
    let doc = files::create(&path).map_err(err)?;
    Ok(install(app, asking, doc))
}

pub fn open_with_dialog(app: &AppHandle, asking: Option<&str>, directory: Option<PathBuf>, title: &str) -> Result<Option<Snapshot>, IpcError> {
    let mut d = dialog(app, asking)
        .set_title(title)
        .add_filter("Super PM or Microsoft Project", &[EXTENSION, "mpp", "xml"])
        .add_filter("Super PM Project", &[EXTENSION])
        .add_filter("Microsoft Project", &["mpp", "xml"]);
    if let Some(dir) = directory.or_else(|| documents_dir(app)) {
        d = d.set_directory(dir);
    }
    let Some(choice) = d.blocking_pick_file() else { return Ok(None) };
    let path = choice.into_path().map_err(|e| err(e.to_string()))?;
    // A Microsoft Project file opens by importing it into a new Super PM project.
    if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("mpp") || x.eq_ignore_ascii_case("xml")) {
        return import_file(app, asking, &path);
    }
    open_path(app, asking, &path)
}

pub fn save_copy_with_dialog(app: &AppHandle, label: &str) -> Result<(), IpcError> {
    let state = app.state::<AppState>();
    let Some(name) = state.docs.lock().expect("lock").get(label).map(|d| files::display_name(&d.path)) else { return Ok(()) };
    let mut d = dialog(app, Some(label))
        .set_title("Save a Copy")
        .set_file_name(format!("{name} copy.{EXTENSION}"))
        .add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = documents_dir(app) {
        d = d.set_directory(dir);
    }
    let Some(choice) = d.blocking_save_file() else { return Ok(()) };
    let path = files::with_extension(choice.into_path().map_err(|e| err(e.to_string()))?);
    if path.exists() {
        // The panel asked before replacing.
        std::fs::remove_file(&path)?;
    }
    if let Some(doc) = state.docs.lock().expect("lock").get(label) {
        doc.save_copy(&path)?;
    }
    Ok(())
}

/// Import: pick a Microsoft Project file (.mpp or XML), convert it, ask where to save the new
/// project, open it, and show the import report.
pub fn import_with_dialog(app: &AppHandle, asking: Option<&str>) -> Result<Option<Snapshot>, IpcError> {
    let mut pick = dialog(app, asking).set_title("Import Microsoft Project File").add_filter("Microsoft Project (.mpp or XML)", &["mpp", "xml"]);
    if let Some(dir) = documents_dir(app) {
        pick = pick.set_directory(dir);
    }
    let Some(choice) = pick.blocking_pick_file() else { return Ok(None) };
    let source = choice.into_path().map_err(|e| err(e.to_string()))?;
    import_file(app, asking, &source)
}

fn import_file(app: &AppHandle, asking: Option<&str>, source: &Path) -> Result<Option<Snapshot>, IpcError> {
    let cannot = |why: String| err(format!("“{}” could not be imported: {why}", files::display_name(source)));
    let text = if crate::mpp::is_mpp(source) {
        crate::mpp::to_xml(app, source).map_err(|e| cannot(e.message))?
    } else {
        String::from_utf8_lossy(&std::fs::read(source)?).into_owned()
    };
    let (project, report) = pm_interop::import_mspdi(&text, &mut new_id).map_err(|e| cannot(e.to_string()))?;

    let mut save = dialog(app, asking)
        .set_title("Save Imported Project")
        .set_file_name(format!("{}.{EXTENSION}", files::display_name(source)))
        .add_filter("Super PM Project", &[EXTENSION]);
    if let Some(dir) = source.parent() {
        save = save.set_directory(dir);
    }
    let Some(choice) = save.blocking_save_file() else { return Ok(None) };
    let path = files::with_extension(choice.into_path().map_err(|e| err(e.to_string()))?);
    let doc = files::create_with(&path, project).map_err(err)?;
    let snapshot = install(app, asking, doc);
    let view = ReportView::new(&files::display_name(source), report);
    let state = app.state::<AppState>();
    let label = state.docs.lock().expect("lock").iter().find(|(_, d)| d.path == path).map(|(l, _)| l.clone());
    if let Some(label) = label {
        state.pending_reports.lock().expect("lock").insert(label.clone(), view.clone());
        let _ = app.emit_to(label.as_str(), IMPORT_REPORT_EVENT, view);
    }
    Ok(snapshot)
}

pub fn export_with_dialog(app: &AppHandle, label: &str) -> Result<(), IpcError> {
    let state = app.state::<AppState>();
    let Some((name, folder)) = state.docs.lock().expect("lock").get(label).map(|d| (files::display_name(&d.path), d.path.parent().map(Path::to_owned)))
    else {
        return Ok(());
    };
    let mut d = dialog(app, Some(label))
        .set_title("Export as Microsoft Project XML")
        .set_file_name(format!("{name}.xml"))
        .add_filter("Microsoft Project XML", &["xml"]);
    if let Some(dir) = folder {
        d = d.set_directory(dir);
    }
    let Some(choice) = d.blocking_save_file() else { return Ok(()) };
    let mut path = choice.into_path().map_err(|e| err(e.to_string()))?;
    if path.extension().is_none() {
        path.set_extension("xml");
    }
    let xml = match state.docs.lock().expect("lock").get(label) {
        Some(d) => pm_interop::export_mspdi(&d.project, d.schedule.as_ref().ok()),
        None => return Ok(()),
    };
    std::fs::write(&path, xml)?;
    Ok(())
}

/// File > Export Task List as CSV.
pub fn export_csv_with_dialog(app: &AppHandle, label: &str) -> Result<(), IpcError> {
    let state = app.state::<AppState>();
    let Some((name, folder)) = state.docs.lock().expect("lock").get(label).map(|d| (files::display_name(&d.path), d.path.parent().map(Path::to_owned)))
    else {
        return Ok(());
    };
    let mut d = dialog(app, Some(label)).set_title("Export Task List as CSV").set_file_name(format!("{name}.csv")).add_filter("CSV (Excel, Numbers)", &["csv"]);
    if let Some(dir) = folder {
        d = d.set_directory(dir);
    }
    let Some(choice) = d.blocking_save_file() else { return Ok(()) };
    let mut path = choice.into_path().map_err(|e| err(e.to_string()))?;
    if path.extension().is_none() {
        path.set_extension("csv");
    }
    let csv = match state.docs.lock().expect("lock").get(label) {
        Some(d) => crate::csv::task_list(&crate::ipc::snapshot(d)),
        None => return Ok(()),
    };
    std::fs::write(&path, csv)?;
    Ok(())
}

/// Runs a dialog action off the main thread; errors are shown in `label`'s window.
pub fn spawn_dialog(app: &AppHandle, label: Option<String>, action: impl FnOnce(&AppHandle, Option<&str>) -> Result<(), IpcError> + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = action(&app, label.as_deref()) {
            match label.or_else(|| focused_label(&app)) {
                Some(l) => {
                    let _ = app.emit_to(l.as_str(), "error", e);
                }
                None => {
                    let _ = app.emit("error", e);
                }
            }
        }
    });
}
