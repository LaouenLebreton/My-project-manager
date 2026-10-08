//! The IPC contract with the web UI (spec 4.9).
//!
//! The UI sends [`Edit`]s and receives [`Snapshot`]s; it never calculates dates. Ids are
//! strings and times are ISO 8601 local date-times. TypeScript types are generated from
//! these definitions into `apps/desktop/src/bindings` by `cargo test -p super-pm`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use pm_core::{outline, LinkType, Moment, PredecessorEntry, ScheduleError, TaskId, Units};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

use crate::document::{this_monday_morning, Document};
use crate::edits::{plan, Edit};
use crate::menu::MenuHandles;

/// Unit sizes for display and typing; per-calendar settings arrive with calendar editing.
pub const UNITS: Units = Units { minutes_per_day: 480, days_per_week: 5 };

#[derive(Default)]
pub struct AppState {
    pub doc: Mutex<Option<Document>>,
    /// True while a grid cell editor has focus: ⌘Z then undoes typing, not project edits.
    pub editing_text: AtomicBool,
}

/// An error shown to the user, in plain words.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct IpcError {
    pub message: String,
}

impl<E: std::fmt::Display> From<E> for IpcError {
    fn from(e: E) -> Self {
        IpcError { message: e.to_string() }
    }
}

fn err(message: &str) -> IpcError {
    IpcError { message: message.into() }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EditResult {
    pub snapshot: Snapshot,
    /// The task the edit created, if any, so the grid can select it.
    pub created_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub project: ProjectInfo,
    /// Every task in outline order.
    pub rows: Vec<Row>,
    pub links: Vec<Link>,
    /// Set when the project cannot be scheduled (for example a loop in an imported file).
    pub problem: Option<Problem>,
    pub conflicts: Vec<Conflict>,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub start: String,
    pub finish: Option<String>,
    #[ts(type = "number")]
    pub minutes_per_day: i64,
    /// For shading the chart: which weekdays are worked (Monday first) and dated exceptions.
    pub working_weekdays: Vec<bool>,
    pub calendar_exceptions: Vec<CalendarException>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CalendarException {
    /// `YYYY-MM-DD`
    pub date: String,
    pub working: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub parent_id: Option<String>,
    pub depth: u32,
    pub is_summary: bool,
    pub is_milestone: bool,
    pub name: String,
    /// Stored duration; for summaries the span of their children, in working minutes.
    #[ts(type = "number")]
    pub duration_minutes: i64,
    pub start: Option<String>,
    pub finish: Option<String>,
    pub late_start: Option<String>,
    pub late_finish: Option<String>,
    #[ts(type = "number | null")]
    pub total_slack_minutes: Option<i64>,
    pub critical: bool,
    pub percent_complete: u8,
    /// Duration as typed, e.g. "3d".
    pub duration_text: String,
    /// Incoming links as typed, e.g. "2, 5SS+1d" (row numbers).
    pub predecessors: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub id: String,
    pub predecessor_id: String,
    pub successor_id: String,
    pub kind: LinkKind,
    #[ts(type = "number")]
    pub lag_minutes: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LinkKind {
    FS,
    SS,
    FF,
    SF,
}

impl From<LinkType> for LinkKind {
    fn from(k: LinkType) -> Self {
        match k {
            LinkType::FinishToStart => LinkKind::FS,
            LinkType::StartToStart => LinkKind::SS,
            LinkType::FinishToFinish => LinkKind::FF,
            LinkType::StartToFinish => LinkKind::SF,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub message: String,
    pub task_ids: Vec<String>,
    pub link_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub task_id: String,
    pub message: String,
}

fn iso(m: Moment) -> String {
    m.to_datetime().to_string()
}

pub fn snapshot(doc: &Document) -> Snapshot {
    let p = &doc.project;
    let sched = doc.schedule.as_ref().ok();
    let outline_rows = outline(p);
    let row_number: HashMap<TaskId, usize> = outline_rows.iter().enumerate().map(|(i, r)| (r.id, i + 1)).collect();
    let mut incoming: HashMap<TaskId, Vec<PredecessorEntry>> = HashMap::new();
    for d in p.dependencies.values() {
        if let Some(&row) = row_number.get(&d.predecessor) {
            incoming.entry(d.successor).or_default().push(PredecessorEntry { row, kind: d.kind, lag: d.lag });
        }
    }
    let rows = outline_rows
        .into_iter()
        .map(|r| {
            let t = &p.tasks[&r.id];
            let s = sched.and_then(|s| s.tasks.get(&r.id));
            let duration_minutes = match (r.has_children, s) {
                (true, Some(s)) => p.calendar.working_between(s.early_start, s.early_finish),
                _ if t.is_milestone => 0,
                _ => t.duration.get(),
            };
            Row {
                id: r.id.0.to_string(),
                parent_id: t.parent.map(|x| x.0.to_string()),
                depth: r.depth as u32,
                is_summary: r.has_children,
                is_milestone: t.is_milestone,
                name: t.name.clone(),
                duration_minutes,
                start: s.map(|s| iso(s.early_start)),
                finish: s.map(|s| iso(s.early_finish)),
                late_start: s.map(|s| iso(s.late_start)),
                late_finish: s.map(|s| iso(s.late_finish)),
                total_slack_minutes: s.map(|s| s.total_slack),
                critical: s.is_some_and(|s| s.critical),
                percent_complete: t.percent_complete.get(),
                duration_text: UNITS.format(duration_minutes),
                predecessors: incoming
                    .get_mut(&r.id)
                    .map(|list| {
                        list.sort_by_key(|e| e.row);
                        list.iter().map(|e| UNITS.format_predecessor(e)).collect::<Vec<_>>().join(", ")
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    let links = p
        .dependencies
        .values()
        .map(|d| Link {
            id: d.id.0.to_string(),
            predecessor_id: d.predecessor.0.to_string(),
            successor_id: d.successor.0.to_string(),
            kind: d.kind.into(),
            lag_minutes: d.lag,
        })
        .collect();
    let name_of = |id: &TaskId| p.tasks.get(id).map_or_else(|| "a missing task".to_owned(), |t| format!("“{}”", t.name));
    let problem = doc.schedule.as_ref().err().map(|e| {
        let (task_ids, link_ids, detail) = match e {
            ScheduleError::Cycle { tasks, links } => (
                tasks.clone(),
                links.clone(),
                format!(" Tasks in the loop: {}. Remove one of the highlighted links to fix it.", tasks.iter().map(name_of).collect::<Vec<_>>().join(" → ")),
            ),
            ScheduleError::MissingTask { link, .. } | ScheduleError::SelfLink { link } => (vec![], vec![*link], String::new()),
            ScheduleError::LinkOnSummary { link, task } => (vec![*task], vec![*link], String::new()),
            ScheduleError::OutlineLoop { task } | ScheduleError::MissingParent { task, .. } => (vec![*task], vec![], String::new()),
        };
        Problem {
            message: format!("Dates cannot be calculated: {e}.{detail}"),
            task_ids: task_ids.iter().map(|t| t.0.to_string()).collect(),
            link_ids: link_ids.iter().map(|l| l.0.to_string()).collect(),
        }
    });
    let conflicts = sched
        .map(|s| {
            s.conflicts
                .iter()
                .map(|c| Conflict {
                    task_id: c.task.0.to_string(),
                    message: format!(
                        "Must start on {}, but its predecessors only allow {} at the earliest. The schedule keeps the constraint date; move the date or change the links.",
                        iso(c.constraint_start),
                        iso(c.link_driven_start)
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    Snapshot {
        project: ProjectInfo {
            name: p.name.clone(),
            path: doc.path.display().to_string(),
            start: iso(p.start),
            finish: sched.map(|s| iso(s.project_finish)),
            minutes_per_day: UNITS.minutes_per_day,
            working_weekdays: p.calendar.week().iter().map(|d| d.is_working()).collect(),
            calendar_exceptions: p
                .calendar
                .exceptions()
                .map(|(date, pattern)| CalendarException { date: date.to_string(), working: pattern.is_working() })
                .collect(),
        },
        rows,
        links,
        problem,
        conflicts,
        undo_label: doc.undo_label(),
        redo_label: doc.redo_label(),
    }
}

/// Snapshot after a change, with the Edit menu brought up to date.
pub fn changed(app: &AppHandle, doc: &Document) -> Snapshot {
    if let Some(menu) = app.try_state::<MenuHandles>() {
        menu.update_undo(doc.undo_label(), doc.redo_label());
    }
    snapshot(doc)
}

#[tauri::command]
pub fn get_snapshot(state: State<AppState>) -> Option<Snapshot> {
    state.doc.lock().expect("state lock").as_ref().map(snapshot)
}

/// Opens the sample project, creating it in the app's data folder the first time.
#[tauri::command]
pub fn open_sample(app: AppHandle) -> Result<Snapshot, IpcError> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("Office Move (Sample).pmproj");
    let doc = if path.exists() {
        Document::open(&path)?
    } else {
        Document::create(&path, crate::sample::office_move(this_monday_morning()))?
    };
    Ok(crate::actions::install(&app, doc))
}

/// Development builds only: opens a freshly generated project of `count` tasks.
#[tauri::command]
pub fn open_generated(count: u32, app: AppHandle) -> Result<Snapshot, IpcError> {
    if !cfg!(debug_assertions) {
        return Err(err("test projects are only available in development builds"));
    }
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("Test Project ({count} tasks).pmproj"));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let doc = Document::create(&path, crate::sample::generated(count as usize, this_monday_morning()))?;
    Ok(crate::actions::install(&app, doc))
}

/// File > New Project: asks where to save, then creates and opens it. `None` if cancelled.
#[tauri::command]
pub async fn new_project(app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::new_with_dialog(&app)
}

/// File > Open. `None` if cancelled.
#[tauri::command]
pub async fn open_project(app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::open_with_dialog(&app, None, "Open Project")
}

#[tauri::command]
pub fn open_recent(path: String, app: AppHandle) -> Result<Snapshot, IpcError> {
    crate::actions::open_path(&app, std::path::Path::new(&path))
}

/// The import report, shaped for the dialog that shows it.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ReportView {
    pub source: String,
    pub imported: Vec<ReportLineView>,
    pub approximated: Vec<ReportLineView>,
    pub ignored: Vec<ReportLineView>,
    pub recalculated: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct ReportLineView {
    pub what: String,
    pub count: u32,
    pub examples: Vec<String>,
}

impl ReportView {
    pub fn new(source: &str, r: pm_interop::ImportReport) -> Self {
        let lines = |v: Vec<pm_interop::ReportLine>| {
            v.into_iter().map(|l| ReportLineView { what: l.what, count: l.count as u32, examples: l.examples }).collect()
        };
        ReportView {
            source: source.to_owned(),
            imported: lines(r.imported),
            approximated: lines(r.approximated),
            ignored: lines(r.ignored),
            recalculated: r.recalculated,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct RecentFile {
    pub name: String,
    pub path: String,
}

#[tauri::command]
pub fn recent_files(app: AppHandle) -> Vec<RecentFile> {
    crate::files::recent(&app)
        .into_iter()
        .map(|p| RecentFile { name: crate::files::display_name(&p), path: p.display().to_string() })
        .collect()
}

#[tauri::command]
pub fn apply_edit(edit: Edit, app: AppHandle, state: State<AppState>) -> Result<EditResult, IpcError> {
    let mut guard = state.doc.lock().expect("state lock");
    let doc = guard.as_mut().ok_or_else(|| err("no project is open"))?;
    let created_id = match plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, edit)? {
        Some(planned) => {
            doc.apply(&planned.command)?;
            planned.created.map(|id| id.0.to_string())
        }
        None => None,
    };
    Ok(EditResult { snapshot: changed(&app, doc), created_id })
}

/// The grid reports when a cell editor opens and closes (see [`AppState::editing_text`]).
#[tauri::command]
pub fn set_text_editing(editing: bool, state: State<AppState>) {
    state.editing_text.store(editing, Ordering::Relaxed);
}

#[tauri::command]
pub fn undo(app: AppHandle, state: State<AppState>) -> Result<Snapshot, IpcError> {
    let mut guard = state.doc.lock().expect("state lock");
    let doc = guard.as_mut().ok_or_else(|| err("no project is open"))?;
    doc.undo()?;
    Ok(changed(&app, doc))
}

#[tauri::command]
pub fn redo(app: AppHandle, state: State<AppState>) -> Result<Snapshot, IpcError> {
    let mut guard = state.doc.lock().expect("state lock");
    let doc = guard.as_mut().ok_or_else(|| err("no project is open"))?;
    doc.redo()?;
    Ok(changed(&app, doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::new_id;

    fn sample_doc() -> Document {
        let dir = std::env::temp_dir().join(format!("super-pm-ipc-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        Document::create(&dir.join("s.pmproj"), crate::sample::office_move(Moment(29_000_000))).unwrap()
    }

    fn row<'a>(s: &'a Snapshot, name: &str) -> &'a Row {
        s.rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no row {name}"))
    }

    #[test]
    fn edits_flow_through_to_the_snapshot_and_undo() {
        let mut doc = sample_doc();
        let before = snapshot(&doc);
        assert!(before.problem.is_none());
        assert_eq!(before.undo_label, None);
        let pack = row(&before, "Pack offices");
        assert_eq!(pack.depth, 1);
        assert!(row(&before, "Preparation").is_summary);

        // Added right after "Pack offices", inside the same summary.
        let cmd = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::AddTask { after_id: Some(pack.id.clone()), name: "Label boxes".into() }).unwrap().unwrap();
        assert!(cmd.created.is_some());
        doc.apply(&cmd.command).unwrap();
        let s = snapshot(&doc);
        let names: Vec<&str> = s.rows.iter().map(|r| r.name.as_str()).collect();
        let at = names.iter().position(|n| *n == "Pack offices").unwrap();
        assert_eq!(names[at + 1], "Label boxes");
        assert_eq!(row(&s, "Label boxes").parent_id, pack.parent_id);
        assert_eq!(s.undo_label.as_deref(), Some("Add Task"));

        // Link it after "Unpack and set up" and stretch it: the project finish moves.
        let new = row(&s, "Label boxes").id.clone();
        let unpack = row(&s, "Unpack and set up").id.clone();
        let unpack_row = s.rows.iter().position(|r| r.id == unpack).unwrap() + 1;
        let edit = |doc: &mut Document, e: Edit| {
            let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, e).unwrap().unwrap();
            doc.apply(&planned.command).unwrap();
        };
        edit(&mut doc, Edit::SetPredecessors { id: new.clone(), text: unpack_row.to_string() });
        edit(&mut doc, Edit::SetDuration { id: new.clone(), text: "2w".into() });
        let s = snapshot(&doc);
        assert!(s.project.finish > before.project.finish);
        assert!(row(&s, "Label boxes").critical);
        assert!(!row(&s, "New office open").critical, "the milestone now has slack");
        assert_eq!(row(&s, "Label boxes").predecessors, unpack_row.to_string());
        assert_eq!(row(&s, "Label boxes").duration_text, "10d");

        doc.undo().unwrap();
        doc.undo().unwrap();
        doc.undo().unwrap();
        let s = snapshot(&doc);
        assert_eq!(s.rows.len(), before.rows.len());
        assert_eq!(s.project.finish, before.project.finish);
        assert_eq!(s.redo_label.as_deref(), Some("Add Task"));

        doc.redo().unwrap();
        let reopened = Document::open(&doc.path).unwrap();
        assert_eq!(snapshot(&reopened).rows.len(), before.rows.len() + 1);
        assert_eq!(snapshot(&reopened).redo_label.as_deref(), Some("Add Link"));
        assert!(!snapshot(&reopened).rows.iter().any(|r| r.name == "Label boxes" && !r.predecessors.is_empty()));
    }

    #[test]
    fn refused_edits_explain_themselves() {
        let mut doc = sample_doc();
        let s = snapshot(&doc);
        let a = row(&s, "List space requirements").id.clone();
        let b = row(&s, "Choose new office").id.clone();
        let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::AddLink { predecessor_id: b, successor_id: a }).unwrap().unwrap();
        let e = IpcError::from(doc.apply(&planned.command).unwrap_err());
        assert_eq!(e.message, "these two tasks are already linked; change the existing link instead");
    }

    /// The full cost of one edit at the spec's sizes: plan, save, schedule, build the snapshot,
    /// serialize it. Run with `cargo test --release -p super-pm edit_cycle -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn edit_cycle_timing() {
        for n in [5_000, 20_000] {
            let dir = std::env::temp_dir().join(format!("super-pm-bench-{}", new_id()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut doc = Document::create(&dir.join("b.pmproj"), crate::sample::generated(n, Moment(29_000_000))).unwrap();
            let target = snapshot(&doc).rows.iter().find(|r| !r.is_summary && !r.is_milestone).unwrap().id.clone();
            let runs = 10;
            let (mut t_apply, mut t_snap, mut t_json, mut bytes) = (0.0, 0.0, 0.0, 0);
            for k in 0..runs {
                let edit = Edit::SetDuration { id: target.clone(), text: format!("{}h", 4 + k) };
                let t0 = std::time::Instant::now();
                let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, edit).unwrap().unwrap();
                doc.apply(&planned.command).unwrap();
                let t1 = std::time::Instant::now();
                let snap = snapshot(&doc);
                let t2 = std::time::Instant::now();
                let json = serde_json::to_vec(&snap).unwrap();
                let t3 = std::time::Instant::now();
                t_apply += (t1 - t0).as_secs_f64();
                t_snap += (t2 - t1).as_secs_f64();
                t_json += (t3 - t2).as_secs_f64();
                bytes = json.len();
            }
            let ms = |t: f64| t * 1000.0 / runs as f64;
            println!(
                "{n:>6} tasks: save+schedule {:6.2} ms, snapshot {:6.2} ms, JSON {:6.2} ms, {:.1} MB per snapshot",
                ms(t_apply),
                ms(t_snap),
                ms(t_json),
                bytes as f64 / 1e6
            );
        }
    }

    #[test]
    fn sample_predecessor_column_reads_like_microsoft_project() {
        let s = snapshot(&sample_doc());
        let number = |name: &str| s.rows.iter().position(|r| r.name == name).unwrap() + 1;
        assert_eq!(row(&s, "Plan network and IT").predecessors, format!("{}SS+1d", number("Lease signed")));
        assert_eq!(row(&s, "Pack offices").predecessors, format!("{}FS+2d", number("Book movers")));
        assert_eq!(row(&s, "Moving day").predecessors, format!("{}, {}", number("Order furniture"), number("Pack offices")));
        assert_eq!(row(&s, "Lease signed").duration_text, "0d");
        assert_eq!(row(&s, "Planning").duration_text, "5d");
    }
}
