//! Projects as Mac documents: new, open, recent files, Save a Copy, and local backups
//! (spec 4.7 and 5.1.1). Every edit is already saved by the store, so there is no Save.

use std::path::{Path, PathBuf};

use pm_core::{Calendar, CalendarId, Project, ProjectId};
use tauri::{AppHandle, Manager};

use crate::document::{new_id, this_monday_morning, Document};

pub const EXTENSION: &str = "pmproj";
const RECENT_LIMIT: usize = 10;
const BACKUPS_KEPT: usize = 20;
/// How often an open, changed project is backed up.
pub const BACKUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// `name.pmproj`, adding the extension if the user left it out.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(EXTENSION);
        PathBuf::from(s)
    }
}

/// The project name shown in the window title: the file name without extension.
pub fn display_name(path: &Path) -> String {
    path.file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned())
}

/// An empty project for File > New, named after its file.
pub fn empty_project(path: &Path) -> Project {
    Project {
        id: ProjectId(new_id()),
        name: display_name(path),
        start: this_monday_morning(),
        calendar_id: CalendarId(new_id()),
        calendar: Calendar::standard(),
        tasks: Default::default(),
        dependencies: Default::default(),
    }
}

/// Creates a project file. The save panel has already asked before replacing a file, so an
/// existing file at `path` is removed first, with its SQLite side files.
pub fn create(path: &Path) -> Result<Document, String> {
    if path.exists() {
        for p in sqlite_files(path) {
            let _ = std::fs::remove_file(p);
        }
    }
    Document::create(path, empty_project(path)).map_err(|e| e.to_string())
}

fn sqlite_files(path: &Path) -> [PathBuf; 3] {
    let s = path.as_os_str().to_owned();
    let with = |suffix: &str| {
        let mut x = s.clone();
        x.push(suffix);
        PathBuf::from(x)
    };
    [path.to_owned(), with("-wal"), with("-shm")]
}

// Recent files ----------------------------------------------------------------------------

fn recent_store(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("recent.json"))
}

/// Recently opened projects that still exist, most recent first.
pub fn recent(app: &AppHandle) -> Vec<PathBuf> {
    let Some(file) = recent_store(app) else { return vec![] };
    let list: Vec<PathBuf> = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.into_iter().filter(|p| p.is_file()).collect()
}

fn save_recent(app: &AppHandle, list: &[PathBuf]) {
    if let Some(file) = recent_store(app) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(list) {
            let _ = std::fs::write(file, json);
        }
    }
}

pub fn note_recent(app: &AppHandle, path: &Path) {
    let mut list = recent(app);
    list.retain(|p| p != path);
    list.insert(0, path.to_owned());
    list.truncate(RECENT_LIMIT);
    save_recent(app, &list);
}

pub fn clear_recent(app: &AppHandle) {
    save_recent(app, &[]);
}

// Backups -----------------------------------------------------------------------------------

/// `…/Backups/<project name> (<first 8 of id>)/`: one folder per project, so two projects
/// with the same name never mix their backups.
pub fn backup_dir(app: &AppHandle, doc: &Document) -> Option<PathBuf> {
    let id = doc.project.id.0.simple().to_string();
    let folder = format!("{} ({})", sanitize(&display_name(&doc.path)), &id[..8]);
    Some(app.path().app_data_dir().ok()?.join("Backups").join(folder))
}

pub fn backups_root(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("Backups"))
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c == '/' || c == ':' { '-' } else { c }).collect()
}

/// Writes a timestamped copy if the project changed since the last backup, then keeps only
/// the newest [`BACKUPS_KEPT`]. Errors are returned for logging; a failed backup never
/// interrupts work.
pub fn backup_if_changed(app: &AppHandle, doc: &mut Document) -> Result<Option<PathBuf>, String> {
    let seq = doc.last_seq();
    if doc.backed_up_seq == Some(seq) {
        return Ok(None);
    }
    let dir = backup_dir(app, doc).ok_or("no app data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d %H.%M.%S").to_string();
    let target = dir.join(format!("{} {stamp}.{EXTENSION}", display_name(&doc.path)));
    if !target.exists() {
        doc.save_copy(&target).map_err(|e| e.to_string())?;
    }
    doc.backed_up_seq = Some(seq);
    prune(&dir);
    Ok(Some(target))
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    // Timestamped names sort by time.
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(BACKUPS_KEPT);
    for old in &files[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_added_once() {
        assert_eq!(with_extension("a/Plan".into()), PathBuf::from("a/Plan.pmproj"));
        assert_eq!(with_extension("a/Plan.PMPROJ".into()), PathBuf::from("a/Plan.PMPROJ"));
        assert_eq!(with_extension("a/v1.2".into()), PathBuf::from("a/v1.2.pmproj"));
        assert_eq!(display_name(Path::new("/x/Office Move.pmproj")), "Office Move");
    }

    #[test]
    fn create_replaces_a_confirmed_existing_file_and_names_the_project() {
        let dir = std::env::temp_dir().join(format!("super-pm-files-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Launch.pmproj");
        std::fs::write(&path, b"old").unwrap();
        let doc = create(&path).unwrap();
        assert_eq!(doc.project.name, "Launch");
        drop(doc);
        assert!(Document::open(&path).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("super-pm-prune-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        for k in 0..25 {
            std::fs::write(dir.join(format!("P 2026-10-09 10.{k:02}.00.pmproj")), b"").unwrap();
        }
        prune(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left.len(), BACKUPS_KEPT);
        assert_eq!(left[0], "P 2026-10-09 10.05.00.pmproj");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
