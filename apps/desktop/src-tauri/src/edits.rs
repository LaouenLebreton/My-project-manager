//! Turning what the user did ([`Edit`]) into core commands. The app chooses ids and sort
//! keys here, so the UI never has to.

use std::collections::{BTreeMap, HashMap, HashSet};

use pm_core::{
    outline, parse_money, parse_resource_list, Money, Assignment, AssignmentId, Calendar, Command, Constraint, DayPattern, Dependency,
    DependencyId, Duration, LinkChange, LinkType, Moment, Percent, Project, ProjectChange, Resource, ResourceChange,
    ResourceId, ResourceKind, ResourceUnits, Schedule, SortKey, Task, TaskChange, TaskId, Units, WorkRange,
    MINUTES_PER_DAY,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::document::new_id;
use crate::ipc::IpcError;

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
pub enum Edit {
    /// A new task right after `afterId` at the same level, or at the end if absent.
    AddTask { after_id: Option<String>, name: String },
    RenameTask { id: String, name: String },
    /// Typed duration such as `3d`, `4h`, `2w`; `0` makes the task a milestone.
    SetDuration { id: String, text: String },
    /// Typed predecessors such as `2, 5SS, 7FF+2d` (row numbers as shown in the grid).
    SetPredecessors { id: String, text: String },
    DeleteTasks { ids: Vec<String> },
    AddLink { predecessor_id: String, successor_id: String },
    Indent { ids: Vec<String> },
    Outdent { ids: Vec<String> },
    /// Dragged bar: start no earlier than this date (`YYYY-MM-DD`), as Microsoft Project does.
    MoveTask { id: String, start_date: String },
    /// Dragged right edge: finish at the end of work on this date (`YYYY-MM-DD`).
    SetFinish { id: String, finish_date: String },
    /// Task Info: only the fields given change, as one undo step.
    UpdateTask {
        id: String,
        name: Option<String>,
        duration_text: Option<String>,
        milestone: Option<bool>,
        constraint: Option<ConstraintEdit>,
        percent_complete: Option<u8>,
        notes: Option<String>,
        /// Typed resources, as in the Resources column.
        resources: Option<String>,
        /// Typed amount, e.g. "1,500".
        fixed_cost: Option<String>,
    },
    /// Project Info: the day the project starts (`YYYY-MM-DD`).
    SetProjectStart { date: String },
    /// Working Time: the whole project calendar.
    SetCalendar { week: Vec<Vec<TimeRange>>, exceptions: Vec<DayException> },
    /// Typed in the Resources column: `Alex, Sam[50%]`. Unknown names become new resources,
    /// as in Microsoft Project.
    SetTaskResources { id: String, text: String },
    /// A new resource after `afterId` in the Resource Sheet, or at the end.
    AddResource { after_id: Option<String>, name: String },
    /// Resource Sheet edits: only the fields given change, as one undo step.
    UpdateResource {
        id: String,
        name: Option<String>,
        initials: Option<String>,
        resource_type: Option<ResourceKindEdit>,
        /// Typed maximum: `100%`, `300%` or `3` (people).
        max_units: Option<String>,
        material_label: Option<String>,
        /// Typed cost per hour (work) or per unit (material).
        rate: Option<String>,
    },
    DeleteResources { ids: Vec<String> },
    /// Project Info: the currency symbol shown with costs.
    SetCurrency { symbol: String },
    /// WBS chart: a new task as the last subtask of `parentId` (Tab in the chart).
    AddSubtask { parent_id: String, name: String },
    /// WBS chart: drag a box onto another (or onto the project) to move it underneath, at the end.
    MoveUnder { id: String, parent_id: Option<String> },
    /// Project > Level Resources: delay tasks so nobody is over-allocated.
    LevelResources,
    /// Project > Clear Leveling: remove every leveling delay.
    ClearLeveling,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub enum ResourceKindEdit {
    Work,
    Material,
}

/// Microsoft Project's abbreviations, which are also the values the UI sends.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub enum ConstraintKind {
    /// As soon as possible.
    ASAP,
    /// As late as possible.
    ALAP,
    /// Start no earlier than.
    SNET,
    /// Start no later than.
    SNLT,
    /// Finish no earlier than.
    FNET,
    /// Finish no later than.
    FNLT,
    /// Must start on.
    MSO,
    /// Must finish on.
    MFO,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct ConstraintEdit {
    pub kind: ConstraintKind,
    /// `YYYY-MM-DD`; required for SNET and MSO.
    pub date: Option<String>,
}

/// Working time within a day, in minutes after midnight.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct TimeRange {
    pub start: u16,
    pub end: u16,
}

/// A dated change to the working week: a holiday (no ranges) or different hours.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct DayException {
    /// `YYYY-MM-DD`
    pub date: String,
    pub ranges: Vec<TimeRange>,
}

/// A core command, plus the id of a task it creates (so the UI can select it).
pub struct Planned {
    pub command: Command,
    pub created: Option<TaskId>,
    /// Something to tell the user about the result (leveling).
    pub message: Option<String>,
}

fn err(message: impl Into<String>) -> IpcError {
    IpcError { message: message.into() }
}

pub fn task_id(s: &str) -> Result<TaskId, IpcError> {
    Uuid::parse_str(s).map(TaskId).map_err(|_| err("this task no longer exists"))
}

fn task(p: &Project, id: TaskId) -> Result<&Task, IpcError> {
    p.tasks.get(&id).ok_or_else(|| err("this task no longer exists"))
}

fn is_summary(p: &Project, id: TaskId) -> bool {
    p.tasks.values().any(|t| t.parent == Some(id))
}

/// Sort keys of a parent's children, ascending.
fn child_keys(p: &Project, parent: Option<TaskId>) -> Vec<&SortKey> {
    let mut keys: Vec<&SortKey> = p.tasks.values().filter(|t| t.parent == parent).map(|t| &t.sort_key).collect();
    keys.sort();
    keys
}

/// The command for an edit, or `None` when it would change nothing (e.g. indenting the
/// first task).
pub fn plan(p: &Project, schedule: Option<&Schedule>, units: &Units, edit: Edit) -> Result<Option<Planned>, IpcError> {
    let plain = |command| Ok(Some(Planned { command, created: None, message: None }));
    match edit {
        Edit::AddTask { after_id, name } => {
            let after = after_id.as_deref().map(task_id).transpose()?.map(|a| task(p, a)).transpose()?;
            let parent = after.and_then(|t| t.parent);
            let keys = child_keys(p, parent);
            let (before, next) = match after {
                Some(a) => (Some(&a.sort_key), keys.iter().copied().find(|k| **k > a.sort_key)),
                None => (keys.last().copied(), None),
            };
            let id = TaskId(new_id());
            let command = Command::InsertTasks {
                tasks: vec![Task {
                    id,
                    parent,
                    sort_key: SortKey::between(before, next),
                    name,
                    duration: Duration::minutes(units.minutes_per_day).expect("positive"),
                    is_milestone: false,
                    constraint: Constraint::AsSoonAsPossible,
                    percent_complete: Percent::default(),
                    leveling_delay: Duration::ZERO,
                    fixed_cost: Money(0),
                    notes: String::new(),
                }],
                dependencies: vec![],
            };
            Ok(Some(Planned { command, created: Some(id), message: None }))
        }
        Edit::RenameTask { id, name } => plain(Command::SetTask { id: task_id(&id)?, change: TaskChange::Name(name) }),
        Edit::SetDuration { id, text } => set_duration(p, units, task_id(&id)?, &text),
        Edit::SetPredecessors { id, text } => set_predecessors(p, units, task_id(&id)?, &text),
        Edit::DeleteTasks { ids } => {
            let ids = ids.iter().map(|s| task_id(s)).collect::<Result<Vec<_>, _>>()?;
            if ids.is_empty() {
                return Ok(None);
            }
            plain(Command::DeleteTasks { ids })
        }
        Edit::AddLink { predecessor_id, successor_id } => plain(Command::InsertLink {
            dependency: Dependency {
                id: DependencyId(new_id()),
                predecessor: task_id(&predecessor_id)?,
                successor: task_id(&successor_id)?,
                kind: LinkType::FinishToStart,
                lag: 0,
            },
        }),
        Edit::Indent { ids } => Ok(indent(p, &parse_ids(&ids)?).map(|command| Planned { command, created: None, message: None })),
        Edit::Outdent { ids } => Ok(outdent(p, &parse_ids(&ids)?).map(|command| Planned { command, created: None, message: None })),
        Edit::UpdateTask { id, name, duration_text, milestone, constraint, percent_complete, notes, resources, fixed_cost } => {
            let id = task_id(&id)?;
            let t = task(p, id)?;
            let set = |change| Command::SetTask { id, change };
            let mut commands = Vec::new();
            if let Some(n) = name.filter(|n| *n != t.name) {
                commands.push(set(TaskChange::Name(n)));
            }
            if let Some(m) = milestone.filter(|m| *m != t.is_milestone) {
                commands.push(set(TaskChange::Milestone(m)));
            }
            if let Some(text) = duration_text.filter(|_| !milestone.unwrap_or(t.is_milestone)) {
                if is_summary(p, id) {
                    return Err(err("a summary task's duration comes from the tasks inside it"));
                }
                let minutes = units.parse_duration(&text)?;
                if minutes == 0 {
                    return Err(err("a task needs a duration; tick Milestone for a zero-length event"));
                }
                if minutes != t.duration.get() {
                    commands.push(set(TaskChange::Duration(Duration::minutes(minutes)?)));
                }
            }
            if let Some(c) = constraint {
                let date = |what: &str| -> Result<Moment, IpcError> {
                    let d = c.date.as_deref().ok_or_else(|| err(format!("choose a date for “{what}”")))?;
                    Ok(p.calendar.next_working(day_start(d)?))
                };
                // Finish dates mean the end of work on that day.
                let finish = |what: &str| -> Result<Moment, IpcError> {
                    let d = c.date.as_deref().ok_or_else(|| err(format!("choose a date for “{what}”")))?;
                    Ok(p.calendar.prev_working_end(Moment(day_start(d)?.0 + MINUTES_PER_DAY)))
                };
                let new = match c.kind {
                    ConstraintKind::ASAP => Constraint::AsSoonAsPossible,
                    ConstraintKind::ALAP => Constraint::AsLateAsPossible,
                    ConstraintKind::SNET => Constraint::StartNoEarlierThan(date("Start no earlier than")?),
                    ConstraintKind::SNLT => Constraint::StartNoLaterThan(date("Start no later than")?),
                    ConstraintKind::FNET => Constraint::FinishNoEarlierThan(finish("Finish no earlier than")?),
                    ConstraintKind::FNLT => Constraint::FinishNoLaterThan(finish("Finish no later than")?),
                    ConstraintKind::MSO => Constraint::MustStartOn(date("Must start on")?),
                    ConstraintKind::MFO => Constraint::MustFinishOn(finish("Must finish on")?),
                };
                if new != t.constraint {
                    commands.push(set(TaskChange::Constraint(new)));
                }
            }
            if let Some(pc) = percent_complete.filter(|pc| *pc != t.percent_complete.get()) {
                let pc = Percent::new(pc).ok_or_else(|| err("% complete must be between 0 and 100"))?;
                commands.push(set(TaskChange::PercentComplete(pc)));
            }
            if let Some(n) = notes.filter(|n| *n != t.notes) {
                commands.push(set(TaskChange::Notes(n)));
            }
            if let Some(text) = resources
                && let Some(planned) = set_task_resources(p, id, &text)?
            {
                commands.push(planned.command);
            }
            if let Some(text) = fixed_cost {
                let cents = if text.trim().is_empty() { 0 } else { parse_money(&text)? };
                if cents != t.fixed_cost.0 {
                    commands.push(set(TaskChange::FixedCost(Money(cents))));
                }
            }
            Ok(batch("Change Task Info", commands).map(|command| Planned { command, created: None, message: None }))
        }
        Edit::SetProjectStart { date } => {
            let start = p.calendar.next_working(day_start(&date)?);
            if start == p.start {
                return Ok(None);
            }
            plain(Command::SetProject { change: ProjectChange::Start(start) })
        }
        Edit::SetCalendar { week, exceptions } => {
            let pattern = |ranges: &[TimeRange]| -> Result<DayPattern, IpcError> {
                let ranges = ranges.iter().map(|r| WorkRange::new(r.start, r.end)).collect::<Result<Vec<_>, _>>()?;
                Ok(DayPattern::new(ranges)?)
            };
            let week: [DayPattern; 7] = week
                .iter()
                .map(|d| pattern(d))
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| err("a working week has seven days"))?;
            let mut calendar = Calendar::new(week)?;
            for e in &exceptions {
                let d: jiff::civil::Date = e.date.parse().map_err(|_| err(format!("{:?} is not a date", e.date)))?;
                calendar.set_exception(d, pattern(&e.ranges)?);
            }
            if calendar == p.calendar {
                return Ok(None);
            }
            plain(Command::SetProject { change: ProjectChange::Calendar(calendar) })
        }
        Edit::SetTaskResources { id, text } => set_task_resources(p, task_id(&id)?, &text),
        Edit::AddResource { after_id, name } => {
            check_resource_name(p, &name, None)?;
            let after = after_id.as_deref().map(resource_id).transpose()?.and_then(|a| p.resources.get(&a));
            let mut keys: Vec<&SortKey> = p.resources.values().map(|r| &r.sort_key).collect();
            keys.sort();
            let (before, next) = match after {
                Some(a) => (Some(&a.sort_key), keys.iter().copied().find(|k| **k > a.sort_key)),
                None => (keys.last().copied(), None),
            };
            let resource = new_resource(&name, SortKey::between(before, next));
            let command = Command::InsertResources { resources: vec![resource], assignments: vec![] };
            Ok(Some(Planned { command, created: None, message: None }))
        }
        Edit::UpdateResource { id, name, initials, resource_type, max_units, material_label, rate } => {
            let id = resource_id(&id)?;
            let r = p.resources.get(&id).ok_or_else(|| err("this resource no longer exists"))?;
            let set = |change| Command::SetResource { id, change };
            let mut commands = Vec::new();
            if let Some(n) = name.map(|n| n.trim().to_owned()).filter(|n| *n != r.name) {
                check_resource_name(p, &n, Some(id))?;
                commands.push(set(ResourceChange::Name(n)));
            }
            if let Some(i) = initials.filter(|i| *i != r.initials) {
                commands.push(set(ResourceChange::Initials(i)));
            }
            if let Some(k) = resource_type {
                let k = match k {
                    ResourceKindEdit::Work => ResourceKind::Work,
                    ResourceKindEdit::Material => ResourceKind::Material,
                };
                if k != r.kind {
                    commands.push(set(ResourceChange::Kind(k)));
                }
            }
            if let Some(text) = max_units {
                let units = parse_max_units(&text)?;
                if units != r.max_units {
                    commands.push(set(ResourceChange::MaxUnits(units)));
                }
            }
            if let Some(l) = material_label.filter(|l| *l != r.material_label) {
                commands.push(set(ResourceChange::MaterialLabel(l)));
            }
            if let Some(text) = rate {
                let cents = if text.trim().is_empty() { 0 } else { parse_money(&text)? };
                if cents < 0 {
                    return Err(err("a rate cannot be negative"));
                }
                if cents != r.rate.0 {
                    commands.push(set(ResourceChange::Rate(Money(cents))));
                }
            }
            Ok(batch("Change Resource", commands).map(|command| Planned { command, created: None, message: None }))
        }
        Edit::DeleteResources { ids } => {
            let ids = ids.iter().map(|s| resource_id(s)).collect::<Result<Vec<_>, _>>()?;
            if ids.is_empty() {
                return Ok(None);
            }
            plain(Command::DeleteResources { ids })
        }
        Edit::SetCurrency { symbol } => {
            let symbol = symbol.trim().to_owned();
            if symbol == p.currency {
                return Ok(None);
            }
            plain(Command::SetProject { change: ProjectChange::Currency(symbol) })
        }
        Edit::AddSubtask { parent_id, name } => {
            let parent = task_id(&parent_id)?;
            task(p, parent)?;
            let last = child_keys(p, Some(parent)).last().copied().cloned();
            let id = TaskId(new_id());
            let command = Command::InsertTasks {
                tasks: vec![Task {
                    id,
                    parent: Some(parent),
                    sort_key: SortKey::between(last.as_ref(), None),
                    name,
                    duration: Duration::minutes(units.minutes_per_day).expect("positive"),
                    is_milestone: false,
                    constraint: Constraint::AsSoonAsPossible,
                    percent_complete: Percent::default(),
                    notes: String::new(),
                    leveling_delay: Duration::ZERO,
                    fixed_cost: Money(0),
                }],
                dependencies: vec![],
            };
            Ok(Some(Planned { command, created: Some(id), message: None }))
        }
        Edit::MoveUnder { id, parent_id } => {
            let id = task_id(&id)?;
            let t = task(p, id)?;
            let parent = parent_id.as_deref().map(task_id).transpose()?;
            if let Some(parent) = parent {
                task(p, parent)?;
            }
            if t.parent == parent {
                return Ok(None);
            }
            let last = child_keys(p, parent).last().copied().cloned();
            let change = TaskChange::Position { parent, sort_key: SortKey::between(last.as_ref(), None) };
            plain(Command::SetTask { id, change })
        }
        Edit::LevelResources => level_resources(p),
        Edit::ClearLeveling => {
            let set = |t: &Task| Command::SetTask { id: t.id, change: TaskChange::LevelingDelay(Duration::ZERO) };
            let commands: Vec<Command> = p.tasks.values().filter(|t| t.leveling_delay.get() > 0).map(set).collect();
            Ok(batch("Clear Leveling", commands).map(|command| Planned { command, created: None, message: None }))
        }
        Edit::MoveTask { id, start_date } => {
            let id = task_id(&id)?;
            task(p, id)?;
            if is_summary(p, id) {
                return Err(err("a summary task moves with the tasks inside it"));
            }
            if p.tasks[&id].constraint.is_fixed() {
                return Err(err("this task has a fixed date (Task Info > Start); change it there to move the task"));
            }
            let start = p.calendar.next_working(day_start(&start_date)?);
            let change = TaskChange::Constraint(Constraint::StartNoEarlierThan(start));
            plain(Command::Batch { label: "Move Task".into(), commands: vec![Command::SetTask { id, change }] })
        }
        Edit::SetFinish { id, finish_date } => {
            let id = task_id(&id)?;
            let t = task(p, id)?;
            if is_summary(p, id) || t.is_milestone {
                return Err(err("only regular tasks can be stretched; summaries follow their tasks"));
            }
            let start = schedule.and_then(|s| s.tasks.get(&id)).ok_or_else(|| err("dates cannot be calculated right now"))?.early_start;
            // The end of work on the chosen day.
            let end = p.calendar.prev_working_end(Moment(day_start(&finish_date)?.0 + MINUTES_PER_DAY));
            let minutes = p.calendar.working_between(start, end);
            if minutes <= 0 {
                return Err(err("the finish cannot be before the start"));
            }
            if minutes == t.duration.get() {
                return Ok(None);
            }
            plain(Command::SetTask { id, change: TaskChange::Duration(Duration::minutes(minutes)?) })
        }
    }
}

pub fn resource_id(s: &str) -> Result<ResourceId, IpcError> {
    Uuid::parse_str(s).map(ResourceId).map_err(|_| err("this resource no longer exists"))
}

/// Resource names are how people type assignments, so they must be usable in that list.
fn check_resource_name(p: &Project, name: &str, except: Option<ResourceId>) -> Result<(), IpcError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(err("a resource needs a name"));
    }
    if name.contains([',', '[', ']']) {
        return Err(err("resource names cannot contain commas or square brackets, because they separate names in the Resources column"));
    }
    if p.resources.values().any(|r| Some(r.id) != except && r.name.eq_ignore_ascii_case(name)) {
        return Err(err(format!("there is already a resource called “{name}”")));
    }
    Ok(())
}

/// Initials from a name: "Ana María Ortiz" → "AMO".
fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).take(3).collect()
}

fn new_resource(name: &str, sort_key: SortKey) -> Resource {
    Resource {
        id: ResourceId(new_id()),
        sort_key,
        name: name.trim().to_owned(),
        initials: initials(name),
        kind: ResourceKind::Work,
        max_units: ResourceUnits::FULL_TIME,
        material_label: String::new(),
        rate: Money(0),
        notes: String::new(),
    }
}

/// `100%`, `300%`, `3` (people) or `1.5` → hundredths.
fn parse_max_units(text: &str) -> Result<ResourceUnits, IpcError> {
    let t = text.trim();
    let (number, percent) = match t.strip_suffix('%') {
        Some(n) => (n.trim(), true),
        None => (t, false),
    };
    let value: f64 = number.replace(',', ".").parse().map_err(|_| err("type maximum units like 100% or 3"))?;
    let hundredths = if percent { value } else { value * 100.0 };
    if !(hundredths.is_finite() && (0.0..1e7).contains(&hundredths)) {
        return Err(err("type maximum units like 100% or 3"));
    }
    Ok(ResourceUnits(hundredths.round() as u32))
}

/// Replaces a task's assignments with the typed list, changing only what differs; unknown
/// names become new work resources, added at the end of the Resource Sheet.
fn set_task_resources(p: &Project, task: TaskId, text: &str) -> Result<Option<Planned>, IpcError> {
    let t = p.tasks.get(&task).ok_or_else(|| err("this task no longer exists"))?;
    let entries = parse_resource_list(text)?;
    if !entries.is_empty() && is_summary(p, task) {
        return Err(err(format!("resources work on the tasks inside “{}”, not on the summary itself", t.name)));
    }
    let mut commands = Vec::new();
    let mut last_key = p.resources.values().map(|r| r.sort_key.clone()).max();
    let mut wanted: Vec<(ResourceId, u32)> = Vec::new();
    let mut created: Vec<Resource> = Vec::new();
    for e in &entries {
        let existing = p.resources.values().chain(created.iter()).find(|r| r.name.eq_ignore_ascii_case(&e.name)).map(|r| r.id);
        let id = match existing {
            Some(id) => id,
            None => {
                check_resource_name(p, &e.name, None)?;
                let key = SortKey::between(last_key.as_ref(), None);
                last_key = Some(key.clone());
                let r = new_resource(&e.name, key);
                let id = r.id;
                created.push(r);
                id
            }
        };
        if wanted.iter().any(|(w, _)| *w == id) {
            return Err(err(format!("“{}” is listed twice", e.name)));
        }
        wanted.push((id, e.units.unwrap_or(100)));
    }
    if !created.is_empty() {
        commands.push(Command::InsertResources { resources: created, assignments: vec![] });
    }
    let existing: Vec<&Assignment> = p.assignments.values().filter(|a| a.task == task).collect();
    for a in &existing {
        if !wanted.iter().any(|(r, _)| *r == a.resource) {
            commands.push(Command::DeleteAssignment { id: a.id });
        }
    }
    for (resource, units) in wanted {
        match existing.iter().find(|a| a.resource == resource) {
            Some(a) if a.units.0 != units => commands.push(Command::SetAssignmentUnits { id: a.id, units: ResourceUnits(units) }),
            Some(_) => {}
            None => commands.push(Command::InsertAssignment {
                assignment: Assignment { id: AssignmentId(new_id()), task, resource, units: ResourceUnits(units) },
            }),
        }
    }
    Ok(match commands.len() {
        0 => None,
        1 => commands.pop().map(|command| Planned { command, created: None, message: None }),
        _ => Some(Planned { command: Command::Batch { label: "Change Resources".into(), commands }, created: None, message: None }),
    })
}

/// Leveling as one undo step, with a plain-words summary of what happened.
fn level_resources(p: &Project) -> Result<Option<Planned>, IpcError> {
    let result = pm_core::level(p).map_err(|e| err(format!("resources cannot be leveled while dates cannot be calculated: {e}")))?;
    let mut commands = Vec::new();
    for t in p.tasks.values() {
        let wanted = result.delays.get(&t.id).copied().unwrap_or(0);
        if wanted != t.leveling_delay.get() {
            commands.push(Command::SetTask { id: t.id, change: TaskChange::LevelingDelay(Duration::minutes(wanted)?) });
        }
    }
    let name = |id: &TaskId| p.tasks.get(id).map_or_else(String::new, |t| format!("“{}”", t.name));
    let person = |id: &ResourceId| p.resources.get(id).map_or_else(String::new, |r| r.name.clone());
    let delayed = result.delays.len();
    let mut message = match delayed {
        0 if result.unresolved.is_empty() => "Nobody is over-allocated, so no task needed to move.".to_owned(),
        0 => "No task could be moved to help.".to_owned(),
        1 => "1 task now starts later so nobody is over-allocated.".to_owned(),
        n => format!("{n} tasks now start later so nobody is over-allocated."),
    };
    for u in &result.unresolved {
        message.push(' ');
        message.push_str(&match u {
            pm_core::Unresolved::TooMuchOnOneTask { resource, task, units, max_units } => format!(
                "Still over-allocated: {} is assigned at {units}% on {} but can give {max_units}%; lower the units or raise the maximum.",
                person(resource),
                name(task)
            ),
            pm_core::Unresolved::FixedDates { resource, tasks } => format!(
                "Still over-allocated: {} on {}, which must start on fixed dates.",
                person(resource),
                tasks.iter().map(name).collect::<Vec<_>>().join(" and ")
            ),
            pm_core::Unresolved::GaveUp { resource, .. } => format!("Leveling stopped before it could clear every clash for {}.", person(resource)),
        });
    }
    if delayed > 0 {
        message.push_str(" Project > Clear Leveling (or Undo) takes the delays away.");
    }
    let command = batch("Level Resources", commands);
    Ok(Some(Planned { command: command.unwrap_or(Command::Batch { label: "Level Resources".into(), commands: vec![] }), created: None, message: Some(message) }))
}

/// Midnight at the start of a `YYYY-MM-DD` date.
fn day_start(date: &str) -> Result<Moment, IpcError> {
    let d: jiff::civil::Date = date.parse().map_err(|_| err(format!("{date:?} is not a date")))?;
    Ok(Moment::from_datetime(d.at(0, 0, 0, 0)))
}

fn parse_ids(ids: &[String]) -> Result<HashSet<TaskId>, IpcError> {
    ids.iter().map(|s| task_id(s)).collect()
}

fn set_duration(p: &Project, units: &Units, id: TaskId, text: &str) -> Result<Option<Planned>, IpcError> {
    let t = task(p, id)?;
    if is_summary(p, id) {
        return Err(err("a summary task's duration comes from the tasks inside it"));
    }
    let minutes = units.parse_duration(text)?;
    let set = |c: TaskChange| Command::SetTask { id, change: c };
    let command = match (minutes, t.is_milestone) {
        // Zero duration means milestone, as in Microsoft Project. The stored duration is
        // kept so that turning it back into a task restores it.
        (0, true) => return Ok(None),
        (0, false) => set(TaskChange::Milestone(true)),
        (m, true) => Command::Batch {
            label: "Change Duration".into(),
            commands: vec![set(TaskChange::Milestone(false)), set(TaskChange::Duration(Duration::minutes(m)?))],
        },
        (m, false) if m == t.duration.get() => return Ok(None),
        (m, false) => set(TaskChange::Duration(Duration::minutes(m)?)),
    };
    Ok(Some(Planned { command, created: None, message: None }))
}

/// Replaces the task's incoming links with the ones typed, changing only what differs.
fn set_predecessors(p: &Project, units: &Units, id: TaskId, text: &str) -> Result<Option<Planned>, IpcError> {
    let rows = outline(p);
    let mut wanted: BTreeMap<TaskId, (LinkType, i64)> = BTreeMap::new();
    for e in units.parse_predecessors(text)? {
        let pred = rows.get(e.row.wrapping_sub(1)).ok_or_else(|| err(format!("there is no task {}", e.row)))?.id;
        if pred == id {
            return Err(err("a task cannot be linked to itself"));
        }
        if wanted.insert(pred, (e.kind, e.lag)).is_some() {
            return Err(err(format!("task {} is listed twice", e.row)));
        }
    }
    let existing: Vec<&Dependency> = p.dependencies.values().filter(|d| d.successor == id).collect();
    let mut commands = Vec::new();
    // Deletions first, so a replaced link cannot look like a loop or a duplicate.
    for d in &existing {
        if !wanted.contains_key(&d.predecessor) {
            commands.push(Command::DeleteLink { id: d.id });
        }
    }
    for (pred, (kind, lag)) in wanted {
        match existing.iter().find(|d| d.predecessor == pred) {
            Some(d) => {
                if d.kind != kind {
                    commands.push(Command::SetLink { id: d.id, change: LinkChange::Kind(kind) });
                }
                if d.lag != lag {
                    commands.push(Command::SetLink { id: d.id, change: LinkChange::Lag(lag) });
                }
            }
            None => commands.push(Command::InsertLink {
                dependency: Dependency { id: DependencyId(new_id()), predecessor: pred, successor: id, kind, lag },
            }),
        }
    }
    Ok(match commands.len() {
        0 => None,
        1 => commands.pop().map(|command| Planned { command, created: None, message: None }),
        _ => Some(Planned { command: Command::Batch { label: "Change Predecessors".into(), commands }, created: None, message: None }),
    })
}

/// Selected tasks whose parent is not selected too, in outline order. Moving these moves
/// everything selected.
fn top_level_of_selection(p: &Project, selected: &HashSet<TaskId>) -> Vec<TaskId> {
    outline(p)
        .into_iter()
        .map(|r| r.id)
        .filter(|id| selected.contains(id))
        .filter(|id| {
            let mut cur = p.tasks[id].parent;
            while let Some(c) = cur {
                if selected.contains(&c) {
                    return false;
                }
                cur = p.tasks.get(&c).and_then(|t| t.parent);
            }
            true
        })
        .collect()
}

fn batch(label: &str, commands: Vec<Command>) -> Option<Command> {
    (!commands.is_empty()).then(|| Command::Batch { label: label.into(), commands })
}

/// Each selected task moves into the nearest unselected sibling above it, at the end.
fn indent(p: &Project, selected: &HashSet<TaskId>) -> Option<Command> {
    let tops = top_level_of_selection(p, selected);
    let top_set: HashSet<TaskId> = tops.iter().copied().collect();
    let mut groups: Vec<(TaskId, Vec<TaskId>)> = Vec::new();
    for &id in &tops {
        let t = &p.tasks[&id];
        let new_parent = p
            .tasks
            .values()
            .filter(|s| s.parent == t.parent && s.sort_key < t.sort_key && !top_set.contains(&s.id))
            .max_by(|a, b| a.sort_key.cmp(&b.sort_key))
            .map(|s| s.id);
        let Some(new_parent) = new_parent else { continue };
        match groups.iter_mut().find(|(np, _)| *np == new_parent) {
            Some((_, list)) => list.push(id),
            None => groups.push((new_parent, vec![id])),
        }
    }
    let mut commands = Vec::new();
    for (new_parent, ids) in groups {
        let last = child_keys(p, Some(new_parent)).last().copied().cloned();
        for (id, key) in ids.iter().zip(SortKey::n_between(last.as_ref(), None, ids.len())) {
            commands.push(Command::SetTask { id: *id, change: TaskChange::Position { parent: Some(new_parent), sort_key: key } });
        }
    }
    batch("Indent", commands)
}

/// Each selected task moves out of its summary, to just below it, and the tasks after it in
/// that summary become its subtasks, as in Microsoft Project. With several selected, each
/// takes the tasks between it and the next selected one.
fn outdent(p: &Project, selected: &HashSet<TaskId>) -> Option<Command> {
    let mut groups: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    let mut order = Vec::new();
    for id in top_level_of_selection(p, selected) {
        if let Some(parent) = p.tasks[&id].parent {
            if !groups.contains_key(&parent) {
                order.push(parent);
            }
            groups.entry(parent).or_default().push(id);
        }
    }
    let mut commands = Vec::new();
    for parent in order {
        let pt = &p.tasks[&parent];
        let after = child_keys(p, pt.parent).into_iter().find(|k| **k > pt.sort_key).cloned();
        let moving = &groups[&parent];
        let moving_set: HashSet<TaskId> = moving.iter().copied().collect();
        // The parent's other children, in order: each goes under the nearest moving task above it.
        let mut siblings: Vec<&Task> = p.tasks.values().filter(|t| t.parent == Some(parent)).collect();
        siblings.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        for (id, key) in moving.iter().zip(SortKey::n_between(Some(&pt.sort_key), after.as_ref(), moving.len())) {
            commands.push(Command::SetTask { id: *id, change: TaskChange::Position { parent: pt.parent, sort_key: key } });
        }
        let mut adopted: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
        let mut current: Option<TaskId> = None;
        for t in siblings {
            if moving_set.contains(&t.id) {
                current = Some(t.id);
            } else if let Some(new_parent) = current {
                adopted.entry(new_parent).or_default().push(t.id);
            }
        }
        for id in moving {
            let Some(kids) = adopted.get(id) else { continue };
            let last = child_keys(p, Some(*id)).last().copied().cloned();
            for (kid, key) in kids.iter().zip(SortKey::n_between(last.as_ref(), None, kids.len())) {
                commands.push(Command::SetTask { id: *kid, change: TaskChange::Position { parent: Some(*id), sort_key: key } });
            }
        }
    }
    batch("Outdent", commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{apply, CalendarId, ProjectId};

    fn project(rows: &[(&str, usize)]) -> (Project, Vec<TaskId>) {
        let mut p = Project {
            id: ProjectId(new_id()),
            name: "t".into(),
            start: pm_core::Moment(29_000_000),
            calendar_id: CalendarId(new_id()),
            calendar: pm_core::Calendar::standard(),
            tasks: Default::default(),
            dependencies: Default::default(),
            resources: Default::default(),
            assignments: Default::default(),
            currency: String::new(),
        };
        let mut ids = Vec::new();
        let mut stack: Vec<TaskId> = Vec::new();
        for &(name, depth) in rows {
            stack.truncate(depth);
            let parent = stack.last().copied();
            let keys = child_keys(&p, parent);
            let key = SortKey::between(keys.last().copied(), None);
            let id = TaskId(new_id());
            p.tasks.insert(
                id,
                Task {
                    id,
                    parent,
                    sort_key: key,
                    name: name.into(),
                    duration: Duration::minutes(480).unwrap(),
                    is_milestone: false,
                    constraint: Constraint::AsSoonAsPossible,
                    percent_complete: Percent::default(),
                    leveling_delay: Duration::ZERO,
                    fixed_cost: Money(0),
                    notes: String::new(),
                },
            );
            stack.push(id);
            ids.push(id);
        }
        (p, ids)
    }

    fn shape(p: &Project) -> Vec<(String, usize)> {
        outline(p).iter().map(|r| (p.tasks[&r.id].name.clone(), r.depth)).collect()
    }

    fn run(p: &mut Project, edit: Edit) {
        let sched = pm_core::schedule(p).ok();
        if let Some(plan) = plan(p, sched.as_ref(), &Units::default(), edit).unwrap() {
            apply(p, &plan.command).unwrap();
        }
    }

    fn s(rows: &[(&str, usize)]) -> Vec<(String, usize)> {
        rows.iter().map(|(n, d)| (n.to_string(), *d)).collect()
    }

    fn ids(list: &[TaskId]) -> Vec<String> {
        list.iter().map(|i| i.0.to_string()).collect()
    }

    #[test]
    fn indent_and_outdent_follow_the_outline() {
        let (mut p, id) = project(&[("A", 0), ("B", 0), ("C", 0), ("D", 0)]);
        run(&mut p, Edit::Indent { ids: ids(&[id[1], id[2]]) });
        assert_eq!(shape(&p), s(&[("A", 0), ("B", 1), ("C", 1), ("D", 0)]));
        run(&mut p, Edit::Indent { ids: ids(&[id[2]]) });
        assert_eq!(shape(&p), s(&[("A", 0), ("B", 1), ("C", 2), ("D", 0)]));
        // Outdenting B takes C with it, and B lands right after A.
        run(&mut p, Edit::Outdent { ids: ids(&[id[1]]) });
        assert_eq!(shape(&p), s(&[("A", 0), ("B", 0), ("C", 1), ("D", 0)]));
        run(&mut p, Edit::Outdent { ids: ids(&[id[2]]) });
        assert_eq!(shape(&p), s(&[("A", 0), ("B", 0), ("C", 0), ("D", 0)]));
    }

    #[test]
    fn indent_first_task_does_nothing() {
        let (p, id) = project(&[("A", 0), ("B", 0)]);
        assert!(plan(&p, None, &Units::default(), Edit::Indent { ids: ids(&[id[0]]) }).unwrap().is_none());
        assert!(plan(&p, None, &Units::default(), Edit::Outdent { ids: ids(&[id[0]]) }).unwrap().is_none());
    }

    #[test]
    fn outdent_takes_the_tasks_below_along_like_microsoft_project() {
        // Phase with A, B, C; outdenting B makes C a subtask of B.
        let (mut p, id) = project(&[("Phase", 0), ("A", 1), ("B", 1), ("C", 1), ("Next", 0)]);
        run(&mut p, Edit::Outdent { ids: ids(&[id[2]]) });
        assert_eq!(shape(&p), s(&[("Phase", 0), ("A", 1), ("B", 0), ("C", 1), ("Next", 0)]));
        // Undo puts everything back as one step.
        let (mut q, id) = project(&[("Phase", 0), ("A", 1), ("B", 1), ("C", 1), ("D", 1)]);
        let before = q.clone();
        let planned = plan(&q, None, &Units::default(), Edit::Outdent { ids: ids(&[id[2], id[3]]) }).unwrap().unwrap();
        let inverse = apply(&mut q, &planned.command).unwrap();
        // B and C both come out; D, after C, goes under C.
        assert_eq!(shape(&q), s(&[("Phase", 0), ("A", 1), ("B", 0), ("C", 0), ("D", 1)]));
        apply(&mut q, &inverse).unwrap();
        assert_eq!(q, before);
    }

    #[test]
    fn outdent_keeps_existing_subtasks_first() {
        let (mut p, id) = project(&[("Phase", 0), ("B", 1), ("B1", 2), ("C", 1)]);
        run(&mut p, Edit::Outdent { ids: ids(&[id[1]]) });
        assert_eq!(shape(&p), s(&[("Phase", 0), ("B", 0), ("B1", 1), ("C", 1)]));
    }

    #[test]
    fn predecessors_text_creates_changes_and_removes_links() {
        let (mut p, id) = project(&[("A", 0), ("B", 0), ("C", 0), ("D", 0)]);
        let d = id[3].0.to_string();
        run(&mut p, Edit::SetPredecessors { id: d.clone(), text: "1, 2SS+1d".into() });
        let links: Vec<(TaskId, LinkType, i64)> = p.dependencies.values().map(|l| (l.predecessor, l.kind, l.lag)).collect();
        assert_eq!(links.len(), 2);
        assert!(links.contains(&(id[0], LinkType::FinishToStart, 0)));
        assert!(links.contains(&(id[1], LinkType::StartToStart, 480)));
        let link_b = p.dependencies.values().find(|l| l.predecessor == id[1]).unwrap().id;

        run(&mut p, Edit::SetPredecessors { id: d.clone(), text: "2FF, 3".into() });
        assert_eq!(p.dependencies.len(), 2);
        let b = p.dependencies.values().find(|l| l.predecessor == id[1]).unwrap();
        assert_eq!((b.id, b.kind, b.lag), (link_b, LinkType::FinishToFinish, 0), "existing link kept and changed, not replaced");

        run(&mut p, Edit::SetPredecessors { id: d, text: "".into() });
        assert!(p.dependencies.is_empty());
    }

    #[test]
    fn predecessor_mistakes_are_explained() {
        let (p, id) = project(&[("A", 0), ("B", 0)]);
        let u = Units::default();
        let msg = |text: &str| match plan(&p, None, &u, Edit::SetPredecessors { id: id[1].0.to_string(), text: text.into() }) {
            Err(e) => e.message,
            Ok(_) => "accepted".into(),
        };
        assert_eq!(msg("9"), "there is no task 9");
        assert_eq!(msg("0"), "there is no task 0");
        assert_eq!(msg("2"), "a task cannot be linked to itself");
        assert_eq!(msg("1, 1SS"), "task 1 is listed twice");
        assert_eq!(msg("one"), "type predecessors as task numbers, like 2, 5SS or 7FF+2d");
    }

    #[test]
    fn zero_duration_makes_a_milestone_and_back() {
        let (mut p, id) = project(&[("A", 0)]);
        let a = id[0].0.to_string();
        run(&mut p, Edit::SetDuration { id: a.clone(), text: "0".into() });
        assert!(p.tasks[&id[0]].is_milestone);
        assert_eq!(p.tasks[&id[0]].duration.get(), 480, "stored duration kept");
        run(&mut p, Edit::SetDuration { id: a.clone(), text: "2w".into() });
        assert!(!p.tasks[&id[0]].is_milestone);
        assert_eq!(p.tasks[&id[0]].duration.get(), 4800);
        let e = plan(&p, None, &Units::default(), Edit::SetDuration { id: a, text: "soon".into() }).err().unwrap();
        assert_eq!(e.message, "type a duration like 3d, 4h or 2w");
    }

    #[test]
    fn dragging_a_bar_sets_start_no_earlier_than_on_working_time() {
        // Project starts Monday 2026-10-05.
        let (mut p, id) = project(&[("A", 0)]);
        p.start = Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0));
        run(&mut p, Edit::MoveTask { id: id[0].0.to_string(), start_date: "2026-10-10".into() }); // a Saturday
        let monday = Moment::from_datetime(jiff::civil::date(2026, 10, 12).at(8, 0, 0, 0));
        assert_eq!(p.tasks[&id[0]].constraint, Constraint::StartNoEarlierThan(monday));
        let s = pm_core::schedule(&p).unwrap();
        assert_eq!(s.tasks[&id[0]].early_start, monday);
    }

    #[test]
    fn dragging_the_right_edge_sets_the_duration() {
        let (mut p, id) = project(&[("A", 0)]);
        p.start = Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0));
        run(&mut p, Edit::SetFinish { id: id[0].0.to_string(), finish_date: "2026-10-07".into() });
        assert_eq!(p.tasks[&id[0]].duration.get(), 3 * 480, "Monday to Wednesday");
        run(&mut p, Edit::SetFinish { id: id[0].0.to_string(), finish_date: "2026-10-11".into() }); // Sunday
        assert_eq!(p.tasks[&id[0]].duration.get(), 5 * 480, "ends with Friday's work");
        let sched = pm_core::schedule(&p).ok();
        let e = plan(&p, sched.as_ref(), &Units::default(), Edit::SetFinish { id: id[0].0.to_string(), finish_date: "2026-10-04".into() });
        assert_eq!(e.err().unwrap().message, "the finish cannot be before the start");
    }

    #[test]
    fn task_info_changes_several_fields_as_one_step() {
        let (mut p, id) = project(&[("A", 0)]);
        let edit = Edit::UpdateTask {
            id: id[0].0.to_string(),
            name: Some("Survey".into()),
            duration_text: Some("2d".into()),
            milestone: Some(false),
            constraint: Some(ConstraintEdit { kind: ConstraintKind::MSO, date: Some("2026-10-10".into()) }),
            percent_complete: Some(25),
            notes: Some("Bring the drone".into()),
            resources: Some("Robin[50%]".into()),
            fixed_cost: Some("1.250,00".into()),
        };
        let planned = plan(&p, None, &Units::default(), edit).unwrap().unwrap();
        assert_eq!(planned.command.label(), "Change Task Info");
        apply(&mut p, &planned.command).unwrap();
        let t = &p.tasks[&id[0]];
        assert_eq!((t.name.as_str(), t.duration.get(), t.percent_complete.get(), t.notes.as_str()), ("Survey", 960, 25, "Bring the drone"));
        // Saturday the 10th moves to Monday the 12th at 08:00.
        let monday = Moment::from_datetime(jiff::civil::date(2026, 10, 12).at(8, 0, 0, 0));
        assert_eq!(t.constraint, Constraint::MustStartOn(monday));

        assert_eq!(p.assignments.values().next().map(|a| a.units.0), Some(50), "resources set in the same step");
        assert_eq!(p.tasks[&id[0]].fixed_cost, Money(125_000));
        let unchanged = Edit::UpdateTask { id: id[0].0.to_string(), name: Some("Survey".into()), duration_text: None, milestone: None, constraint: None, percent_complete: None, notes: None, resources: Some("robin[50%]".into()), fixed_cost: Some("1250".into()) };
        assert!(plan(&p, None, &Units::default(), unchanged).unwrap().is_none(), "nothing to undo when nothing changed");
        let no_date = Edit::UpdateTask {
            id: id[0].0.to_string(),
            name: None,
            duration_text: None,
            milestone: None,
            constraint: Some(ConstraintEdit { kind: ConstraintKind::SNET, date: None }),
            percent_complete: None,
            notes: None,
            resources: None,
            fixed_cost: None,
        };
        assert_eq!(plan(&p, None, &Units::default(), no_date).err().unwrap().message, "choose a date for “Start no earlier than”");
    }

    #[test]
    fn working_time_and_holidays() {
        let (mut p, _) = project(&[("A", 0)]);
        let day = vec![TimeRange { start: 540, end: 1020 }];
        let mut week = vec![day.clone(); 4];
        week.extend([vec![TimeRange { start: 540, end: 780 }], vec![], vec![]]);
        let edit = Edit::SetCalendar { week: week.clone(), exceptions: vec![DayException { date: "2026-12-25".into(), ranges: vec![] }] };
        run(&mut p, edit);
        assert_eq!(p.calendar.week()[0].working_minutes(), 480);
        assert_eq!(p.calendar.week()[4].working_minutes(), 240);
        assert!(!p.calendar.pattern(pm_core::time::day_number(jiff::civil::date(2026, 12, 25))).is_working());
        let none = Edit::SetCalendar { week: vec![vec![]; 7], exceptions: vec![] };
        let e = plan(&p, None, &Units::default(), none).err().unwrap();
        assert_eq!(e.message, "the working week has no working time, so no task could ever be scheduled");
        let overlap = Edit::SetCalendar { week: vec![vec![TimeRange { start: 480, end: 720 }, TimeRange { start: 700, end: 800 }]; 7], exceptions: vec![] };
        assert_eq!(plan(&p, None, &Units::default(), overlap).err().unwrap().message, "working time ranges on the same day overlap");
    }

    #[test]
    fn typing_resources_assigns_and_creates_them() {
        let (mut p, id) = project(&[("A", 0), ("B", 0)]);
        let a = id[0].0.to_string();
        run(&mut p, Edit::SetTaskResources { id: a.clone(), text: "Alex, Sam[50%]".into() });
        assert_eq!(p.resources.len(), 2, "unknown names became resources");
        let units = |p: &Project, name: &str| {
            let r = p.resources.values().find(|r| r.name == name).unwrap().id;
            p.assignments.values().find(|x| x.resource == r && x.task == id[0]).map(|x| x.units.0)
        };
        assert_eq!((units(&p, "Alex"), units(&p, "Sam")), (Some(100), Some(50)));
        assert_eq!(p.resources.values().find(|r| r.name == "Sam").unwrap().initials, "S");
        // Change Sam, drop Alex, add nobody new; names match regardless of case.
        let sam_assignment = p.assignments.values().find(|x| units(&p, "Sam") == Some(x.units.0)).unwrap().id;
        run(&mut p, Edit::SetTaskResources { id: a.clone(), text: "sam[75%]".into() });
        assert_eq!(p.resources.len(), 2);
        assert_eq!(p.assignments.len(), 1);
        assert_eq!(p.assignments[&sam_assignment].units.0, 75, "kept and changed, not replaced");
        run(&mut p, Edit::SetTaskResources { id: a, text: "".into() });
        assert!(p.assignments.is_empty());
    }

    #[test]
    fn resource_mistakes_are_explained() {
        let (mut p, id) = project(&[("Phase", 0), ("A", 1)]);
        let u = Units::default();
        let msg = |p: &Project, e: Edit| plan(p, None, &u, e).err().map(|e| e.message).unwrap_or_default();
        assert_eq!(msg(&p, Edit::SetTaskResources { id: id[0].0.to_string(), text: "Alex".into() }), "resources work on the tasks inside “Phase”, not on the summary itself");
        assert_eq!(msg(&p, Edit::SetTaskResources { id: id[1].0.to_string(), text: "Alex, alex".into() }), "“alex” is listed twice");
        run(&mut p, Edit::AddResource { after_id: None, name: "Alex".into() });
        assert_eq!(msg(&p, Edit::AddResource { after_id: None, name: "ALEX".into() }), "there is already a resource called “ALEX”");
        let r = p.resources.values().next().unwrap().id.0.to_string();
        let upd = |max: &str| Edit::UpdateResource { id: r.clone(), name: None, initials: None, resource_type: None, max_units: Some(max.into()), material_label: None, rate: None };
        assert_eq!(msg(&p, upd("lots")), "type maximum units like 100% or 3");
        run(&mut p, upd("3"));
        assert_eq!(p.resources.values().next().unwrap().max_units.0, 300);
    }

    #[test]
    fn building_a_wbs_from_the_chart() {
        let (mut p, id) = project(&[("Website", 0), ("Launch", 0)]);
        let web = id[0].0.to_string();
        run(&mut p, Edit::AddSubtask { parent_id: web.clone(), name: "Design".into() });
        run(&mut p, Edit::AddSubtask { parent_id: web.clone(), name: "Build".into() });
        assert_eq!(shape(&p), s(&[("Website", 0), ("Design", 1), ("Build", 1), ("Launch", 0)]));
        // Drag "Launch" onto "Website", then "Design" back to the top.
        run(&mut p, Edit::MoveUnder { id: id[1].0.to_string(), parent_id: Some(web) });
        let design = p.tasks.values().find(|t| t.name == "Design").unwrap().id.0.to_string();
        run(&mut p, Edit::MoveUnder { id: design, parent_id: None });
        assert_eq!(shape(&p), s(&[("Website", 0), ("Build", 1), ("Launch", 1), ("Design", 0)]));
        // A box cannot go inside itself.
        let e = plan(&p, None, &Units::default(), Edit::MoveUnder { id: id[0].0.to_string(), parent_id: Some(id[1].0.to_string()) });
        let planned = e.unwrap().unwrap();
        assert!(pm_core::apply(&mut p.clone(), &planned.command).is_err());
    }

    #[test]
    fn summary_duration_is_not_editable() {
        let (p, id) = project(&[("A", 0), ("A1", 1)]);
        let e = plan(&p, None, &Units::default(), Edit::SetDuration { id: id[0].0.to_string(), text: "3d".into() }).err().unwrap();
        assert_eq!(e.message, "a summary task's duration comes from the tasks inside it");
    }
}
