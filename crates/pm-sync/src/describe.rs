//! Changes in plain words, for the History view: "changed the duration of “Survey” from
//! 3 days to 5 days". Worked out from the project as it was just before the change, so names
//! and old values are the ones people saw.

use pm_core::{apply, Actual, Command, Constraint, LinkChange, LinkType, Moment, Project, ProjectChange, ResourceChange, ResourceKind, TaskChange, TaskId};

/// One sentence (without the person's name) per thing the change did.
pub fn describe(before: &Project, command: &Command) -> Vec<String> {
    let mut lines = Vec::new();
    let mut scratch = before.clone();
    walk(&mut scratch, command, &mut lines);
    lines
}

/// The whole change as one line: the first sentence, plus how many more there are.
pub fn summary(before: &Project, command: &Command) -> String {
    let lines = describe(before, command);
    match lines.len() {
        0 => command.label().to_lowercase(),
        1 => lines[0].clone(),
        n => format!("{} (and {} more)", lines[0], n - 1),
    }
}

/// Every task a change touches, for "show the task" and filtering.
pub fn tasks(command: &Command) -> Vec<TaskId> {
    let mut t = pm_core::Touched::default();
    command.touched(&mut t);
    t.tasks.into_iter().collect()
}

/// Describes `cmd` against `p`, then applies it, so later steps of a batch see earlier ones.
fn walk(p: &mut Project, cmd: &Command, out: &mut Vec<String>) {
    if let Command::Batch { commands, .. } = cmd {
        for c in commands {
            walk(p, c, out);
        }
        return;
    }
    out.extend(one(p, cmd));
    let _ = apply(p, cmd);
}

fn quoted(s: &str) -> String {
    format!("“{s}”")
}

fn task_name(p: &Project, id: TaskId) -> String {
    p.tasks.get(&id).map_or_else(|| "a task".to_owned(), |t| quoted(&t.name))
}

/// "3 days", "1 day", "4 hours", "1.5 days".
fn duration(minutes: i64) -> String {
    let plural = |n: f64, one: &str, many: &str| {
        let s = format!("{n:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        format!("{s} {}", if s == "1" { one } else { many })
    };
    if minutes == 0 {
        "0 days".into()
    } else if minutes % 240 == 0 || minutes.abs() >= 480 {
        plural(minutes as f64 / 480.0, "day", "days")
    } else if minutes % 60 == 0 {
        plural(minutes as f64 / 60.0, "hour", "hours")
    } else {
        format!("{minutes} minutes")
    }
}

/// "Mon 5 Oct 2026".
fn date(m: Moment) -> String {
    m.to_datetime().strftime("%a %-d %b %Y").to_string()
}

fn money(p: &Project, cents: i64) -> String {
    let n = format!("{}.{:02}", cents.abs() / 100, cents.abs() % 100);
    let sign = if cents < 0 { "-" } else { "" };
    if p.currency.is_empty() { format!("{sign}{n}") } else { format!("{sign}{}{n}", p.currency) }
}

fn constraint(c: Constraint) -> String {
    match c {
        Constraint::AsSoonAsPossible => "as soon as possible".into(),
        Constraint::AsLateAsPossible => "as late as possible".into(),
        Constraint::StartNoEarlierThan(d) => format!("start no earlier than {}", date(d)),
        Constraint::StartNoLaterThan(d) => format!("start no later than {}", date(d)),
        Constraint::FinishNoEarlierThan(d) => format!("finish no earlier than {}", date(d)),
        Constraint::FinishNoLaterThan(d) => format!("finish no later than {}", date(d)),
        Constraint::MustStartOn(d) => format!("must start on {}", date(d)),
        Constraint::MustFinishOn(d) => format!("must finish on {}", date(d)),
    }
}

fn link_kind(k: LinkType) -> &'static str {
    match k {
        LinkType::FinishToStart => "finish-to-start",
        LinkType::StartToStart => "start-to-start",
        LinkType::FinishToFinish => "finish-to-finish",
        LinkType::StartToFinish => "start-to-finish",
    }
}

fn names(p: &Project, ids: &[TaskId]) -> String {
    let shown: Vec<String> = ids.iter().take(3).map(|id| task_name(p, *id)).collect();
    if ids.len() > 3 { format!("{}, …", shown.join(", ")) } else { shown.join(", ") }
}

fn units(p: &Project, resource: pm_core::ResourceId, u: u32) -> String {
    match p.resources.get(&resource) {
        Some(r) if r.kind == ResourceKind::Material => {
            let n = format!("{:.2}", f64::from(u) / 100.0);
            format!("{} {}", n.trim_end_matches('0').trim_end_matches('.'), r.material_label).trim().to_owned()
        }
        _ => format!("{u}%"),
    }
}

fn resource_name(p: &Project, id: pm_core::ResourceId) -> String {
    p.resources.get(&id).map_or_else(|| "a resource".to_owned(), |r| quoted(&r.name))
}

fn one(p: &Project, cmd: &Command) -> Vec<String> {
    let line = match cmd {
        Command::Batch { .. } => return Vec::new(),
        Command::InsertTasks { tasks, .. } => match tasks.as_slice() {
            [t] => format!("added the task {}", quoted(&t.name)),
            many => format!("added {} tasks", many.len()),
        },
        Command::DeleteTasks { ids } => match ids.as_slice() {
            [id] => format!("deleted the task {}", task_name(p, *id)),
            many => format!("deleted {} tasks ({})", many.len(), names(p, many)),
        },
        Command::SetTask { id, change } => {
            let Some(t) = p.tasks.get(id) else { return vec![format!("changed a task that no longer exists")] };
            let n = quoted(&t.name);
            match change {
                TaskChange::Name(new) => format!("renamed {n} to {}", quoted(new)),
                TaskChange::Duration(d) => format!("changed the duration of {n} from {} to {}", duration(t.duration.get()), duration(d.get())),
                TaskChange::Milestone(true) => format!("made {n} a milestone"),
                TaskChange::Milestone(false) => format!("made {n} a regular task"),
                TaskChange::Constraint(c) => format!("set {n} to {} (was {})", constraint(*c), constraint(t.constraint)),
                TaskChange::PercentComplete(pc) => format!("set {n} to {}% complete (was {}%)", pc.get(), t.percent_complete.get()),
                TaskChange::Notes(_) => format!("edited the notes of {n}"),
                TaskChange::Position { parent, .. } if *parent != t.parent => match parent {
                    Some(parent) => format!("moved {n} under {}", task_name(p, *parent)),
                    None => format!("moved {n} to the top level"),
                },
                TaskChange::Position { .. } => format!("moved {n} in the outline"),
                TaskChange::LevelingDelay(d) if d.get() == 0 => format!("removed the leveling delay of {n}"),
                TaskChange::LevelingDelay(d) => format!("delayed {n} by {} to level resources", duration(d.get())),
                TaskChange::FixedCost(m) => format!("changed the fixed cost of {n} to {} (was {})", money(p, m.0), money(p, t.fixed_cost.0)),
                TaskChange::ManualStart(Some(m)) => format!("scheduled {n} manually, starting {}", date(*m)),
                TaskChange::ManualStart(None) => format!("scheduled {n} automatically again"),
                TaskChange::Actual(Actual::NotStarted) => format!("cleared the actual dates of {n}"),
                TaskChange::Actual(Actual::Started { start }) => format!("recorded that {n} started on {}", date(*start)),
                TaskChange::Actual(Actual::Finished { finish, .. }) => format!("recorded that {n} finished on {}", date(*finish)),
            }
        }
        Command::InsertLink { dependency: d } => {
            format!("linked {} to {} ({})", task_name(p, d.predecessor), task_name(p, d.successor), link_kind(d.kind))
        }
        Command::DeleteLink { id } => match p.dependencies.get(id) {
            Some(d) => format!("removed the link from {} to {}", task_name(p, d.predecessor), task_name(p, d.successor)),
            None => "removed a link".into(),
        },
        Command::SetLink { id, change } => {
            let Some(d) = p.dependencies.get(id) else { return vec!["changed a link".into()] };
            let what = format!("the link from {} to {}", task_name(p, d.predecessor), task_name(p, d.successor));
            match change {
                LinkChange::Kind(k) => format!("made {what} {}", link_kind(*k)),
                LinkChange::Lag(l) if *l < 0 => format!("set a lead of {} on {what}", duration(-l)),
                LinkChange::Lag(l) => format!("set a lag of {} on {what}", duration(*l)),
            }
        }
        Command::SetProject { change } => match change {
            ProjectChange::Name(n) => format!("renamed the project to {}", quoted(n)),
            ProjectChange::Start(m) => format!("moved the project start to {} (was {})", date(*m), date(p.start)),
            ProjectChange::Calendar(_) => "changed the working time".into(),
            ProjectChange::Currency(c) => format!("changed the currency to {c}"),
            ProjectChange::Baseline(Some(_)) => "saved a baseline".into(),
            ProjectChange::Baseline(None) => "cleared the baseline".into(),
            ProjectChange::StatusDate(Some(m)) => format!("set the status date to {}", date(*m)),
            ProjectChange::StatusDate(None) => "cleared the status date".into(),
        },
        Command::InsertResources { resources, .. } => match resources.as_slice() {
            [r] => format!("added the resource {}", quoted(&r.name)),
            many => format!("added {} resources", many.len()),
        },
        Command::DeleteResources { ids } => match ids.as_slice() {
            [id] => format!("deleted the resource {}", resource_name(p, *id)),
            many => format!("deleted {} resources", many.len()),
        },
        Command::SetResource { id, change } => {
            let n = resource_name(p, *id);
            match change {
                ResourceChange::Name(new) => format!("renamed the resource {n} to {}", quoted(new)),
                ResourceChange::Initials(i) => format!("set the initials of {n} to {i}"),
                ResourceChange::Kind(ResourceKind::Work) => format!("made {n} a work resource"),
                ResourceChange::Kind(ResourceKind::Material) => format!("made {n} a material resource"),
                ResourceChange::MaxUnits(u) => format!("set the maximum units of {n} to {}%", u.0),
                ResourceChange::MaterialLabel(l) => format!("set the material label of {n} to {l}"),
                ResourceChange::Notes(_) => format!("edited the notes of {n}"),
                ResourceChange::Position(_) => format!("moved {n} in the Resource Sheet"),
                ResourceChange::Rate(m) => format!("set the rate of {n} to {}", money(p, m.0)),
            }
        }
        Command::InsertAssignment { assignment: a } => {
            format!("assigned {} to {} ({})", resource_name(p, a.resource), task_name(p, a.task), units(p, a.resource, a.units.0))
        }
        Command::DeleteAssignment { id } => match p.assignments.get(id) {
            Some(a) => format!("removed {} from {}", resource_name(p, a.resource), task_name(p, a.task)),
            None => "removed an assignment".into(),
        },
        Command::SetAssignmentUnits { id, units: u } => match p.assignments.get(id) {
            Some(a) => format!(
                "changed {} on {} to {} (was {})",
                resource_name(p, a.resource),
                task_name(p, a.task),
                units(p, a.resource, u.0),
                units(p, a.resource, a.units.0)
            ),
            None => "changed an assignment".into(),
        },
    };
    vec![line]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::*;
    use uuid::Uuid;

    fn project() -> (Project, TaskId, TaskId) {
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
            currency: "€".into(),
            baseline: None,
            status_date: None,
        };
        let mk = |n: u128, name: &str, key: &str| Task {
            id: TaskId(Uuid::from_u128(n)),
            parent: None,
            sort_key: SortKey(key.into()),
            name: name.into(),
            duration: Duration::minutes(3 * 480).unwrap(),
            is_milestone: false,
            constraint: Constraint::AsSoonAsPossible,
            percent_complete: Percent::default(),
            leveling_delay: Duration::ZERO,
            fixed_cost: Money(0),
            manual_start: None,
            actual: Actual::default(),
            notes: String::new(),
        };
        let (a, b) = (mk(10, "Survey", "B"), mk(11, "Pack", "D"));
        let ids = (a.id, b.id);
        p.tasks.insert(a.id, a);
        p.tasks.insert(b.id, b);
        (p, ids.0, ids.1)
    }

    #[test]
    fn changes_read_as_sentences_with_old_values() {
        let (p, survey, pack) = project();
        let set = |id, change| Command::SetTask { id, change };
        assert_eq!(summary(&p, &set(survey, TaskChange::Duration(Duration::minutes(5 * 480).unwrap()))), "changed the duration of “Survey” from 3 days to 5 days");
        assert_eq!(summary(&p, &set(survey, TaskChange::Name("Site survey".into()))), "renamed “Survey” to “Site survey”");
        assert_eq!(summary(&p, &set(pack, TaskChange::PercentComplete(Percent::new(50).unwrap()))), "set “Pack” to 50% complete (was 0%)");
        assert_eq!(summary(&p, &set(pack, TaskChange::FixedCost(Money(120_050)))), "changed the fixed cost of “Pack” to €1200.50 (was €0.00)");
        let monday = Moment::from_datetime(jiff::civil::date(2026, 10, 12).at(8, 0, 0, 0));
        assert_eq!(summary(&p, &set(pack, TaskChange::Constraint(Constraint::StartNoEarlierThan(monday)))), "set “Pack” to start no earlier than Mon 12 Oct 2026 (was as soon as possible)");
        let link = Command::InsertLink { dependency: Dependency { id: DependencyId(Uuid::from_u128(50)), predecessor: survey, successor: pack, kind: LinkType::FinishToStart, lag: 0 } };
        assert_eq!(summary(&p, &link), "linked “Survey” to “Pack” (finish-to-start)");
        assert_eq!(summary(&p, &Command::DeleteTasks { ids: vec![pack] }), "deleted the task “Pack”");
    }

    #[test]
    fn a_batch_lists_each_step_with_names_as_they_were_then() {
        let (p, survey, _) = project();
        let batch = Command::Batch {
            label: "Change Task Info".into(),
            commands: vec![
                Command::SetTask { id: survey, change: TaskChange::Name("Site survey".into()) },
                Command::SetTask { id: survey, change: TaskChange::Duration(Duration::minutes(240).unwrap()) },
            ],
        };
        assert_eq!(describe(&p, &batch), ["renamed “Survey” to “Site survey”", "changed the duration of “Site survey” from 3 days to 0.5 days"]);
        assert_eq!(summary(&p, &batch), "renamed “Survey” to “Site survey” (and 1 more)");
        assert_eq!(tasks(&batch), [survey]);
    }
}
