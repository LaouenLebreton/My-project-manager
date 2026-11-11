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
    /// Costs in cents: per assignment (work × rate, or quantity × price), per task (fixed cost
    /// plus its assignments; summaries add up their tasks), per resource, and in total.
    pub assignment_cost: BTreeMap<AssignmentId, i64>,
    pub task_cost: BTreeMap<TaskId, i64>,
    pub resource_cost: BTreeMap<ResourceId, i64>,
    pub project_cost: i64,
}

/// `a × b / c`, rounded half away from zero, without overflow.
fn mul_div(a: i64, b: i64, c: i64) -> i64 {
    let n = i128::from(a) * i128::from(b);
    let c = i128::from(c);
    ((n + n.signum() * c / 2) / c) as i64
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
    costs(p, &mut out);
    out
}

/// Today's plan as a baseline: dates from the schedule, work and cost from the loads.
/// Work an assignment asks for on each day it runs, in working minutes (Workload view).
/// Materials and milestones ask for no time.
pub fn assignment_days(p: &Project, s: &Schedule, a: &crate::model::Assignment) -> BTreeMap<i64, i64> {
    let mut days = BTreeMap::new();
    let (Some(r), Some(t)) = (p.resources.get(&a.resource), s.tasks.get(&a.task)) else { return days };
    if r.kind == ResourceKind::Material {
        return days;
    }
    for day in t.early_start.day()..=t.early_finish.day() {
        let from = t.early_start.max(Moment::from_day_minute(day, 0));
        let to = t.early_finish.min(Moment::from_day_minute(day + 1, 0));
        let minutes = mul_div(p.calendar.working_between(from, to), i64::from(a.units.0), 100);
        if minutes > 0 {
            days.insert(day, minutes);
        }
    }
    days
}

/// What a work resource can give on a day: its working time × maximum units, in minutes.
pub fn capacity(p: &Project, resource: ResourceId, day: i64) -> i64 {
    p.resources.get(&resource).map_or(0, |r| mul_div(p.calendar.pattern(day).working_minutes(), i64::from(r.max_units.0), 100))
}

pub fn capture_baseline(p: &Project, s: &Schedule, l: &Loads, saved_at: &str) -> crate::model::Baseline {
    let tasks = s
        .tasks
        .iter()
        .map(|(id, t)| {
            let work = p.assignments.values().filter(|a| a.task == *id).filter_map(|a| l.work.get(&a.id)).sum();
            let entry = crate::model::BaselineTask {
                start: t.early_start,
                finish: t.early_finish,
                duration: p.calendar.working_between(t.early_start, t.early_finish),
                work,
                cost: crate::model::Money(l.task_cost.get(id).copied().unwrap_or(0)),
            };
            (*id, entry)
        })
        .collect();
    crate::model::Baseline { saved_at: saved_at.to_owned(), tasks }
}

fn costs(p: &Project, out: &mut Loads) {
    for a in p.assignments.values() {
        let Some(r) = p.resources.get(&a.resource) else { continue };
        let cost = match r.kind {
            ResourceKind::Work => mul_div(out.work.get(&a.id).copied().unwrap_or(0), r.rate.0, 60),
            ResourceKind::Material => mul_div(i64::from(a.units.0), r.rate.0, 100),
        };
        out.assignment_cost.insert(a.id, cost);
        *out.resource_cost.entry(a.resource).or_default() += cost;
        *out.task_cost.entry(a.task).or_default() += cost;
    }
    // Fixed costs, then roll up from the deepest tasks so every summary includes everything
    // inside it.
    let depth = |mut id: TaskId| {
        let mut d = 0;
        while let Some(parent) = p.tasks.get(&id).and_then(|t| t.parent) {
            d += 1;
            id = parent;
            if d > p.tasks.len() {
                break;
            }
        }
        d
    };
    let mut order: Vec<(usize, TaskId)> = p.tasks.keys().map(|id| (depth(*id), *id)).collect();
    order.sort_by(|a, b| b.cmp(a));
    for (_, id) in order {
        let t = &p.tasks[&id];
        let own = out.task_cost.get(&id).copied().unwrap_or(0) + t.fixed_cost.0;
        out.task_cost.insert(id, own);
        match t.parent {
            Some(parent) => *out.task_cost.entry(parent).or_default() += own,
            None => out.project_cost += own,
        }
    }
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
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                deadline: None,
                actual: Actual::default(),
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
                rate: Money(0),
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
    fn workload_by_day() {
        // 1 runs Monday–Wednesday at 100%, 2 Monday afternoon (half a day) at 50%.
        let p = P::new().task(1, 3 * DAY).task(2, DAY / 2).resource(1, Work, 100).assign(1, 1, 100).assign(2, 1, 50);
        let s = schedule(&p.0).unwrap();
        let days = |n: u128| {
            let a = p.0.assignments.values().find(|a| a.task == tid(n)).unwrap();
            assignment_days(&p.0, &s, a).into_iter().collect::<Vec<_>>()
        };
        let mon = at(5, 8).day();
        assert_eq!(days(1), [(mon, 480), (mon + 1, 480), (mon + 2, 480)]);
        assert_eq!(days(2), [(mon, 120)]);
        assert_eq!(capacity(&p.0, rid(1), mon), 480);
        assert_eq!(capacity(&p.0, rid(1), mon + 5), 0, "Saturday");
    }

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

    #[test]
    fn costs_add_up_from_assignments_to_the_project() {
        // Alex: €45.50 an hour; concrete: €120 a ton. Task 1: 2 days at 50% = 8h → €364.
        // Task 2: 2.5 tons → €300, plus a €99.99 fixed cost.
        let mut p = P::new().task(1, 2 * DAY).task(2, DAY).resource(1, Work, 100).resource(2, Material, 0).assign(1, 1, 50).assign(2, 2, 250);
        p.0.resources.get_mut(&rid(1)).unwrap().rate = Money(4_550);
        p.0.resources.get_mut(&rid(2)).unwrap().rate = Money(12_000);
        p.0.tasks.get_mut(&tid(2)).unwrap().fixed_cost = Money(9_999);
        let l = p.loads();
        assert_eq!(l.task_cost[&tid(1)], 36_400);
        assert_eq!(l.task_cost[&tid(2)], 30_000 + 9_999);
        assert_eq!(l.resource_cost[&rid(1)], 36_400);
        assert_eq!(l.project_cost, 36_400 + 39_999);
    }

    #[test]
    fn summaries_add_up_their_tasks_and_their_own_fixed_cost() {
        let mut p = P::new().task(10, DAY).task(1, DAY).task(2, DAY);
        for c in [1, 2] {
            p.0.tasks.get_mut(&tid(c)).unwrap().parent = Some(tid(10));
            p.0.tasks.get_mut(&tid(c)).unwrap().fixed_cost = Money(100);
        }
        p.0.tasks.get_mut(&tid(10)).unwrap().fixed_cost = Money(5);
        let l = p.loads();
        assert_eq!(l.task_cost[&tid(10)], 205);
        assert_eq!(l.project_cost, 205);
    }

    #[test]
    fn a_baseline_records_dates_work_and_cost() {
        let mut p = P::new().task(1, 2 * DAY).resource(1, Work, 100).assign(1, 1, 50);
        p.0.resources.get_mut(&rid(1)).unwrap().rate = Money(6_000);
        let s = schedule(&p.0).unwrap();
        let b = capture_baseline(&p.0, &s, &loads(&p.0, &s), "2026-10-09T10:00:00Z");
        let t = b.tasks[&tid(1)];
        assert_eq!((t.start, t.finish, t.duration, t.work, t.cost), (at(5, 8), at(6, 17), 2 * DAY, DAY, Money(48_000)));
        assert_eq!(b.saved_at, "2026-10-09T10:00:00Z");
    }

    #[test]
    fn cost_rounding_is_to_the_nearest_cent() {
        assert_eq!(mul_div(1, 1, 60), 0);
        assert_eq!(mul_div(30, 1, 60), 1, "half a cent rounds up");
        assert_eq!(mul_div(-30, 1, 60), -1);
        assert_eq!(mul_div(i64::MAX / 2, 4, 4), i64::MAX / 2, "no overflow");
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
