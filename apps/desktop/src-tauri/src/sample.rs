//! The sample project shown to new users (and, until step 8, the only way to open one).

use std::collections::BTreeMap;

use pm_core::*;

use crate::document::new_id;

/// "Office move": three phases, all four link types, a lag, milestones.
pub fn office_move(start: Moment) -> Project {
    const DAY: i64 = 480;
    let mut tasks = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    let mut keys: BTreeMap<Option<TaskId>, SortKey> = BTreeMap::new();
    let mut add = |parent: Option<TaskId>, name: &str, days: i64, milestone: bool| -> TaskId {
        let id = TaskId(new_id());
        let key = SortKey::between(keys.get(&parent), None);
        keys.insert(parent, key.clone());
        tasks.insert(
            id,
            Task {
                id,
                parent,
                sort_key: key,
                name: name.into(),
                duration: Duration::minutes(days * DAY).expect("positive"),
                is_milestone: milestone,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                leveling_delay: Duration::ZERO,
                notes: String::new(),
            },
        );
        id
    };

    let planning = add(None, "Planning", 0, false);
    let requirements = add(Some(planning), "List space requirements", 2, false);
    let choose = add(Some(planning), "Choose new office", 3, false);
    let lease = add(Some(planning), "Lease signed", 0, true);

    let prep = add(None, "Preparation", 0, false);
    let movers = add(Some(prep), "Book movers", 1, false);
    let furniture = add(Some(prep), "Order furniture", 5, false);
    let it = add(Some(prep), "Plan network and IT", 3, false);
    let pack = add(Some(prep), "Pack offices", 3, false);

    let moving = add(None, "Move", 0, false);
    let move_day = add(Some(moving), "Moving day", 1, false);
    let unpack = add(Some(moving), "Unpack and set up", 2, false);
    let open = add(Some(moving), "New office open", 0, true);

    let mut link = |predecessor, successor, kind, lag| {
        let id = DependencyId(new_id());
        dependencies.insert(id, Dependency { id, predecessor, successor, kind, lag });
    };
    use LinkType::*;
    link(requirements, choose, FinishToStart, 0);
    link(choose, lease, FinishToStart, 0);
    link(lease, movers, FinishToStart, 0);
    link(lease, furniture, FinishToStart, 0);
    link(lease, it, StartToStart, DAY);
    link(movers, pack, FinishToStart, 2 * DAY);
    link(pack, move_day, FinishToStart, 0);
    link(furniture, move_day, FinishToStart, 0);
    link(it, unpack, FinishToFinish, 0);
    link(move_day, unpack, FinishToStart, 0);
    link(unpack, open, FinishToStart, 0);

    Project {
        id: ProjectId(new_id()),
        name: "Office Move (Sample)".into(),
        start,
        calendar_id: CalendarId(new_id()),
        calendar: Calendar::standard(),
        tasks,
        dependencies,
        resources: Default::default(),
        assignments: Default::default(),
    }
}

/// A large generated project for performance testing (spec section 3): phases of ten
/// chained tasks, each phase also waiting on a task in a recent phase, milestones, all four
/// link types and lags. Deterministic for a given `n`.
pub fn generated(n: usize, start: Moment) -> Project {
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |m: u64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng % m
    };
    let mut tasks = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    let mut phase_key: Option<SortKey> = None;
    let mut child_key: Option<SortKey> = None;
    let mut phase = None;
    let mut leaves: Vec<TaskId> = Vec::new();
    for k in 0..n {
        let id = TaskId(new_id());
        let is_phase = k % 11 == 0;
        let key = if is_phase {
            child_key = None;
            SortKey::between(phase_key.as_ref(), None)
        } else {
            SortKey::between(child_key.as_ref(), None)
        };
        if is_phase {
            phase_key = Some(key.clone());
        } else {
            child_key = Some(key.clone());
        }
        tasks.insert(
            id,
            Task {
                id,
                parent: if is_phase { None } else { phase },
                sort_key: key,
                name: if is_phase { format!("Phase {}", k / 11 + 1) } else { format!("Task {k}") },
                duration: Duration::minutes(60 * (1 + next(40)) as i64).expect("positive"),
                is_milestone: !is_phase && next(20) == 0,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                leveling_delay: Duration::ZERO,
                notes: String::new(),
            },
        );
        if is_phase {
            phase = Some(id);
        } else {
            leaves.push(id);
        }
    }
    let kinds = [LinkType::FinishToStart, LinkType::StartToStart, LinkType::FinishToFinish, LinkType::StartToFinish];
    for w in 1..leaves.len() {
        let mut froms = Vec::new();
        if w % 10 != 0 {
            froms.push(w - 1);
        }
        if w > 60 && (w % 10 == 0 || next(8) == 0) {
            froms.push(w - 60 + next(40) as usize);
        }
        for from in froms {
            let id = DependencyId(new_id());
            let kind = if next(4) == 0 { kinds[next(4) as usize] } else { LinkType::FinishToStart };
            let lag = next(5) as i64 * 240 - 480;
            dependencies.insert(id, Dependency { id, predecessor: leaves[from], successor: leaves[w], kind, lag });
        }
    }
    Project {
        id: ProjectId(new_id()),
        name: format!("Test Project ({n} tasks)"),
        start,
        calendar_id: CalendarId(new_id()),
        calendar: Calendar::standard(),
        tasks,
        dependencies,
        resources: Default::default(),
        assignments: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sample_schedules_with_a_critical_path() {
        let p = super::office_move(pm_core::Moment(29_000_000));
        let s = pm_core::schedule(&p).unwrap();
        assert!(s.tasks.values().any(|t| t.critical));
        assert!(s.tasks.values().any(|t| !t.critical), "some tasks have slack, to show the difference");
    }

    #[test]
    fn generated_project_schedules() {
        let p = super::generated(500, pm_core::Moment(29_000_000));
        assert_eq!(p.tasks.len(), 500);
        assert!(pm_core::schedule(&p).is_ok());
    }
}
