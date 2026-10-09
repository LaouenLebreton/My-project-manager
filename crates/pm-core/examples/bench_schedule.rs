//! Scheduling benchmark for the spec's performance target (under 100 ms for 5,000 tasks).
//!
//! Run with: `cargo run --release -p pm-core --example bench_schedule`

use std::collections::BTreeMap;
use std::time::Instant;

use pm_core::*;
use uuid::Uuid;

/// A project shaped like real plans: phases of 10 chained tasks, phases linked to
/// recent phases, a few milestones, lags of all signs, and some holidays.
fn generate(n: usize) -> Project {
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |m: u64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng % m
    };
    let mut calendar = Calendar::standard();
    for k in 0..40 {
        let day = jiff::civil::date(2027, 1, 1).saturating_add(jiff::Span::new().days(k * 9));
        calendar.set_exception(day, DayPattern::non_working());
    }
    let mut tasks = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    let id = |k: usize| TaskId(Uuid::from_u128(k as u128 + 1));
    let mut summary = None;
    let mut leaves = Vec::new();
    for k in 0..n {
        let is_summary = k % 11 == 0;
        tasks.insert(
            id(k),
            Task {
                id: id(k),
                parent: if is_summary { None } else { summary },
                sort_key: SortKey(format!("{k:08}")),
                name: format!("Task {k}"),
                duration: Duration::minutes(60 * (1 + next(40)) as i64).unwrap(),
                is_milestone: !is_summary && next(20) == 0,
                constraint: Constraint::AsSoonAsPossible,
                percent_complete: Percent::default(),
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                notes: String::new(),
            },
        );
        if is_summary {
            summary = Some(id(k));
        } else {
            leaves.push(k);
        }
    }
    let kinds = [LinkType::FinishToStart, LinkType::StartToStart, LinkType::FinishToFinish, LinkType::StartToFinish];
    for w in 1..leaves.len() {
        // Chained inside a phase; each phase also waits on one task from a recent phase,
        // so phases run in parallel instead of forming one long chain.
        let mut froms = Vec::new();
        if w % 10 != 0 {
            froms.push(w - 1);
        }
        if w > 60 && (w % 10 == 0 || next(8) == 0) {
            froms.push(w - 60 + next(40) as usize);
        }
        for from in froms {
            let d = DependencyId(Uuid::from_u128(1 << 64 | dependencies.len() as u128));
            let kind = if next(4) == 0 { kinds[next(4) as usize] } else { LinkType::FinishToStart };
            let lag = next(5) as i64 * 240 - 480;
            dependencies.insert(d, Dependency { id: d, predecessor: id(leaves[from]), successor: id(leaves[w]), kind, lag });
        }
    }
    Project {
        id: ProjectId(Uuid::from_u128(0)),
        name: format!("bench {n}"),
        start: Moment::from_datetime(jiff::civil::date(2027, 1, 4).at(8, 0, 0, 0)),
        calendar_id: CalendarId(Uuid::from_u128(0)),
        calendar,
        tasks,
        dependencies,
        resources: Default::default(),
        assignments: Default::default(),
        currency: String::new(),
        baseline: None,
    }
}

fn main() {
    for n in [5_000, 20_000] {
        let p = generate(n);
        let s = schedule(&p).expect("benchmark project schedules");
        let runs = 20;
        let t = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(schedule(std::hint::black_box(&p)).unwrap());
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        let critical = s.tasks.values().filter(|t| t.critical).count();
        println!(
            "{n:>6} tasks, {:>6} links: {ms:7.2} ms per full schedule; finish {}, {critical} critical",
            p.dependencies.len(),
            s.project_finish.to_datetime(),
        );
    }
}
