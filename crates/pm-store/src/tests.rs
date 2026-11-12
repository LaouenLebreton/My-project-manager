use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use pm_core::*;
use uuid::Uuid;

use super::*;

/// A fresh, empty directory for one test.
fn temp_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("pm-store-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn meta(n: u128) -> EntryMeta {
    EntryMeta { id: Uuid::from_u128(n), at: format!("2026-10-08T12:00:{:02}Z", n % 60) }
}

fn tid(n: u128) -> TaskId {
    TaskId(Uuid::from_u128(n))
}

fn task(n: u128) -> Task {
    Task {
        id: tid(n),
        parent: None,
        sort_key: SortKey(format!("{n:06}")),
        name: format!("Task {n}"),
        duration: Duration::minutes(480).unwrap(),
        is_milestone: false,
        constraint: Constraint::AsSoonAsPossible,
        percent_complete: Percent::default(),
        leveling_delay: Duration::ZERO,
        fixed_cost: Money(0),
        manual_start: None,
        deadline: None,
        elapsed: false,
        task_type: Default::default(),
        effort_driven: false,
        actual: Actual::default(),
        notes: String::new(),
    }
}

fn add(n: u128) -> Command {
    Command::InsertTasks { tasks: vec![task(n)], dependencies: vec![] }
}

fn rich_project() -> Project {
    let mut calendar = Calendar::standard();
    calendar.set_exception(jiff::civil::date(2026, 12, 25), DayPattern::non_working());
    let mut p = Project {
        id: ProjectId(Uuid::from_u128(7)),
        name: "Office move".into(),
        currency: "€".into(),
        baseline: None,
        status_date: Some(Moment(29_001_000)),
        start: Moment(29_000_000),
        calendar_id: CalendarId(Uuid::from_u128(8)),
        calendar,
        tasks: BTreeMap::new(),
        dependencies: BTreeMap::new(),
        resources: Default::default(),
        assignments: Default::default(),
    };
    let mut summary = task(1);
    summary.name = "Phase 1 – Planning ✓".into();
    let mut child = task(2);
    child.parent = Some(tid(1));
    child.constraint = Constraint::MustStartOn(Moment(29_001_000));
    child.percent_complete = Percent::new(40).unwrap();
    child.notes = "Ask facilities\nabout keys".into();
    let mut milestone = task(3);
    milestone.is_milestone = true;
    milestone.constraint = Constraint::AsLateAsPossible;
    let mut late = task(4);
    late.constraint = Constraint::StartNoEarlierThan(Moment(29_002_000));
    late.leveling_delay = Duration::minutes(240).unwrap();
    late.fixed_cost = Money(150_000);
    let mut deadline = task(5);
    deadline.constraint = Constraint::FinishNoLaterThan(Moment(29_003_000));
    deadline.manual_start = Some(Moment(29_000_480));
    late.actual = Actual::Started { start: Moment(29_002_100) };
    child.actual = Actual::Finished { start: Moment(29_000_000), finish: Moment(29_000_900) };
    for t in [summary, child, milestone, late, deadline] {
        p.tasks.insert(t.id, t);
    }
    let d = Dependency { id: DependencyId(Uuid::from_u128(50)), predecessor: tid(2), successor: tid(3), kind: LinkType::StartToFinish, lag: -240, lag_format: pm_core::LagFormat::Working };
    p.dependencies.insert(d.id, d);
    p
}

fn staff(p: &mut Project) {
    let r = Resource {
        id: ResourceId(Uuid::from_u128(70)),
        sort_key: SortKey("V".into()),
        name: "Alex Müller".into(),
        initials: "AM".into(),
        kind: ResourceKind::Work,
        max_units: ResourceUnits(150),
        material_label: String::new(),
        notes: "Part-time Fridays".into(),
        availability: Default::default(),
        rate: Money(4_550),
    };
    let m = Resource {
        id: ResourceId(Uuid::from_u128(71)),
        sort_key: SortKey("W".into()),
        name: "Concrete".into(),
        initials: "C".into(),
        kind: ResourceKind::Material,
        max_units: ResourceUnits(0),
        material_label: "tons".into(),
        rate: Money(0),
        notes: String::new(),
        availability: Default::default(),
    };
    let a = Assignment { id: AssignmentId(Uuid::from_u128(80)), task: tid(4), resource: r.id, units: ResourceUnits(50) };
    let b = Assignment { id: AssignmentId(Uuid::from_u128(81)), task: tid(4), resource: m.id, units: ResourceUnits(250) };
    pm_core::apply(p, &Command::InsertResources { resources: vec![r, m], assignments: vec![a, b] }).unwrap();
}

#[test]
fn resources_and_assignments_are_saved_and_restored() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    staff(&mut p);
    drop(Store::create(&path, &p).unwrap());
    let (mut s, mut loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded, p);
    s.apply(&mut loaded, &Command::SetAssignmentUnits { id: AssignmentId(Uuid::from_u128(80)), units: ResourceUnits(75) }, &meta(1)).unwrap();
    s.apply(&mut loaded, &Command::DeleteResources { ids: vec![ResourceId(Uuid::from_u128(71))] }, &meta(2)).unwrap();
    drop(s);
    let (mut s, mut again) = Store::open(&path).unwrap();
    assert_eq!(again, loaded);
    assert_eq!(again.assignments.len(), 1);
    s.undo(&mut again, &meta(3)).unwrap();
    assert_eq!(again.resources.len(), 2, "undo brings back the material and its assignment");
    assert_eq!(again.assignments.len(), 2);
}

#[test]
fn the_baseline_is_saved_restored_and_undone() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    let mut tasks = BTreeMap::new();
    tasks.insert(tid(4), BaselineTask { start: Moment(100), finish: Moment(900), duration: 480, work: 240, cost: Money(1_000) });
    let b = Baseline { saved_at: "2026-10-09T10:00:00Z".into(), tasks };
    s.apply(&mut p, &Command::SetProject { change: ProjectChange::Baseline(Some(b.clone())) }, &meta(1)).unwrap();
    s.apply(&mut p, &Command::SetProject { change: ProjectChange::Name("Renamed".into()) }, &meta(2)).unwrap();
    drop(s);
    let (mut s, mut loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded.baseline, Some(b));
    assert_eq!(loaded, p);
    s.undo(&mut loaded, &meta(3)).unwrap();
    s.undo(&mut loaded, &meta(4)).unwrap();
    assert_eq!(loaded.baseline, None);
    drop(s);
    assert_eq!(Store::open(&path).unwrap().1.baseline, None);
}

#[test]
fn version_1_files_are_upgraded() {
    let path = temp_dir().join("old.pmproj");
    let p = rich_project();
    drop(Store::create(&path, &p).unwrap());
    // Turn it back into a version 1 file, as an iteration 1 build would have written it.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE assignment; DROP TABLE resource; ALTER TABLE task DROP COLUMN leveling_delay_min; ALTER TABLE task DROP COLUMN fixed_cost_cents; ALTER TABLE project DROP COLUMN currency; ALTER TABLE task DROP COLUMN manual_start_min; ALTER TABLE project DROP COLUMN baseline_saved_at; DROP TABLE baseline_task; ALTER TABLE task DROP COLUMN actual_start_min; ALTER TABLE task DROP COLUMN actual_finish_min; ALTER TABLE project DROP COLUMN status_date_min; ALTER TABLE task DROP COLUMN deadline_min; ALTER TABLE task DROP COLUMN elapsed; ALTER TABLE dependency DROP COLUMN lag_format; ALTER TABLE task DROP COLUMN task_type; ALTER TABLE task DROP COLUMN effort_driven; PRAGMA user_version = 1;").unwrap();
    drop(conn);
    let (mut s, mut loaded) = Store::open(&path).unwrap();
    // Version 1 had no leveling delays; they come back as zero.
    let mut expected = p.clone();
    for t in expected.tasks.values_mut() {
        t.leveling_delay = Duration::ZERO;
        t.fixed_cost = Money(0);
        t.manual_start = None;
        t.actual = Actual::NotStarted;
        t.deadline = None;
    }
    expected.currency = String::new();
    expected.status_date = None;
    assert_eq!(loaded, expected, "everything from version 1 is still there");
    let v: i64 = s.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v, CURRENT_VERSION);
    staff(&mut loaded.clone()); // the new tables accept data
    let r = Resource {
        id: ResourceId(Uuid::from_u128(90)),
        sort_key: SortKey("V".into()),
        name: "New".into(),
        initials: "N".into(),
        kind: ResourceKind::Work,
        max_units: ResourceUnits::FULL_TIME,
        material_label: String::new(),
        rate: Money(0),
        notes: String::new(),
        availability: Default::default(),
    };
    s.apply(&mut loaded, &Command::InsertResources { resources: vec![r], assignments: vec![] }, &meta(1)).unwrap();
}

#[test]
fn create_then_open_restores_everything() {
    let path = temp_dir().join("a.pmproj");
    let p = rich_project();
    drop(Store::create(&path, &p).unwrap());
    let (_, loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded, p);
}

#[test]
fn create_refuses_to_overwrite() {
    let path = temp_dir().join("a.pmproj");
    std::fs::write(&path, b"precious").unwrap();
    assert!(matches!(Store::create(&path, &rich_project()), Err(StoreError::AlreadyExists)));
    assert_eq!(std::fs::read(&path).unwrap(), b"precious");
}

#[test]
fn edits_are_saved_immediately() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    s.apply(&mut p, &add(10), &meta(1)).unwrap();
    s.apply(&mut p, &Command::SetTask { id: tid(10), change: TaskChange::Name("Pack".into()) }, &meta(2)).unwrap();
    s.apply(&mut p, &Command::DeleteTasks { ids: vec![tid(1)] }, &meta(3)).unwrap();
    s.apply(&mut p, &Command::SetProject { change: ProjectChange::Name("Renamed".into()) }, &meta(4)).unwrap();
    // No save step: a second connection sees the same state while the first is still open.
    let (_, loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded, p);
    assert!(!p.tasks.contains_key(&tid(2)), "deleting the summary deleted its child");
}

#[test]
fn undo_and_redo_survive_reopening() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let original = p.clone();
    {
        let mut s = Store::create(&path, &p).unwrap();
        s.apply(&mut p, &add(10), &meta(1)).unwrap();
        let dur = Command::SetTask { id: tid(10), change: TaskChange::Duration(Duration::minutes(60).unwrap()) };
        s.apply(&mut p, &dur, &meta(2)).unwrap();
        assert_eq!(s.undo_label().unwrap().as_deref(), Some("Change Duration"));
    }
    let (mut s, mut p) = Store::open(&path).unwrap();
    assert_eq!(s.undo_label().unwrap().as_deref(), Some("Change Duration"));
    let undone = s.undo(&mut p, &meta(3)).unwrap().unwrap();
    assert_eq!((undone.label.as_str(), undone.origin), ("Change Duration", Origin::Undo));
    assert_eq!(p.tasks[&tid(10)].duration.get(), 480);
    s.undo(&mut p, &meta(4)).unwrap().unwrap();
    assert_eq!(p, original);
    assert!(s.undo(&mut p, &meta(5)).unwrap().is_none(), "nothing left to undo");
    assert_eq!(s.redo_label().unwrap().as_deref(), Some("Add Task"));
    drop(s);

    let (mut s, mut p) = Store::open(&path).unwrap();
    assert_eq!(p, original);
    s.redo(&mut p, &meta(6)).unwrap().unwrap();
    s.redo(&mut p, &meta(7)).unwrap().unwrap();
    assert_eq!(p.tasks[&tid(10)].duration.get(), 60);
    assert!(s.redo(&mut p, &meta(8)).unwrap().is_none());
    let (_, loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded, p);
}

#[test]
fn a_new_edit_clears_redo() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    s.apply(&mut p, &add(10), &meta(1)).unwrap();
    s.undo(&mut p, &meta(2)).unwrap();
    assert!(s.redo_label().unwrap().is_some());
    s.apply(&mut p, &add(11), &meta(3)).unwrap();
    assert!(s.redo_label().unwrap().is_none());
    assert_eq!(s.undo_label().unwrap().as_deref(), Some("Add Task"));
}

#[test]
fn undo_history_is_capped() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    let extra = 5;
    for n in 0..(UNDO_LIMIT as u128 + extra) {
        s.apply(&mut p, &add(1_000 + n), &meta(10_000 + n)).unwrap();
    }
    let mut undone = 0;
    while s.undo(&mut p, &meta(100_000 + undone)).unwrap().is_some() {
        undone += 1;
    }
    assert_eq!(undone, UNDO_LIMIT as u128);
    assert_eq!(p.tasks.len(), rich_project().tasks.len() + extra as usize, "the oldest edits stay");
}

#[test]
fn refused_command_changes_nothing() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    let before = (p.clone(), s.last_seq().unwrap());
    let loop_link = Dependency { id: DependencyId(Uuid::from_u128(51)), predecessor: tid(3), successor: tid(2), kind: LinkType::FinishToStart, lag: 0, lag_format: pm_core::LagFormat::Working };
    let err = s.apply(&mut p, &Command::InsertLink { dependency: loop_link }, &meta(1)).unwrap_err();
    assert!(matches!(err, StoreError::Command(CommandError::DuplicateLink { .. })), "{err}");
    assert_eq!((p.clone(), s.last_seq().unwrap()), before);
    let (_, loaded) = Store::open(&path).unwrap();
    assert_eq!(loaded, p);
}

#[test]
fn change_log_records_every_change_in_order() {
    let path = temp_dir().join("a.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    s.apply(&mut p, &add(10), &meta(1)).unwrap();
    s.undo(&mut p, &meta(2)).unwrap();
    s.redo(&mut p, &meta(3)).unwrap();
    let rows: Vec<(i64, String, Option<i64>, String)> = s
        .conn
        .prepare("SELECT seq, origin, target_seq, entry_id FROM change_log ORDER BY seq")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["edit", "undo", "redo"]);
    assert_eq!(rows[1].2, Some(rows[0].0));
    assert_eq!(rows[2].3, Uuid::from_u128(3).to_string());
}

#[test]
fn rejects_files_that_are_not_projects() {
    let dir = temp_dir();
    let text = dir.join("notes.pmproj");
    std::fs::write(&text, "just some text, not a database at all ......................................................").unwrap();
    assert!(matches!(Store::open(&text), Err(StoreError::NotAProject)));

    let other = dir.join("other.sqlite");
    rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE x (y)").unwrap();
    assert!(matches!(Store::open(&other), Err(StoreError::NotAProject)));
}

#[test]
fn refuses_files_from_a_newer_version() {
    let path = temp_dir().join("a.pmproj");
    drop(Store::create(&path, &rich_project()).unwrap());
    rusqlite::Connection::open(&path).unwrap().pragma_update(None, "user_version", CURRENT_VERSION + 1).unwrap();
    match Store::open(&path) {
        Err(StoreError::NewerVersion { found, supported }) => assert_eq!((found, supported), (CURRENT_VERSION + 1, CURRENT_VERSION)),
        other => panic!("expected NewerVersion, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn reopening_keeps_the_schema_version() {
    let path = temp_dir().join("a.pmproj");
    drop(Store::create(&path, &rich_project()).unwrap());
    drop(Store::open(&path).unwrap());
    let v: i64 = rusqlite::Connection::open(&path).unwrap().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v, CURRENT_VERSION);
}

#[test]
fn save_a_copy_is_a_complete_standalone_file() {
    let dir = temp_dir();
    let mut p = rich_project();
    let mut s = Store::create(&dir.join("a.pmproj"), &p).unwrap();
    s.apply(&mut p, &add(10), &meta(1)).unwrap();
    let copy = dir.join("copy.pmproj");
    s.save_copy(&copy).unwrap();
    drop(s);
    std::fs::remove_file(dir.join("a.pmproj")).unwrap();
    let (s2, loaded) = Store::open(&copy).unwrap();
    assert_eq!(loaded, p);
    assert_eq!(s2.undo_label().unwrap().as_deref(), Some("Add Task"));
}

/// Writer half of the crash test. Runs only when the crash test starts it.
#[test]
#[ignore]
fn crash_writer_child() {
    let Ok(path) = std::env::var("PM_STORE_CRASH_FILE") else { return };
    let (mut s, mut p) = Store::open(std::path::Path::new(&path)).unwrap();
    // Continue numbering after earlier rounds so ids stay unique.
    // Writes until the crash test kills this process.
    let mut n = p.tasks.len() as u128;
    loop {
        s.apply(&mut p, &add(1_000_000 + n), &meta(1_000_000 + n)).unwrap();
        n += 1;
    }
}

/// Spec section 6: kill the process during writes, reopen, and find a consistent file in
/// which every logged command is in the data and nothing else is.
#[test]
fn killed_writer_leaves_a_consistent_file() {
    let path = temp_dir().join("crash.pmproj");
    let base = rich_project();
    drop(Store::create(&path, &base).unwrap());
    for round in 0..3 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "tests::crash_writer_child", "--test-threads=1"])
            .env("PM_STORE_CRASH_FILE", &path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300 + 150 * round));
        child.kill().unwrap();
        child.wait().unwrap();

        let (s, p) = Store::open(&path).unwrap();
        let check: String = s.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");
        let edits: i64 = s.conn.query_row("SELECT COUNT(*) FROM change_log WHERE origin = 'edit'", [], |r| r.get(0)).unwrap();
        assert!(edits > 0, "round {round}: the writer made progress");
        assert_eq!(p.tasks.len() as i64, base.tasks.len() as i64 + edits, "round {round}: data matches the log");
    }
}

#[test]
fn team_changes_are_kept_out_of_undo_and_the_outbox_saved_with_edits() {
    let path = temp_dir().join("shared.pmproj");
    let mut p = rich_project();
    let mut s = Store::create(&path, &p).unwrap();
    let t2 = TaskId(Uuid::from_u128(2));
    // A local edit, saved with the outbox that lists it.
    let rename = Command::SetTask { id: t2, change: TaskChange::Name("Mine".into()) };
    let outbox = |c: &Command, inv: &Command| format!("{{\"cmd\":{},\"inv\":{}}}", serde_json::to_string(c).unwrap(), serde_json::to_string(inv).unwrap());
    let applied = s.apply_with(&mut p, &rename, &meta(1), Some(&outbox)).unwrap();
    assert_eq!(applied.command, rename);
    assert!(s.meta(TEAM_OUTBOX).unwrap().unwrap().contains("Mine"));

    // A change from the team: applied and saved, but Undo still undoes the local edit.
    let theirs = Command::SetTask { id: t2, change: TaskChange::Notes("from Alex".into()) };
    s.apply_remote(&mut p, &theirs, &meta(2), "{}").unwrap();
    assert_eq!(s.meta(TEAM_OUTBOX).unwrap().as_deref(), Some("{}"));
    assert_eq!(s.undo_label().unwrap().as_deref(), Some("Rename Task"));
    drop(s);
    let (mut s, mut reopened) = Store::open(&path).unwrap();
    assert_eq!(reopened, p);
    s.undo(&mut reopened, &meta(3)).unwrap();
    assert_eq!(reopened.tasks[&t2].notes, "from Alex", "the team's change stays");
    assert_ne!(reopened.tasks[&t2].name, "Mine");

    s.set_meta(TEAM_SERVER, Some("https://pm.example.com")).unwrap();
    assert_eq!(s.meta(TEAM_SERVER).unwrap().as_deref(), Some("https://pm.example.com"));
    s.set_meta(TEAM_SERVER, None).unwrap();
    assert_eq!(s.meta(TEAM_SERVER).unwrap(), None);
}

#[test]
fn the_undo_history_lists_steps_newest_first() {
    let path = temp_dir().join("h.pmproj");
    drop(Store::create(&path, &rich_project()).unwrap());
    let (mut s, mut p) = Store::open(&path).unwrap();
    for n in 1..=3 {
        s.apply(&mut p, &add(200 + n), &meta(n)).unwrap();
    }
    s.undo(&mut p, &meta(4)).unwrap();
    let steps = s.history(50).unwrap();
    assert_eq!(steps.iter().map(|x| x.undone).collect::<Vec<_>>(), [true, false, false], "the undone step (redo) first, then what can be undone");
    assert_eq!(steps[1].at, meta(2).at, "each step keeps when it was done");
    assert!(steps.iter().all(|x| !x.label.is_empty()));
    assert_eq!(s.history(1).unwrap().len(), 1);
}

#[test]
fn deadlines_are_saved() {
    let path = temp_dir().join("d.pmproj");
    drop(Store::create(&path, &rich_project()).unwrap());
    let (mut s, mut p) = Store::open(&path).unwrap();
    let id = *p.tasks.keys().next().unwrap();
    s.apply(&mut p, &Command::SetTask { id, change: pm_core::TaskChange::Deadline(Some(Moment(29_000_000))) }, &meta(1)).unwrap();
    drop(s);
    let (_, again) = Store::open(&path).unwrap();
    assert_eq!(again.tasks[&id].deadline, Some(Moment(29_000_000)));
}

#[test]
fn elapsed_durations_and_lag_formats_are_saved() {
    let path = temp_dir().join("e.pmproj");
    let mut p = rich_project();
    for t in p.tasks.values_mut() {
        t.elapsed = true;
        t.task_type = pm_core::TaskType::FixedWork;
        t.effort_driven = true;
    }
    for d in p.dependencies.values_mut() {
        d.lag_format = pm_core::LagFormat::Percent;
    }
    drop(Store::create(&path, &p).unwrap());
    let (_, again) = Store::open(&path).unwrap();
    assert_eq!(again, p);
}

#[test]
fn a_persons_working_time_is_saved() {
    let path = temp_dir().join("w.pmproj");
    let mut p = rich_project();
    staff(&mut p);
    let id = *p.resources.keys().next().unwrap();
    let a = pm_core::Availability { off_weekdays: [false, false, false, false, true, false, false], days_off: [20_400, 20_401].into_iter().collect() };
    drop(Store::create(&path, &p).unwrap());
    let (mut s, mut loaded) = Store::open(&path).unwrap();
    s.apply(&mut loaded, &Command::SetResource { id, change: pm_core::ResourceChange::Availability(a.clone()) }, &meta(1)).unwrap();
    drop(s);
    let (_, again) = Store::open(&path).unwrap();
    assert_eq!(again.resources[&id].availability, a);
}
