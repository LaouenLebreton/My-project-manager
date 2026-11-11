//! The IPC contract with the web UI (spec 4.9).
//!
//! The UI sends [`Edit`]s and receives [`Snapshot`]s; it never calculates dates. Ids are
//! strings and times are ISO 8601 local date-times. TypeScript types are generated from
//! these definitions into `apps/desktop/src/bindings` by `cargo test -p super-pm`.

use std::collections::HashMap;

use pm_core::{format_resource, outline, LinkType, Moment, PredecessorEntry, ResourceKind, ScheduleError, TaskId, Units};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use ts_rs::TS;

use crate::document::{this_monday_morning, Document};
use crate::edits::{plan, Edit};
use crate::menu::MenuHandles;

/// Unit sizes for display and typing; per-calendar settings arrive with calendar editing.
pub const UNITS: Units = Units { minutes_per_day: 480, days_per_week: 5 };

pub use crate::actions::AppState;

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
    /// A result to tell the user about (leveling), in plain words.
    pub message: Option<String>,
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
    /// The resource pool, in Resource Sheet order.
    pub resources: Vec<ResourceRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRow {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub resource_type: crate::edits::ResourceKindEdit,
    /// Maximum units as typed: "100%", "300%"; empty for materials.
    pub max_units_text: String,
    pub material_label: String,
    /// Total work ("32h") or quantity ("2.5 tons").
    pub total_text: String,
    /// Highest load as a percentage of full time (work resources).
    pub peak_percent: u32,
    pub task_count: u32,
    /// The tasks it works on, as typed in the Tasks column: "Survey, 7[50%]".
    pub tasks_text: String,
    pub overloads: Vec<OverloadView>,
    /// Cost per hour (work) or per unit (material), in cents.
    #[ts(type = "number")]
    pub rate_cents: i64,
    /// Total cost of the resource's assignments, in cents.
    #[ts(type = "number")]
    pub cost_cents: i64,
    /// Workload view: work asked of the resource on each day it has any, and per task.
    pub days: Vec<DayWork>,
    pub assignments: Vec<AssignmentWork>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DayWork {
    /// `YYYY-MM-DD`
    pub date: String,
    /// Working minutes asked for.
    #[ts(type = "number")]
    pub minutes: i64,
    /// Working minutes the resource has that day (working time × max units).
    #[ts(type = "number")]
    pub capacity: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentWork {
    pub task_id: String,
    pub task_name: String,
    /// The task's row number in the task grid.
    pub row: u32,
    /// Units as typed: "50%".
    pub units_text: String,
    /// Days with work; `capacity` is unused (0).
    pub days: Vec<DayWork>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OverloadView {
    pub start: String,
    pub finish: String,
    pub peak_percent: u32,
    pub tasks: Vec<String>,
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
    /// Working hours per weekday, Monday first, for the Working Time window.
    pub working_week: Vec<Vec<crate::edits::TimeRange>>,
    /// Currency symbol for costs; empty if not set.
    pub currency: String,
    /// When the baseline was saved (RFC 3339), if there is one.
    pub baseline_saved_at: Option<String>,
    /// The "as of" date for progress, drawn as a line in the chart.
    pub status_date: Option<String>,
    #[ts(type = "number")]
    pub cost_cents: i64,
    /// Earned value as of the status date (or today); None without a baseline.
    pub earned_value: Option<EarnedView>,
    /// When the baseline had the project finish.
    pub baseline_finish: Option<String>,
    /// Set for projects shared through a team server.
    pub team: Option<TeamInfo>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TeamInfo {
    pub server: String,
    /// Edits not yet confirmed by the server.
    pub pending: u32,
    /// The project's id on the server (its chat channel carries it).
    pub project_id: String,
    /// What the person using the app may change, once the server has said.
    pub access: Option<crate::team::MyAccess>,
}

/// Earned value in cents, as in Microsoft Project's Earned Value table.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EarnedView {
    pub as_of: String,
    /// Planned value: the baseline cost of the work due by then.
    #[ts(type = "number")]
    pub bcws: i64,
    /// Earned value: the baseline cost of the work done.
    #[ts(type = "number")]
    pub bcwp: i64,
    /// Actual cost of the work done.
    #[ts(type = "number")]
    pub acwp: i64,
    pub spi: Option<f64>,
    pub cpi: Option<f64>,
}

impl EarnedView {
    fn new(as_of: Moment, ev: pm_core::Ev) -> Self {
        EarnedView { as_of: iso(as_of), bcws: ev.bcws, bcwp: ev.bcwp, acwp: ev.acwp, spi: ev.spi(), cpi: ev.cpi() }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CalendarException {
    /// `YYYY-MM-DD`
    pub date: String,
    pub working: bool,
    pub ranges: Vec<crate::edits::TimeRange>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub parent_id: Option<String>,
    pub depth: u32,
    /// Work breakdown structure code: "1", "1.2", "1.2.3".
    pub wbs: String,
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
    pub constraint: crate::edits::ConstraintEdit,
    pub notes: String,
    /// Assignments as typed: "Alex, Sam[50%]".
    pub resource_names: String,
    /// One of the task's resources is over-allocated while it runs.
    pub overloaded: bool,
    /// Leveling delay in working minutes (0 if none).
    #[ts(type = "number")]
    pub leveling_delay_minutes: i64,
    /// Where the task would start without its leveling delay; drawn as a line before the bar.
    pub leveled_from: Option<String>,
    /// Manually scheduled: its start is kept whatever its links say.
    pub manual: bool,
    /// The saved baseline, if any: dates, and how far the finish has moved since (working
    /// minutes; positive = later).
    pub baseline_start: Option<String>,
    pub baseline_finish: Option<String>,
    #[ts(type = "number | null")]
    pub finish_variance_minutes: Option<i64>,
    #[ts(type = "number | null")]
    pub baseline_cost_cents: Option<i64>,
    /// Total cost in cents (fixed cost plus resources; summaries add up their tasks).
    #[ts(type = "number")]
    pub cost_cents: i64,
    #[ts(type = "number")]
    pub fixed_cost_cents: i64,
    /// Progress: when work really started and finished.
    pub actual_start: Option<String>,
    pub actual_finish: Option<String>,
    /// Earned value for this task (summaries add up theirs); None without a baseline.
    pub earned_value: Option<EarnedView>,
    /// When it should be finished by, if set.
    pub deadline: Option<String>,
    /// Not finished, and scheduled to finish after its deadline.
    pub missed_deadline: bool,
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

fn ranges(pattern: &pm_core::DayPattern) -> Vec<crate::edits::TimeRange> {
    pattern.ranges().iter().map(|r| crate::edits::TimeRange { start: r.start(), end: r.end() }).collect()
}

fn constraint_view(c: pm_core::Constraint) -> crate::edits::ConstraintEdit {
    use crate::edits::{ConstraintEdit, ConstraintKind as K};
    use pm_core::Constraint as C;
    let kind = match c {
        C::AsSoonAsPossible => K::ASAP,
        C::AsLateAsPossible => K::ALAP,
        C::StartNoEarlierThan(_) => K::SNET,
        C::StartNoLaterThan(_) => K::SNLT,
        C::FinishNoEarlierThan(_) => K::FNET,
        C::FinishNoLaterThan(_) => K::FNLT,
        C::MustStartOn(_) => K::MSO,
        C::MustFinishOn(_) => K::MFO,
    };
    ConstraintEdit { kind, date: c.date().map(|m| m.to_datetime().date().to_string()) }
}

/// "32h", "1.5h": work is shown in hours, as Microsoft Project does.
fn hours(minutes: i64) -> String {
    let s = format!("{:.2}", minutes as f64 / 60.0);
    format!("{}h", s.trim_end_matches('0').trim_end_matches('.'))
}

fn resource_rows(doc: &Document) -> Vec<ResourceRow> {
    let p = &doc.project;
    let outline_rows = outline(p);
    let row_of: HashMap<TaskId, u32> = outline_rows.iter().enumerate().map(|(i, r)| (r.id, i as u32 + 1)).collect();
    let date = |day: i64| pm_core::time::date_from_day_number(day).to_string();
    let workload = |r: &pm_core::Resource| -> (Vec<DayWork>, Vec<AssignmentWork>) {
        let Ok(s) = doc.schedule.as_ref() else { return Default::default() };
        let mut total: std::collections::BTreeMap<i64, i64> = Default::default();
        let mut list: Vec<AssignmentWork> = p
            .assignments
            .values()
            .filter(|a| a.resource == r.id)
            .map(|a| {
                let days = pm_core::assignment_days(p, s, a);
                for (d, m) in &days {
                    *total.entry(*d).or_default() += m;
                }
                AssignmentWork {
                    task_id: a.task.0.to_string(),
                    task_name: p.tasks.get(&a.task).map(|t| t.name.clone()).unwrap_or_default(),
                    row: row_of.get(&a.task).copied().unwrap_or(0),
                    units_text: format!("{}%", a.units.0),
                    days: days.into_iter().map(|(d, minutes)| DayWork { date: date(d), minutes, capacity: 0 }).collect(),
                }
            })
            .collect();
        list.sort_by_key(|a| a.row);
        let days = total.into_iter().map(|(d, minutes)| DayWork { date: date(d), minutes, capacity: pm_core::capacity(p, r.id, d) }).collect();
        (days, list)
    };
    let mut list: Vec<&pm_core::Resource> = p.resources.values().collect();
    list.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.cmp(&b.id)));
    list.into_iter()
        .map(|r| {
            let load = doc.loads.resources.get(&r.id);
            let (days, assignments) = workload(r);
            let material = r.kind == ResourceKind::Material;
            let total = load.map_or(0, |l| l.total);
            let number = |h: i64| {
                let s = format!("{:.2}", h as f64 / 100.0);
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            };
            ResourceRow {
                id: r.id.0.to_string(),
                name: r.name.clone(),
                initials: r.initials.clone(),
                resource_type: if material { crate::edits::ResourceKindEdit::Material } else { crate::edits::ResourceKindEdit::Work },
                max_units_text: if material { String::new() } else { format!("{}%", r.max_units.0) },
                material_label: r.material_label.clone(),
                total_text: if material { format!("{} {}", number(total), r.material_label).trim().to_owned() } else { hours(total) },
                peak_percent: load.map_or(0, |l| l.peak.0),
                task_count: p.assignments.values().filter(|a| a.resource == r.id).count() as u32,
                tasks_text: crate::edits::resource_tasks_text(p, &outline_rows, r.id),
                rate_cents: r.rate.0,
                cost_cents: doc.loads.resource_cost.get(&r.id).copied().unwrap_or(0),
                days,
                assignments,
                overloads: load
                    .map(|l| {
                        l.overloads
                            .iter()
                            .map(|o| OverloadView {
                                start: iso(o.start),
                                finish: iso(o.finish),
                                peak_percent: o.peak.0,
                                tasks: o.tasks.iter().filter_map(|t| p.tasks.get(t)).map(|t| t.name.clone()).collect(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// "Alex, Sam[50%]" for one task, in Resource Sheet order.
fn resource_names(p: &pm_core::Project, task: TaskId) -> String {
    let mut list: Vec<(&pm_core::Resource, u32)> = p
        .assignments
        .values()
        .filter(|a| a.task == task)
        .filter_map(|a| p.resources.get(&a.resource).map(|r| (r, a.units.0)))
        .collect();
    list.sort_by(|(a, _), (b, _)| a.sort_key.cmp(&b.sort_key));
    list.iter()
        .map(|(r, units)| format_resource(&r.name, (r.kind == ResourceKind::Material).then_some(r.material_label.as_str()), *units))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn snapshot(doc: &Document) -> Snapshot {
    let p = &doc.project;
    let sched = doc.schedule.as_ref().ok();
    let outline_rows = outline(p);
    let as_of = p.status_date.unwrap_or_else(|| Moment::from_datetime(jiff::Zoned::now().datetime()));
    let earned = pm_core::earned_value(p, &doc.loads, as_of);
    let row_number: HashMap<TaskId, usize> = outline_rows.iter().enumerate().map(|(i, r)| (r.id, i + 1)).collect();
    let mut incoming: HashMap<TaskId, Vec<PredecessorEntry>> = HashMap::new();
    for d in p.dependencies.values() {
        if let Some(&row) = row_number.get(&d.predecessor) {
            incoming.entry(d.successor).or_default().push(PredecessorEntry { row, kind: d.kind, lag: d.lag });
        }
    }
    let mut counters: Vec<usize> = Vec::new();
    let rows = outline_rows
        .into_iter()
        .map(|r| {
            counters.truncate(r.depth + 1);
            counters.resize(r.depth + 1, 0);
            counters[r.depth] += 1;
            let wbs = counters.iter().map(usize::to_string).collect::<Vec<_>>().join(".");
            let t = &p.tasks[&r.id];
            let s = sched.and_then(|s| s.tasks.get(&r.id));
            let base = p.baseline.as_ref().and_then(|b| b.tasks.get(&r.id));
            let duration_minutes = match (r.has_children, s) {
                (true, Some(s)) => p.calendar.working_between(s.early_start, s.early_finish),
                _ if t.is_milestone => 0,
                _ => t.duration.get(),
            };
            Row {
                id: r.id.0.to_string(),
                parent_id: t.parent.map(|x| x.0.to_string()),
                depth: r.depth as u32,
                wbs,
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
                constraint: constraint_view(t.constraint),
                notes: t.notes.clone(),
                resource_names: resource_names(p, r.id),
                overloaded: doc.loads.overloaded_tasks.contains(&r.id),
                leveling_delay_minutes: t.leveling_delay.get(),
                manual: t.manual_start.is_some() && !r.has_children,
                baseline_start: base.map(|b| iso(b.start)),
                baseline_finish: base.map(|b| iso(b.finish)),
                finish_variance_minutes: base.zip(s).map(|(b, s)| p.calendar.working_between(b.finish, s.early_finish)),
                baseline_cost_cents: base.map(|b| b.cost.0),
                cost_cents: doc.loads.task_cost.get(&r.id).copied().unwrap_or(0),
                fixed_cost_cents: t.fixed_cost.0,
                actual_start: t.actual.start().map(iso),
                actual_finish: t.actual.finish().map(iso),
                deadline: t.deadline.map(iso),
                missed_deadline: t.percent_complete.get() < 100 && t.deadline.zip(s).is_some_and(|(d, s)| s.early_finish > d),
                earned_value: earned.as_ref().and_then(|e| e.tasks.get(&r.id)).map(|ev| EarnedView::new(as_of, *ev)),
                leveled_from: s
                    .filter(|_| t.leveling_delay.get() > 0)
                    .map(|s| iso(p.calendar.add_working(s.early_start, -t.leveling_delay.get()))),
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
            ScheduleError::FinishLinkToSummary { link, task } => (vec![*task], vec![*link], String::new()),
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
                    message: if c.manual {
                        format!(
                            "Scheduled manually to start {}, but its predecessors finish later (it could start {} at the earliest). Move it, or switch it to automatic scheduling in Task Info.",
                            iso(c.constraint_start),
                            iso(c.link_driven_start)
                        )
                    } else {
                        format!(
                            "Its constraint needs it to start by {}, but its predecessors only allow {} at the earliest. The schedule keeps the constraint date; change the date in Task Info or change the links.",
                            iso(c.constraint_start),
                            iso(c.link_driven_start)
                        )
                    },
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
                .map(|(date, pattern)| CalendarException { date: date.to_string(), working: pattern.is_working(), ranges: ranges(pattern) })
                .collect(),
            working_week: p.calendar.week().iter().map(ranges).collect(),
            currency: p.currency.clone(),
            baseline_saved_at: p.baseline.as_ref().map(|b| b.saved_at.clone()),
            status_date: p.status_date.map(iso),
            cost_cents: doc.loads.project_cost,
            earned_value: earned.as_ref().map(|e| EarnedView::new(as_of, e.project)),
            baseline_finish: p.baseline.as_ref().and_then(|b| b.tasks.values().map(|t| t.finish).max()).map(iso),
            team: crate::team::snapshot_info(doc).map(|(server, pending, project_id, access)| TeamInfo { server, pending, project_id, access }),
        },
        rows,
        links,
        problem,
        conflicts,
        undo_label: doc.undo_label(),
        redo_label: doc.redo_label(),
        resources: resource_rows(doc),
    }
}

/// Snapshot after a change in `label`'s window, with the Edit menu brought up to date if that
/// window is in front.
pub fn changed(app: &AppHandle, label: &str, doc: &Document) -> Snapshot {
    if crate::actions::is_focused(app, label)
        && let Some(menu) = app.try_state::<MenuHandles>()
    {
        menu.update_undo(doc.undo_label(), doc.redo_label());
    }
    snapshot(doc)
}

/// Runs `f` on the project in `label`'s window.
pub fn with_doc<T>(app: &AppHandle, label: &str, f: impl FnOnce(&mut Document) -> Result<T, IpcError>) -> Result<T, IpcError> {
    let state = app.state::<AppState>();
    let mut docs = state.docs.lock().expect("state lock");
    let doc = docs.get_mut(label).ok_or_else(|| err("no project is open in this window"))?;
    f(doc)
}

/// The project in the calling window, if any (a new window asks for its project on load).
#[tauri::command]
pub fn get_snapshot(window: WebviewWindow, state: State<AppState>) -> Option<Snapshot> {
    state.docs.lock().expect("state lock").get(window.label()).map(snapshot)
}

/// An import report waiting for this window (it may have opened while the import finished).
#[tauri::command]
pub fn take_import_report(window: WebviewWindow, state: State<AppState>) -> Option<ReportView> {
    state.pending_reports.lock().expect("state lock").remove(window.label())
}

/// Opens the sample project, creating it in the app's data folder the first time. Returns the
/// snapshot if it opened in the calling window; otherwise another window shows it.
#[tauri::command]
pub async fn open_sample(window: WebviewWindow, app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("Office Move (Sample).pmproj");
    if path.exists() {
        return crate::actions::open_path(&app, Some(window.label()), &path);
    }
    let doc = Document::create(&path, crate::sample::office_move(this_monday_morning()))?;
    Ok(crate::actions::install(&app, Some(window.label()), doc))
}

/// Development builds only: opens a freshly generated project of `count` tasks.
pub fn open_generated(app: &AppHandle, asking: Option<&str>, count: u32) -> Result<Option<Snapshot>, IpcError> {
    if !cfg!(debug_assertions) {
        return Err(err("test projects are only available in development builds"));
    }
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("Test Project ({count} tasks).pmproj"));
    let already = app.state::<AppState>().docs.lock().expect("state lock").values().any(|d| d.path == path);
    if already {
        return crate::actions::open_path(app, asking, &path);
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let doc = Document::create(&path, crate::sample::generated(count as usize, this_monday_morning()))?;
    Ok(crate::actions::install(app, asking, doc))
}

/// File > New Project: asks where to save, then creates and opens it. `None` if cancelled
/// or if it opened in another window.
#[tauri::command]
pub async fn new_project(window: WebviewWindow, app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::new_with_dialog(&app, Some(window.label()))
}

/// File > Open. `None` if cancelled or if it opened in another window.
#[tauri::command]
pub async fn open_project(window: WebviewWindow, app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::open_with_dialog(&app, Some(window.label()), None, "Open Project")
}

/// File > Import Microsoft Project XML. `None` if cancelled or if it opened in another window.
#[tauri::command]
pub async fn import_project(window: WebviewWindow, app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::import_with_dialog(&app, Some(window.label()))
}

/// File > Export as Microsoft Project XML.
#[tauri::command]
pub async fn export_project(window: WebviewWindow, app: AppHandle) -> Result<(), IpcError> {
    crate::actions::export_with_dialog(&app, window.label())
}

#[tauri::command]
pub async fn open_recent(path: String, window: WebviewWindow, app: AppHandle) -> Result<Option<Snapshot>, IpcError> {
    crate::actions::open_path(&app, Some(window.label()), std::path::Path::new(&path))
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
pub fn apply_edit(edit: Edit, window: WebviewWindow, app: AppHandle) -> Result<EditResult, IpcError> {
    let label = window.label();
    with_doc(&app, label, |doc| {
        let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, edit)?;
        apply_planned(&app, label, doc, planned)
    })
}

/// Edit > Copy in the task list: keeps the tasks for pasting (in any project). `text` is what
/// the list put on the system clipboard; a later paste of the same text pastes these tasks.
#[tauri::command]
pub fn copy_tasks(ids: Vec<String>, text: String, window: WebviewWindow, app: AppHandle) -> Result<(), IpcError> {
    let selected = ids.iter().map(|i| uuid::Uuid::parse_str(i).map(pm_core::TaskId)).collect::<Result<_, _>>().map_err(|e| IpcError { message: e.to_string() })?;
    let clip = with_doc(&app, window.label(), |doc| Ok(crate::edits::copy_tasks(&doc.project, &selected)))?.map(|c| crate::edits::TaskClip { text, ..c });
    *app.state::<crate::actions::AppState>().clipboard.lock().expect("lock") = clip;
    Ok(())
}

/// Edit > Paste in the task list: the copied tasks if `text` (the system clipboard) is still
/// what was copied, otherwise one task per line of the text.
#[tauri::command]
pub fn paste_tasks(after_id: Option<String>, text: String, window: WebviewWindow, app: AppHandle) -> Result<EditResult, IpcError> {
    let clip = app.state::<crate::actions::AppState>().clipboard.lock().expect("lock").clone().filter(|c| c.text.trim() == text.trim());
    let label = window.label();
    with_doc(&app, label, |doc| {
        let planned = match &clip {
            Some(c) => {
                let after = after_id.as_deref().map(uuid::Uuid::parse_str).transpose().map_err(|e| IpcError { message: e.to_string() })?.map(pm_core::TaskId);
                crate::edits::paste_tasks(&doc.project, c, after)?
            }
            None => plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::PasteText { after_id, text })?,
        };
        apply_planned(&app, label, doc, planned)
    })
}

fn apply_planned(app: &AppHandle, label: &str, doc: &mut crate::document::Document, planned: Option<crate::edits::Planned>) -> Result<EditResult, IpcError> {
    {
        let (created_id, message) = match planned {
            Some(planned) => {
                // An empty batch (leveling with nothing to change) only carries its message.
                if !matches!(&planned.command, pm_core::Command::Batch { commands, .. } if commands.is_empty()) {
                    doc.apply(&planned.command)?;
                }
                (planned.created.map(|id| id.0.to_string()), planned.message)
            }
            None => (None, None),
        };
        Ok(EditResult { snapshot: changed(app, label, doc), created_id, message })
    }
}

/// The grid reports when a cell editor opens and closes (see [`AppState::editing_text`]).
#[tauri::command]
pub fn set_text_editing(editing: bool, window: WebviewWindow, state: State<AppState>) {
    let mut set = state.editing_text.lock().expect("state lock");
    if editing {
        set.insert(window.label().to_owned());
    } else {
        set.remove(window.label());
    }
}

pub fn undo_in(app: &AppHandle, label: &str) -> Result<Snapshot, IpcError> {
    with_doc(app, label, |doc| {
        doc.undo()?;
        Ok(changed(app, label, doc))
    })
}

pub fn redo_in(app: &AppHandle, label: &str) -> Result<Snapshot, IpcError> {
    with_doc(app, label, |doc| {
        doc.redo()?;
        Ok(changed(app, label, doc))
    })
}

/// One step in Edit > Undo History.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct UndoStep {
    pub label: String,
    /// When it was done.
    pub at: String,
    /// Undone: Redo would bring it back.
    pub undone: bool,
}

#[tauri::command]
pub fn undo_history(window: WebviewWindow, app: AppHandle) -> Result<Vec<UndoStep>, IpcError> {
    with_doc(&app, window.label(), |doc| Ok(doc.history(200).into_iter().map(|s| UndoStep { label: s.label, at: s.at, undone: s.undone }).collect()))
}

/// Edit > Undo History: undo (or redo) several steps at once.
#[tauri::command]
pub fn step_history(undo: u32, redo: u32, window: WebviewWindow, app: AppHandle) -> Result<Snapshot, IpcError> {
    let label = window.label();
    with_doc(&app, label, |doc| {
        for _ in 0..undo {
            if doc.undo()?.is_none() {
                break;
            }
        }
        for _ in 0..redo {
            if doc.redo()?.is_none() {
                break;
            }
        }
        Ok(changed(&app, label, doc))
    })
}

#[tauri::command]
pub fn undo(window: WebviewWindow, app: AppHandle) -> Result<Snapshot, IpcError> {
    undo_in(&app, window.label())
}

#[tauri::command]
pub fn redo(window: WebviewWindow, app: AppHandle) -> Result<Snapshot, IpcError> {
    redo_in(&app, window.label())
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
    fn assigning_one_person_to_parallel_tasks_shows_the_overload() {
        let mut doc = sample_doc();
        let s = snapshot(&doc);
        // "Book movers" and "Order furniture" both start right after the lease is signed.
        let movers = row(&s, "Book movers").id.clone();
        let furniture = row(&s, "Order furniture").id.clone();
        for (id, text) in [(movers, "Alex"), (furniture, "Alex, Sam[50%]")] {
            let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::SetTaskResources { id, text: text.into() }).unwrap().unwrap();
            doc.apply(&planned.command).unwrap();
        }
        let s = snapshot(&doc);
        assert_eq!(row(&s, "Order furniture").resource_names, "Alex, Sam[50%]");
        assert!(row(&s, "Book movers").overloaded && row(&s, "Order furniture").overloaded);
        assert!(!row(&s, "Pack offices").overloaded);
        let alex = s.resources.iter().find(|r| r.name == "Alex").unwrap();
        assert_eq!(alex.peak_percent, 200);
        assert_eq!(alex.task_count, 2);
        assert_eq!(alex.overloads.len(), 1);
        let mut names = alex.overloads[0].tasks.clone();
        names.sort();
        assert_eq!(names, ["Book movers", "Order furniture"]);
        assert_eq!(alex.total_text, "48h", "1 day + 5 days, full time");
        let sam = s.resources.iter().find(|r| r.name == "Sam").unwrap();
        assert!(sam.overloads.is_empty());
        assert_eq!(sam.total_text, "20h");
    }

    #[test]
    fn leveling_clears_an_overload_and_says_so() {
        let mut doc = sample_doc();
        let s = snapshot(&doc);
        for name in ["Book movers", "Order furniture"] {
            let id = row(&s, name).id.clone();
            let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::SetTaskResources { id, text: "Alex".into() }).unwrap().unwrap();
            doc.apply(&planned.command).unwrap();
        }
        assert!(row(&snapshot(&doc), "Book movers").overloaded);
        let snapshot_before_finish = snapshot(&doc).project.finish;

        let planned = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::LevelResources).unwrap().unwrap();
        assert_eq!(planned.message.as_deref(), Some("1 task now starts later so nobody is over-allocated. Project > Clear Leveling (or Undo) takes the delays away."));
        doc.apply(&planned.command).unwrap();
        let s = snapshot(&doc);
        assert!(s.rows.iter().all(|r| !r.overloaded));
        assert!(s.resources.iter().all(|r| r.overloads.is_empty()));
        assert_eq!(s.undo_label.as_deref(), Some("Level Resources"));
        // "Book movers" leads a 6-day chain (1 day, 2-day wait, 3 days packing) to moving day;
        // "Order furniture" (5 days) has a day of slack, so it is the one that waits, and the
        // project still finishes on the same day.
        assert_eq!(row(&s, "Order furniture").leveling_delay_minutes, 480);
        assert_eq!(row(&s, "Book movers").leveling_delay_minutes, 0);
        assert_eq!(s.project.finish, snapshot_before_finish);

        let clear = plan(&doc.project, doc.schedule.as_ref().ok(), &UNITS, Edit::ClearLeveling).unwrap().unwrap();
        doc.apply(&clear.command).unwrap();
        assert!(row(&snapshot(&doc), "Book movers").overloaded);
        doc.undo().unwrap();
        assert!(!row(&snapshot(&doc), "Book movers").overloaded, "undo of Clear Leveling restores the delays");
    }

    #[test]
    fn wbs_codes_follow_the_outline() {
        let s = snapshot(&sample_doc());
        let code = |name: &str| row(&s, name).wbs.clone();
        assert_eq!((code("Planning"), code("Choose new office"), code("Preparation"), code("Pack offices")), ("1".into(), "1.2".into(), "2".into(), "2.4".into()));
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
