use std::collections::BTreeMap;

use pm_core::*;
use proptest::prelude::*;
use uuid::Uuid;

use crate::{export_mspdi, import_mspdi, ImportReport};

fn ids() -> impl FnMut() -> Uuid {
    let mut n = 0u128;
    move || {
        n += 1;
        Uuid::from_u128(n)
    }
}

/// Task row, start, finish, duration, work, cost of one baseline entry.
type CanonBaseline = (usize, Moment, Moment, i64, i64, i64);

/// Depth, name, duration (None for summaries), milestone, constraint, % complete, notes.
type CanonRow = (usize, String, Option<i64>, bool, Constraint, u8, String, Option<Moment>);

/// Everything a round trip must keep, independent of ids and sort-key spelling.
#[derive(Debug, PartialEq)]
struct Canon {
    name: String,
    start: Moment,
    calendar: Calendar,
    rows: Vec<CanonRow>,
    links: Vec<(usize, usize, LinkType, i64)>,
    /// Name, initials, kind, max units (work), label (material), notes, in sheet order.
    resources: Vec<(String, String, ResourceKind, u32, String, String)>,
    /// Task row, resource name, units.
    assignments: Vec<(usize, String, u32)>,
    currency: String,
    /// Baseline entries by task row.
    baseline: Option<Vec<CanonBaseline>>,
    /// Fixed cost per task row, and rate per resource name.
    costs: (Vec<i64>, Vec<(String, i64)>),
    /// Actuals per work-task row, and the status date.
    progress: (Vec<Actual>, Option<Moment>),
}

fn canon(p: &Project) -> Canon {
    let rows = outline(p);
    let index: BTreeMap<TaskId, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    let mut links: Vec<_> = p.dependencies.values().map(|d| (index[&d.predecessor], index[&d.successor], d.kind, d.lag)).collect();
    links.sort_by_key(|l| (l.0, l.1));
    Canon {
        name: p.name.clone(),
        start: p.start,
        calendar: p.calendar.clone(),
        rows: rows
            .iter()
            .map(|r| {
                let t = &p.tasks[&r.id];
                // A summary's stored duration is unused; a milestone's is zero.
                let duration = (!r.has_children).then_some(if t.is_milestone { 0 } else { t.duration.get() });
                let milestone = t.is_milestone && !r.has_children;
                let manual = t.manual_start.filter(|_| !r.has_children);
                (r.depth, t.name.clone(), duration, milestone, t.constraint, t.percent_complete.get(), t.notes.clone(), manual)
            })
            .collect(),
        links,
        resources: {
            let mut list: Vec<&Resource> = p.resources.values().collect();
            list.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
            list.iter()
                .map(|r| {
                    let work = r.kind == ResourceKind::Work;
                    (r.name.clone(), r.initials.clone(), r.kind, if work { r.max_units.0 } else { 0 }, if work { String::new() } else { r.material_label.clone() }, r.notes.clone())
                })
                .collect()
        },
        assignments: {
            let mut list: Vec<(usize, String, u32)> = p.assignments.values().map(|a| (index[&a.task], p.resources[&a.resource].name.clone(), a.units.0)).collect();
            list.sort();
            list
        },
        currency: p.currency.clone(),
        baseline: p.baseline.as_ref().map(|b| {
            let mut v: Vec<_> = b.tasks.iter().filter_map(|(t, e)| index.get(t).map(|i| (*i, e.start, e.finish, e.duration, e.work, e.cost.0))).collect();
            v.sort();
            v
        }),
        costs: (
            rows.iter().map(|r| p.tasks[&r.id].fixed_cost.0).collect(),
            {
                let mut v: Vec<(String, i64)> = p.resources.values().map(|r| (r.name.clone(), r.rate.0)).collect();
                v.sort();
                v
            },
        ),
        progress: (rows.iter().map(|r| if r.has_children { Actual::NotStarted } else { p.tasks[&r.id].actual }).collect(), p.status_date),
    }
}

fn lines(r: &ImportReport) -> Vec<String> {
    r.approximated.iter().chain(&r.ignored).map(|l| l.what.clone()).collect()
}

const FIXTURE: &str = include_str!("../../../fixtures/mspdi/handwritten-msp2016-style.xml");

#[test]
fn fixture_imports_what_it_can_and_reports_the_rest() {
    let (p, report) = import_mspdi(FIXTURE, &mut ids()).unwrap();
    assert_eq!(p.name, "Website Relaunch");
    assert_eq!(p.currency, "€");
    let b = p.baseline.as_ref().expect("Baseline 0 of “Wireframes & sitemap” is imported");
    let wire = p.tasks.values().find(|t| t.name == "Wireframes & sitemap").unwrap().id;
    assert_eq!(b.tasks[&wire].duration, 2400);
    assert_eq!(p.tasks.values().find(|t| t.name == "Content migration").unwrap().fixed_cost, Money(125_050));
    assert_eq!(p.resources.values().find(|r| r.name == "Alex").unwrap().rate, Money(8_500));
    let names: Vec<(usize, String)> = outline(&p).iter().map(|r| (r.depth, p.tasks[&r.id].name.clone())).collect();
    assert_eq!(
        names,
        [(0, "Design"), (1, "Wireframes & sitemap"), (1, "Visual design"), (0, "Build"), (1, "Templates"), (1, "Content migration"), (0, "Go live 🚀")]
            .map(|(d, n)| (d, n.to_owned()))
    );
    let task = |name: &str| p.tasks.values().find(|t| t.name == name).unwrap();
    assert_eq!(task("Wireframes & sitemap").duration.get(), 2400);
    assert_eq!(task("Wireframes & sitemap").percent_complete.get(), 60);
    assert_eq!(task("Wireframes & sitemap").notes, "Agree with marketing first.");
    assert_eq!(task("Visual design").duration.get(), 960, "2 elapsed days → 2 working days");
    assert!(task("Go live 🚀").is_milestone);
    assert!(matches!(task("Go live 🚀").constraint, Constraint::MustStartOn(_)));
    assert!(matches!(task("Templates").constraint, Constraint::StartNoEarlierThan(_)));
    assert!(matches!(task("Content migration").constraint, Constraint::MustFinishOn(_)));
    assert_eq!(task("Visual design").constraint, Constraint::AsLateAsPossible);
    assert!(task("Templates").manual_start.is_none(), "Templates is manual but the file gives it no start");

    // Calendar: Friday afternoons off, Christmas off (three days), one working Saturday.
    assert_eq!(p.calendar.week()[4].working_minutes(), 240);
    assert!(!p.calendar.week()[5].is_working());
    let day = |y, m, d| time::day_number(jiff::civil::date(y, m, d));
    assert!(!p.calendar.pattern(day(2026, 12, 25)).is_working());
    assert!(!p.calendar.pattern(day(2026, 12, 26)).is_working());
    assert_eq!(p.calendar.pattern(day(2026, 10, 24)).working_minutes(), 240);

    // Links: percentage lag 50% of 2 days = 1 day; FF −1 day; elapsed 1 day → 1 working day.
    let link = |a: &str, b: &str| p.dependencies.values().find(|d| p.tasks[&d.predecessor].name == a && p.tasks[&d.successor].name == b);
    assert_eq!(link("Wireframes & sitemap", "Visual design").map(|d| (d.kind, d.lag)), Some((LinkType::FinishToStart, 240)));
    assert_eq!(link("Visual design", "Templates").map(|d| (d.kind, d.lag)), Some((LinkType::StartToStart, 480)));
    assert_eq!(link("Templates", "Content migration").map(|d| (d.kind, d.lag)), Some((LinkType::FinishToFinish, -480)));
    assert_eq!(link("Wireframes & sitemap", "Content migration").map(|d| d.lag), Some(480));
    // Resources and assignments (iteration 2): Sam and Alex work, Paper is a material; the
    // cost resource and the assignment on the "Build" summary are reported instead.
    let mut names: Vec<(&str, u32)> = p.resources.values().map(|r| (r.name.as_str(), r.max_units.0)).collect();
    names.sort();
    assert_eq!(names, [("Alex", 50), ("Paper", 0), ("Sam", 100)]);
    let assigned: Vec<(String, String, u32)> = {
        let mut v: Vec<_> = p.assignments.values().map(|a| (p.tasks[&a.task].name.clone(), p.resources[&a.resource].name.clone(), a.units.0)).collect();
        v.sort();
        v
    };
    assert_eq!(assigned, [("Go live 🚀".to_owned(), "Paper".to_owned(), 250), ("Wireframes & sitemap".to_owned(), "Sam".to_owned(), 100)]);
    // Links on summary tasks are kept (R25); the link to another project is not.
    assert_eq!(p.dependencies.len(), 7);
    assert!(link("Design", "Build").is_some(), "summary to summary");
    assert!(link("Build", "Go live 🚀").is_some(), "summary to milestone");
    assert!(schedule(&p).is_ok());

    let reported = lines(&report).join("\n");
    for expected in [
        "Elapsed durations",
        "Milestones with a duration",
        "Lags given as a percentage",
        "Elapsed lags",
        "Links to tasks in other projects",
        "Blank rows",
        "Custom fields",
        "Deadlines",
        "Task calendars",
        "Task priorities",
        "Cost resources",
        "Overtime rates and cost per use",
        "Assignments that Super PM cannot hold (resources work on the tasks inside a summary",
        "Other calendars",
        "Recurring calendar exceptions",
        "Alternative work weeks",
        "Project options Super PM does not use",
    ] {
        assert!(reported.contains(expected), "report should mention {expected:?}; it says:\n{reported}");
    }
    assert!(report.imported.iter().any(|l| l.what.starts_with("Resources") && l.count == 3));
    assert!(report.imported.iter().any(|l| l.what.starts_with("Actual start") && l.count == 1));
    assert_eq!(p.status_date, Some(Moment::from_datetime(jiff::civil::date(2026, 10, 16).at(17, 0, 0, 0))));
    let wire = p.tasks.values().find(|t| t.name == "Wireframes & sitemap").unwrap();
    assert_eq!(wire.actual, Actual::Started { start: Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0)) });
    assert!(report.imported.iter().any(|l| l.what.starts_with("Assignments") && l.count == 2));
    let options = report.ignored.iter().find(|l| l.what.starts_with("Project options")).unwrap();
    assert!(options.examples.contains(&"HonorConstraints".to_owned()));
    assert!(!options.examples.contains(&"CurrencySymbol".to_owned()), "the currency is imported now");
    assert!(report.recalculated.contains(&"TotalSlack".to_owned()) || report.recalculated.contains(&"Critical".to_owned()));
    assert!(report.imported.iter().any(|l| l.what.starts_with("Tasks") && l.count == 7));
    assert!(report.imported.iter().any(|l| l.what.starts_with("Links") && l.count == 7));
}

#[test]
fn export_then_import_keeps_everything_and_reports_nothing() {
    let (p, _) = import_mspdi(FIXTURE, &mut ids()).unwrap();
    let xml = export_mspdi(&p, schedule(&p).ok().as_ref());
    let (q, report) = import_mspdi(&xml, &mut ids()).unwrap();
    assert_eq!(canon(&q), canon(&p));
    assert!(lines(&report).is_empty(), "a Super PM export has nothing to approximate: {:?}", lines(&report));
    // And the second export is identical to the first.
    assert_eq!(export_mspdi(&q, schedule(&q).ok().as_ref()), xml);
}

#[test]
fn export_escapes_text() {
    let (mut p, _) = import_mspdi(FIXTURE, &mut ids()).unwrap();
    p.name = "R&D <Phase 2> \"quoted\"".into();
    let xml = export_mspdi(&p, None);
    assert!(xml.contains("<Title>R&amp;D &lt;Phase 2&gt; &quot;quoted&quot;</Title>"));
    let (q, _) = import_mspdi(&xml, &mut ids()).unwrap();
    assert_eq!(q.name, p.name);
}

fn random_project() -> impl Strategy<Value = Project> {
    let names = prop::sample::select(vec!["Plan", "R&D <1>", "“Quotes” & 'apostrophes'", "Ünïcødé 🚀", "Tab\there", "  spaced  ", ""]);
    let task = (names, 0usize..3, 0i64..5000, prop::bool::weighted(0.15), 0u8..=100, 0usize..8);
    let tasks = prop::collection::vec(task, 1..30);
    let links = prop::collection::vec((0usize..30, 0usize..30, 0usize..4, -2000i64..2000), 0..40);
    let holidays = prop::collection::vec((0i64..120, prop::bool::ANY), 0..6);
    let people = prop::collection::vec((prop::sample::select(vec!["Alex", "Ana María", "O'Brien", "Team <A>", "Crane & Co"]), 1u32..400, any::<bool>()), 0..5);
    let staffing = prop::collection::vec((0usize..30, 0usize..5, 1u32..300), 0..30);
    let money = (prop::sample::select(vec!["", "€", "$", "CHF"]), prop::collection::vec(-100_000i64..10_000_000, 30), prop::collection::vec(0i64..50_000, 5));
    (tasks, links, holidays, people, staffing, money).prop_map(|(tasks, links, holidays, people, staffing, (currency, fixed, rates))| {
        let mut next = ids();
        let mut p = Project {
            id: ProjectId(next()),
            name: "Random".into(),
            start: Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0)),
            calendar_id: CalendarId(next()),
            calendar: Calendar::standard(),
            tasks: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            resources: Default::default(),
            assignments: Default::default(),
            currency: String::new(),
            baseline: None,
            status_date: None,
        };
        for (offset, working) in holidays {
            let date = jiff::civil::date(2026, 10, 5).checked_add(jiff::Span::new().days(offset)).unwrap();
            let pattern = if working { DayPattern::new(vec![WorkRange::hours(9, 12).unwrap()]).unwrap() } else { DayPattern::non_working() };
            p.calendar.set_exception(date, pattern);
        }
        let mut created: Vec<(TaskId, usize)> = Vec::new();
        let mut last_key: BTreeMap<Option<TaskId>, SortKey> = BTreeMap::new();
        for (k, (name, depth, minutes, milestone, percent, constraint)) in tasks.into_iter().enumerate() {
            // A depth can only go one deeper than the task above.
            let depth = depth.min(created.last().map_or(0, |&(_, d)| d + 1));
            let parent = created.iter().rev().find(|&&(_, d)| d + 1 == depth).map(|&(id, _)| id).filter(|_| depth > 0);
            let id = TaskId(next());
            let key = SortKey::between(last_key.get(&parent), None);
            last_key.insert(parent, key.clone());
            let date = Moment::from_datetime(jiff::civil::date(2026, 10, 7 + (k % 10) as i8).at(8, 0, 0, 0));
            let task = Task {
                id,
                parent,
                sort_key: key,
                name: name.to_owned(),
                duration: Duration::minutes(if milestone { 0 } else { minutes }).unwrap(),
                is_milestone: milestone,
                constraint: [
                    Constraint::AsSoonAsPossible,
                    Constraint::StartNoEarlierThan(date),
                    Constraint::MustStartOn(date),
                    Constraint::AsLateAsPossible,
                    Constraint::StartNoLaterThan(date),
                    Constraint::FinishNoEarlierThan(date),
                    Constraint::FinishNoLaterThan(date),
                    Constraint::MustFinishOn(date),
                ][constraint],
                percent_complete: Percent::new(percent).unwrap(),
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(0),
                manual_start: None,
                actual: Actual::default(),
                notes: if k % 3 == 0 { format!("Note {k}: <b>&amp;</b>\nline two") } else { String::new() },
            };
            // Through the command layer, so the project is one the app could hold.
            if apply(&mut p, &Command::InsertTasks { tasks: vec![task], dependencies: vec![] }).is_ok() {
                created.push((id, depth));
            }
        }
        let kinds = [LinkType::FinishToStart, LinkType::StartToStart, LinkType::FinishToFinish, LinkType::StartToFinish];
        for (a, b, kind, lag) in links {
            let (Some(&(pa, _)), Some(&(pb, _))) = (created.get(a), created.get(b)) else { continue };
            let dependency = Dependency { id: DependencyId(next()), predecessor: pa, successor: pb, kind: kinds[kind], lag };
            let _ = apply(&mut p, &Command::InsertLink { dependency });
        }
        let mut resources = Vec::new();
        let mut last: Option<SortKey> = None;
        for (name, max, material) in people {
            if resources.iter().any(|r: &Resource| r.name == name) {
                continue;
            }
            let key = SortKey::between(last.as_ref(), None);
            last = Some(key.clone());
            resources.push(Resource {
                id: ResourceId(next()),
                sort_key: key,
                name: name.to_owned(),
                initials: name.chars().take(2).collect(),
                kind: if material { ResourceKind::Material } else { ResourceKind::Work },
                max_units: ResourceUnits(if material { 0 } else { max }),
                material_label: if material { "tons".into() } else { String::new() },
                rate: Money(0),
                notes: String::new(),
            });
        }
        for (r, rate) in resources.iter_mut().zip(&rates) {
            r.rate = Money(*rate);
        }
        p.currency = currency.to_owned();
        p.status_date = (currency == "€").then(|| Moment::from_datetime(jiff::civil::date(2026, 10, 8).at(17, 0, 0, 0)));
        for (k, (t, cost)) in p.tasks.values_mut().zip(&fixed).enumerate() {
            t.fixed_cost = Money(*cost);
            if k % 4 == 1 {
                t.manual_start = Some(Moment::from_datetime(jiff::civil::date(2026, 10, 6).at(8, 0, 0, 0)));
            }
            let day = |d| Moment::from_datetime(jiff::civil::date(2026, 10, d).at(8, 0, 0, 0));
            t.actual = match k % 5 {
                2 => Actual::Started { start: day(5) },
                3 => Actual::Finished { start: day(5), finish: day(7) },
                _ => Actual::NotStarted,
            };
        }
        if currency == "€" {
            // Some projects carry a baseline: one entry per task, with odd values.
            let tasks = p.tasks.keys().enumerate().map(|(k, id)| {
                (*id, BaselineTask { start: Moment(29_000_000 + k as i64 * 60), finish: Moment(29_001_000 + k as i64 * 60), duration: 480 + k as i64, work: 60 * k as i64, cost: Money(k as i64 * 101) })
            });
            p.baseline = Some(Baseline { saved_at: "imported".into(), tasks: tasks.collect() });
        }
        let ids: Vec<ResourceId> = resources.iter().map(|r| r.id).collect();
        let _ = apply(&mut p, &Command::InsertResources { resources, assignments: vec![] });
        for (t, r, units) in staffing {
            let (Some(&(task, _)), Some(&resource)) = (created.get(t), ids.get(r)) else { continue };
            let assignment = Assignment { id: AssignmentId(next()), task, resource, units: ResourceUnits(units) };
            let _ = apply(&mut p, &Command::InsertAssignment { assignment });
        }
        p
    })
}

proptest! {
    /// Spec section 3: export then import keeps tasks, hierarchy, durations, links, lags and
    /// calendars intact.
    #[test]
    fn round_trip_keeps_the_project(p in random_project()) {
        let xml = export_mspdi(&p, schedule(&p).ok().as_ref());
        let (q, report) = import_mspdi(&xml, &mut ids()).unwrap();
        prop_assert_eq!(canon(&q), canon(&p));
        prop_assert!(lines(&report).is_empty(), "{:?}", lines(&report));
    }
}
