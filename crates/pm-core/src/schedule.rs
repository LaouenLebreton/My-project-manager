//! The scheduling engine: `schedule(project) -> Schedule` (spec 4.5, rules R10 to R29).
//!
//! Leaf tasks are scheduled; summary tasks roll up from their children. Links may start or
//! end at a summary (R25): a link into a summary holds back every task inside it, and a link
//! out of a summary waits for every task inside it. Time-valued results are start-type for
//! starts and finish-type for finishes (see [`crate::calendar`]).
//!
//! The graph has one node per leaf and two per summary: its *entry* (links into the summary
//! arrive here and pass down to the children) and its *exit* (rolls up the children; links out
//! of the summary leave from here). This keeps the graph linear in the size of the project.

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
    /// The fixed start is a manually scheduled task's own date (R34), not a constraint.
    #[serde(default)]
    pub manual: bool,
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
    /// R25b: only finish-to-start and start-to-start links may point to a summary.
    #[error("finish-to-finish and start-to-finish links cannot point to a summary task; link to a task inside it instead")]
    FinishLinkToSummary { link: DependencyId, task: TaskId },
    #[error("a task cannot be its own predecessor")]
    SelfLink { link: DependencyId },
    #[error("a task is inside itself in the outline")]
    OutlineLoop { task: TaskId },
    #[error("a task's parent does not exist")]
    MissingParent { task: TaskId, parent: TaskId },
}

/// No date at all: the identity for `min`.
const NEVER: Moment = Moment(i64::MAX);

/// Schedules every task. Pure and deterministic (R29): same input, same output.
pub fn schedule(project: &Project) -> Result<Schedule, ScheduleError> {
    let g = Graph::build(project)?;
    let order = g.topological_order()?;
    let cal = &project.calendar;
    let n = g.tasks.len();
    let start = cal.next_working(project.start);
    let snap = |m: Moment| cal.next_working(m);

    // Forward pass (R10 to R18, R25). Per node: a leaf's (ES, EF); a summary entry's floor
    // (the earliest any task inside may start); a summary exit's rolled-up (ES, EF).
    let mut early = vec![(start, start); 2 * n];
    let mut conflicts = Vec::new();
    for &node in &order {
        let i = node % n;
        let task = g.tasks[i];
        let inherited = g.parent[i].map_or(start, |p| early[p].0);
        if node >= n {
            // Summary exit: roll up the children.
            let kids = g.children[i].iter().map(|&c| early[g.exit(c)]);
            early[node] = kids.fold((NEVER, Moment(i64::MIN)), |(es, ef), (ces, cef)| (es.min(ces), ef.max(cef)));
            continue;
        }
        let summary = !g.children[i].is_empty();
        let dur = if summary { 0 } else { g.duration(i) };
        let mut es = start.max(inherited);
        for &(from, link) in &g.preds[node] {
            let (pes, pef) = early[from];
            let candidate = match link.kind {
                LinkType::FinishToStart => cal.add_working(pef, link.lag),
                LinkType::StartToStart => cal.add_working(pes, link.lag),
                LinkType::FinishToFinish => cal.add_working(cal.add_working(pef, link.lag), -dur),
                LinkType::StartToFinish => cal.add_working(cal.add_working(pes, link.lag), -dur),
            };
            es = es.max(candidate);
        }
        if summary {
            // A summary's own constraint holds back everything inside it.
            if let Constraint::StartNoEarlierThan(d) | Constraint::MustStartOn(d) = task.constraint {
                es = es.max(snap(d));
            }
            early[node] = (es, es);
            continue;
        }
        // Milestones keep a finish-type instant such as 17:00 (R14); tasks with work
        // start at the next working minute (R5).
        if dur > 0 {
            es = snap(es);
        }
        // Manually scheduled (R34): the typed start wins; broken links are reported.
        if let Some(m) = task.manual_start {
            let target = if dur > 0 { snap(m) } else { m };
            if es > target {
                conflicts.push(ConstraintConflict { task: task.id, constraint_start: target, link_driven_start: es, manual: true });
            }
            early[node] = (target, cal.add_working(target, dur));
            continue;
        }
        // Leveling waits after everything else that drives the start (R30).
        let delay = task.leveling_delay.get();
        if delay > 0 && !task.constraint.is_fixed() {
            es = cal.add_working(es.max(match task.constraint {
                Constraint::StartNoEarlierThan(d) => snap(d),
                _ => es,
            }), delay);
            if dur > 0 {
                es = snap(es);
            }
        }
        // Constraints (R15 to R18, R32). Limits that the links would break are honoured anyway
        // and reported as conflicts, so the problem is visible (negative slack), never hidden.
        let finish_on = |d: Moment| if dur > 0 { cal.prev_working_end(d) } else { d };
        let start_for_finish = |f: Moment| cal.add_working(f, -dur);
        let mut honour = |target: Moment, es: &mut Moment| {
            if *es > target {
                conflicts.push(ConstraintConflict { task: task.id, constraint_start: target, link_driven_start: *es, manual: false });
            }
            *es = target;
        };
        match task.constraint {
            Constraint::AsSoonAsPossible | Constraint::AsLateAsPossible => {}
            Constraint::StartNoEarlierThan(d) => es = es.max(snap(d)),
            Constraint::FinishNoEarlierThan(d) => es = es.max(start_for_finish(finish_on(d))),
            Constraint::MustStartOn(d) => honour(snap(d), &mut es),
            Constraint::MustFinishOn(d) => honour(start_for_finish(finish_on(d)), &mut es),
            Constraint::StartNoLaterThan(d) => {
                if es > snap(d) {
                    honour(snap(d), &mut es);
                }
            }
            Constraint::FinishNoLaterThan(d) => {
                let latest = start_for_finish(finish_on(d));
                if es > latest {
                    honour(latest, &mut es);
                }
            }
        }
        early[node] = (es, cal.add_working(es, dur));
    }

    let project_finish = (0..n).filter(|&i| g.children[i].is_empty()).map(|i| early[i].1).max().unwrap_or(start);

    // As late as possible (R33): latest tasks first, each moves as late as it can without
    // moving anything after it (at their early dates) or the project's finish.
    let alap: Vec<usize> = order
        .iter()
        .rev()
        .copied()
        .filter(|&v| v < n && g.children[v].is_empty() && g.tasks[v].manual_start.is_none() && g.tasks[v].constraint == Constraint::AsLateAsPossible)
        .collect();
    if !alap.is_empty() {
        for i in alap {
            let dur = g.duration(i);
            let mut finish = project_finish;
            for &(to, link) in &g.succs[i] {
                // A successor summary starts when its first task does.
                let (ses, sef) = if g.children[to].is_empty() { early[to] } else { early[n + to] };
                finish = finish.min(match link.kind {
                    LinkType::FinishToStart => cal.add_working(ses, -link.lag),
                    LinkType::FinishToFinish => cal.add_working(sef, -link.lag),
                    LinkType::StartToStart => cal.add_working(cal.add_working(ses, -link.lag), dur),
                    LinkType::StartToFinish => cal.add_working(cal.add_working(sef, -link.lag), dur),
                });
            }
            if dur > 0 {
                finish = cal.prev_working_end(finish);
            }
            let es = cal.add_working(finish, -dur);
            if es > early[i].0 {
                early[i] = (es, finish);
            }
        }
        // Roll the summaries up again around the moved tasks.
        for &node in &order {
            if node >= n {
                let i = node - n;
                let kids = g.children[i].iter().map(|&c| early[g.exit(c)]);
                early[node] = kids.fold((NEVER, Moment(i64::MIN)), |(es, ef), (ces, cef)| (es.min(ces), ef.max(cef)));
            }
        }
    }

    // Backward pass (R19 to R21, R25), the mirror image. Per node: a leaf's (LS, LF); a
    // summary exit's ceilings (latest start for the tasks that start it, latest finish for
    // all tasks inside); a summary entry's rolled-up late start.
    let mut late = vec![(project_finish, project_finish); 2 * n];
    for &node in order.iter().rev() {
        let i = node % n;
        let summary = !g.children[i].is_empty();
        if summary && node < n {
            // Summary entry: the latest it can start is when its first task must start.
            let ls = g.children[i].iter().map(|&c| late[c].0).min().unwrap_or(project_finish);
            late[node] = (ls, ls);
            continue;
        }
        // Ceilings handed down from the enclosing summary's exit.
        let (mut start_ceiling, mut lf) = (NEVER, project_finish);
        if let Some(p) = g.parent[i] {
            let (sc, fc) = late[n + p];
            lf = lf.min(fc);
            // A summary's start is its earliest task's start, so a limit on the summary's
            // start binds only the tasks that start it.
            if early[g.exit(i)].0 == early[n + p].0 {
                start_ceiling = sc;
            }
        }
        let dur = if summary { 0 } else { g.duration(i) };
        for &(to, link) in &g.succs[node] {
            let (sls, slf) = late[to];
            match link.kind {
                LinkType::FinishToStart => lf = lf.min(cal.add_working(sls, -link.lag)),
                LinkType::FinishToFinish => lf = lf.min(cal.add_working(slf, -link.lag)),
                LinkType::StartToStart => start_ceiling = start_ceiling.min(cal.add_working(sls, -link.lag)),
                LinkType::StartToFinish => start_ceiling = start_ceiling.min(cal.add_working(slf, -link.lag)),
            }
        }
        if summary {
            late[node] = (start_ceiling, lf);
            continue;
        }
        if start_ceiling != NEVER {
            lf = lf.min(cal.add_working(start_ceiling, dur));
        }
        if dur > 0 {
            lf = cal.prev_working_end(lf);
        }
        let finish_on = |d: Moment| if dur > 0 { cal.prev_working_end(d) } else { d };
        if g.tasks[i].manual_start.is_some() {
            // A manual task's dates are fixed, like Must Start On.
            lf = early[i].1;
        }
        match g.tasks[i].constraint {
            Constraint::MustStartOn(d) => lf = cal.add_working(snap(d), dur),
            Constraint::MustFinishOn(d) => lf = finish_on(d),
            Constraint::StartNoLaterThan(d) => lf = lf.min(cal.add_working(snap(d), dur)),
            Constraint::FinishNoLaterThan(d) => lf = lf.min(finish_on(d)),
            _ => {}
        }
        late[node] = (cal.add_working(lf, -dur), lf);
    }

    let mut result = vec![None; n];
    for i in (0..n).filter(|&i| g.children[i].is_empty()) {
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

/// Checks that the links and outline can be scheduled: no loops (through links or through
/// summaries), no finish links into summaries. The command layer uses this to refuse edits.
pub fn check(project: &Project) -> Result<(), ScheduleError> {
    Graph::build(project)?.topological_order().map(|_| ())
}

/// Tasks indexed in id order. Node `i` is task `i` (a leaf, or a summary's entry); node
/// `n + i` is summary `i`'s exit.
struct Graph<'a> {
    tasks: Vec<&'a Task>,
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    depth: Vec<usize>,
    /// Per node, incoming and outgoing links in dependency-id order.
    preds: Vec<Vec<(usize, &'a Dependency)>>,
    succs: Vec<Vec<(usize, &'a Dependency)>>,
    /// Per node, every node that must come before it (links and outline), for ordering.
    after: Vec<Vec<usize>>,
    before_count: Vec<usize>,
}

impl<'a> Graph<'a> {
    fn exit(&self, i: usize) -> usize {
        if self.children[i].is_empty() { i } else { self.tasks.len() + i }
    }

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

        let mut g = Graph {
            tasks,
            parent,
            children,
            depth,
            preds: vec![Vec::new(); 2 * n],
            succs: vec![Vec::new(); 2 * n],
            after: vec![Vec::new(); 2 * n],
            before_count: vec![0; 2 * n],
        };
        let edge = |g: &mut Graph, from: usize, to: usize| {
            g.after[from].push(to);
            g.before_count[to] += 1;
        };
        for i in 0..n {
            if let Some(p) = g.parent[i] {
                edge(&mut g, p, i); // the summary's entry before the task's (floors flow down)
                let exit = g.exit(i);
                edge(&mut g, exit, n + p); // the task's exit before the summary's (roll-up)
            }
        }
        for d in project.dependencies.values() {
            if d.predecessor == d.successor {
                return Err(ScheduleError::SelfLink { link: d.id });
            }
            let lookup = |task: TaskId| index.get(&task).copied().ok_or(ScheduleError::MissingTask { link: d.id, task });
            let (p, s) = (lookup(d.predecessor)?, lookup(d.successor)?);
            if !g.children[s].is_empty() && matches!(d.kind, LinkType::FinishToFinish | LinkType::StartToFinish) {
                return Err(ScheduleError::FinishLinkToSummary { link: d.id, task: d.successor });
            }
            let from = g.exit(p);
            g.preds[s].push((from, d));
            g.succs[from].push((s, d));
            edge(&mut g, from, s);
        }
        Ok(g)
    }

    fn duration(&self, i: usize) -> i64 {
        let t = self.tasks[i];
        if t.is_milestone { 0 } else { t.duration.get() }
    }

    fn nodes(&self) -> impl Iterator<Item = usize> + '_ {
        let n = self.tasks.len();
        (0..n).chain((0..n).filter(|&i| !self.children[i].is_empty()).map(move |i| n + i))
    }

    /// Nodes in an order where everything a node depends on comes first (Kahn's algorithm).
    fn topological_order(&self) -> Result<Vec<usize>, ScheduleError> {
        let mut indegree = self.before_count.clone();
        let mut queue: VecDeque<usize> = self.nodes().filter(|&v| indegree[v] == 0).collect();
        let total = self.nodes().count();
        let mut order = Vec::with_capacity(total);
        while let Some(v) = queue.pop_front() {
            order.push(v);
            for &w in &self.after[v] {
                indegree[w] -= 1;
                if indegree[w] == 0 {
                    queue.push_back(w);
                }
            }
        }
        if order.len() == total { Ok(order) } else { Err(self.find_cycle(&indegree)) }
    }

    /// Every node left with a nonzero in-degree after Kahn's algorithm has a predecessor
    /// that is also left, so walking predecessors from any of them must revisit a node.
    fn find_cycle(&self, indegree: &[usize]) -> ScheduleError {
        let n = self.tasks.len();
        let mut before: Vec<Vec<usize>> = vec![Vec::new(); 2 * n];
        for v in self.nodes() {
            for &w in &self.after[v] {
                before[w].push(v);
            }
        }
        let remaining = |v: usize| indegree[v] > 0;
        let first = self.nodes().find(|&v| remaining(v)).expect("a cycle exists");
        let mut path = vec![first];
        let mut seen_at = HashMap::from([(first, 0)]);
        let cycle_start = loop {
            let cur = *path.last().unwrap();
            let &p = before[cur].iter().find(|&&p| remaining(p)).expect("remaining predecessor");
            if let Some(&at) = seen_at.get(&p) {
                break at;
            }
            seen_at.insert(p, path.len());
            path.push(p);
        };
        // The walk went backwards; reverse it. Then name tasks (an entry and an exit are the
        // same summary) and the links on the loop, starting at the smallest task.
        let nodes: Vec<usize> = path[cycle_start..].iter().rev().copied().collect();
        let mut links = Vec::new();
        for k in 0..nodes.len() {
            let (a, b) = (nodes[k], nodes[(k + 1) % nodes.len()]);
            if let Some(&(_, d)) = self.succs[a].iter().find(|&&(s, _)| s == b) {
                links.push(d.id);
            }
        }
        let mut tasks: Vec<usize> = Vec::new();
        for v in &nodes {
            let t = v % n;
            if tasks.last() != Some(&t) && !tasks.contains(&t) {
                tasks.push(t);
            }
        }
        let min_pos = tasks.iter().enumerate().min_by_key(|&(_, &i)| i).map(|(pos, _)| pos).unwrap_or(0);
        tasks.rotate_left(min_pos);
        links.sort_by_key(|id| {
            let d = self.succs.iter().flatten().find(|(_, d)| d.id == *id).expect("link").1;
            tasks.iter().position(|&t| self.tasks[t].id == d.predecessor).unwrap_or(usize::MAX)
        });
        ScheduleError::Cycle { tasks: tasks.iter().map(|&i| self.tasks[i].id).collect(), links }
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
                resources: Default::default(),
                assignments: Default::default(),
                currency: String::new(),
                baseline: None,
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
                    leveling_delay: Duration::ZERO,
                    fixed_cost: Money(0),
                    manual_start: None,
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
            vec![ConstraintConflict { task: tid(2), constraint_start: at(6, 8, 0), link_driven_start: at(7, 8, 0), manual: false }]
        );
        // Task 1 would have to finish a day earlier than it can.
        assert_eq!(s.tasks[&tid(1)].total_slack, -DAY);
        assert!(s.tasks[&tid(1)].critical);
    }

    #[test]
    fn r30_leveling_delay_pushes_the_start_and_uses_up_slack() {
        // 1 (3d) and 2 (1d) in parallel; 2 delayed by a day has one day of slack left.
        let mut p = P::new().task(1, 3 * DAY).task(2, DAY);
        p.0.tasks.get_mut(&tid(2)).unwrap().leveling_delay = Duration::minutes(DAY).unwrap();
        let s = p.run();
        assert_eq!(dates(&s, 2), (at(6, 8, 0), at(6, 17, 0)));
        assert_eq!(s.tasks[&tid(2)].total_slack, DAY);
        // After a start-no-earlier-than date, the delay counts from that date.
        p.0.tasks.get_mut(&tid(2)).unwrap().constraint = Constraint::StartNoEarlierThan(at(7, 8, 0));
        assert_eq!(dates(&p.run(), 2), (at(8, 8, 0), at(8, 17, 0)));
        // Must start on is never moved by leveling.
        p.0.tasks.get_mut(&tid(2)).unwrap().constraint = Constraint::MustStartOn(at(5, 8, 0));
        assert_eq!(dates(&p.run(), 2).0, at(5, 8, 0));
    }

    #[test]
    fn r32_finish_no_earlier_than_holds_the_task_back() {
        let s = P::new().task(1, DAY).constraint(1, Constraint::FinishNoEarlierThan(at(7, 17, 0))).run();
        assert_eq!(dates(&s, 1), (at(7, 8, 0), at(7, 17, 0)));
    }

    #[test]
    fn r32_must_finish_on_fixes_the_finish_and_reports_a_clash() {
        let s = P::new().task(1, 2 * DAY).constraint(1, Constraint::MustFinishOn(at(9, 17, 0))).run();
        assert_eq!(dates(&s, 1), (at(8, 8, 0), at(9, 17, 0)));
        // Its predecessor takes three days, so the fixed finish is too early.
        let s = P::new().task(1, 3 * DAY).task(2, DAY).fs(1, 2).constraint(2, Constraint::MustFinishOn(at(6, 17, 0))).run();
        assert_eq!(dates(&s, 2), (at(6, 8, 0), at(6, 17, 0)));
        assert_eq!(s.conflicts.len(), 1);
        assert!(s.tasks[&tid(1)].total_slack < 0);
    }

    #[test]
    fn r32_no_later_than_limits_are_honoured_and_reported() {
        // Links want task 2 to start on Thursday; it must start by Tuesday.
        let s = P::new().task(1, 3 * DAY).task(2, DAY).fs(1, 2).constraint(2, Constraint::StartNoLaterThan(at(6, 8, 0))).run();
        assert_eq!(dates(&s, 2).0, at(6, 8, 0));
        assert_eq!(s.conflicts.len(), 1);
        // Without a clash, the limit only takes away slack: 2 runs Monday, may start up to Wednesday.
        let s = P::new().task(1, 5 * DAY).task(2, DAY).constraint(2, Constraint::StartNoLaterThan(at(7, 8, 0))).run();
        assert_eq!(dates(&s, 2).0, at(5, 8, 0));
        assert!(s.conflicts.is_empty());
        assert_eq!(s.tasks[&tid(2)].total_slack, 2 * DAY);
        let s = P::new().task(1, 5 * DAY).task(2, DAY).constraint(2, Constraint::FinishNoLaterThan(at(6, 17, 0))).run();
        assert_eq!(s.tasks[&tid(2)].total_slack, DAY);
    }

    #[test]
    fn r33_as_late_as_possible_moves_up_to_its_successor() {
        // 1 (1d) -> 3; 2 (3d) -> 3. ALAP task 1 runs the day before 3 instead of Monday.
        let s = P::new().task(1, DAY).task(2, 3 * DAY).task(3, DAY).fs(1, 3).fs(2, 3).constraint(1, Constraint::AsLateAsPossible).run();
        assert_eq!(dates(&s, 1), (at(7, 8, 0), at(7, 17, 0)));
        assert_eq!(dates(&s, 3), (at(8, 8, 0), at(8, 17, 0)), "the successor does not move");
        assert_eq!(s.tasks[&tid(1)].total_slack, 0);
        // Unlinked, it ends with the project.
        let s = P::new().task(1, DAY).task(2, 4 * DAY).constraint(1, Constraint::AsLateAsPossible).run();
        assert_eq!(dates(&s, 1), (at(8, 8, 0), at(8, 17, 0)));
    }

    #[test]
    fn r33_chained_as_late_as_possible_tasks() {
        let s = P::new()
            .task(1, DAY)
            .task(2, DAY)
            .task(9, 5 * DAY)
            .fs(1, 2)
            .constraint(1, Constraint::AsLateAsPossible)
            .constraint(2, Constraint::AsLateAsPossible)
            .run();
        assert_eq!(dates(&s, 2), (at(9, 8, 0), at(9, 17, 0)));
        assert_eq!(dates(&s, 1), (at(8, 8, 0), at(8, 17, 0)));
    }

    #[test]
    fn r34_manual_tasks_keep_their_start_and_successors_follow_them() {
        // 1 (2d) -> 2 (1d, manual on Monday) -> 3 (1d).
        let mut p = P::new().task(1, 2 * DAY).task(2, DAY).task(3, DAY).fs(1, 2).fs(2, 3);
        p.0.tasks.get_mut(&tid(2)).unwrap().manual_start = Some(at(5, 8, 0));
        p.0.tasks.get_mut(&tid(2)).unwrap().constraint = Constraint::StartNoEarlierThan(at(9, 8, 0));
        let s = p.run();
        assert_eq!(dates(&s, 2), (at(5, 8, 0), at(5, 17, 0)), "constraints and links do not move it");
        assert_eq!(dates(&s, 3), (at(6, 8, 0), at(6, 17, 0)), "its successor follows its real dates");
        assert_eq!(s.conflicts.len(), 1);
        assert!(s.conflicts[0].manual);
        // A manual task later than its links allow is simply later; no conflict.
        p.0.tasks.get_mut(&tid(2)).unwrap().manual_start = Some(at(12, 8, 0));
        let s = p.run();
        assert_eq!(dates(&s, 2).0, at(12, 8, 0));
        assert!(s.conflicts.is_empty());
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
    fn r25a_link_from_a_summary_waits_for_everything_inside() {
        // Summary 10 holds 1 (1d) and 2 (2d, after 1). Task 3 follows the summary.
        let s = P::new()
            .task(10, 0)
            .task(1, DAY)
            .task(2, 2 * DAY)
            .child_of(1, 10)
            .child_of(2, 10)
            .fs(1, 2)
            .task(3, DAY)
            .fs(10, 3)
            .run();
        assert_eq!(dates(&s, 3), (at(8, 8, 0), at(8, 17, 0)));
        assert!(s.tasks[&tid(3)].critical);
        assert!(s.tasks[&tid(1)].critical && s.tasks[&tid(2)].critical);
    }

    #[test]
    fn r25b_link_into_a_summary_holds_back_every_task_inside_at_any_depth() {
        // 1 (2d) -> summary 10 { 2 (1d), summary 20 { 3 (1d) } }.
        let s = P::new()
            .task(1, 2 * DAY)
            .task(10, 0)
            .task(2, DAY)
            .task(20, 0)
            .task(3, DAY)
            .child_of(2, 10)
            .child_of(20, 10)
            .child_of(3, 20)
            .fs(1, 10)
            .run();
        assert_eq!(dates(&s, 2), (at(7, 8, 0), at(7, 17, 0)));
        assert_eq!(dates(&s, 3), (at(7, 8, 0), at(7, 17, 0)));
        assert_eq!(dates(&s, 10), (at(7, 8, 0), at(7, 17, 0)));
        assert!(s.tasks[&tid(1)].critical, "the predecessor of a critical summary is critical");
    }

    #[test]
    fn r25b_start_to_start_into_a_summary_with_lag() {
        let s = P::new().task(1, 3 * DAY).task(10, 0).task(2, DAY).child_of(2, 10).link(1, 10, StartToStart, DAY).run();
        assert_eq!(dates(&s, 2), (at(6, 8, 0), at(6, 17, 0)));
    }

    #[test]
    fn r25_slack_through_a_summary() {
        // 1 (1d) -> summary 10 { 2 (1d) }; 3 (4d) runs in parallel and finishes last.
        let s = P::new().task(1, DAY).task(10, 0).task(2, DAY).child_of(2, 10).fs(1, 10).task(3, 4 * DAY).run();
        assert_eq!(s.tasks[&tid(1)].total_slack, 2 * DAY);
        assert_eq!(s.tasks[&tid(2)].total_slack, 2 * DAY);
        assert!(s.tasks[&tid(3)].critical);
    }

    #[test]
    fn r25a_start_to_start_from_a_summary_binds_only_the_tasks_that_start_it() {
        // Summary 10 { 1 (1d), 2 (1d, waits 3 days for task 5) }; 3 starts with the summary.
        // Task 2 starts later than the summary, so the SS link must not squeeze its slack.
        let s = P::new()
            .task(10, 0)
            .task(1, DAY)
            .task(2, DAY)
            .child_of(1, 10)
            .child_of(2, 10)
            .task(5, 3 * DAY)
            .fs(5, 2)
            .task(3, DAY)
            .link(10, 3, StartToStart, 0)
            .run();
        assert_eq!(dates(&s, 3), (at(5, 8, 0), at(5, 17, 0)));
        assert!(s.tasks.values().all(|t| t.total_slack >= 0), "{:?}", s.tasks);
    }

    #[test]
    fn r25c_a_link_between_a_summary_and_a_task_inside_it_is_a_loop() {
        let p = P::new().task(10, 0).task(1, DAY).child_of(1, 10).fs(10, 1);
        match schedule(&p.0) {
            Err(ScheduleError::Cycle { tasks, links }) => {
                assert_eq!(tasks, vec![tid(1), tid(10)]);
                assert_eq!(links.len(), 1);
            }
            other => panic!("expected a loop, got {other:?}"),
        }
    }

    #[test]
    fn r25b_finish_links_into_a_summary_are_refused() {
        let p = P::new().task(1, DAY).task(10, 0).task(2, DAY).child_of(2, 10).link(1, 10, FinishToFinish, 0);
        assert!(matches!(schedule(&p.0), Err(ScheduleError::FinishLinkToSummary { task, .. }) if task == tid(10)));
        // Out of a summary, any type is fine.
        let p = P::new().task(1, DAY).task(10, 0).task(2, DAY).child_of(2, 10).link(10, 1, FinishToFinish, 0);
        assert!(schedule(&p.0).is_ok());
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

    /// Random outlines with links between any tasks, summaries included. Links that cannot
    /// be held (loops, finish links into summaries) are left out, as the command layer would.
    fn outlined_projects() -> impl Strategy<Value = Project> {
        let kinds = prop_oneof![Just(FinishToStart), Just(StartToStart), Just(FinishToFinish), Just(StartToFinish)];
        let tasks = prop::collection::vec((0i64..4 * DAY, 0usize..3), 2..25);
        let links = prop::collection::vec((0usize..25, 0usize..25, kinds, -DAY..DAY), 0..30);
        (tasks, links).prop_map(|(tasks, links)| {
            let mut p = P::new();
            let mut depths: Vec<usize> = Vec::new();
            for (i, &(d, depth)) in tasks.iter().enumerate() {
                let depth = depth.min(depths.last().map_or(0, |&x| x + 1));
                p = p.task(i as u128, d);
                if depth > 0 {
                    let parent = (0..i).rev().find(|&j| depths[j] == depth - 1).unwrap();
                    p = p.child_of(i as u128, parent as u128);
                }
                depths.push(depth);
            }
            for (a, b, kind, lag) in links {
                let (a, b) = (a % tasks.len(), b % tasks.len());
                if a == b {
                    continue;
                }
                let before = p.0.dependencies.clone();
                p = p.link(a as u128, b as u128, kind, lag);
                if check(&p.0).is_err() {
                    p.0.dependencies = before;
                }
            }
            p.0
        })
    }

    proptest! {
        #[test]
        fn links_with_summaries_are_respected_and_slack_is_never_negative(p in outlined_projects()) {
            let s = schedule(&p).unwrap();
            let cal = &p.calendar;
            for d in p.dependencies.values() {
                let (a, b) = (s.tasks[&d.predecessor], s.tasks[&d.successor]);
                let (required, actual) = match d.kind {
                    FinishToStart => (cal.add_working(a.early_finish, d.lag), b.early_start),
                    StartToStart => (cal.add_working(a.early_start, d.lag), b.early_start),
                    FinishToFinish => (cal.add_working(a.early_finish, d.lag), b.early_finish),
                    StartToFinish => (cal.add_working(a.early_start, d.lag), b.early_finish),
                };
                prop_assert!(cal.working_between(required, actual) >= 0, "{:?} violated: {:?} -> {:?}", d, a, b);
            }
            for t in s.tasks.values() {
                prop_assert!(t.total_slack >= 0, "{:?}", t);
            }
        }

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
