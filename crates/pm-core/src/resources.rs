//! The resource phase (spec 4.10): from scheduled dates, each resource's load over time, its
//! peak, the periods where it is over-allocated, and the work on each assignment.
//!
//! Durations drive dates ("fixed duration", see `docs/iteration-2-plan.md`): an assignment's
//! work is the task's working time times its units, and assigning people never moves a task.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{AssignmentId, Project, ResourceId, ResourceKind, ResourceUnits, TaskId};
use crate::schedule::Schedule;
use crate::time::Moment;

/// A stretch of time in which a resource is asked for more than it has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overload {
    pub start: Moment,
    pub finish: Moment,
    /// The most asked for during the stretch.
    pub peak: ResourceUnits,
    /// Tasks the resource works on during the stretch, in id order.
    pub tasks: Vec<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLoad {
    /// Most asked of the resource at any moment (work resources).
    pub peak: ResourceUnits,
    /// Total work in working minutes (work resources), or total quantity in hundredths
    /// (materials).
    pub total: i64,
    pub overloads: Vec<Overload>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loads {
    pub resources: BTreeMap<ResourceId, ResourceLoad>,
    /// Work per assignment in working minutes (0 for materials and milestones).
    pub work: BTreeMap<AssignmentId, i64>,
    /// Tasks during which one of their resources is over-allocated.
    pub overloaded_tasks: BTreeSet<TaskId>,
}

/// Works out resource loads for a scheduled project. Pure and deterministic.
pub fn loads(p: &Project, s: &Schedule) -> Loads {
    let mut out = Loads::default();
    for (id, r) in &p.resources {
        let mine: Vec<_> = p.assignments.values().filter(|a| a.resource == *id).collect();
        if r.kind == ResourceKind::Material {
            let total = mine.iter().map(|a| i64::from(a.units.0)).sum();
            for a in &mine {
                out.work.insert(a.id, 0);
            }
            out.resources.insert(*id, ResourceLoad { peak: ResourceUnits(0), total, overloads: vec![] });
            continue;
        }
        // Each assignment adds its units while its task runs. At equal instants, finishes
        // come before starts, so back-to-back tasks never overlap.
        let mut events: Vec<(Moment, i8, u32, TaskId)> = Vec::new();
        let mut total = 0;
        for a in &mine {
            let Some(t) = s.tasks.get(&a.task) else { continue };
            let work = p.calendar.working_between(t.early_start, t.early_finish) * i64::from(a.units.0) / 100;
            out.work.insert(a.id, work);
            total += work;
            if t.early_finish > t.early_start {
                events.push((t.early_start, 1, a.units.0, a.task));
                events.push((t.early_finish, -1, a.units.0, a.task));
            }
        }
        events.sort_by_key(|&(at, delta, _, task)| (at, delta, task));

        let mut level: u32 = 0;
        let mut peak: u32 = 0;
        let mut running: BTreeSet<TaskId> = BTreeSet::new();
        let mut overloads: Vec<Overload> = Vec::new();
        let mut open: Option<Overload> = None;
        for (at, delta, units, task) in events {
            if delta < 0 {
                level -= units;
                running.remove(&task);
            } else {
                level += units;
                running.insert(task);
            }
            peak = peak.max(level);
            let over = level > r.max_units.0;
            match (&mut open, over) {
                (None, true) => {
                    open = Some(Overload { start: at, finish: at, peak: ResourceUnits(level), tasks: running.iter().copied().collect() })
                }
                (Some(o), true) => {
                    o.peak = o.peak.max(ResourceUnits(level));
                    for t in &running {
                        if !o.tasks.contains(t) {
                            o.tasks.push(*t);
                        }
                    }
                }
                (Some(_), false) => {
                    let mut o = open.take().expect("open");
                    o.finish = at;
                    if o.finish > o.start {
                        o.tasks.sort();
                        overloads.push(o);
                    }
                }
                (None, false) => {}
            }
        }
        for o in &overloads {
            out.overloaded_tasks.extend(o.tasks.iter().copied());
        }
        out.resources.insert(*id, ResourceLoad { peak: ResourceUnits(peak), total, overloads });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Calendar;
    use crate::command::{apply, Command};
    use crate::model::*;
    use crate::schedule::schedule;
    use jiff::civil::date;
    use proptest::prelude::*;
    use uuid::Uuid;

    const DAY: i64 = 480;

    fn at(day: i8, h: i8) -> Moment {
        Moment::from_datetime(date(2026, 10, day).at(h, 0, 0, 0))
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
                    start: at(5, 8),
                    calendar_id: CalendarId(Uuid::from_u128(0)),
                    calendar: Calendar::standard(),
                    tasks: Default::default(),
                    dependencies: Default::default(),
                    resources: Default::default(),
                    assignments: Default::default(),
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
                leveling_delay: Duration::ZERO,
                notes: String::new(),
            };
            apply(&mut self.0, &Command::InsertTasks { tasks: vec![t], dependencies: vec![] }).unwrap();
            self
        }

        fn fs(mut self, a: u128, b: u128) -> Self {
            self.1 += 1;
            let d = Dependency { id: DependencyId(Uuid::from_u128(self.1)), predecessor: tid(a), successor: tid(b), kind: LinkType::FinishToStart, lag: 0 };
            apply(&mut self.0, &Command::InsertLink { dependency: d }).unwrap();
            self
        }

        fn resource(mut self, n: u128, kind: ResourceKind, max: u32) -> Self {
            let r = Resource {
                id: rid(n),
                sort_key: SortKey(format!("{n:04}")),
                name: format!("R{n}"),
                initials: String::new(),
                kind,
                max_units: ResourceUnits(max),
                material_label: String::new(),
                notes: String::new(),
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

        fn loads(&self) -> Loads {
            loads(&self.0, &schedule(&self.0).unwrap())
        }
    }

    use ResourceKind::*;

    #[test]
    fn two_full_time_tasks_at_once_overload_one_person() {
        // Both start Monday; 1 runs 3 days, 2 runs 1 day.
        let l = P::new().task(1, 3 * DAY).task(2, DAY).resource(1, Work, 100).assign(1, 1, 100).assign(2, 1, 100).loads();
        let r = &l.resources[&rid(1)];
        assert_eq!(r.peak, ResourceUnits(200));
        assert_eq!(r.overloads, vec![Overload { start: at(5, 8), finish: at(5, 17), peak: ResourceUnits(200), tasks: vec![tid(1), tid(2)] }]);
        assert_eq!(l.overloaded_tasks, [tid(1), tid(2)].into());
        assert_eq!(r.total, 4 * DAY);
    }

    #[test]
    fn half_time_twice_is_fine() {
        let l = P::new().task(1, DAY).task(2, DAY).resource(1, Work, 100).assign(1, 1, 50).assign(2, 1, 50).loads();
        assert!(l.resources[&rid(1)].overloads.is_empty());
        assert_eq!(l.resources[&rid(1)].peak, ResourceUnits(100));
        assert!(l.overloaded_tasks.is_empty());
    }

    #[test]
    fn back_to_back_tasks_do_not_overlap() {
        let l = P::new().task(1, DAY).task(2, DAY).fs(1, 2).resource(1, Work, 100).assign(1, 1, 100).assign(2, 1, 100).loads();
        assert!(l.resources[&rid(1)].overloads.is_empty());
    }

    #[test]
    fn a_team_can_take_more() {
        let l = P::new().task(1, DAY).task(2, DAY).task(3, DAY).resource(1, Work, 300).assign(1, 1, 100).assign(2, 1, 100).assign(3, 1, 100).loads();
        assert!(l.resources[&rid(1)].overloads.is_empty());
        let l = P::new().task(1, DAY).task(2, DAY).task(3, DAY).resource(1, Work, 200).assign(1, 1, 100).assign(2, 1, 100).assign(3, 1, 100).loads();
        assert_eq!(l.resources[&rid(1)].overloads.len(), 1);
    }

    #[test]
    fn work_is_duration_times_units() {
        let p = P::new().task(1, 2 * DAY).resource(1, Work, 100).assign(1, 1, 50);
        let l = p.loads();
        assert_eq!(l.work.values().copied().collect::<Vec<_>>(), vec![DAY]);
    }

    #[test]
    fn milestones_and_materials_never_overload() {
        let l = P::new().task(1, 0).task(2, DAY).resource(1, Work, 100).resource(2, Material, 0).assign(1, 1, 100).assign(2, 1, 100).assign(2, 2, 250).loads();
        assert!(l.resources[&rid(1)].overloads.is_empty());
        assert_eq!(l.resources[&rid(2)].total, 250, "2.5 of the material");
        assert!(l.overloaded_tasks.is_empty());
    }

    #[test]
    fn overload_spanning_a_weekend_is_one_stretch() {
        // 1: Thursday to the next Tuesday (4 days); 2: Friday to Monday (2 days).
        let mut p = P::new().task(1, 4 * DAY).task(2, 2 * DAY).resource(1, Work, 100).assign(1, 1, 100).assign(2, 1, 100);
        p.0.start = at(8, 8);
        let mut t2 = p.0.tasks[&tid(2)].clone();
        t2.constraint = Constraint::StartNoEarlierThan(at(9, 8));
        p.0.tasks.insert(tid(2), t2);
        let l = p.loads();
        let o = &l.resources[&rid(1)].overloads;
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].start, o[0].finish), (at(9, 8), at(12, 17)));
    }

    proptest! {
        /// Whatever the plan, a resource is over-allocated exactly when its peak exceeds its
        /// maximum, and every overload names tasks of that resource.
        #[test]
        fn overloads_agree_with_the_peak(durations in prop::collection::vec(0i64..3 * DAY, 1..8), units in prop::collection::vec(10u32..150, 1..8), max in 50u32..300) {
            let mut p = P::new().resource(1, Work, max);
            for (i, d) in durations.iter().enumerate() {
                p = p.task(i as u128 + 1, *d);
            }
            for (i, u) in units.iter().enumerate().take(durations.len()) {
                p = p.assign(i as u128 + 1, 1, *u);
            }
            let l = p.loads();
            let r = &l.resources[&rid(1)];
            prop_assert_eq!(r.overloads.is_empty(), r.peak.0 <= max);
            for o in &r.overloads {
                prop_assert!(o.peak.0 > max && o.finish > o.start);
                for t in &o.tasks {
                    prop_assert!(p.0.assignments.values().any(|a| a.task == *t && a.resource == rid(1)));
                }
            }
        }
    }
}
