//! Outline order and sort keys.
//!
//! Siblings are ordered by [`SortKey`]: base-62 fraction strings compared bytewise. There is
//! always a key between any two keys, so inserting or moving a task changes only that task
//! (one row, one command), which also keeps future sync conflicts small.

use std::collections::BTreeMap;

use crate::model::{Project, SortKey, TaskId};

/// Digits in ascending byte order: '0'-'9' < 'A'-'Z' < 'a'-'z'.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn digit_value(c: u8) -> u8 {
    DIGITS.iter().position(|&d| d == c).expect("sort key uses base-62 digits") as u8
}

impl SortKey {
    /// A key strictly between `before` and `after` (either may be absent: start or end).
    ///
    /// Panics if `before >= after`; callers pass neighbouring siblings in order.
    pub fn between(before: Option<&SortKey>, after: Option<&SortKey>) -> SortKey {
        let a: Vec<u8> = before.map(|k| k.0.bytes().map(digit_value).collect()).unwrap_or_default();
        let b: Option<Vec<u8>> = after.map(|k| k.0.bytes().map(digit_value).collect());
        if let (Some(x), Some(y)) = (before, after) {
            assert!(x < y, "sort keys out of order: {x:?} >= {y:?}");
        }
        SortKey(midpoint(&a, b.as_deref()).into_iter().map(|d| DIGITS[d as usize] as char).collect())
    }

    /// `n` keys in ascending order between `before` and `after`, for inserting several rows.
    pub fn n_between(before: Option<&SortKey>, after: Option<&SortKey>, n: usize) -> Vec<SortKey> {
        let mut keys = Vec::with_capacity(n);
        let mut prev = before.cloned();
        for _ in 0..n {
            let k = SortKey::between(prev.as_ref(), after);
            prev = Some(k.clone());
            keys.push(k);
        }
        keys
    }
}

/// Midpoint of two base-62 fractions `0.a` and `0.b` (`b = None` means 1). Keys never end in
/// digit 0, so there is always room below them.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        let common = (0..b.len()).take_while(|&i| a.get(i).copied().unwrap_or(0) == b[i]).count();
        if common > 0 {
            let mut out = b[..common].to_vec();
            out.extend(midpoint(a.get(common..).unwrap_or(&[]), Some(&b[common..])));
            return out;
        }
    }
    let da = a.first().copied().unwrap_or(0);
    let db = b.map_or(62, |b| b[0]);
    if db - da > 1 {
        return vec![(da + db) / 2];
    }
    // Neighbouring first digits.
    match b {
        Some(b) if b.len() > 1 => vec![b[0]],
        _ => {
            let mut out = vec![da];
            out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
            out
        }
    }
}

/// One row of the outline: a task and how deep it is (0 = top level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineRow {
    pub id: TaskId,
    pub depth: usize,
    pub has_children: bool,
}

/// All tasks in display order: depth-first, siblings by sort key (then id, for ties).
/// Tasks whose parent is missing or part of a loop are listed at the end at depth 0, so a
/// damaged file still shows every task.
pub fn outline(p: &Project) -> Vec<OutlineRow> {
    let mut children: BTreeMap<Option<TaskId>, Vec<(&SortKey, TaskId)>> = BTreeMap::new();
    for t in p.tasks.values() {
        let parent = t.parent.filter(|id| p.tasks.contains_key(id));
        children.entry(parent).or_default().push((&t.sort_key, t.id));
    }
    for list in children.values_mut() {
        list.sort();
    }
    let mut rows = Vec::with_capacity(p.tasks.len());
    let mut seen = std::collections::HashSet::with_capacity(p.tasks.len());
    let mut stack: Vec<(TaskId, usize)> = children.get(&None).into_iter().flatten().rev().map(|&(_, id)| (id, 0)).collect();
    loop {
        while let Some((id, depth)) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let kids = children.get(&Some(id));
            rows.push(OutlineRow { id, depth, has_children: kids.is_some_and(|k| !k.is_empty()) });
            stack.extend(kids.into_iter().flatten().rev().map(|&(_, c)| (c, depth + 1)));
        }
        // Anything not reached sits in a parent loop; surface it rather than hide it.
        match p.tasks.keys().find(|id| !seen.contains(id)) {
            Some(&id) => stack.push((id, 0)),
            None => return rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Calendar;
    use crate::model::*;
    use crate::time::Moment;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn key(s: &str) -> SortKey {
        SortKey(s.into())
    }

    #[test]
    fn between_basics() {
        let first = SortKey::between(None, None);
        assert_eq!(first, key("V"));
        assert!(SortKey::between(None, Some(&first)) < first);
        assert!(SortKey::between(Some(&first), None) > first);
        assert_eq!(SortKey::between(Some(&key("A")), Some(&key("B"))), key("AV"));
        assert_eq!(SortKey::between(Some(&key("A")), Some(&key("B1"))), key("B"));
        let k = SortKey::between(Some(&key("z")), None);
        assert!(k > key("z") && k.0.starts_with('z'));
    }

    #[test]
    fn n_between_is_ascending() {
        let keys = SortKey::n_between(Some(&key("A")), Some(&key("B")), 50);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys[0] > key("A") && keys[49] < key("B"));
    }

    fn keys() -> impl Strategy<Value = SortKey> {
        "[0-9A-Za-z]{0,5}[1-9A-Za-z]".prop_map(SortKey)
    }

    proptest! {
        #[test]
        fn between_is_strictly_between(a in keys(), b in keys()) {
            prop_assume!(a != b);
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            let m = SortKey::between(Some(&lo), Some(&hi));
            prop_assert!(lo < m && m < hi, "{lo:?} < {m:?} < {hi:?}");
            prop_assert!(!m.0.ends_with('0'));
        }

        #[test]
        fn repeated_inserts_stay_ordered_and_short(n in 1usize..200, at_front in any::<bool>()) {
            let mut list = vec![SortKey::between(None, None)];
            for _ in 0..n {
                let k = if at_front { SortKey::between(None, list.first()) } else { SortKey::between(list.last(), None) };
                if at_front { list.insert(0, k) } else { list.push(k) }
            }
            prop_assert!(list.windows(2).all(|w| w[0] < w[1]));
            // Appending repeatedly on one side grows keys by about one digit per 6 inserts.
            prop_assert!(list.iter().all(|k| k.0.len() <= 2 + n / 5));
        }
    }

    fn task(n: u128, parent: Option<u128>, key: &str) -> Task {
        Task {
            id: TaskId(Uuid::from_u128(n)),
            parent: parent.map(|p| TaskId(Uuid::from_u128(p))),
            sort_key: SortKey(key.into()),
            name: String::new(),
            duration: Duration::ZERO,
            is_milestone: false,
            constraint: Constraint::AsSoonAsPossible,
            percent_complete: Percent::default(),
            leveling_delay: Duration::ZERO,
            fixed_cost: Money(0),
            notes: String::new(),
        }
    }

    fn project(tasks: Vec<Task>) -> Project {
        Project {
            id: ProjectId(Uuid::from_u128(0)),
            name: String::new(),
            start: Moment(0),
            calendar_id: CalendarId(Uuid::from_u128(0)),
            calendar: Calendar::standard(),
            tasks: tasks.into_iter().map(|t| (t.id, t)).collect(),
            dependencies: Default::default(),
            resources: Default::default(),
            assignments: Default::default(),
            currency: String::new(),
        }
    }

    fn ids(rows: &[OutlineRow]) -> Vec<(u128, usize)> {
        rows.iter().map(|r| (r.id.0.as_u128(), r.depth)).collect()
    }

    #[test]
    fn outline_is_depth_first_by_sort_key() {
        let p = project(vec![task(1, None, "B"), task(2, None, "A"), task(3, Some(1), "Z"), task(4, Some(1), "M"), task(5, Some(4), "V")]);
        let rows = outline(&p);
        assert_eq!(ids(&rows), vec![(2, 0), (1, 0), (4, 1), (5, 2), (3, 1)]);
        assert!(rows[1].has_children && rows[2].has_children && !rows[3].has_children);
    }

    #[test]
    fn damaged_outline_still_lists_every_task() {
        let p = project(vec![task(1, Some(2), "A"), task(2, Some(1), "B"), task(3, Some(99), "C")]);
        let rows = outline(&p);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id.0.as_u128(), 3, "orphan shown at top level");
    }
}
