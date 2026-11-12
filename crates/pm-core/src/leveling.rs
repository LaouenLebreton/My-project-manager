//! Resource leveling: delay tasks until no work resource is over-allocated (iteration 2).
//!
//! Like Microsoft Project's manual leveling: on demand, starting from no delays, the earliest
//! overload is resolved first by delaying its least important task until the work it clashes
//! with is done. Important means: least total slack, then earliest start, then outline order.
//! Tasks fixed with "must start on" or "must finish on" are never moved. Links and constraints keep working,
//! because a delay only pushes a task later (rule R30).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::{Duration, Project, ResourceId, TaskId};
use crate::outline::outline;
use crate::resources::loads;
use crate::schedule::{schedule, ScheduleError};
use crate::time::Moment;

/// An overload leveling could not remove, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unresolved {
    /// One task alone asks for more than the resource has (200% of a full-time person).
    TooMuchOnOneTask { resource: ResourceId, task: TaskId, units: u32, max_units: u32 },
    /// Every task in the clash is fixed with "must start on".
    FixedDates { resource: ResourceId, tasks: Vec<TaskId> },
    /// Leveling stopped after many rounds without clearing this clash.
    GaveUp { resource: ResourceId, tasks: Vec<TaskId> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leveled {
    /// The leveling delay each task should have, in working minutes (tasks not listed: none).
    pub delays: BTreeMap<TaskId, i64>,
    pub unresolved: Vec<Unresolved>,
}

/// Works out leveling delays for the project. Pure and deterministic; the project itself is
/// not changed (apply the delays with commands, so the result is one undo step).
pub fn level(p: &Project) -> Result<Leveled, ScheduleError> {
    let mut q = p.clone();
    for t in q.tasks.values_mut() {
        t.leveling_delay = Duration::ZERO;
    }
    let position: HashMap<TaskId, usize> = outline(&q).iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    let mut unresolved = Vec::new();
    let mut skipped: BTreeSet<(ResourceId, Moment)> = BTreeSet::new();
    let rounds = 100 + 20 * q.assignments.len();

    for round in 0..=rounds {
        let s = schedule(&q)?;
        let l = loads(&q, &s);
        let next = l
            .resources
            .iter()
            .flat_map(|(r, load)| load.overloads.iter().map(move |o| (o.start, *r, o)))
            .filter(|(start, r, _)| !skipped.contains(&(*r, *start)))
            .min_by_key(|(start, r, _)| (*start, *r));
        let Some((start, resource, overload)) = next else { break };
        if round == rounds {
            unresolved.push(Unresolved::GaveUp { resource, tasks: overload.tasks.clone() });
            break;
        }
        let mut tasks = overload.tasks.clone();
        if tasks.len() == 1 {
            let task = tasks[0];
            let units = q.assignments.values().filter(|a| a.task == task && a.resource == resource).map(|a| a.units.0).sum();
            unresolved.push(Unresolved::TooMuchOnOneTask { resource, task, units, max_units: q.resources[&resource].max_units.0 });
            skipped.insert((resource, start));
            continue;
        }
        // Most important first; the last task that may move is the one to delay.
        tasks.sort_by_key(|t| {
            let ts = s.tasks[t];
            (ts.total_slack, ts.early_start, position.get(t).copied().unwrap_or(usize::MAX))
        });
        let movable = |t: &TaskId| !q.tasks[t].constraint.is_fixed() && q.tasks[t].manual_start.is_none();
        let Some(&pick) = tasks.iter().rev().find(|t| movable(t)) else {
            unresolved.push(Unresolved::FixedDates { resource, tasks: overload.tasks.clone() });
            skipped.insert((resource, start));
            continue;
        };
        // Wait until the first clashing task ends (or, failing that, the overload does).
        let pick_start = s.tasks[&pick].early_start;
        let target = tasks
            .iter()
            .filter(|t| **t != pick)
            .map(|t| s.tasks[t].early_finish)
            .filter(|f| *f > pick_start)
            .min()
            .unwrap_or(overload.finish);
        let delta = q.calendar.working_between(pick_start, target).max(1);
        let t = q.tasks.get_mut(&pick).expect("task in overload");
        t.leveling_delay = Duration::minutes(t.leveling_delay.get() + delta).expect("positive");
    }

    let delays = q.tasks.values().filter(|t| t.leveling_delay.get() > 0).map(|t| (t.id, t.leveling_delay.get())).collect();
    Ok(Leveled { delays, unresolved })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Calendar;
    use crate::command::{apply, Command};
    use crate::model::*;
    use jiff::civil::date;
    use proptest::prelude::*;
    use uuid::Uuid;

    const DAY: i64 = 480;

    fn at(day: i8) -> Moment {
        Moment::from_datetime(date(2026, 10, day).at(8, 0, 0, 0))
    }

    fn tid(n: u128) -> TaskId {
        TaskId(Uuid::from_u128(n))
    }

    fn rid(n: u128) -> ResourceId {
        ResourceId(Uuid::from_u128(500 + n))
    }

    struct P(Project, u128);

    impl P {
        fn new() -> Self {
            P(
                Project {
                    id: ProjectId(Uuid::from_u128(0)),
                    name: "t".into(),
                    start: at(5),
                    calendar_id: CalendarId(Uuid::from_u128(0)),
                    calendar: Calendar::standard(),
                    tasks: Default::default(),
                    dependencies: Default::default(),
                    resources: Default::default(),
                    assignments: Default::default(),
                    currency: String::new(),
                    baseline: None,
                    status_date: None,
                },
                900,
            )
        }

        fn task(mut self, n: u128, minutes: i64) -> Self {
            let t = Task {
                id: tid(n),
                parent: None,
                sort_key: SortKey(format!("{n:04}")),
                name: format!("T{n}"),
                duration: Duration::minutes(minutes).unwrap(),
                is_milestone: minutes == 0,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                notes: String::new(),
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                deadline: None,
                elapsed: false,
                task_type: Default::default(),
                effort_driven: false,
                actual: Actual::default(),
            };
            apply(&mut self.0, &Command::InsertTasks { tasks: vec![t], dependencies: vec![] }).unwrap();
            self
        }

        fn fs(mut self, a: u128, b: u128) -> Self {
            self.1 += 1;
            let d = Dependency { id: DependencyId(Uuid::from_u128(self.1)), predecessor: tid(a), successor: tid(b), kind: LinkType::FinishToStart, lag: 0, lag_format: crate::LagFormat::Working };
            apply(&mut self.0, &Command::InsertLink { dependency: d }).unwrap();
            self
        }

        fn person(mut self, n: u128, max: u32) -> Self {
            let r = Resource {
                id: rid(n),
                sort_key: SortKey(format!("{n:04}")),
                name: format!("R{n}"),
                initials: String::new(),
                kind: ResourceKind::Work,
                max_units: ResourceUnits(max),
                material_label: String::new(),
                rate: Money(0),
                notes: String::new(),
                availability: Default::default(),
            };
            apply(&mut self.0, &Command::InsertResources { resources: vec![r], assignments: vec![] }).unwrap();
            self
        }

        fn assign(mut self, task: u128, resource: u128, units: u32) -> Self {
            self.1 += 1;
            let a = Assignment { id: AssignmentId(Uuid::from_u128(self.1)), task: tid(task), resource: rid(resource), units: ResourceUnits(units) };
            apply(&mut self.0, &Command::InsertAssignment { assignment: a }).unwrap();
            self
        }

        fn fix(mut self, n: u128, day: i8) -> Self {
            self.0.tasks.get_mut(&tid(n)).unwrap().constraint = Constraint::MustStartOn(at(day));
            self
        }

        /// Levels and returns the leveled project with its result.
        fn level(&self) -> (Project, Leveled) {
            let r = level(&self.0).unwrap();
            let mut q = self.0.clone();
            for (t, d) in &r.delays {
                q.tasks.get_mut(t).unwrap().leveling_delay = Duration::minutes(*d).unwrap();
            }
            (q, r)
        }
    }

    fn starts(p: &Project) -> BTreeMap<TaskId, Moment> {
        schedule(p).unwrap().tasks.iter().map(|(t, s)| (*t, s.early_start)).collect()
    }

    fn overloads(p: &Project) -> usize {
        loads(p, &schedule(p).unwrap()).resources.values().map(|r| r.overloads.len()).sum()
    }

    #[test]
    fn the_task_with_slack_waits_for_the_critical_one() {
        // 1 (3d) is critical; 2 (1d) has slack; one person on both.
        let (q, r) = P::new().task(1, 3 * DAY).task(2, DAY).person(1, 100).assign(1, 1, 100).assign(2, 1, 100).level();
        assert_eq!(r.delays.keys().copied().collect::<Vec<_>>(), vec![tid(2)]);
        assert_eq!(starts(&q)[&tid(2)], at(8), "after task 1 finishes on Wednesday");
        assert_eq!(overloads(&q), 0);
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn must_start_on_is_never_moved() {
        // 2 is fixed on Monday, so 1 waits even though it is longer.
        let (q, r) = P::new().task(1, 3 * DAY).task(2, DAY).fix(2, 5).person(1, 100).assign(1, 1, 100).assign(2, 1, 100).level();
        assert!(r.delays.contains_key(&tid(1)) && !r.delays.contains_key(&tid(2)));
        assert_eq!(starts(&q)[&tid(2)], at(5));
        assert_eq!(overloads(&q), 0);
    }

    #[test]
    fn what_cannot_be_fixed_is_reported() {
        let (_, r) = P::new().task(1, DAY).person(1, 100).assign(1, 1, 200).level();
        assert_eq!(r.unresolved, vec![Unresolved::TooMuchOnOneTask { resource: rid(1), task: tid(1), units: 200, max_units: 100 }]);
        let (_, r) = P::new().task(1, DAY).task(2, DAY).fix(1, 5).fix(2, 5).person(1, 100).assign(1, 1, 100).assign(2, 1, 100).level();
        assert_eq!(r.unresolved, vec![Unresolved::FixedDates { resource: rid(1), tasks: vec![tid(1), tid(2)] }]);
        assert!(r.delays.is_empty());
    }

    #[test]
    fn successors_follow_a_delayed_task() {
        // 2 waits for 1; 3 follows 2 by a link.
        let (q, _) = P::new().task(1, 2 * DAY).task(2, DAY).task(3, DAY).fs(2, 3).person(1, 100).assign(1, 1, 100).assign(2, 1, 100).level();
        let s = starts(&q);
        assert_eq!(s[&tid(2)], at(7));
        assert_eq!(s[&tid(3)], at(8));
    }

    #[test]
    fn leveling_starts_from_scratch_each_time() {
        let mut p = P::new().task(1, DAY).task(2, DAY).person(1, 100).assign(1, 1, 100);
        p.0.tasks.get_mut(&tid(2)).unwrap().leveling_delay = Duration::minutes(5 * DAY).unwrap();
        let (_, r) = p.level();
        assert!(r.delays.is_empty(), "an old delay that is no longer needed goes away");
    }

    fn staffed_projects() -> impl Strategy<Value = Project> {
        let tasks = prop::collection::vec(1i64..4 * DAY, 2..14);
        let links = prop::collection::vec((0usize..14, 0usize..14), 0..8);
        let staff = prop::collection::vec((0usize..14, 0u128..3, 20u32..100), 1..20);
        let fixed = prop::collection::vec(0usize..14, 0..2);
        (tasks, links, staff, fixed).prop_map(|(tasks, links, staff, fixed)| {
            let mut p = P::new().person(0, 100).person(1, 100).person(2, 200);
            for (i, d) in tasks.iter().enumerate() {
                p = p.task(i as u128, *d);
            }
            let mut linked = BTreeSet::new();
            for (a, b) in links {
                let (a, b) = (a % tasks.len(), b % tasks.len());
                if a < b && linked.insert((a, b)) {
                    p = p.fs(a as u128, b as u128);
                }
            }
            for f in fixed {
                p = p.fix((f % tasks.len()) as u128, 5);
            }
            let mut seen = BTreeSet::new();
            for (t, r, u) in staff {
                let t = t % tasks.len();
                if seen.insert((t, r)) {
                    p = p.assign(t as u128, r, u);
                }
            }
            p.0
        })
    }

    proptest! {
        /// After leveling, the only overloads left are the ones it reported, links still
        /// hold, and leveling the leveled project again changes nothing.
        #[test]
        fn leveling_clears_what_it_can_and_reports_the_rest(p in staffed_projects()) {
            let r = level(&p).unwrap();
            let mut q = p.clone();
            for (t, d) in &r.delays {
                q.tasks.get_mut(t).unwrap().leveling_delay = Duration::minutes(*d).unwrap();
            }
            let s = schedule(&q).unwrap();
            let l = loads(&q, &s);
            let left: usize = l.resources.values().map(|x| x.overloads.len()).sum();
            if r.unresolved.is_empty() {
                prop_assert_eq!(left, 0, "{:?}", l);
            }
            // "Must start on" overrides links by design (reported as a conflict), so only
            // links into other tasks must hold.
            for d in q.dependencies.values().filter(|d| !q.tasks[&d.successor].constraint.is_fixed()) {
                prop_assert!(s.tasks[&d.successor].early_start >= s.tasks[&d.predecessor].early_finish);
            }
            prop_assert_eq!(level(&q).unwrap(), r, "leveling is repeatable");
        }
    }
}
