//! The scheduling engine: `schedule(project) -> Schedule` (spec 4.5, rules R10 to R29).
//!
//! Only leaf tasks are scheduled; summary tasks roll up from their children. Time-valued
//! results are start-type for starts and finish-type for finishes (see [`crate::calendar`]).

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::model::{Constraint, Dependency, DependencyId, LinkType, Project, Task, TaskId};
use crate::time::Moment;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSchedule {
    pub early_start: Moment,
    pub early_finish: Moment,
    pub late_start: Moment,
    pub late_finish: Moment,
    /// Working minutes; negative when a constraint conflicts with links (R23).
    pub total_slack: i64,
    pub critical: bool,
}

/// A Must Start On date earlier than the links allow (R17). The task is scheduled on the
/// constraint date anyway, and the conflict is reported so the UI can show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintConflict {
    pub task: TaskId,
    pub constraint_start: Moment,
    pub link_driven_start: Moment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub tasks: BTreeMap<TaskId, TaskSchedule>,
    pub project_finish: Moment,
    pub conflicts: Vec<ConstraintConflict>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ScheduleError {
    /// Task ids around one cycle, in link order, starting at the smallest id (R27).
    #[error("these tasks depend on each other in a loop, so none of them can start first")]
    Cycle { tasks: Vec<TaskId>, links: Vec<DependencyId> },
    #[error("a link points to a task that does not exist")]
    MissingTask { link: DependencyId, task: TaskId },
    #[error("links to or from summary tasks are not supported yet; link the tasks inside it instead")]
    LinkOnSummary { link: DependencyId, task: TaskId },
    #[error("a task cannot be its own predecessor")]
    SelfLink { link: DependencyId },
    #[error("a task is inside itself in the outline")]
    OutlineLoop { task: TaskId },
    #[error("a task's parent does not exist")]
    MissingParent { task: TaskId, parent: TaskId },
}

/// Schedules every task. Pure and deterministic (R29): same input, same output.
pub fn schedule(project: &Project) -> Result<Schedule, ScheduleError> {
    let g = Graph::build(project)?;
    let order = g.topological_order()?;
    let cal = &project.calendar;
    let n = g.tasks.len();
    let start = cal.next_working(project.start);
    let snap = |m: Moment| cal.next_working(m);

    // Forward pass (R10 to R18).
    let mut early = vec![(start, start); n];
    let mut conflicts = Vec::new();
    for &i in &order {
        let task = g.tasks[i];
        let dur = g.duration(i);
        let mut es = start;
        for &(p, link) in &g.preds[i] {
            let (pes, pef) = early[p];
            let candidate = match link.kind {
                LinkType::FinishToStart => cal.add_working(pef, link.lag),
                LinkType::StartToStart => cal.add_working(pes, link.lag),
                LinkType::FinishToFinish => cal.add_working(cal.add_working(pef, link.lag), -dur),
                LinkType::StartToFinish => cal.add_working(cal.add_working(pes, link.lag), -dur),
            };
            es = es.max(candidate);
        }
        // Milestones keep a finish-type instant such as 17:00 (R14); tasks with work
        // start at the next working minute (R5).
        if dur > 0 {
            es = snap(es);
        }
        match task.constraint {
            Constraint::AsSoonAsPossible => {}
            Constraint::StartNoEarlierThan(d) => es = es.max(snap(d)),
            Constraint::MustStartOn(d) => {
                let target = snap(d);
                if es > target {
                    conflicts.push(ConstraintConflict { task: task.id, constraint_start: target, link_driven_start: es });
                }
                es = target;
            }
        }
        early[i] = (es, cal.add_working(es, dur));
    }

    let project_finish = order.iter().map(|&i| early[i].1).max().unwrap_or(start);

    // Backward pass (R19 to R21): the mirror image of the forward pass.
    let mut late = vec![(project_finish, project_finish); n];
    for &i in order.iter().rev() {
        let dur = g.duration(i);
        let mut lf = project_finish;
        for &(s, link) in &g.succs[i] {
            let (sls, slf) = late[s];
            let candidate = match link.kind {
                LinkType::FinishToStart => cal.add_working(sls, -link.lag),
                LinkType::StartToStart => cal.add_working(cal.add_working(sls, -link.lag), dur),
                LinkType::FinishToFinish => cal.add_working(slf, -link.lag),
                LinkType::StartToFinish => cal.add_working(cal.add_working(slf, -link.lag), dur),
            };
            lf = lf.min(candidate);
        }
        if dur > 0 {
            lf = cal.prev_working_end(lf);
        }
        if let Constraint::MustStartOn(d) = g.tasks[i].constraint {
            lf = cal.add_working(snap(d), dur);
        }
        late[i] = (cal.add_working(lf, -dur), lf);
    }

    let mut result = vec![None; n];
    for &i in &order {
        let ((es, ef), (ls, lf)) = (early[i], late[i]);
        let total_slack = cal.working_between(ef, lf);
        result[i] = Some(TaskSchedule {
            early_start: es,
            early_finish: ef,
            late_start: ls,
            late_finish: lf,
            total_slack,
            critical: total_slack <= 0,
        });
    }

    // Summary roll-up (R24), deepest summaries first so nested ones are ready.
    let mut summaries: Vec<usize> = (0..n).filter(|&i| !g.children[i].is_empty()).collect();
    summaries.sort_by_key(|&i| std::cmp::Reverse(g.depth[i]));
    for i in summaries {
        let kids: Vec<TaskSchedule> = g.children[i].iter().map(|&c| result[c].expect("child scheduled")).collect();
        let total_slack = kids.iter().map(|k| k.total_slack).min().expect("summary has children");
        result[i] = Some(TaskSchedule {
            early_start: kids.iter().map(|k| k.early_start).min().unwrap(),
            early_finish: kids.iter().map(|k| k.early_finish).max().unwrap(),
            late_start: kids.iter().map(|k| k.late_start).min().unwrap(),
            late_finish: kids.iter().map(|k| k.late_finish).max().unwrap(),
            total_slack,
            critical: total_slack <= 0,
        });
    }

    Ok(Schedule {
        tasks: g.tasks.iter().zip(result).map(|(t, r)| (t.id, r.expect("every task scheduled"))).collect(),
        project_finish,
        conflicts,
    })
}

/// Tasks indexed in id order, with validated outline and links.
struct Graph<'a> {
    tasks: Vec<&'a Task>,
    children: Vec<Vec<usize>>,
    depth: Vec<usize>,
    /// Per leaf task, incoming and outgoing links in dependency-id order.
    preds: Vec<Vec<(usize, &'a Dependency)>>,
    succs: Vec<Vec<(usize, &'a Dependency)>>,
}

impl<'a> Graph<'a> {
    fn build(project: &'a Project) -> Result<Self, ScheduleError> {
        let tasks: Vec<&Task> = project.tasks.values().collect();
        let n = tasks.len();
        let index: HashMap<TaskId, usize> = tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();

        let mut parent = vec![None; n];
        let mut children = vec![Vec::new(); n];
        for (i, t) in tasks.iter().enumerate() {
            if let Some(p) = t.parent {
                let &pi = index.get(&p).ok_or(ScheduleError::MissingParent { task: t.id, parent: p })?;
                parent[i] = Some(pi);
                children[pi].push(i);
            }
        }
        let mut depth = vec![0; n];
        for i in 0..n {
            let mut cur = i;
            while let Some(p) = parent[cur] {
                depth[i] += 1;
                if depth[i] > n {
                    return Err(ScheduleError::OutlineLoop { task: tasks[i].id });
                }
                cur = p;
            }
        }

        let mut preds = vec![Vec::new(); n];
        let mut succs = vec![Vec::new(); n];
        for d in project.dependencies.values() {
            if d.predecessor == d.successor {
                return Err(ScheduleError::SelfLink { link: d.id });
            }
            let lookup = |task: TaskId| -> Result<usize, ScheduleError> {
                let &i = index.get(&task).ok_or(ScheduleError::MissingTask { link: d.id, task })?;
                if children[i].is_empty() {
                    Ok(i)
                } else {
                    Err(ScheduleError::LinkOnSummary { link: d.id, task })
                }
            };
            let (p, s) = (lookup(d.predecessor)?, lookup(d.successor)?);
            preds[s].push((p, d));
            succs[p].push((s, d));
        }
        Ok(Graph { tasks, children, depth, preds, succs })
    }

    fn duration(&self, i: usize) -> i64 {
        let t = self.tasks[i];
        if t.is_milestone { 0 } else { t.duration.get() }
    }

    /// Leaf tasks in an order where every predecessor comes first (Kahn's algorithm).
    fn topological_order(&self) -> Result<Vec<usize>, ScheduleError> {
        let n = self.tasks.len();
        let leaves: Vec<usize> = (0..n).filter(|&i| self.children[i].is_empty()).collect();
        let mut indegree: Vec<usize> = self.preds.iter().map(Vec::len).collect();
        let mut queue: VecDeque<usize> = leaves.iter().copied().filter(|&i| indegree[i] == 0).collect();
        let mut order = Vec::with_capacity(leaves.len());
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &(s, _) in &self.succs[i] {
                indegree[s] -= 1;
                if indegree[s] == 0 {
                    queue.push_back(s);
                }
            }
        }
        if order.len() == leaves.len() {
            Ok(order)
        } else {
            Err(self.find_cycle(&indegree))
        }
    }

    /// Every task left with a nonzero in-degree after Kahn's algorithm has a predecessor
    /// that is also left, so walking predecessors from any of them must revisit a task.
    fn find_cycle(&self, indegree: &[usize]) -> ScheduleError {
        let remaining = |i: usize| indegree[i] > 0;
        let first = (0..self.tasks.len()).find(|&i| remaining(i)).expect("a cycle exists");
        let mut path = vec![first];
        let mut seen_at = HashMap::from([(first, 0)]);
        let cycle_start = loop {
            let cur = *path.last().unwrap();
            let &(p, _) = self.preds[cur].iter().find(|&&(p, _)| remaining(p)).expect("remaining predecessor");
            if let Some(&at) = seen_at.get(&p) {
                break at;
            }
            seen_at.insert(p, path.len());
            path.push(p);
        };
        // The walk went backwards along links; reverse it and start at the smallest id.
        let mut cycle: Vec<usize> = path[cycle_start..].iter().rev().copied().collect();
        let min_pos = cycle.iter().enumerate().min_by_key(|&(_, &i)| i).map(|(pos, _)| pos).unwrap();
        cycle.rotate_left(min_pos);
        let links = (0..cycle.len())
            .map(|k| {
                let (a, b) = (cycle[k], cycle[(k + 1) % cycle.len()]);
                self.succs[a].iter().find(|&&(s, _)| s == b).expect("link on cycle").1.id
            })
            .collect();
        ScheduleError::Cycle { tasks: cycle.iter().map(|&i| self.tasks[i].id).collect(), links }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{Calendar, DayPattern};
    use crate::model::*;
    use jiff::civil::date;
    use proptest::prelude::*;
    use uuid::Uuid;

    const HOUR: i64 = 60;
    const DAY: i64 = 8 * HOUR;

    /// 2026-10-05 is a Monday.
    fn at(day: i8, h: i8, m: i8) -> Moment {
        Moment::from_datetime(date(2026, 10, day).at(h, m, 0, 0))
    }

    struct P(Project);

    fn tid(n: u128) -> TaskId {
        TaskId(Uuid::from_u128(n))
    }

    impl P {
        fn new() -> Self {
            P(Project {
                id: ProjectId(Uuid::from_u128(0)),
                name: "test".into(),
                start: at(5, 8, 0),
                calendar_id: CalendarId(Uuid::from_u128(0)),
                calendar: Calendar::standard(),
                tasks: BTreeMap::new(),
                dependencies: BTreeMap::new(),
            })
        }

        fn task(mut self, n: u128, minutes: i64) -> Self {
            self.0.tasks.insert(
                tid(n),
                Task {
                    id: tid(n),
                    parent: None,
                    sort_key: SortKey(format!("{n:08}")),
                    name: format!("T{n}"),
                    duration: Duration::minutes(minutes).unwrap(),
                    is_milestone: false,
                    constraint: Constraint::AsSoonAsPossible,
                    percent_complete: Percent::default(),
                    notes: String::new(),
                },
            );
            self
        }

        fn milestone(mut self, n: u128) -> Self {
            self = self.task(n, 0);
            self.0.tasks.get_mut(&tid(n)).unwrap().is_milestone = true;
            self
        }

        fn child_of(mut self, n: u128, parent: u128) -> Self {
            self.0.tasks.get_mut(&tid(n)).unwrap().parent = Some(tid(parent));
            self
        }

        fn constraint(mut self, n: u128, c: Constraint) -> Self {
            self.0.tasks.get_mut(&tid(n)).unwrap().constraint = c;
            self
        }

        fn link(mut self, pred: u128, succ: u128, kind: LinkType, lag: i64) -> Self {
            let id = DependencyId(Uuid::from_u128(1_000_000 + self.0.dependencies.len() as u128));
            self.0.dependencies.insert(
                id,
                Dependency { id, predecessor: tid(pred), successor: tid(succ), kind, lag },
            );
            self
        }

        fn fs(self, pred: u128, succ: u128) -> Self {
            self.link(pred, succ, LinkType::FinishToStart, 0)
        }

        fn run(&self) -> Schedule {
            schedule(&self.0).expect("schedules")
        }
    }

    fn dates(s: &Schedule, n: u128) -> (Moment, Moment) {
        let t = s.tasks[&tid(n)];
        (t.early_start, t.early_finish)
    }

    use LinkType::*;

    #[test]
    fn empty_project_finishes_at_its_start() {
        let s = P::new().run();
        assert!(s.tasks.is_empty());
        assert_eq!(s.project_finish, at(5, 8, 0));
    }

    #[test]
    fn r10_unlinked_task_starts_at_project_start() {
        let s = P::new().task(1, 2 * DAY).run();
        assert_eq!(dates(&s, 1), (at(5, 8, 0), at(6, 17, 0)));
    }

    #[test]
    fn r10_project_start_on_weekend_snaps_to_monday() {
        let mut p = P::new().task(1, DAY);
        p.0.start = at(3, 0, 0);
        assert_eq!(dates(&p.run(), 1), (at(5, 8, 0), at(5, 17, 0)));
    }

    #[test]
    fn r11_finish_to_start_next_morning() {
        let s = P::new().task(1, DAY).task(2, DAY).fs(1, 2).run();
        assert_eq!(dates(&s, 2), (at(6, 8, 0), at(6, 17, 0)));
    }

    #[test]
    fn r11_finish_to_start_with_lag() {
        let s = P::new().task(1, DAY).task(2, DAY).link(1, 2, FinishToStart, DAY).run();
        assert_eq!(dates(&s, 2), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r13_negative_lag_overlaps() {
        let s = P::new().task(1, 3 * DAY).task(2, DAY).link(1, 2, FinishToStart, -DAY).run();
        assert_eq!(dates(&s, 2), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r13_negative_lag_cannot_start_before_project_start() {
        let s = P::new().task(1, DAY).task(2, DAY).link(1, 2, FinishToStart, -3 * DAY).run();
        assert_eq!(dates(&s, 2), (at(5, 8, 0), at(5, 17, 0)));
    }

    #[test]
    fn r11_start_to_start_with_lag() {
        let s = P::new().task(1, 3 * DAY).task(2, DAY).link(1, 2, StartToStart, 4 * HOUR).run();
        assert_eq!(dates(&s, 2), (at(5, 13, 0), at(6, 12, 0)));
    }

    #[test]
    fn r11_finish_to_finish() {
        let s = P::new().task(1, 3 * DAY).task(2, DAY).link(1, 2, FinishToFinish, 0).run();
        assert_eq!(dates(&s, 2), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r11_start_to_finish() {
        // B must finish no earlier than one day after A starts.
        let s = P::new().task(1, DAY).task(2, 2 * HOUR).link(1, 2, StartToFinish, DAY).run();
        assert_eq!(dates(&s, 2), (at(5, 15, 0), at(5, 17, 0)));
    }

    #[test]
    fn r12_latest_driver_wins() {
        let s = P::new().task(1, DAY).task(2, 3 * DAY).task(3, DAY).fs(1, 3).fs(2, 3).run();
        assert_eq!(dates(&s, 3), (at(8, 8, 0), at(8, 17, 0)));
    }

    #[test]
    fn r14_milestone_sits_at_predecessor_finish() {
        let s = P::new().task(1, DAY).milestone(2).fs(1, 2).run();
        assert_eq!(dates(&s, 2), (at(5, 17, 0), at(5, 17, 0)));
    }

    #[test]
    fn r14_milestone_ignores_stored_duration() {
        let mut p = P::new().task(1, DAY);
        p.0.tasks.get_mut(&tid(1)).unwrap().is_milestone = true;
        assert_eq!(dates(&p.run(), 1), (at(5, 8, 0), at(5, 8, 0)));
    }

    #[test]
    fn r16_start_no_earlier_than_pushes_later() {
        let s = P::new().task(1, DAY).constraint(1, Constraint::StartNoEarlierThan(at(7, 10, 0))).run();
        assert_eq!(dates(&s, 1), (at(7, 10, 0), at(8, 10, 0)));
    }

    #[test]
    fn r16_start_no_earlier_than_does_not_pull_earlier() {
        let s = P::new()
            .task(1, 2 * DAY)
            .task(2, DAY)
            .fs(1, 2)
            .constraint(2, Constraint::StartNoEarlierThan(at(5, 8, 0)))
            .run();
        assert_eq!(dates(&s, 2), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r18_constraint_date_snaps_to_working_time() {
        let s = P::new().task(1, DAY).constraint(1, Constraint::StartNoEarlierThan(at(10, 9, 0))).run();
        assert_eq!(dates(&s, 1).0, at(12, 8, 0));
    }

    #[test]
    fn r17_must_start_on_later_than_links() {
        let s = P::new().task(1, DAY).constraint(1, Constraint::MustStartOn(at(7, 8, 0))).run();
        assert_eq!(dates(&s, 1), (at(7, 8, 0), at(7, 17, 0)));
        assert!(s.conflicts.is_empty());
    }

    #[test]
    fn r17_r23_must_start_on_conflict_is_reported_with_negative_slack() {
        let s = P::new()
            .task(1, 2 * DAY)
            .task(2, DAY)
            .fs(1, 2)
            .constraint(2, Constraint::MustStartOn(at(6, 8, 0)))
            .run();
        assert_eq!(dates(&s, 2), (at(6, 8, 0), at(6, 17, 0)));
        assert_eq!(
            s.conflicts,
            vec![ConstraintConflict { task: tid(2), constraint_start: at(6, 8, 0), link_driven_start: at(7, 8, 0) }]
        );
        // Task 1 would have to finish a day earlier than it can.
        assert_eq!(s.tasks[&tid(1)].total_slack, -DAY);
        assert!(s.tasks[&tid(1)].critical);
    }

    #[test]
    fn r19_r22_critical_path_and_slack() {
        // 1 (3d) -> 3 (1d); 2 (1d) -> 3. Task 2 has 2 days of slack.
        let s = P::new().task(1, 3 * DAY).task(2, DAY).task(3, DAY).fs(1, 3).fs(2, 3).run();
        assert_eq!(s.project_finish, at(8, 17, 0));
        assert!(s.tasks[&tid(1)].critical);
        assert!(s.tasks[&tid(3)].critical);
        let t2 = s.tasks[&tid(2)];
        assert!(!t2.critical);
        assert_eq!(t2.total_slack, 2 * DAY);
        assert_eq!((t2.late_start, t2.late_finish), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r22_unlinked_short_task_has_slack_to_project_finish() {
        let s = P::new().task(1, 3 * DAY).task(2, DAY).run();
        assert_eq!(s.tasks[&tid(2)].total_slack, 2 * DAY);
    }

    #[test]
    fn r20_backward_pass_through_each_link_type() {
        for kind in [FinishToStart, StartToStart, FinishToFinish, StartToFinish] {
            let s = P::new().task(1, DAY).task(2, DAY).link(1, 2, kind, 0).run();
            for n in [1, 2] {
                let t = s.tasks[&tid(n)];
                assert!(t.total_slack >= 0, "{kind:?} task {n}: {t:?}");
                assert!(t.late_start >= t.early_start, "{kind:?} task {n}");
            }
            assert!(s.tasks[&tid(2)].critical, "{kind:?}");
        }
    }

    #[test]
    fn r14_milestone_on_critical_path_has_zero_slack() {
        let s = P::new().task(1, DAY).milestone(2).task(3, DAY).fs(1, 2).fs(2, 3).run();
        assert_eq!(s.tasks[&tid(2)].total_slack, 0);
        assert!(s.tasks[&tid(2)].critical);
        assert_eq!(dates(&s, 3), (at(6, 8, 0), at(6, 17, 0)));
    }

    #[test]
    fn r24_summary_rolls_up_children() {
        let s = P::new()
            .task(10, 0)
            .task(1, DAY)
            .task(2, 2 * DAY)
            .child_of(1, 10)
            .child_of(2, 10)
            .fs(1, 2)
            .run();
        assert_eq!(dates(&s, 10), (at(5, 8, 0), at(7, 17, 0)));
        assert!(s.tasks[&tid(10)].critical);
    }

    #[test]
    fn r24_nested_summaries() {
        let s = P::new()
            .task(100, 0)
            .task(10, 0)
            .task(1, DAY)
            .task(2, 3 * DAY)
            .child_of(10, 100)
            .child_of(1, 10)
            .child_of(2, 100)
            .run();
        assert_eq!(dates(&s, 10), (at(5, 8, 0), at(5, 17, 0)));
        assert_eq!(dates(&s, 100), (at(5, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r25_link_on_summary_is_rejected() {
        let p = P::new().task(10, 0).task(1, DAY).task(2, DAY).child_of(1, 10).fs(10, 2);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::LinkOnSummary { task, .. }) if task == tid(10)));
    }

    #[test]
    fn r27_cycle_names_its_tasks_in_order() {
        let p = P::new().task(1, DAY).task(2, DAY).task(3, DAY).task(4, DAY).fs(4, 1).fs(1, 2).fs(2, 3).fs(3, 1);
        match schedule(&p.0) {
            Err(ScheduleError::Cycle { tasks, links }) => {
                assert_eq!(tasks, vec![tid(1), tid(2), tid(3)]);
                assert_eq!(links.len(), 3);
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn r28_self_link_is_rejected() {
        let p = P::new().task(1, DAY).fs(1, 1);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::SelfLink { .. })));
    }

    #[test]
    fn missing_task_and_parent_are_rejected() {
        let p = P::new().task(1, DAY).fs(1, 9);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::MissingTask { task, .. }) if task == tid(9)));
        let p = P::new().task(1, DAY).child_of(1, 9);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::MissingParent { .. })));
    }

    #[test]
    fn outline_loop_is_rejected() {
        let p = P::new().task(1, DAY).task(2, DAY).child_of(1, 2).child_of(2, 1);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::OutlineLoop { .. })));
    }

    #[test]
    fn holiday_moves_successor() {
        let mut p = P::new().task(1, DAY).task(2, DAY).fs(1, 2);
        p.0.calendar.set_exception(date(2026, 10, 6), DayPattern::non_working());
        assert_eq!(dates(&p.run(), 2), (at(7, 8, 0), at(7, 17, 0)));
    }

    /// Random acyclic projects: links only go from lower to higher task numbers.
    fn projects() -> impl Strategy<Value = Project> {
        let kinds = prop_oneof![Just(FinishToStart), Just(StartToStart), Just(FinishToFinish), Just(StartToFinish)];
        let tasks = prop::collection::vec((0i64..5 * DAY, prop::bool::weighted(0.1)), 1..25);
        let links = prop::collection::vec((0usize..25, 0usize..25, kinds, -DAY..2 * DAY), 0..40);
        (tasks, links).prop_map(|(tasks, links)| {
            let mut p = P::new();
            for (i, (d, ms)) in tasks.iter().enumerate() {
                p = if *ms { p.milestone(i as u128) } else { p.task(i as u128, *d) };
            }
            let mut seen = std::collections::BTreeSet::new();
            for (a, b, kind, lag) in links {
                let (a, b) = (a % tasks.len(), b % tasks.len());
                if a < b && seen.insert((a, b)) {
                    p = p.link(a as u128, b as u128, kind, lag);
                }
            }
            p.0
        })
    }

    proptest! {
        #[test]
        fn every_link_is_respected(p in projects()) {
            let s = schedule(&p).unwrap();
            let cal = &p.calendar;
            for d in p.dependencies.values() {
                let (a, b) = (s.tasks[&d.predecessor], s.tasks[&d.successor]);
                // Compared in working time: Friday 17:00 and Monday 08:00 are the same point.
                let (required, actual) = match d.kind {
                    FinishToStart => (cal.add_working(a.early_finish, d.lag), b.early_start),
                    StartToStart => (cal.add_working(a.early_start, d.lag), b.early_start),
                    FinishToFinish => (cal.add_working(a.early_finish, d.lag), b.early_finish),
                    StartToFinish => (cal.add_working(a.early_start, d.lag), b.early_finish),
                };
                let ok = cal.working_between(required, actual) >= 0;
                prop_assert!(ok, "{:?} violated: {:?} -> {:?}", d, a, b);
            }
        }

        #[test]
        fn without_hard_constraints_slack_is_never_negative_and_a_critical_task_finishes_last(p in projects()) {
            let s = schedule(&p).unwrap();
            for t in s.tasks.values() {
                prop_assert!(t.total_slack >= 0, "{:?}", t);
                prop_assert!(t.early_start >= p.start);
                prop_assert!(t.late_finish <= s.project_finish);
            }
            prop_assert!(s.tasks.values().any(|t| t.critical && t.early_finish == s.project_finish));
        }

        #[test]
        fn moving_the_start_by_a_week_moves_everything_by_a_week(p in projects()) {
            let a = schedule(&p).unwrap();
            let mut q = p.clone();
            q.start = Moment(q.start.0 + 7 * crate::time::MINUTES_PER_DAY);
            let b = schedule(&q).unwrap();
            let week = 7 * crate::time::MINUTES_PER_DAY;
            for (id, x) in &a.tasks {
                let y = b.tasks[id];
                prop_assert_eq!(y.early_start.0 - x.early_start.0, week);
                prop_assert_eq!(y.late_finish.0 - x.late_finish.0, week);
                prop_assert_eq!(y.total_slack, x.total_slack);
            }
        }

        #[test]
        fn deterministic(p in projects()) {
            prop_assert_eq!(schedule(&p).unwrap(), schedule(&p.clone()).unwrap());
        }
    }
}
