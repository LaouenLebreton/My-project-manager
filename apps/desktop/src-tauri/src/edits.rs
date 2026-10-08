//! Turning what the user did ([`Edit`]) into core commands. The app chooses ids and sort
//! keys here, so the UI never has to.

use std::collections::{BTreeMap, HashMap, HashSet};

use pm_core::{
    outline, Command, Constraint, Dependency, DependencyId, Duration, LinkChange, LinkType, Moment, Percent, Project,
    Schedule, SortKey, Task, TaskChange, TaskId, Units, MINUTES_PER_DAY,
};
use serde::Deserialize;
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
}

/// A core command, plus the id of a task it creates (so the UI can select it).
pub struct Planned {
    pub command: Command,
    pub created: Option<TaskId>,
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
    let plain = |command| Ok(Some(Planned { command, created: None }));
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
                    notes: String::new(),
                }],
                dependencies: vec![],
            };
            Ok(Some(Planned { command, created: Some(id) }))
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
        Edit::Indent { ids } => Ok(indent(p, &parse_ids(&ids)?).map(|command| Planned { command, created: None })),
        Edit::Outdent { ids } => Ok(outdent(p, &parse_ids(&ids)?).map(|command| Planned { command, created: None })),
        Edit::MoveTask { id, start_date } => {
            let id = task_id(&id)?;
            task(p, id)?;
            if is_summary(p, id) {
                return Err(err("a summary task moves with the tasks inside it"));
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
    Ok(Some(Planned { command, created: None }))
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
        1 => commands.pop().map(|command| Planned { command, created: None }),
        _ => Some(Planned { command: Command::Batch { label: "Change Predecessors".into(), commands }, created: None }),
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

/// Each selected task moves out of its summary, to just below it.
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
        let ids = &groups[&parent];
        for (id, key) in ids.iter().zip(SortKey::n_between(Some(&pt.sort_key), after.as_ref(), ids.len())) {
            commands.push(Command::SetTask { id: *id, change: TaskChange::Position { parent: pt.parent, sort_key: key } });
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
    fn outdent_lands_between_parent_and_its_next_sibling() {
        let (mut p, id) = project(&[("A", 0), ("A1", 1), ("A2", 1), ("B", 0)]);
        run(&mut p, Edit::Outdent { ids: ids(&[id[1]]) });
        assert_eq!(shape(&p), s(&[("A", 0), ("A2", 1), ("A1", 0), ("B", 0)]));
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
    fn summary_duration_is_not_editable() {
        let (p, id) = project(&[("A", 0), ("A1", 1)]);
        let e = plan(&p, None, &Units::default(), Edit::SetDuration { id: id[0].0.to_string(), text: "3d".into() }).err().unwrap();
        assert_eq!(e.message, "a summary task's duration comes from the tasks inside it");
    }
}
