//! The open project: file, in-memory model and current schedule.

use std::path::{Path, PathBuf};

use std::cell::RefCell;
use std::sync::Arc;

use pm_core::{loads, schedule, Command, Loads, Moment, Project, Schedule, ScheduleError};
use pm_store::{Applied, EntryMeta, Store, StoreError, TEAM_OUTBOX, TEAM_SERVER};
use pm_sync::{Change, Outbox, Received, SyncError};

/// Why a change from the server could not be taken in.
#[derive(Debug)]
pub enum ReceiveError {
    /// Changes from this version on are missing; fetch them, then try again.
    Gap(i64),
    /// This copy no longer matches the server's and must be downloaded again.
    Diverged(String),
}
use uuid::Uuid;

/// New ids are UUID v7: sortable and safe to create offline (spec 4.3).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

fn entry_meta() -> EntryMeta {
    EntryMeta { id: new_id(), at: jiff::Timestamp::now().to_string() }
}

/// Monday of the current week at 08:00, local time.
pub fn this_monday_morning() -> Moment {
    let today = jiff::Zoned::now().date();
    let monday = today
        .checked_sub(jiff::Span::new().days(i64::from(today.weekday().to_monday_zero_offset())))
        .unwrap_or(today);
    Moment::from_datetime(monday.at(8, 0, 0, 0))
}

pub struct Document {
    pub path: PathBuf,
    store: Store,
    pub project: Project,
    pub schedule: Result<Schedule, ScheduleError>,
    /// Resource loads for the current schedule (empty while it cannot be scheduled).
    pub loads: Loads,
    /// Change-log position of the last backup, so unchanged projects are not copied again.
    pub backed_up_seq: Option<i64>,
    /// Set for projects shared through a team server (iteration 3).
    pub team: Option<TeamLink>,
}

/// A shared project's link to its team server.
pub struct TeamLink {
    /// Base URL, e.g. `https://pm.example.com`.
    pub server: String,
    /// Unsent edits and the last server version this copy includes.
    pub outbox: Outbox,
    /// Wakes the sync task when there is something to send.
    pub wake: Arc<tokio::sync::Notify>,
}

/// What a change from the team did here, for the window to report.
pub struct TeamUpdate {
    /// Labels of the user's own unsent edits that no longer fit and were dropped.
    pub dropped: Vec<String>,
    /// The copy changed (the window should redraw).
    pub changed: bool,
}

fn outbox_json(o: &Outbox) -> String {
    serde_json::to_string(o).expect("outboxes serialize")
}

impl Document {
    pub fn open(path: &Path) -> Result<Document, StoreError> {
        let (store, project) = Store::open(path)?;
        Ok(Document::new(path, store, project))
    }

    pub fn create(path: &Path, project: Project) -> Result<Document, StoreError> {
        let store = Store::create(path, &project)?;
        Ok(Document::new(path, store, project))
    }

    fn new(path: &Path, store: Store, project: Project) -> Document {
        let schedule = schedule(&project);
        let loads = schedule.as_ref().map(|s| loads(&project, s)).unwrap_or_default();
        let team = store.meta(TEAM_SERVER).ok().flatten().map(|server| TeamLink {
            server,
            outbox: store.meta(TEAM_OUTBOX).ok().flatten().and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default(),
            wake: Arc::default(),
        });
        Document { path: path.to_owned(), store, project, schedule, loads, backed_up_seq: None, team }
    }

    /// Runs a store operation; for shared projects the edit also goes into the outbox, saved in
    /// the same transaction.
    fn edit<T>(&mut self, op: impl FnOnce(&mut Store, &mut Project, Option<pm_store::OutboxFn>) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let Some(team) = &self.team else { return op(&mut self.store, &mut self.project, None) };
        let next = RefCell::new(None);
        let base = team.outbox.clone();
        let record = |c: &Command, inv: &Command| {
            let mut o = base.clone();
            o.push(c.clone(), inv.clone());
            let json = outbox_json(&o);
            *next.borrow_mut() = Some(o);
            json
        };
        let result = op(&mut self.store, &mut self.project, Some(&record));
        if let (Ok(_), Some(o), Some(team)) = (&result, next.take(), self.team.as_mut()) {
            team.outbox = o;
            team.wake.notify_one();
        }
        result
    }

    pub fn apply(&mut self, cmd: &Command) -> Result<Applied, StoreError> {
        let meta = entry_meta();
        let applied = self.edit(|store, project, outbox| store.apply_with(project, cmd, &meta, outbox))?;
        self.reschedule();
        Ok(applied)
    }

    pub fn undo(&mut self) -> Result<Option<Applied>, StoreError> {
        let meta = entry_meta();
        let applied = self.edit(|store, project, outbox| store.undo_with(project, &meta, outbox))?;
        self.reschedule();
        Ok(applied)
    }

    pub fn redo(&mut self) -> Result<Option<Applied>, StoreError> {
        let meta = entry_meta();
        let applied = self.edit(|store, project, outbox| store.redo_with(project, &meta, outbox))?;
        self.reschedule();
        Ok(applied)
    }

    /// Links this project to a team server, starting from `version` with nothing unsent.
    pub fn link_team(&mut self, server: &str, version: i64) -> Result<(), StoreError> {
        let outbox = Outbox::new(version);
        self.store.set_meta(TEAM_SERVER, Some(server))?;
        self.store.set_meta(TEAM_OUTBOX, Some(&outbox_json(&outbox)))?;
        self.team = Some(TeamLink { server: server.to_owned(), outbox, wake: Arc::default() });
        Ok(())
    }

    /// A change from the server. `Err(Gap)` asks for the missing changes first; `Err(Diverged)`
    /// means this copy must be downloaded again.
    pub fn receive(&mut self, change: &Change) -> Result<TeamUpdate, ReceiveError> {
        let Some(team) = self.team.as_mut() else { return Ok(TeamUpdate { dropped: vec![], changed: false }) };
        let mut outbox = team.outbox.clone();
        let received = outbox.receive(&self.project, change).map_err(|e| match e {
            SyncError::Gap { expected } => ReceiveError::Gap(expected),
            SyncError::Diverged(e) => ReceiveError::Diverged(e.to_string()),
        })?;
        let result = match received {
            Received::Duplicate => return Ok(TeamUpdate { dropped: vec![], changed: false }),
            Received::Confirmed(_) => {
                // Nothing changes in the copy; only the outbox shrinks.
                let _ = self.store.set_meta(TEAM_OUTBOX, Some(&outbox_json(&outbox)));
                TeamUpdate { dropped: vec![], changed: false }
            }
            Received::Apply { delta, dropped } => {
                self.store
                    .apply_remote(&mut self.project, &delta, &entry_meta(), &outbox_json(&outbox))
                    .map_err(|e| ReceiveError::Diverged(e.to_string()))?;
                TeamUpdate { dropped: dropped.into_iter().map(|(p, _)| p.command.label()).collect(), changed: true }
            }
        };
        if let Some(team) = self.team.as_mut() {
            team.outbox = outbox;
        }
        if result.changed {
            self.reschedule();
        }
        Ok(result)
    }

    /// The server refused an unsent edit: take it out of this copy. Returns whether the copy
    /// changed, and the labels of later edits that no longer fit.
    pub fn refused(&mut self, id: Uuid) -> (bool, Vec<String>) {
        let Some(team) = self.team.as_mut() else { return (false, vec![]) };
        let mut outbox = team.outbox.clone();
        let Some((delta, dropped)) = outbox.refused(&self.project, id) else { return (false, vec![]) };
        if self.store.apply_remote(&mut self.project, &delta, &entry_meta(), &outbox_json(&outbox)).is_err() {
            return (false, vec![]);
        }
        if let Some(team) = self.team.as_mut() {
            team.outbox = outbox;
        }
        self.reschedule();
        (true, dropped.into_iter().map(|(p, _)| p.command.label()).collect())
    }

    pub fn undo_label(&self) -> Option<String> {
        self.store.undo_label().ok().flatten()
    }

    pub fn redo_label(&self) -> Option<String> {
        self.store.redo_label().ok().flatten()
    }

    pub fn last_seq(&self) -> i64 {
        self.store.last_seq().unwrap_or(0)
    }

    pub fn save_copy(&self, path: &Path) -> Result<(), StoreError> {
        self.store.save_copy(path)
    }

    fn reschedule(&mut self) {
        self.schedule = schedule(&self.project);
        self.loads = self.schedule.as_ref().map(|s| loads(&self.project, s)).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::TaskChange;

    fn change(version: i64, command: Command) -> Change {
        Change { version, client_change: Uuid::now_v7(), author: Uuid::nil(), author_name: "Alex".into(), at: String::new(), label: command.label(), summary: String::new(), command }
    }

    #[test]
    fn a_shared_document_keeps_unsent_edits_and_takes_in_the_team_changes() {
        let dir = std::env::temp_dir().join(format!("sp-doc-team-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.pmproj");
        let mut doc = Document::create(&path, crate::sample::office_move(Moment(29_000_000))).unwrap();
        doc.link_team("https://pm.example.com", 0).unwrap();
        let mut ids = doc.project.tasks.keys().copied();
        let (a, b) = (ids.next().unwrap(), ids.next().unwrap());

        doc.apply(&Command::SetTask { id: a, change: TaskChange::Name("Mine".into()) }).unwrap();
        assert_eq!(doc.team.as_ref().unwrap().outbox.pending.len(), 1);

        // Reopened offline: the unsent edit is still there.
        drop(doc);
        let mut doc = Document::open(&path).unwrap();
        let team = doc.team.as_ref().unwrap();
        assert_eq!((team.server.as_str(), team.outbox.pending.len()), ("https://pm.example.com", 1));

        // A teammate's change arrives: applied under the unsent edit, which stays.
        let update = doc.receive(&change(1, Command::SetTask { id: b, change: TaskChange::Name("Theirs".into()) })).ok().unwrap();
        assert!(update.changed && update.dropped.is_empty());
        assert_eq!((doc.project.tasks[&a].name.as_str(), doc.project.tasks[&b].name.as_str()), ("Mine", "Theirs"));
        assert!(matches!(doc.receive(&change(5, Command::DeleteTasks { ids: vec![b] })), Err(ReceiveError::Gap(2))));

        // The server refuses the edit: it is taken out again.
        let id = doc.team.as_ref().unwrap().outbox.pending[0].id;
        let (changed_copy, _) = doc.refused(id);
        assert!(changed_copy);
        assert_ne!(doc.project.tasks[&a].name, "Mine");
        assert!(doc.team.as_ref().unwrap().outbox.pending.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
