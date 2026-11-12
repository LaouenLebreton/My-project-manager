use std::collections::{BTreeSet, HashSet};

use pm_core::*;
use proptest::prelude::*;
use uuid::Uuid;

use super::*;

fn task(id: TaskId, key: &str, name: &str) -> Task {
    Task {
        id,
        parent: None,
        sort_key: SortKey(key.into()),
        name: name.into(),
        duration: Duration::minutes(480).unwrap(),
        is_milestone: false,
        constraint: Constraint::AsSoonAsPossible,
        percent_complete: Percent::default(),
        leveling_delay: Duration::ZERO,
        fixed_cost: Money(0),
        manual_start: None,
        deadline: None,
        elapsed: false,
        actual: Actual::default(),
        notes: String::new(),
    }
}

fn base() -> Project {
    let mut p = Project {
        id: ProjectId(Uuid::from_u128(1)),
        name: "Shared".into(),
        start: Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0)),
        calendar_id: CalendarId(Uuid::from_u128(2)),
        calendar: Calendar::standard(),
        tasks: Default::default(),
        dependencies: Default::default(),
        resources: Default::default(),
        assignments: Default::default(),
        currency: String::new(),
        baseline: None,
        status_date: None,
    };
    for (n, key) in [(10u128, "B"), (11, "D"), (12, "F")] {
        let t = task(TaskId(Uuid::from_u128(n)), key, &format!("T{n}"));
        p.tasks.insert(t.id, t);
    }
    p
}

/// The server's side: one order for everything, refusing what does not apply.
struct Server {
    project: Project,
    log: Vec<Change>,
}

impl Server {
    fn submit(&mut self, author: usize, p: &Pending) -> Result<(), CommandError> {
        if self.log.iter().any(|c| c.client_change == p.id) {
            return Ok(()); // a resend after a lost reply
        }
        apply(&mut self.project, &p.command)?;
        self.log.push(Change {
            version: self.log.len() as i64 + 1,
            client_change: p.id,
            author: Uuid::from_u128(author as u128),
            author_name: format!("C{author}"),
            at: String::new(),
            label: p.command.label(),
            summary: String::new(),
            command: p.command.clone(),
        });
        Ok(())
    }
}

struct Client {
    current: Project,
    outbox: Outbox,
    sent: HashSet<Uuid>,
}

impl Client {
    fn local(&mut self, command: Command) {
        // Edits that do not apply locally are refused before they are ever sent.
        if let Ok(inverse) = apply(&mut self.current, &command) {
            self.outbox.push(command, inverse);
        }
    }

    fn receive(&mut self, change: &Change) {
        if let Received::Apply { delta, .. } = self.outbox.receive(&self.current, change).expect("changes apply in order") {
            apply(&mut self.current, &delta).expect("the delta applies to the copy it was made for");
        }
    }

    fn refused(&mut self, id: Uuid) {
        if let Some((delta, _)) = self.outbox.refused(&self.current, id) {
            apply(&mut self.current, &delta).expect("the delta applies");
        }
    }
}

#[derive(Clone, Debug)]
enum Step {
    Edit { client: usize, kind: u8, a: usize, b: usize },
    Send { client: usize },
    Deliver { client: usize },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0..3usize, 0..6u8, 0..8usize, 0..8usize).prop_map(|(client, kind, a, b)| Step::Edit { client, kind, a, b }),
        2 => (0..3usize).prop_map(|client| Step::Send { client }),
        2 => (0..3usize).prop_map(|client| Step::Deliver { client }),
    ]
}

/// A command a user could make on what they currently see.
fn edit(p: &Project, kind: u8, a: usize, b: usize, fresh: &mut u128) -> Option<Command> {
    let ids: Vec<TaskId> = p.tasks.keys().copied().collect();
    let pick = |i: usize| ids.get(i % ids.len().max(1)).copied();
    *fresh += 1;
    Some(match kind {
        0 => Command::SetTask { id: pick(a)?, change: TaskChange::Name(format!("n{fresh}")) },
        1 => Command::SetTask { id: pick(a)?, change: TaskChange::Duration(Duration::minutes(480 * (b as i64 + 1)).unwrap()) },
        2 => {
            let keys: BTreeSet<&SortKey> = p.tasks.values().filter(|t| t.parent.is_none()).map(|t| &t.sort_key).collect();
            let key = SortKey::between(keys.iter().last().copied(), None);
            Command::InsertTasks { tasks: vec![task(TaskId(Uuid::from_u128(1000 + *fresh)), &key.0, "new")], dependencies: vec![] }
        }
        3 => Command::DeleteTasks { ids: vec![pick(a)?] },
        4 => {
            let (x, y) = (pick(a)?, pick(b)?);
            if x == y || p.dependencies.values().any(|d| (d.predecessor, d.successor) == (x, y)) {
                return None;
            }
            Command::InsertLink { dependency: Dependency { id: DependencyId(Uuid::from_u128(5000 + *fresh)), predecessor: x, successor: y, kind: LinkType::FinishToStart, lag: 0, lag_format: pm_core::LagFormat::Working } }
        }
        _ => Command::Batch {
            label: "Change Task Info".into(),
            commands: vec![
                Command::SetTask { id: pick(a)?, change: TaskChange::Notes(format!("note {fresh}")) },
                Command::SetTask { id: pick(a)?, change: TaskChange::PercentComplete(Percent::new((b * 10) as u8).unwrap()) },
            ],
        },
    })
}

fn send(server: &mut Server, clients: &mut [Client], c: usize) {
    let next = clients[c].outbox.pending.iter().find(|p| !clients[c].sent.contains(&p.id)).cloned();
    if let Some(p) = next {
        clients[c].sent.insert(p.id);
        if server.submit(c, &p).is_err() {
            clients[c].refused(p.id);
        }
    }
}

fn deliver(server: &Server, clients: &mut [Client], c: usize) -> bool {
    let v = clients[c].outbox.version as usize;
    match server.log.get(v) {
        Some(change) => {
            clients[c].receive(change);
            true
        }
        None => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    #[test]
    fn every_copy_ends_up_like_the_server(steps in prop::collection::vec(step(), 1..60)) {
        let mut server = Server { project: base(), log: Vec::new() };
        let mut clients: Vec<Client> = (0..3).map(|_| Client { current: base(), outbox: Outbox::new(0), sent: HashSet::new() }).collect();
        let mut fresh = 0u128;
        for s in steps {
            match s {
                Step::Edit { client, kind, a, b } => {
                    if let Some(cmd) = edit(&clients[client].current, kind, a, b, &mut fresh) {
                        clients[client].local(cmd);
                    }
                }
                Step::Send { client } => send(&mut server, &mut clients, client),
                Step::Deliver { client } => {
                    deliver(&server, &mut clients, client);
                }
            }
        }
        // Everyone reconnects: send everything, hear everything.
        loop {
            let mut busy = false;
            for c in 0..3 {
                if clients[c].outbox.pending.iter().any(|p| !clients[c].sent.contains(&p.id)) {
                    send(&mut server, &mut clients, c);
                    busy = true;
                }
            }
            for c in 0..3 {
                while deliver(&server, &mut clients, c) {
                    busy = true;
                }
            }
            if !busy {
                break;
            }
        }
        for c in &clients {
            prop_assert!(c.outbox.pending.is_empty(), "every edit was confirmed or dropped");
            prop_assert_eq!(&c.current, &server.project);
            prop_assert_eq!(c.outbox.version, server.log.len() as i64);
        }
    }
}

fn change(version: i64, client_change: Uuid, command: Command) -> Change {
    Change { version, client_change, author: Uuid::nil(), author_name: "Alex".into(), at: String::new(), label: command.label(), summary: String::new(), command }
}

#[test]
fn own_edits_are_confirmed_and_others_rebased() {
    let t10 = TaskId(Uuid::from_u128(10));
    let mut c = Client { current: base(), outbox: Outbox::new(0), sent: HashSet::new() };
    c.local(Command::SetTask { id: t10, change: TaskChange::Name("Mine".into()) });
    let mine = c.outbox.pending[0].id;
    assert_eq!(c.current.tasks[&t10].name, "Mine");

    // Someone else renamed it first: theirs is applied, then ours on top (last writer wins).
    let theirs = change(1, Uuid::now_v7(), Command::SetTask { id: t10, change: TaskChange::Name("Theirs".into()) });
    let Ok(Received::Apply { delta, dropped }) = c.outbox.receive(&c.current, &theirs) else { panic!() };
    assert!(dropped.is_empty());
    apply(&mut c.current, &delta).unwrap();
    assert_eq!(c.current.tasks[&t10].name, "Mine");
    assert_eq!(c.outbox.receive(&c.current, &theirs), Ok(Received::Duplicate));

    let ours = change(2, mine, Command::SetTask { id: t10, change: TaskChange::Name("Mine".into()) });
    assert_eq!(c.outbox.receive(&c.current, &ours), Ok(Received::Confirmed(mine)));
    assert!(c.outbox.pending.is_empty());

    // A gap is reported, not guessed at.
    assert_eq!(c.outbox.receive(&c.current, &change(9, Uuid::now_v7(), Command::DeleteTasks { ids: vec![t10] })), Err(SyncError::Gap { expected: 3 }));
}

#[test]
fn an_edit_to_a_task_someone_deleted_is_dropped() {
    let t11 = TaskId(Uuid::from_u128(11));
    let mut c = Client { current: base(), outbox: Outbox::new(0), sent: HashSet::new() };
    c.local(Command::SetTask { id: t11, change: TaskChange::Name("Renamed".into()) });
    let mine = c.outbox.pending[0].id;
    let deleted = change(1, Uuid::now_v7(), Command::DeleteTasks { ids: vec![t11] });
    let Ok(Received::Apply { delta, dropped }) = c.outbox.receive(&c.current, &deleted) else { panic!() };
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].0.id, mine);
    apply(&mut c.current, &delta).unwrap();
    assert!(c.outbox.pending.is_empty());
    assert!(!c.current.tasks.contains_key(&t11));
}

#[test]
fn the_outbox_survives_a_restart() {
    let t10 = TaskId(Uuid::from_u128(10));
    let mut c = Client { current: base(), outbox: Outbox::new(4), sent: HashSet::new() };
    c.local(Command::SetTask { id: t10, change: TaskChange::Duration(Duration::minutes(960).unwrap()) });
    let saved = serde_json::to_string(&c.outbox).unwrap();
    assert_eq!(serde_json::from_str::<Outbox>(&saved).unwrap(), c.outbox);
}

#[test]
fn changes_travel_as_json() {
    let c = change(1, Uuid::now_v7(), Command::SetTask { id: TaskId(Uuid::from_u128(10)), change: TaskChange::Duration(Duration::minutes(960).unwrap()) });
    let text = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<Change>(&text).unwrap(), c);
}
