//! One project at a glance, for the Portfolio (iteration 4): dates, progress, cost, and a
//! status with its reasons in plain words.
//!
//! Status rules (shown to the user, so kept simple):
//! - **Done**: every work task is 100% complete.
//! - **Late**: it now finishes after its baseline finish, or tasks that should have finished
//!   by today are not complete.
//! - **At risk**: someone is over-allocated, or tasks that should have started by today have
//!   not started.
//! - **On track**: none of the above.

use crate::model::{Project, TaskId};
use crate::resources::Loads;
use crate::schedule::Schedule;
use crate::time::Moment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    OnTrack,
    AtRisk,
    Late,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub start: Moment,
    pub finish: Moment,
    pub baseline_finish: Option<Moment>,
    /// Working minutes later (+) or earlier (−) than the baseline finish.
    pub finish_variance: Option<i64>,
    /// Work tasks' % complete, weighted by duration (0–100).
    pub percent_complete: u8,
    pub cost: i64,
    pub baseline_cost: Option<i64>,
    /// People asked for more than they have at some point.
    pub overallocated: u32,
    /// Work tasks due by `today` that are not complete.
    pub overdue: u32,
    /// The next milestone not reached yet.
    pub next_milestone: Option<(TaskId, String, Moment)>,
    pub status: Status,
    /// Why, in plain words; empty when on track.
    pub reasons: Vec<String>,
}

pub fn summarize(p: &Project, s: &Schedule, l: &Loads, today: Moment) -> Summary {
    let parents: std::collections::BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
    let leaves: Vec<_> = p.tasks.values().filter(|t| !parents.contains(&t.id)).collect();
    let start = s.tasks.values().map(|t| t.early_start).min().unwrap_or(p.start);
    let finish = s.project_finish;

    let (mut done_weight, mut weight) = (0i64, 0i64);
    for t in &leaves {
        // Milestones take no time, so they do not count.
        let w = if t.is_milestone { 0 } else { t.duration.get() };
        weight += w;
        done_weight += w * i64::from(t.percent_complete.get());
    }
    let percent_complete = if weight == 0 { 0 } else { (done_weight / weight).clamp(0, 100) as u8 };

    let baseline_finish = p.baseline.as_ref().and_then(|b| b.tasks.values().map(|t| t.finish).max());
    let finish_variance = baseline_finish.map(|b| if finish >= b { p.calendar.working_between(b, finish) } else { -p.calendar.working_between(finish, b) });
    let baseline_cost = p.baseline.as_ref().map(|b| b.tasks.iter().filter(|(id, _)| !parents.contains(id)).map(|(_, t)| t.cost.0).sum::<i64>());
    let overallocated = l.resources.values().filter(|r| !r.overloads.is_empty()).count() as u32;

    let open = |t: &&&crate::model::Task| t.percent_complete.get() < 100;
    let overdue = leaves.iter().filter(open).filter(|t| s.tasks.get(&t.id).is_some_and(|ts| ts.early_finish <= today)).count() as u32;
    let not_started = leaves
        .iter()
        .filter(|t| !t.is_milestone && t.percent_complete.get() == 0 && t.actual.start().is_none())
        .filter(|t| s.tasks.get(&t.id).is_some_and(|ts| ts.early_start < today && ts.early_finish > today))
        .count();
    let next_milestone = leaves
        .iter()
        .filter(|t| t.is_milestone && t.percent_complete.get() < 100)
        .filter_map(|t| s.tasks.get(&t.id).map(|ts| (t.id, t.name.clone(), ts.early_start)))
        .filter(|(_, _, at)| *at >= today)
        .min_by_key(|(id, _, at)| (*at, *id));

    let mut reasons = Vec::new();
    let all_done = !leaves.is_empty() && leaves.iter().all(|t| t.percent_complete.get() == 100);
    let status = if all_done {
        Status::Done
    } else {
        if let Some(v) = finish_variance.filter(|v| *v > 0) {
            reasons.push(format!("finishes {} later than its baseline", days(v)));
        }
        if overdue > 0 {
            reasons.push(format!("{overdue} task{} should have finished by now", if overdue == 1 { "" } else { "s" }));
        }
        if !reasons.is_empty() {
            Status::Late
        } else {
            if overallocated > 0 {
                reasons.push(format!("{overallocated} {} over-allocated", if overallocated == 1 { "person is" } else { "people are" }));
            }
            if not_started > 0 {
                reasons.push(format!("{not_started} task{} should have started by now", if not_started == 1 { "" } else { "s" }));
            }
            if reasons.is_empty() { Status::OnTrack } else { Status::AtRisk }
        }
    };
    Summary {
        start,
        finish,
        baseline_finish,
        finish_variance,
        percent_complete,
        cost: l.project_cost,
        baseline_cost,
        overallocated,
        overdue,
        next_milestone,
        status,
        reasons,
    }
}

/// "3 working days", "1 working day", "4 working hours".
fn days(minutes: i64) -> String {
    if minutes >= 480 {
        let d = (minutes as f64 / 480.0 * 10.0).round() / 10.0;
        format!("{d} working day{}", if d == 1.0 { "" } else { "s" })
    } else {
        let h = (minutes as f64 / 60.0).round();
        format!("{h} working hour{}", if h == 1.0 { "" } else { "s" })
    }
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

    fn task(n: u128, days: i64, milestone: bool) -> Task {
        Task {
            id: TaskId(Uuid::from_u128(n)),
            parent: None,
            sort_key: SortKey(format!("{n:04}")),
            name: format!("T{n}"),
            duration: Duration::minutes(days * 480).unwrap(),
            is_milestone: milestone,
            constraint: Constraint::AsSoonAsPossible,
            percent_complete: Percent::default(),
            leveling_delay: Duration::ZERO,
            fixed_cost: Money(100),
            manual_start: None,
            actual: Actual::default(),
            notes: String::new(),
        }
    }

    /// 1 (2 days) -> 2 (2 days) -> 3 (milestone), starting Monday 5 October.
    fn project() -> Project {
        let mut p = Project {
            id: ProjectId(Uuid::from_u128(0)),
            name: "P".into(),
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
        let tasks = vec![task(1, 2, false), task(2, 2, false), task(3, 0, true)];
        let link = |n: u128, a: u128, b: u128| Dependency { id: DependencyId(Uuid::from_u128(n)), predecessor: TaskId(Uuid::from_u128(a)), successor: TaskId(Uuid::from_u128(b)), kind: LinkType::FinishToStart, lag: 0 };
        apply(&mut p, &Command::InsertTasks { tasks, dependencies: vec![link(10, 1, 2), link(11, 2, 3)] }).unwrap();
        p
    }

    fn sum(p: &Project, today: Moment) -> Summary {
        let s = schedule(p).unwrap();
        summarize(p, &s, &loads(p, &s), today)
    }

    #[test]
    fn a_project_on_plan_is_on_track_and_one_that_slipped_is_late() {
        let mut p = project();
        let s = schedule(&p).unwrap();
        p.baseline = Some(capture_baseline(&p, &s, &loads(&p, &s), "t"));
        // The Friday before it starts: nothing is due yet.
        assert_eq!(sum(&p, at(2, 9)).status, Status::OnTrack);
        // Monday noon and task 1 has not started: at risk.
        let idle = sum(&p, at(5, 12));
        assert_eq!((idle.status, idle.reasons.as_slice()), (Status::AtRisk, ["1 task should have started by now".to_owned()].as_slice()));
        let mut started = p.clone();
        started.tasks.get_mut(&TaskId(Uuid::from_u128(1))).unwrap().percent_complete = Percent::new(10).unwrap();
        let s1 = sum(&started, at(5, 10));
        assert_eq!((s1.status, s1.percent_complete, s1.cost), (Status::OnTrack, 5, 300));
        assert_eq!(s1.next_milestone.as_ref().map(|m| m.1.as_str()), Some("T3"));

        // Task 1 takes 4 days now: two days late against the baseline.
        started.tasks.get_mut(&TaskId(Uuid::from_u128(1))).unwrap().duration = Duration::minutes(4 * 480).unwrap();
        let late = sum(&started, at(5, 10));
        assert_eq!(late.status, Status::Late);
        assert_eq!(late.finish_variance, Some(960));
        assert_eq!(late.reasons, ["finishes 2 working days later than its baseline"]);

        // Wednesday evening with task 1 (due Tuesday) unfinished: overdue.
        let overdue = sum(&p, at(7, 18));
        assert_eq!((overdue.status, overdue.overdue), (Status::Late, 1));
    }

    #[test]
    fn a_finished_project_is_done() {
        let mut p = project();
        for t in p.tasks.values_mut() {
            t.percent_complete = Percent::new(100).unwrap();
        }
        let s = sum(&p, at(30, 9));
        assert_eq!((s.status, s.percent_complete), (Status::Done, 100));
        assert!(s.reasons.is_empty());
    }
}
