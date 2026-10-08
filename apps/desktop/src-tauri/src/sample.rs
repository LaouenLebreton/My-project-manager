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
}
