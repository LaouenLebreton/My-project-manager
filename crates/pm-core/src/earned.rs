//! Earned value (spec iteration 2, step 14): how much work was planned, done and paid for by
//! the status date, in the baseline's money.
//!
//! As in Microsoft Project with "actual costs are always calculated":
//! - BCWS (planned value): the baseline cost, spread evenly over the baseline's working time,
//!   up to the status date.
//! - BCWP (earned value): the baseline cost × % complete.
//! - ACWP (actual cost): today's cost × % complete.
//!
//! Work tasks are measured; summaries add up the tasks inside them.

use std::collections::BTreeMap;

use crate::model::{Project, TaskId};
use crate::resources::Loads;
use crate::time::Moment;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ev {
    /// Budgeted cost of work scheduled (planned value), in cents.
    pub bcws: i64,
    /// Budgeted cost of work performed (earned value), in cents.
    pub bcwp: i64,
    /// Actual cost of work performed, in cents.
    pub acwp: i64,
}

impl Ev {
    /// Schedule variance: positive means ahead of plan.
    pub fn sv(self) -> i64 {
        self.bcwp - self.bcws
    }
    /// Cost variance: positive means under budget.
    pub fn cv(self) -> i64 {
        self.bcwp - self.acwp
    }
    /// Schedule performance index; None before anything was planned.
    pub fn spi(self) -> Option<f64> {
        (self.bcws != 0).then(|| self.bcwp as f64 / self.bcws as f64)
    }
    /// Cost performance index; None before anything was spent.
    pub fn cpi(self) -> Option<f64> {
        (self.acwp != 0).then(|| self.bcwp as f64 / self.acwp as f64)
    }

    fn add(&mut self, o: Ev) {
        self.bcws += o.bcws;
        self.bcwp += o.bcwp;
        self.acwp += o.acwp;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EarnedValue {
    pub as_of: Moment,
    /// Every task, summaries included.
    pub tasks: BTreeMap<TaskId, Ev>,
    pub project: Ev,
}

fn share(total: i64, part: i64, whole: i64) -> i64 {
    let n = i128::from(total) * i128::from(part);
    let d = i128::from(whole);
    ((n + n.signum() * d / 2) / d) as i64
}

/// Earned value as of `as_of` (the status date, or today). None without a baseline.
pub fn earned_value(p: &Project, l: &Loads, as_of: Moment) -> Option<EarnedValue> {
    let baseline = p.baseline.as_ref()?;
    let has_children: std::collections::BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
    let mut out = EarnedValue { as_of, tasks: BTreeMap::new(), project: Ev::default() };
    for t in p.tasks.values().filter(|t| !has_children.contains(&t.id)) {
        let pc = i64::from(t.percent_complete.get());
        let mut ev = Ev { acwp: share(l.task_cost.get(&t.id).copied().unwrap_or(0), pc, 100), ..Ev::default() };
        if let Some(b) = baseline.tasks.get(&t.id) {
            ev.bcwp = share(b.cost.0, pc, 100);
            let span = p.calendar.working_between(b.start, b.finish);
            ev.bcws = if as_of >= b.finish || (span == 0 && as_of >= b.start) {
                b.cost.0
            } else if as_of <= b.start {
                0
            } else {
                share(b.cost.0, p.calendar.working_between(b.start, as_of), span)
            };
        }
        // Add the task to itself and every summary above it.
        let mut at = Some(t.id);
        while let Some(id) = at {
            out.tasks.entry(id).or_default().add(ev);
            at = p.tasks.get(&id).and_then(|t| t.parent);
        }
        out.project.add(ev);
    }
    for id in p.tasks.keys() {
        out.tasks.entry(*id).or_default();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Calendar;
    use crate::command::{apply, Command};
    use crate::model::*;
    use crate::resources::{capture_baseline, loads};
    use crate::schedule::schedule;
    use jiff::civil::date;
    use uuid::Uuid;

    fn at(day: i8, h: i8) -> Moment {
        Moment::from_datetime(date(2026, 10, day).at(h, 0, 0, 0))
    }

    fn tid(n: u128) -> TaskId {
        TaskId(Uuid::from_u128(n))
    }

    fn task(n: u128, parent: Option<u128>, days: i64, cost: i64) -> Task {
        Task {
            id: tid(n),
            parent: parent.map(tid),
            sort_key: SortKey(format!("{n:04}")),
            name: format!("T{n}"),
            duration: Duration::minutes(days * 480).unwrap(),
            is_milestone: false,
            constraint: Constraint::AsSoonAsPossible,
            percent_complete: Percent::default(),
            leveling_delay: Duration::ZERO,
            fixed_cost: Money(cost),
            manual_start: None,
            actual: Actual::default(),
            notes: String::new(),
        }
    }

    #[test]
    fn planned_earned_and_actual_by_the_status_date() {
        let mut p = Project {
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
        };
        // Phase 1 holds 2 (4 days, 1000) then 3 (4 days, 2000).
        let tasks = vec![task(1, None, 1, 0), task(2, Some(1), 4, 1000), task(3, Some(1), 4, 2000)];
        let link = Dependency { id: DependencyId(Uuid::from_u128(9)), predecessor: tid(2), successor: tid(3), kind: LinkType::FinishToStart, lag: 0 };
        apply(&mut p, &Command::InsertTasks { tasks, dependencies: vec![link] }).unwrap();
        let s = schedule(&p).unwrap();
        p.baseline = Some(capture_baseline(&p, &s, &loads(&p, &s), "t"));
        assert!(earned_value(&p, &loads(&p, &s), at(7, 17)).unwrap().project == Ev { bcws: 750, bcwp: 0, acwp: 0 });

        // Half done after three days, and it now costs 1200.
        let t2 = p.tasks.get_mut(&tid(2)).unwrap();
        t2.percent_complete = Percent::new(50).unwrap();
        t2.fixed_cost = Money(1200);
        let s = schedule(&p).unwrap();
        let ev = earned_value(&p, &loads(&p, &s), at(7, 17)).unwrap();
        assert_eq!(ev.tasks[&tid(2)], Ev { bcws: 750, bcwp: 500, acwp: 600 });
        assert_eq!(ev.tasks[&tid(3)], Ev::default());
        assert_eq!(ev.tasks[&tid(1)], ev.tasks[&tid(2)], "the summary adds up its tasks");
        assert_eq!((ev.project.sv(), ev.project.cv()), (-250, -100));
        assert!((ev.project.spi().unwrap() - 2.0 / 3.0).abs() < 1e-9);
        assert!((ev.project.cpi().unwrap() - 5.0 / 6.0).abs() < 1e-9);

        // After the baseline finish, everything was planned.
        assert_eq!(earned_value(&p, &loads(&p, &s), at(30, 17)).unwrap().project.bcws, 3000);
        p.baseline = None;
        assert!(earned_value(&p, &loads(&p, &s), at(7, 17)).is_none());
    }
}
