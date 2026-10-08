//! The IPC contract with the web UI (spec 4.9).
//!
//! The UI sends [`Edit`]s and receives [`Snapshot`]s; it never calculates dates. Ids are
//! strings and times are ISO 8601 local date-times. TypeScript types are generated from
//! these definitions into `apps/desktop/src/bindings` by `cargo test -p super-pm`.

use std::sync::Mutex;

use pm_core::{
    outline, Command, Constraint, Dependency, DependencyId, Duration, LinkType, Moment, Percent, ScheduleError, SortKey,
    Task, TaskChange, TaskId,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;
use uuid::Uuid;

use crate::document::{new_id, this_monday_morning, Document};
use crate::menu::MenuHandles;

/// Minutes in a working day for display; the calendar setting arrives with calendar editing.
const MINUTES_PER_DAY: i64 = 480;

#[derive(Default)]
pub struct AppState {
    pub doc: Mutex<Option<Document>>,
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

/// What the user did. The app turns it into a core command, choosing ids and positions.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
pub enum Edit {
    /// A new task right after `afterId` at the same level, or at the end if absent.
    AddTask { after_id: Option<String>, name: String },
    RenameTask { id: String, name: String },
    SetDuration {
        id: String,
        #[ts(type = "number")]
        minutes: i64,
    },
    DeleteTasks { ids: Vec<String> },
    AddLink { predecessor_id: String, successor_id: String },
}

fn iso(m: Moment) -> String {
    m.to_datetime().to_string()
}

fn task_id(s: &str) -> Result<TaskId, IpcError> {
    Uuid::parse_str(s).map(TaskId).map_err(|_| err("this task no longer exists"))
}

fn to_command(doc: &Document, edit: Edit) -> Result<Command, IpcError> {
    let p = &doc.project;
    Ok(match edit {
        Edit::AddTask { after_id, name } => {
            let after = after_id.as_deref().map(task_id).transpose()?;
            let parent = after.and_then(|a| p.tasks.get(&a)).and_then(|t| t.parent);
            let mut siblings: Vec<&SortKey> = p.tasks.values().filter(|t| t.parent == parent).map(|t| &t.sort_key).collect();
            siblings.sort();
            let (before, next) = match after.and_then(|a| p.tasks.get(&a)) {
                Some(a) => (Some(&a.sort_key), siblings.iter().copied().find(|k| **k > a.sort_key)),
                None => (siblings.last().copied(), None),
            };
            let id = TaskId(new_id());
            Command::InsertTasks {
                tasks: vec![Task {
                    id,
                    parent,
                    sort_key: SortKey::between(before, next),
                    name,
                    duration: Duration::minutes(MINUTES_PER_DAY).expect("positive"),
                    is_milestone: false,
                    constraint: Constraint::AsSoonAsPossible,
                    percent_complete: Percent::default(),
                    notes: String::new(),
                }],
                dependencies: vec![],
            }
        }
        Edit::RenameTask { id, name } => Command::SetTask { id: task_id(&id)?, change: TaskChange::Name(name) },
        Edit::SetDuration { id, minutes } => Command::SetTask {
            id: task_id(&id)?,
            change: TaskChange::Duration(Duration::minutes(minutes).map_err(|_| err("a duration cannot be negative"))?),
        },
        Edit::DeleteTasks { ids } => Command::DeleteTasks { ids: ids.iter().map(|s| task_id(s)).collect::<Result<_, _>>()? },
        Edit::AddLink { predecessor_id, successor_id } => Command::InsertLink {
            dependency: Dependency {
                id: DependencyId(new_id()),
                predecessor: task_id(&predecessor_id)?,
                successor: task_id(&successor_id)?,
                kind: LinkType::FinishToStart,
                lag: 0,
            },
        },
    })
}

pub fn snapshot(doc: &Document) -> Snapshot {
    let p = &doc.project;
    let sched = doc.schedule.as_ref().ok();
    let rows = outline(p)
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
            minutes_per_day: MINUTES_PER_DAY,
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
pub fn open_sample(app: AppHandle, state: State<AppState>) -> Result<Snapshot, IpcError> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("Office Move (Sample).pmproj");
    let doc = if path.exists() {
        Document::open(&path)?
    } else {
        Document::create(&path, crate::sample::office_move(this_monday_morning()))?
    };
    let snap = changed(&app, &doc);
    *state.doc.lock().expect("state lock") = Some(doc);
    Ok(snap)
}

#[tauri::command]
pub fn apply_edit(edit: Edit, app: AppHandle, state: State<AppState>) -> Result<Snapshot, IpcError> {
    let mut guard = state.doc.lock().expect("state lock");
    let doc = guard.as_mut().ok_or_else(|| err("no project is open"))?;
    let cmd = to_command(doc, edit)?;
    doc.apply(&cmd)?;
    Ok(changed(&app, doc))
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
        let cmd = to_command(&doc, Edit::AddTask { after_id: Some(pack.id.clone()), name: "Label boxes".into() }).unwrap();
        doc.apply(&cmd).unwrap();
        let s = snapshot(&doc);
        let names: Vec<&str> = s.rows.iter().map(|r| r.name.as_str()).collect();
        let at = names.iter().position(|n| *n == "Pack offices").unwrap();
        assert_eq!(names[at + 1], "Label boxes");
        assert_eq!(row(&s, "Label boxes").parent_id, pack.parent_id);
        assert_eq!(s.undo_label.as_deref(), Some("Add Task"));

        // Link it after "Unpack and set up" and stretch it: the project finish moves.
        let new = row(&s, "Label boxes").id.clone();
        let unpack = row(&s, "Unpack and set up").id.clone();
        doc.apply(&to_command(&doc, Edit::AddLink { predecessor_id: unpack, successor_id: new.clone() }).unwrap()).unwrap();
        doc.apply(&to_command(&doc, Edit::SetDuration { id: new.clone(), minutes: 10 * 480 }).unwrap()).unwrap();
        let s = snapshot(&doc);
        assert!(s.project.finish > before.project.finish);
        assert!(row(&s, "Label boxes").critical);
        assert!(!row(&s, "New office open").critical, "the milestone now has slack");

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
    }

    #[test]
    fn refused_edits_explain_themselves() {
        let mut doc = sample_doc();
        let s = snapshot(&doc);
        let a = row(&s, "List space requirements").id.clone();
        let b = row(&s, "Choose new office").id.clone();
        let cmd = to_command(&doc, Edit::AddLink { predecessor_id: b, successor_id: a }).unwrap();
        let e = IpcError::from(doc.apply(&cmd).unwrap_err());
        assert_eq!(e.message, "these two tasks are already linked; change the existing link instead");
        let e = to_command(&doc, Edit::SetDuration { id: row(&s, "Moving day").id.clone(), minutes: -5 }).unwrap_err();
        assert_eq!(e.message, "a duration cannot be negative");
    }
}
