//! What a member may change (iteration 3, step 5), checked on every change the server accepts.
//!
//! Owners may change everything and Viewers nothing. For Editors the owner chooses which kinds
//! of change are allowed (schedule, progress, resources, costs, project settings) and which
//! tasks (all, only those they are assigned to, or only inside chosen phases).

use pm_core::{Command, Project, ProjectChange, ResourceChange, TaskChange, TaskId};

use crate::access::{Limits, Role, TaskScope};
use crate::storage::User;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Schedule,
    Progress,
    Resources,
    Costs,
    Project,
}

impl Kind {
    fn allowed(self, l: &Limits) -> bool {
        match self {
            Kind::Schedule => l.schedule,
            Kind::Progress => l.progress,
            Kind::Resources => l.resources,
            Kind::Costs => l.costs,
            Kind::Project => l.project,
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            Kind::Schedule => "the project's owner has not let you change the schedule (tasks, durations, links, constraints)",
            Kind::Progress => "the project's owner has not let you update progress",
            Kind::Resources => "the project's owner has not let you change resources or assignments",
            Kind::Costs => "the project's owner has not let you change costs",
            Kind::Project => "the project's owner has not let you change project settings",
        }
    }
}

/// Each kind of change in `cmd`, with the task it is about (if any). New tasks are checked
/// through their parent: a member allowed inside a phase may add tasks there.
fn parts(p: &Project, cmd: &Command, out: &mut Vec<(Kind, Option<TaskId>)>) {
    match cmd {
        Command::Batch { commands, .. } => commands.iter().for_each(|c| parts(p, c, out)),
        Command::InsertTasks { tasks, dependencies } => {
            for t in tasks {
                // A new task is judged by where it goes; at the top level, by itself.
                out.push((Kind::Schedule, t.parent.or(Some(t.id))));
            }
            for d in dependencies {
                out.push((Kind::Schedule, Some(d.successor)));
            }
        }
        Command::DeleteTasks { ids } => out.extend(ids.iter().map(|id| (Kind::Schedule, Some(*id)))),
        Command::SetTask { id, change } => {
            let kind = match change {
                TaskChange::PercentComplete(_) | TaskChange::Actual(_) => Kind::Progress,
                TaskChange::FixedCost(_) => Kind::Costs,
                TaskChange::LevelingDelay(_) => Kind::Project,
                _ => Kind::Schedule,
            };
            out.push((kind, Some(*id)));
            if let TaskChange::Position { parent: Some(parent), .. } = change {
                out.push((Kind::Schedule, Some(*parent)));
            }
        }
        Command::InsertLink { dependency: d } => out.push((Kind::Schedule, Some(d.successor))),
        Command::DeleteLink { id } | Command::SetLink { id, .. } => out.push((Kind::Schedule, p.dependencies.get(id).map(|d| d.successor))),
        Command::SetProject { change } => out.push((
            match change {
                ProjectChange::StatusDate(_) => Kind::Progress,
                _ => Kind::Project,
            },
            None,
        )),
        Command::InsertResources { assignments, .. } => {
            out.push((Kind::Resources, None));
            out.extend(assignments.iter().map(|a| (Kind::Resources, Some(a.task))));
        }
        Command::DeleteResources { .. } => out.push((Kind::Resources, None)),
        Command::SetResource { change: ResourceChange::Rate(_), .. } => out.push((Kind::Costs, None)),
        Command::SetResource { .. } => out.push((Kind::Resources, None)),
        Command::InsertAssignment { assignment: a } => out.push((Kind::Resources, Some(a.task))),
        Command::DeleteAssignment { id } | Command::SetAssignmentUnits { id, .. } => out.push((Kind::Resources, p.assignments.get(id).map(|a| a.task))),
    }
}

/// Is `user` assigned to `task`, as a resource with their name or email?
fn assigned(p: &Project, user: &User, task: TaskId) -> bool {
    let local = user.email.split('@').next().unwrap_or_default();
    p.assignments.values().filter(|a| a.task == task).filter_map(|a| p.resources.get(&a.resource)).any(|r| {
        let n = r.name.trim();
        n.eq_ignore_ascii_case(&user.name) || n.eq_ignore_ascii_case(&user.email) || (!local.is_empty() && n.eq_ignore_ascii_case(local))
    })
}

/// Is `task` one of `phases` or inside one?
fn inside(p: &Project, phases: &[TaskId], task: TaskId) -> bool {
    let mut at = Some(task);
    while let Some(id) = at {
        if phases.contains(&id) {
            return true;
        }
        at = p.tasks.get(&id).and_then(|t| t.parent);
    }
    false
}

/// Ok, or why this member may not make this change, in plain words.
pub fn check(role: Role, limits: &Limits, user: &User, project: &Project, command: &Command) -> Result<(), String> {
    match role {
        Role::Owner => return Ok(()),
        Role::Viewer => return Err("you can view this project but not change it".into()),
        Role::Editor => {}
    }
    let mut list = Vec::new();
    parts(project, command, &mut list);
    for (kind, task) in list {
        if !kind.allowed(limits) {
            return Err(kind.refusal().into());
        }
        let Some(task) = task else {
            // Project-wide changes need access to every task.
            if limits.tasks != TaskScope::All && matches!(kind, Kind::Project | Kind::Schedule) {
                return Err("the project's owner lets you change only some tasks, so project-wide changes are not open to you".into());
            }
            continue;
        };
        let name = || project.tasks.get(&task).map_or_else(|| "this task".to_owned(), |t| format!("“{}”", t.name));
        match &limits.tasks {
            TaskScope::All => {}
            TaskScope::Assigned if !assigned(project, user, task) => {
                return Err(format!("the project's owner lets you change only tasks you are assigned to, and {} is not one of them", name()));
            }
            TaskScope::Phases { ids } if !inside(project, ids, task) => {
                return Err(format!("the project's owner lets you change only some phases, and {} is not in them", name()));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::*;
    use uuid::Uuid;

    fn tid(n: u128) -> TaskId {
        TaskId(Uuid::from_u128(n))
    }

    /// Phase 1 holds Survey (Alex assigned) and Pack; Phase 2 holds Move.
    fn project() -> Project {
        let mut p = Project {
            id: ProjectId(Uuid::from_u128(1)),
            name: "Move".into(),
            start: Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0)),
            calendar_id: CalendarId(Uuid::from_u128(2)),
            calendar: Calendar::standard(),
            tasks: Default::default(),
            dependencies: Default::default(),
            resources: Default::default(),
            assignments: Default::default(),
            currency: String::new(),
            baseline: None,
            status_date: None,
        };
        for (n, parent, name, key) in [(1, None, "Phase 1", "B"), (2, Some(1), "Survey", "B"), (3, Some(1), "Pack", "D"), (4, None, "Phase 2", "D"), (5, Some(4), "Move", "B")] {
            let t = Task {
                id: tid(n),
                parent: parent.map(tid),
                sort_key: SortKey(key.into()),
                name: name.into(),
                duration: Duration::minutes(480).unwrap(),
                is_milestone: false,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                deadline: None,
                elapsed: false,
                actual: Actual::default(),
                notes: String::new(),
            };
            p.tasks.insert(t.id, t);
        }
        let r = Resource {
            id: ResourceId(Uuid::from_u128(70)),
            sort_key: SortKey("V".into()),
            name: "alex".into(),
            initials: "A".into(),
            kind: ResourceKind::Work,
            max_units: ResourceUnits(100),
            material_label: String::new(),
            rate: Money(0),
            notes: String::new(),
            availability: Default::default(),
        };
        let a = Assignment { id: AssignmentId(Uuid::from_u128(80)), task: tid(2), resource: r.id, units: ResourceUnits(100) };
        p.resources.insert(r.id, r);
        p.assignments.insert(a.id, a);
        p
    }

    fn alex() -> User {
        User { id: Uuid::from_u128(9), name: "Alex Martin".into(), email: "alex@example.com".into(), is_admin: false, created_at: String::new() }
    }

    fn set(n: u128, change: TaskChange) -> Command {
        Command::SetTask { id: tid(n), change }
    }

    #[test]
    fn editors_are_held_to_the_kinds_of_change_the_owner_allows() {
        let p = project();
        let progress_only = Limits { schedule: false, resources: false, costs: false, project: false, ..Limits::default() };
        let may = |c: &Command| super::check(Role::Editor, &progress_only, &alex(), &p, c);
        assert_eq!(may(&set(3, TaskChange::PercentComplete(Percent::new(50).unwrap()))), Ok(()));
        assert_eq!(may(&set(3, TaskChange::Duration(Duration::minutes(960).unwrap()))), Err(Kind::Schedule.refusal().into()));
        assert_eq!(may(&set(3, TaskChange::FixedCost(Money(10)))), Err(Kind::Costs.refusal().into()));
        let calendar = Command::SetProject { change: ProjectChange::Currency("$".into()) };
        assert_eq!(may(&calendar), Err(Kind::Project.refusal().into()));
        // A batch is refused as a whole if any part is.
        let batch = Command::Batch { label: "Change Task Info".into(), commands: vec![set(3, TaskChange::PercentComplete(Percent::new(10).unwrap())), set(3, TaskChange::Name("x".into()))] };
        assert!(may(&batch).is_err());
        // Owners may do anything; viewers nothing.
        assert_eq!(super::check(Role::Owner, &progress_only, &alex(), &p, &calendar), Ok(()));
        assert!(super::check(Role::Viewer, &Limits::default(), &alex(), &p, &set(3, TaskChange::Notes("x".into()))).is_err());
    }

    #[test]
    fn editors_can_be_kept_to_their_own_tasks_or_some_phases() {
        let p = project();
        let own = Limits { tasks: TaskScope::Assigned, ..Limits::default() };
        let may = |l: &Limits, c: &Command| super::check(Role::Editor, l, &alex(), &p, c);
        assert_eq!(may(&own, &set(2, TaskChange::Name("Site survey".into()))), Ok(()), "assigned as “alex”, the start of the email");
        let refused = may(&own, &set(3, TaskChange::Name("x".into()))).unwrap_err();
        assert_eq!(refused, "the project's owner lets you change only tasks you are assigned to, and “Pack” is not one of them");

        let phase1 = Limits { tasks: TaskScope::Phases { ids: vec![tid(1)] }, ..Limits::default() };
        assert_eq!(may(&phase1, &set(3, TaskChange::Name("x".into()))), Ok(()));
        assert!(may(&phase1, &set(5, TaskChange::Name("x".into()))).is_err());
        // Moving a task out of the phase, or a project-wide change, is refused.
        assert!(may(&phase1, &set(3, TaskChange::Position { parent: Some(tid(4)), sort_key: SortKey("Z".into()) })).is_err());
        assert!(may(&phase1, &Command::SetProject { change: ProjectChange::Currency("$".into()) }).is_err());
    }
}
