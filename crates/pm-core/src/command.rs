//! Commands: the only way to change a project (spec 4.6).
//!
//! [`apply`] either changes the project and returns the inverse command, or returns an
//! error and leaves the project untouched. Applying the inverse restores the project
//! exactly, which is what undo, redo and later sync are built on.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::calendar::Calendar;
use crate::schedule::ScheduleError;
use crate::model::{
    Constraint, Dependency, DependencyId, Duration, LinkType, Percent, Project, SortKey, Task, TaskId,
};
use crate::time::Moment;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    /// Adds tasks (parents may be earlier in the same list) and then links.
    InsertTasks { tasks: Vec<Task>, dependencies: Vec<Dependency> },
    /// Deletes tasks with everything inside them and every link touching them.
    DeleteTasks { ids: Vec<TaskId> },
    SetTask { id: TaskId, change: TaskChange },
    InsertLink { dependency: Dependency },
    DeleteLink { id: DependencyId },
    SetLink { id: DependencyId, change: LinkChange },
    SetProject { change: ProjectChange },
    /// Several commands as one undo step, e.g. indenting a selection or a drag.
    Batch { label: String, commands: Vec<Command> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value")]
pub enum TaskChange {
    Name(String),
    Duration(Duration),
    Milestone(bool),
    Constraint(Constraint),
    PercentComplete(Percent),
    Notes(String),
    /// Outline position: parent and place among siblings.
    Position { parent: Option<TaskId>, sort_key: SortKey },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value")]
pub enum LinkChange {
    Kind(LinkType),
    Lag(i64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value")]
pub enum ProjectChange {
    Name(String),
    Start(Moment),
    Calendar(Calendar),
}

/// Why a command was refused. Messages are written for the person using the app.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CommandError {
    #[error("this task no longer exists")]
    NoSuchTask { id: TaskId },
    #[error("this link no longer exists")]
    NoSuchLink { id: DependencyId },
    #[error("a task with this id already exists")]
    TaskExists { id: TaskId },
    #[error("a link with this id already exists")]
    LinkExists { id: DependencyId },
    #[error("a task cannot be linked to itself")]
    SelfLink,
    #[error("these two tasks are already linked; change the existing link instead")]
    DuplicateLink { existing: DependencyId },
    #[error("this link would make the tasks wait for each other in a loop")]
    WouldCreateCycle { path: Vec<TaskId> },
    #[error("finish-to-finish and start-to-finish links cannot point to a summary task; link to a task inside it instead")]
    FinishLinkToSummary { task: TaskId },
    #[error("a summary task cannot be linked to a task inside it")]
    LinkInsideSummary,
    #[error("this change would leave a project that cannot be scheduled: {reason}")]
    Unschedulable { reason: String },
    #[error("a task cannot be moved inside itself")]
    OutlineLoop { id: TaskId },
}

/// Ids a command (or its inverse) may have changed. The store rewrites exactly these.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    pub tasks: BTreeSet<TaskId>,
    pub links: BTreeSet<DependencyId>,
    pub project: bool,
}

impl Command {
    /// Name for the Edit menu: "Undo Change Duration".
    pub fn label(&self) -> String {
        match self {
            Command::InsertTasks { tasks, .. } if tasks.len() == 1 => "Add Task".into(),
            Command::InsertTasks { .. } => "Add Tasks".into(),
            Command::DeleteTasks { ids } if ids.len() == 1 => "Delete Task".into(),
            Command::DeleteTasks { .. } => "Delete Tasks".into(),
            Command::SetTask { change, .. } => match change {
                TaskChange::Name(_) => "Rename Task",
                TaskChange::Duration(_) => "Change Duration",
                TaskChange::Milestone(true) => "Make Milestone",
                TaskChange::Milestone(false) => "Make Regular Task",
                TaskChange::Constraint(_) => "Change Constraint",
                TaskChange::PercentComplete(_) => "Change Percent Complete",
                TaskChange::Notes(_) => "Edit Notes",
                TaskChange::Position { .. } => "Move Task",
            }
            .into(),
            Command::InsertLink { .. } => "Add Link".into(),
            Command::DeleteLink { .. } => "Delete Link".into(),
            Command::SetLink { change: LinkChange::Kind(_), .. } => "Change Link Type".into(),
            Command::SetLink { change: LinkChange::Lag(_), .. } => "Change Lag".into(),
            Command::SetProject { change } => match change {
                ProjectChange::Name(_) => "Rename Project",
                ProjectChange::Start(_) => "Change Project Start",
                ProjectChange::Calendar(_) => "Change Calendar",
            }
            .into(),
            Command::Batch { label, .. } => label.clone(),
        }
    }

    /// Adds every id this command names to `t`. Call it for a command and its inverse:
    /// together they cover everything that changed (a delete's inverse lists the subtree).
    pub fn touched(&self, t: &mut Touched) {
        match self {
            Command::InsertTasks { tasks, dependencies } => {
                t.tasks.extend(tasks.iter().map(|x| x.id));
                t.links.extend(dependencies.iter().map(|d| d.id));
            }
            Command::DeleteTasks { ids } => t.tasks.extend(ids),
            Command::SetTask { id, .. } => {
                t.tasks.insert(*id);
            }
            Command::InsertLink { dependency } => {
                t.links.insert(dependency.id);
            }
            Command::DeleteLink { id } | Command::SetLink { id, .. } => {
                t.links.insert(*id);
            }
            Command::SetProject { .. } => t.project = true,
            Command::Batch { commands, .. } => commands.iter().for_each(|c| c.touched(t)),
        }
    }
}

/// Applies `cmd` and returns its inverse, or refuses it and leaves `p` unchanged.
pub fn apply(p: &mut Project, cmd: &Command) -> Result<Command, CommandError> {
    match cmd {
        Command::InsertTasks { tasks, dependencies } => insert_tasks(p, tasks, dependencies),
        Command::DeleteTasks { ids } => delete_tasks(p, ids),
        Command::SetTask { id, change } => set_task(p, *id, change),
        Command::InsertLink { dependency } => {
            check_link(p, dependency)?;
            p.dependencies.insert(dependency.id, dependency.clone());
            if let Err(e) = validate(p) {
                p.dependencies.remove(&dependency.id);
                return Err(e);
            }
            Ok(Command::DeleteLink { id: dependency.id })
        }
        Command::DeleteLink { id } => {
            let dependency = p.dependencies.remove(id).ok_or(CommandError::NoSuchLink { id: *id })?;
            Ok(Command::InsertLink { dependency })
        }
        Command::SetLink { id, change } => {
            let link = p.dependencies.get_mut(id).ok_or(CommandError::NoSuchLink { id: *id })?;
            let old = match change {
                LinkChange::Kind(k) => LinkChange::Kind(std::mem::replace(&mut link.kind, *k)),
                LinkChange::Lag(l) => LinkChange::Lag(std::mem::replace(&mut link.lag, *l)),
            };
            if let Err(e) = validate(p) {
                apply(p, &Command::SetLink { id: *id, change: old }).expect("restoring the old value");
                return Err(e);
            }
            Ok(Command::SetLink { id: *id, change: old })
        }
        Command::SetProject { change } => {
            let old = match change {
                ProjectChange::Name(n) => ProjectChange::Name(std::mem::replace(&mut p.name, n.clone())),
                ProjectChange::Start(s) => ProjectChange::Start(std::mem::replace(&mut p.start, *s)),
                ProjectChange::Calendar(c) => ProjectChange::Calendar(std::mem::replace(&mut p.calendar, c.clone())),
            };
            Ok(Command::SetProject { change: old })
        }
        Command::Batch { label, commands } => {
            let mut inverses = Vec::with_capacity(commands.len());
            for c in commands {
                match apply(p, c) {
                    Ok(inv) => inverses.push(inv),
                    Err(e) => {
                        for inv in inverses.iter().rev() {
                            apply(p, inv).expect("inverse of an applied command applies");
                        }
                        return Err(e);
                    }
                }
            }
            inverses.reverse();
            Ok(Command::Batch { label: label.clone(), commands: inverses })
        }
    }
}

/// After an edit that touches links or the outline: the project must still be schedulable
/// (R25: no link between a summary and a task inside it, no finish link into a summary, no
/// loops through links or summaries). The engine's own check decides, so the two agree.
fn validate(p: &Project) -> Result<(), CommandError> {
    if p.dependencies.values().any(|d| is_inside(p, d.successor, d.predecessor) || is_inside(p, d.predecessor, d.successor)) {
        return Err(CommandError::LinkInsideSummary);
    }
    crate::schedule::check(p).map_err(|e| match e {
        ScheduleError::Cycle { tasks, .. } => CommandError::WouldCreateCycle { path: tasks },
        ScheduleError::FinishLinkToSummary { task, .. } => CommandError::FinishLinkToSummary { task },
        other => CommandError::Unschedulable { reason: other.to_string() },
    })
}

/// Checks a new link against the project as it is now; [`validate`] runs after it is added.
fn check_link(p: &Project, d: &Dependency) -> Result<(), CommandError> {
    if p.dependencies.contains_key(&d.id) {
        return Err(CommandError::LinkExists { id: d.id });
    }
    if d.predecessor == d.successor {
        return Err(CommandError::SelfLink);
    }
    for id in [d.predecessor, d.successor] {
        if !p.tasks.contains_key(&id) {
            return Err(CommandError::NoSuchTask { id });
        }
    }
    if let Some(existing) = p.dependencies.values().find(|x| {
        (x.predecessor, x.successor) == (d.predecessor, d.successor)
            || (x.predecessor, x.successor) == (d.successor, d.predecessor)
    }) {
        return Err(CommandError::DuplicateLink { existing: existing.id });
    }
    Ok(())
}

fn is_inside(p: &Project, id: TaskId, ancestor: TaskId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        cur = p.tasks.get(&c).and_then(|t| t.parent);
    }
    false
}

fn insert_tasks(p: &mut Project, tasks: &[Task], links: &[Dependency]) -> Result<Command, CommandError> {
    let mut new_ids = BTreeSet::new();
    for t in tasks {
        if p.tasks.contains_key(&t.id) || !new_ids.insert(t.id) {
            return Err(CommandError::TaskExists { id: t.id });
        }
        if let Some(parent) = t.parent
            && !p.tasks.contains_key(&parent)
            && !new_ids.contains(&parent)
        {
            return Err(CommandError::NoSuchTask { id: parent });
        }
    }
    for t in tasks {
        p.tasks.insert(t.id, t.clone());
    }
    let inverse = Command::DeleteTasks { ids: tasks.iter().map(|t| t.id).collect() };
    for (k, d) in links.iter().enumerate() {
        if let Err(e) = check_link(p, d) {
            for d in &links[..k] {
                p.dependencies.remove(&d.id);
            }
            for t in tasks {
                p.tasks.remove(&t.id);
            }
            return Err(e);
        }
        p.dependencies.insert(d.id, d.clone());
    }
    // A new task under a linked task turns it into a linked summary; check that still works.
    if let Err(e) = validate(p) {
        for d in links {
            p.dependencies.remove(&d.id);
        }
        for t in tasks {
            p.tasks.remove(&t.id);
        }
        return Err(e);
    }
    Ok(inverse)
}

fn delete_tasks(p: &mut Project, ids: &[TaskId]) -> Result<Command, CommandError> {
    if let Some(&id) = ids.iter().find(|id| !p.tasks.contains_key(id)) {
        return Err(CommandError::NoSuchTask { id });
    }
    let roots: Vec<TaskId> = ids.to_vec();
    // Parents before children, so the inverse can re-insert in order.
    let mut doomed: Vec<Task> = p.tasks.values().filter(|t| roots.iter().any(|&r| is_inside(p, t.id, r))).cloned().collect();
    doomed.sort_by_key(|t| (depth(p, t.id), t.id));
    let doomed_ids: BTreeSet<TaskId> = doomed.iter().map(|t| t.id).collect();
    let links: Vec<Dependency> = p
        .dependencies
        .values()
        .filter(|d| doomed_ids.contains(&d.predecessor) || doomed_ids.contains(&d.successor))
        .cloned()
        .collect();
    for d in &links {
        p.dependencies.remove(&d.id);
    }
    for id in &doomed_ids {
        p.tasks.remove(id);
    }
    Ok(Command::InsertTasks { tasks: doomed, dependencies: links })
}

fn depth(p: &Project, id: TaskId) -> usize {
    std::iter::successors(p.tasks.get(&id).and_then(|t| t.parent), |&x| p.tasks.get(&x).and_then(|t| t.parent)).count()
}

fn set_task(p: &mut Project, id: TaskId, change: &TaskChange) -> Result<Command, CommandError> {
    if !p.tasks.contains_key(&id) {
        return Err(CommandError::NoSuchTask { id });
    }
    if let TaskChange::Position { parent: Some(parent), .. } = change {
        if !p.tasks.contains_key(parent) {
            return Err(CommandError::NoSuchTask { id: *parent });
        }
        if is_inside(p, *parent, id) {
            return Err(CommandError::OutlineLoop { id });
        }
    }
    let t = p.tasks.get_mut(&id).expect("checked above");
    let old = match change {
        TaskChange::Name(v) => TaskChange::Name(std::mem::replace(&mut t.name, v.clone())),
        TaskChange::Duration(v) => TaskChange::Duration(std::mem::replace(&mut t.duration, *v)),
        TaskChange::Milestone(v) => TaskChange::Milestone(std::mem::replace(&mut t.is_milestone, *v)),
        TaskChange::Constraint(v) => TaskChange::Constraint(std::mem::replace(&mut t.constraint, *v)),
        TaskChange::PercentComplete(v) => TaskChange::PercentComplete(std::mem::replace(&mut t.percent_complete, *v)),
        TaskChange::Notes(v) => TaskChange::Notes(std::mem::replace(&mut t.notes, v.clone())),
        TaskChange::Position { parent, sort_key } => TaskChange::Position {
            parent: std::mem::replace(&mut t.parent, *parent),
            sort_key: std::mem::replace(&mut t.sort_key, sort_key.clone()),
        },
    };
    // Moving a task can put it inside a task it is linked to; refuse that.
    if matches!(change, TaskChange::Position { .. })
        && let Err(e) = validate(p)
    {
        set_task(p, id, &old).expect("restoring the old position");
        return Err(e);
    }
    Ok(Command::SetTask { id, change: old })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CalendarId, ProjectId};
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn tid(n: u128) -> TaskId {
        TaskId(Uuid::from_u128(n))
    }

    fn lid(n: u128) -> DependencyId {
        DependencyId(Uuid::from_u128(1000 + n))
    }

    fn task(n: u128, parent: Option<u128>) -> Task {
        Task {
            id: tid(n),
            parent: parent.map(tid),
            sort_key: SortKey(format!("{n:04}")),
            name: format!("T{n}"),
            duration: Duration::minutes(480).unwrap(),
            is_milestone: false,
            constraint: Constraint::AsSoonAsPossible,
            percent_complete: Percent::default(),
            notes: String::new(),
        }
    }

    fn link(n: u128, a: u128, b: u128) -> Dependency {
        Dependency { id: lid(n), predecessor: tid(a), successor: tid(b), kind: LinkType::FinishToStart, lag: 0 }
    }

    fn project() -> Project {
        Project {
            id: ProjectId(Uuid::from_u128(0)),
            name: "p".into(),
            start: Moment(0),
            calendar_id: CalendarId(Uuid::from_u128(0)),
            calendar: Calendar::standard(),
            tasks: BTreeMap::new(),
            dependencies: BTreeMap::new(),
        }
    }

    /// Summary 1 holds 2 and 3; 4 is top level. Links 2 -> 3 -> 4.
    fn sample() -> Project {
        let mut p = project();
        apply(
            &mut p,
            &Command::InsertTasks {
                tasks: vec![task(1, None), task(2, Some(1)), task(3, Some(1)), task(4, None)],
                dependencies: vec![link(1, 2, 3), link(2, 3, 4)],
            },
        )
        .unwrap();
        p
    }

    fn roundtrip(cmd: Command) -> Project {
        let mut p = sample();
        let before = p.clone();
        let inv = apply(&mut p, &cmd).unwrap();
        let after = p.clone();
        let redo = apply(&mut p, &inv).unwrap();
        assert_eq!(p, before, "undo restores");
        apply(&mut p, &redo).unwrap();
        assert_eq!(p, after, "redo restores");
        after
    }

    #[test]
    fn delete_summary_removes_subtree_and_links_and_undo_restores() {
        let p = roundtrip(Command::DeleteTasks { ids: vec![tid(1)] });
        assert_eq!(p.tasks.keys().copied().collect::<Vec<_>>(), vec![tid(4)]);
        assert!(p.dependencies.is_empty());
    }

    #[test]
    fn set_fields_roundtrip() {
        roundtrip(Command::SetTask { id: tid(2), change: TaskChange::Name("Design".into()) });
        roundtrip(Command::SetTask { id: tid(2), change: TaskChange::Duration(Duration::minutes(60).unwrap()) });
        roundtrip(Command::SetTask { id: tid(4), change: TaskChange::Position { parent: Some(tid(1)), sort_key: SortKey("9".into()) } });
        roundtrip(Command::SetLink { id: lid(1), change: LinkChange::Lag(-480) });
        roundtrip(Command::SetProject { change: ProjectChange::Start(Moment(99)) });
    }

    #[test]
    fn batch_is_one_step_and_atomic() {
        let ok = Command::Batch {
            label: "Indent".into(),
            commands: vec![
                Command::DeleteLink { id: lid(2) },
                Command::SetTask { id: tid(4), change: TaskChange::Position { parent: Some(tid(1)), sort_key: SortKey("9".into()) } },
            ],
        };
        roundtrip(ok);

        let mut p = sample();
        let before = p.clone();
        let failing = Command::Batch {
            label: "x".into(),
            commands: vec![
                Command::SetTask { id: tid(2), change: TaskChange::Name("changed".into()) },
                Command::InsertLink { dependency: link(9, 4, 2) }, // 2 -> 3 -> 4 -> 2 is a loop
            ],
        };
        assert!(matches!(apply(&mut p, &failing), Err(CommandError::WouldCreateCycle { .. })));
        assert_eq!(p, before, "nothing from a failed batch remains");
    }

    #[test]
    fn link_rules() {
        let mut p = sample();
        let before = p.clone();
        assert_eq!(apply(&mut p, &Command::InsertLink { dependency: link(9, 2, 2) }), Err(CommandError::SelfLink));
        assert_eq!(
            apply(&mut p, &Command::InsertLink { dependency: link(9, 3, 2) }),
            Err(CommandError::DuplicateLink { existing: lid(1) })
        );
        assert_eq!(
            apply(&mut p, &Command::InsertLink { dependency: link(9, 4, 2) }),
            Err(CommandError::WouldCreateCycle { path: vec![tid(2), tid(3), tid(4)] })
        );
        assert_eq!(apply(&mut p, &Command::InsertLink { dependency: link(9, 1, 2) }), Err(CommandError::LinkInsideSummary));
        assert_eq!(p, before);
        // R25: a summary may be linked, but finish links may not point into one.
        let mut ff = link(9, 4, 1);
        ff.kind = LinkType::FinishToFinish;
        let mut q = sample();
        apply(&mut q, &Command::DeleteLink { id: lid(2) }).unwrap(); // so 4 -> 1 is not a loop
        assert_eq!(apply(&mut q, &Command::InsertLink { dependency: ff }), Err(CommandError::FinishLinkToSummary { task: tid(1) }));
        assert!(apply(&mut q, &Command::InsertLink { dependency: link(10, 4, 1) }).is_ok(), "finish-to-start into a summary is fine");
    }

    #[test]
    fn summary_links_are_allowed_and_changing_one_to_a_finish_link_is_refused() {
        let mut p = sample();
        apply(&mut p, &Command::InsertTasks { tasks: vec![task(5, None)], dependencies: vec![] }).unwrap();
        apply(&mut p, &Command::InsertLink { dependency: link(9, 5, 1) }).unwrap();
        let before = p.clone();
        let change = Command::SetLink { id: lid(9), change: LinkChange::Kind(LinkType::FinishToFinish) };
        assert_eq!(apply(&mut p, &change), Err(CommandError::FinishLinkToSummary { task: tid(1) }));
        assert_eq!(p, before, "the link type is restored");
    }

    #[test]
    fn cannot_move_into_itself_or_inside_a_task_it_is_linked_to() {
        let mut p = sample();
        let pos = |parent| TaskChange::Position { parent: Some(tid(parent)), sort_key: SortKey("x".into()) };
        assert_eq!(apply(&mut p, &Command::SetTask { id: tid(1), change: pos(2) }), Err(CommandError::OutlineLoop { id: tid(1) }));
        assert_eq!(apply(&mut p, &Command::SetTask { id: tid(1), change: pos(1) }), Err(CommandError::OutlineLoop { id: tid(1) }));
        // 4 is linked from 3, which is inside 1: putting 1 under 4 would link 4 to a task inside it.
        assert_eq!(apply(&mut p, &Command::SetTask { id: tid(1), change: pos(4) }), Err(CommandError::LinkInsideSummary));
        assert_eq!(p, sample(), "the move is undone");
    }

    #[test]
    fn inserting_under_a_linked_task_makes_a_linked_summary() {
        let mut p = sample();
        let cmd = Command::InsertTasks { tasks: vec![task(7, Some(4))], dependencies: vec![] };
        assert!(apply(&mut p, &cmd).is_ok(), "4 becomes a summary with its incoming link");
        assert!(crate::schedule::schedule(&p).is_ok());
        let before = p.clone();
        let cmd = Command::InsertTasks { tasks: vec![task(8, None)], dependencies: vec![link(7, 8, 8)] };
        assert_eq!(apply(&mut p, &cmd), Err(CommandError::SelfLink));
        assert_eq!(p, before);
    }

    #[test]
    fn labels_and_serialized_shape() {
        let c = Command::SetTask { id: tid(2), change: TaskChange::Duration(Duration::minutes(60).unwrap()) };
        assert_eq!(c.label(), "Change Duration");
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["type"], "SetTask");
        assert_eq!(json["change"], serde_json::json!({"field": "Duration", "value": 60}));
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), c);
    }

    fn commands() -> impl Strategy<Value = Command> {
        let t = || 1u128..8;
        prop_oneof![
            (8u128..12, prop::option::of(t())).prop_map(|(n, parent)| Command::InsertTasks { tasks: vec![task(n, parent)], dependencies: vec![] }),
            t().prop_map(|n| Command::DeleteTasks { ids: vec![tid(n)] }),
            (t(), 0i64..2000).prop_map(|(n, d)| Command::SetTask { id: tid(n), change: TaskChange::Duration(Duration::minutes(d).unwrap()) }),
            (t(), prop::option::of(t())).prop_map(|(n, parent)| Command::SetTask {
                id: tid(n),
                change: TaskChange::Position { parent: parent.map(tid), sort_key: SortKey("m".into()) }
            }),
            (20u128..40, t(), t()).prop_map(|(n, a, b)| Command::InsertLink { dependency: link(n, a, b) }),
            (1u128..40).prop_map(|n| Command::DeleteLink { id: lid(n) }),
            (1u128..40, -960i64..960).prop_map(|(n, l)| Command::SetLink { id: lid(n), change: LinkChange::Lag(l) }),
        ]
    }

    proptest! {
        /// After any sequence of commands, undoing each successful one in reverse order
        /// gets back to the start, and refused commands change nothing.
        #[test]
        fn undo_everything_restores_the_start(cmds in prop::collection::vec(commands(), 1..30)) {
            let mut p = sample();
            let start = p.clone();
            let mut inverses = Vec::new();
            for c in &cmds {
                let before = p.clone();
                match apply(&mut p, c) {
                    Ok(inv) => inverses.push(inv),
                    Err(_) => prop_assert_eq!(&p, &before),
                }
                prop_assert!(crate::schedule::schedule(&p).is_ok(), "commands keep the project schedulable: {:?}", c);
            }
            for inv in inverses.iter().rev() {
                apply(&mut p, inv).unwrap();
            }
            prop_assert_eq!(p, start);
        }
    }
}
