//! Reminders (iteration 4, automation): which tasks need a nudge now. Late: due by now and not
//! done (the portfolio's "should have finished by now"). Due soon: due within the next
//! working day. The app decides who sees them and sends each once.

use crate::{Moment, Project, ResourceId, Schedule, TaskId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReminderKind {
    Late,
    DueSoon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub task: TaskId,
    pub name: String,
    pub kind: ReminderKind,
    /// When it is (or was) due: the scheduled finish.
    pub due: Moment,
    /// The resources assigned to it.
    pub people: Vec<ResourceId>,
}

/// Working minutes ahead counted as "soon": one standard working day.
pub const SOON: i64 = 8 * 60;

/// Tasks (not summaries) that are late or due soon at `now`, late first, then by due date.
pub fn reminders(p: &Project, s: &Schedule, now: Moment) -> Vec<Reminder> {
    let parents: std::collections::BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
    let soon = p.calendar.add_working(p.calendar.next_working(now), SOON);
    let mut out: Vec<Reminder> = p
        .tasks
        .values()
        .filter(|t| !parents.contains(&t.id) && t.percent_complete.get() < 100)
        .filter_map(|t| {
            let due = s.tasks.get(&t.id)?.early_finish;
            let kind = if due <= now {
                ReminderKind::Late
            } else if due <= soon {
                ReminderKind::DueSoon
            } else {
                return None;
            };
            let people = p.assignments.values().filter(|a| a.task == t.id).map(|a| a.resource).collect();
            Some(Reminder { task: t.id, name: t.name.clone(), kind, due, people })
        })
        .collect();
    out.sort_by(|a, b| (a.kind, a.due, &a.name).cmp(&(b.kind, b.due, &b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use jiff::civil::date;
    use uuid::Uuid;

    /// One-day tasks linked one after another, from `start`.
    fn chain(names: &[&str], start: Moment) -> Project {
        let mut p = Project {
            id: ProjectId(Uuid::from_u128(0)),
            name: "P".into(),
            start,
            calendar_id: CalendarId(Uuid::from_u128(0)),
            calendar: Calendar::standard(),
            tasks: Default::default(),
            dependencies: Default::default(),
            resources: Default::default(),
            assignments: Default::default(),
            currency: String::new(),
            baseline: None,
            status_date: None,
        };
        let id = |i: usize| TaskId(Uuid::from_u128(i as u128 + 1));
        let mut key: Option<SortKey> = None;
        let tasks = names
            .iter()
            .enumerate()
            .map(|(i, n)| Task {
                id: id(i),
                parent: None,
                sort_key: {
                    let k = SortKey::between(key.as_ref(), None);
                    key = Some(k.clone());
                    k
                },
                name: (*n).into(),
                duration: Duration::minutes(480).unwrap(),
                is_milestone: false,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                deadline: None,
                elapsed: false,
                task_type: Default::default(),
                effort_driven: false,
                actual: Actual::default(),
                notes: String::new(),
            })
            .collect();
        let links = (1..names.len())
            .map(|i| Dependency { id: DependencyId(Uuid::from_u128(100 + i as u128)), predecessor: id(i - 1), successor: id(i), kind: LinkType::FinishToStart, lag: 0, lag_format: crate::LagFormat::Working })
            .collect();
        apply(&mut p, &Command::InsertTasks { tasks, dependencies: links }).unwrap();
        p
    }

    fn at(y: i16, m: i8, d: i8, h: i8) -> Moment {
        Moment::from_datetime(date(y, m, d).at(h, 0, 0, 0))
    }

    #[test]
    fn late_due_soon_and_done_tasks() {
        // Three one-day tasks in a row from Monday 2 November 2026: A Mon, B Tue, C Wed.
        let mut p = chain(&["A", "B", "C"], at(2026, 11, 2, 8));
        let ids: Vec<TaskId> = crate::outline(&p).iter().map(|r| r.id).collect();
        let s = schedule(&p).unwrap();
        // Tuesday 10:00: A is late, B is due today; C ends Wednesday 17:00, more than a working
        // day away.
        let names = |rs: Vec<Reminder>| rs.into_iter().map(|r| (r.name, r.kind)).collect::<Vec<_>>();
        assert_eq!(
            names(reminders(&p, &s, at(2026, 11, 3, 10))),
            [("A".into(), ReminderKind::Late), ("B".into(), ReminderKind::DueSoon)]
        );
        // Monday 9:00: nothing is late; A ends today, B ends Tuesday 17:00 (after Tuesday 9:00).
        assert_eq!(names(reminders(&p, &s, at(2026, 11, 2, 9))), [("A".into(), ReminderKind::DueSoon)]);
        // A done: no longer late.
        apply(&mut p, &Command::SetTask { id: ids[0], change: TaskChange::PercentComplete(Percent::new(100).unwrap()) }).unwrap();
        let s = schedule(&p).unwrap();
        assert_eq!(names(reminders(&p, &s, at(2026, 11, 3, 10))).first().map(|x| x.0.clone()), Some("B".to_owned()));
    }

    #[test]
    fn friday_evening_looks_to_monday() {
        let p = chain(&["A", "B", "C", "D", "E", "F"], at(2026, 11, 2, 8));
        let s = schedule(&p).unwrap();
        // Friday 18:00: E (Friday) is late; F (Monday) is due within the next working day.
        let got: Vec<String> = reminders(&p, &s, at(2026, 11, 6, 18)).into_iter().filter(|r| r.kind == ReminderKind::DueSoon).map(|r| r.name).collect();
        assert_eq!(got, ["F"]);
    }
}
