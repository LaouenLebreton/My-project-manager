//! SQLite persistence and change log for `.pmproj` files (spec 4.7).
//!
//! One file per project. Every command is applied to the in-memory project and written,
//! with its change-log entry, in a single transaction. If the write fails, the in-memory
//! change is reverted, so memory and disk never disagree.

mod rows;
mod schema;

use std::path::Path;

use pm_core::{Command, CommandError, Project};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use uuid::Uuid;

pub use schema::CURRENT_VERSION;

/// How many edits can be undone, including after reopening the file.
pub const UNDO_LIMIT: i64 = 500;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("this file is not a Super PM project")]
    NotAProject,
    #[error("this project was saved by a newer version of Super PM (file version {found}, this app reads up to {supported}); update the app to open it")]
    NewerVersion { found: i64, supported: i64 },
    #[error("a file already exists at this location")]
    AlreadyExists,
    #[error("the project file is damaged: {0}")]
    Damaged(String),
    #[error("could not read or write the project file: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not read or write the project file: {0}")]
    Json(#[from] serde_json::Error),
}

/// Identity and time of one change-log entry, supplied by the caller (the core has no clock).
#[derive(Clone, Debug)]
pub struct EntryMeta {
    /// UUID v7.
    pub id: Uuid,
    /// RFC 3339 timestamp.
    pub at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Edit,
    Undo,
    Redo,
}

impl Origin {
    fn as_str(self) -> &'static str {
        match self {
            Origin::Edit => "edit",
            Origin::Undo => "undo",
            Origin::Redo => "redo",
        }
    }
}

/// Result of applying, undoing or redoing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub seq: i64,
    pub label: String,
    pub origin: Origin,
    /// What was applied and what undoes it (for team sync's outbox).
    #[serde(skip)]
    pub command: Command,
    #[serde(skip)]
    pub inverse: Command,
}

/// Where a shared project syncs (iteration 3): kept in the file's `meta` table.
pub const TEAM_SERVER: &str = "team.server";
pub const TEAM_OUTBOX: &str = "team.outbox";

/// For shared projects: the outbox to save in the same transaction as a change, given what was
/// applied and its inverse. Keeps unsent edits and the file in step, even after a crash.
pub type OutboxFn<'a> = &'a dyn Fn(&Command, &Command) -> String;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Creates a new project file. Refuses to overwrite an existing file.
    pub fn create(path: &Path, project: &Project) -> Result<Store, StoreError> {
        if path.exists() {
            return Err(StoreError::AlreadyExists);
        }
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        conn.pragma_update(None, "application_id", schema::APPLICATION_ID)?;
        migrate(&mut conn)?;
        let tx = conn.transaction()?;
        rows::write_project(&tx, project)?;
        for t in project.tasks.values() {
            rows::upsert_task(&tx, t)?;
        }
        for d in project.dependencies.values() {
            rows::upsert_dependency(&tx, d)?;
        }
        for r in project.resources.values() {
            rows::upsert_resource(&tx, r)?;
        }
        for a in project.assignments.values() {
            rows::upsert_assignment(&tx, a)?;
        }
        rows::write_baseline(&tx, project.baseline.as_ref())?;
        tx.commit()?;
        Ok(Store { conn })
    }

    /// Opens a project file, upgrading its schema if it is older than this app.
    pub fn open(path: &Path) -> Result<(Store, Project), StoreError> {
        if !path.is_file() {
            return Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                Some(format!("no file at {}", path.display())),
            )));
        }
        let mut conn = Connection::open(path)?;
        let app_id: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0)).map_err(not_a_project)?;
        if app_id != schema::APPLICATION_ID {
            return Err(StoreError::NotAProject);
        }
        configure(&conn)?;
        migrate(&mut conn)?;
        let project = rows::read_project(&conn)?;
        Ok((Store { conn }, project))
    }

    /// Applies a user edit and records it. On any error `project` is left unchanged.
    pub fn apply(&mut self, project: &mut Project, cmd: &Command, meta: &EntryMeta) -> Result<Applied, StoreError> {
        self.apply_with(project, cmd, meta, None)
    }

    /// `apply`, also saving the shared project's outbox.
    pub fn apply_with(&mut self, project: &mut Project, cmd: &Command, meta: &EntryMeta, outbox: Option<OutboxFn>) -> Result<Applied, StoreError> {
        let inverse = pm_core::apply(project, cmd)?;
        let label = cmd.label();
        let saved = outbox.map(|f| f(cmd, &inverse));
        let result = self.persist(project, cmd, &inverse, Origin::Edit, None, &label, meta, saved.as_deref(), true);
        if result.is_err() {
            pm_core::apply(project, &inverse).expect("inverse of an applied command applies");
        }
        result
    }

    /// Undoes the most recent edit that has not been undone. `Ok(None)` if there is none.
    pub fn undo(&mut self, project: &mut Project, meta: &EntryMeta) -> Result<Option<Applied>, StoreError> {
        self.undo_with(project, meta, None)
    }

    pub fn undo_with(&mut self, project: &mut Project, meta: &EntryMeta, outbox: Option<OutboxFn>) -> Result<Option<Applied>, StoreError> {
        let Some((pos, target, label)) = self.top(false)? else { return Ok(None) };
        let cmd: Command = serde_json::from_str(&self.log_field(target, "inverse")?)?;
        self.replay(project, &cmd, Origin::Undo, pos, target, &label, meta, outbox).map(Some)
    }

    /// Redoes the most recently undone edit. `Ok(None)` if there is none.
    pub fn redo(&mut self, project: &mut Project, meta: &EntryMeta) -> Result<Option<Applied>, StoreError> {
        self.redo_with(project, meta, None)
    }

    pub fn redo_with(&mut self, project: &mut Project, meta: &EntryMeta, outbox: Option<OutboxFn>) -> Result<Option<Applied>, StoreError> {
        let Some((pos, target, label)) = self.top(true)? else { return Ok(None) };
        let cmd: Command = serde_json::from_str(&self.log_field(target, "payload")?)?;
        self.replay(project, &cmd, Origin::Redo, pos, target, &label, meta, outbox).map(Some)
    }

    /// Changes from the team (a delta from team sync): recorded in the change log but not in
    /// the user's undo history, with the outbox that goes with them.
    pub fn apply_remote(&mut self, project: &mut Project, cmd: &Command, meta: &EntryMeta, outbox: &str) -> Result<Applied, StoreError> {
        let inverse = pm_core::apply(project, cmd)?;
        let label = cmd.label();
        let result = self.persist(project, cmd, &inverse, Origin::Edit, None, &label, meta, Some(outbox), false);
        if result.is_err() {
            pm_core::apply(project, &inverse).expect("inverse of an applied command applies");
        }
        result
    }

    /// A value from the file's settings (`TEAM_SERVER`, `TEAM_OUTBOX`).
    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    /// Sets (or with `None` removes) a setting.
    pub fn set_meta(&mut self, key: &str, value: Option<&str>) -> Result<(), StoreError> {
        match value {
            Some(v) => self.conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2", params![key, v])?,
            None => self.conn.execute("DELETE FROM meta WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Name of the edit that Undo would reverse, for "Undo Change Duration".
    pub fn undo_label(&self) -> Result<Option<String>, StoreError> {
        Ok(self.top(false)?.map(|(_, _, label)| label))
    }

    pub fn redo_label(&self) -> Result<Option<String>, StoreError> {
        Ok(self.top(true)?.map(|(_, _, label)| label))
    }

    /// Highest change-log sequence number, or 0 for a new file.
    pub fn last_seq(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM change_log", [], |r| r.get(0))?)
    }

    /// Copies the whole project to a new file, safely while it is open (Save a Copy, backups).
    pub fn save_copy(&self, path: &Path) -> Result<(), StoreError> {
        if path.exists() {
            return Err(StoreError::AlreadyExists);
        }
        let mut dest = Connection::open(path)?;
        rusqlite::backup::Backup::new(&self.conn, &mut dest)?.run_to_completion(256, std::time::Duration::ZERO, None)?;
        dest.pragma_update(None, "journal_mode", "DELETE")?;
        Ok(())
    }

    /// `(history position, change-log seq, label)` of the next undo (`undone = false`) or redo.
    fn top(&self, undone: bool) -> Result<Option<(i64, i64, String)>, StoreError> {
        let sql = if undone {
            "SELECT h.pos, h.log_seq, c.label FROM undo_history h JOIN change_log c ON c.seq = h.log_seq
             WHERE h.undone = 1 ORDER BY h.pos ASC LIMIT 1"
        } else {
            "SELECT h.pos, h.log_seq, c.label FROM undo_history h JOIN change_log c ON c.seq = h.log_seq
             WHERE h.undone = 0 ORDER BY h.pos DESC LIMIT 1"
        };
        Ok(self.conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?)
    }

    fn log_field(&self, seq: i64, column: &str) -> Result<String, StoreError> {
        let sql = match column {
            "payload" => "SELECT payload FROM change_log WHERE seq = ?1",
            _ => "SELECT inverse FROM change_log WHERE seq = ?1",
        };
        Ok(self.conn.query_row(sql, [seq], |r| r.get(0))?)
    }

    #[allow(clippy::too_many_arguments)]
    fn replay(
        &mut self,
        project: &mut Project,
        cmd: &Command,
        origin: Origin,
        pos: i64,
        target: i64,
        label: &str,
        meta: &EntryMeta,
        outbox: Option<OutboxFn>,
    ) -> Result<Applied, StoreError> {
        let inverse = pm_core::apply(project, cmd).map_err(|e| StoreError::Damaged(format!("undo history no longer matches the project: {e}")))?;
        let saved = outbox.map(|f| f(cmd, &inverse));
        let result = self.persist(project, cmd, &inverse, origin, Some((pos, target)), label, meta, saved.as_deref(), true);
        if result.is_err() {
            pm_core::apply(project, &inverse).expect("inverse of an applied command applies");
        }
        result
    }

    /// Writes changed rows, the log entry and the undo history in one transaction.
    #[allow(clippy::too_many_arguments)]
    fn persist(
        &mut self,
        project: &Project,
        cmd: &Command,
        inverse: &Command,
        origin: Origin,
        history: Option<(i64, i64)>,
        label: &str,
        meta: &EntryMeta,
        outbox: Option<&str>,
        undoable: bool,
    ) -> Result<Applied, StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(o) = outbox {
            tx.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2", params![TEAM_OUTBOX, o])?;
        }
        let mut touched = pm_core::Touched::default();
        cmd.touched(&mut touched);
        inverse.touched(&mut touched);
        for id in &touched.tasks {
            match project.tasks.get(id) {
                Some(t) => rows::upsert_task(&tx, t)?,
                None => rows::delete_task(&tx, *id)?,
            }
        }
        for id in &touched.links {
            match project.dependencies.get(id) {
                Some(d) => rows::upsert_dependency(&tx, d)?,
                None => rows::delete_dependency(&tx, *id)?,
            }
        }
        if touched.project {
            rows::write_project(&tx, project)?;
        }
        if touched.baseline {
            rows::write_baseline(&tx, project.baseline.as_ref())?;
        }
        for id in &touched.resources {
            match project.resources.get(id) {
                Some(r) => rows::upsert_resource(&tx, r)?,
                None => rows::delete_resource(&tx, *id)?,
            }
        }
        for id in &touched.assignments {
            match project.assignments.get(id) {
                Some(a) => rows::upsert_assignment(&tx, a)?,
                None => rows::delete_assignment(&tx, *id)?,
            }
        }
        tx.execute(
            "INSERT INTO change_log (entry_id, at, origin, target_seq, label, payload, inverse)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                meta.id.to_string(),
                meta.at,
                origin.as_str(),
                history.map(|(_, t)| t),
                label,
                serde_json::to_string(cmd)?,
                serde_json::to_string(inverse)?,
            ],
        )?;
        let seq = tx.last_insert_rowid();
        match (origin, history) {
            (Origin::Edit, _) if !undoable => {}
            (Origin::Edit, _) => {
                // A new edit discards whatever could have been redone, then caps the history.
                tx.execute("DELETE FROM undo_history WHERE undone = 1", [])?;
                tx.execute("INSERT INTO undo_history (log_seq) VALUES (?1)", [seq])?;
                tx.execute("DELETE FROM undo_history WHERE pos <= (SELECT MAX(pos) FROM undo_history) - ?1", [UNDO_LIMIT])?;
            }
            (Origin::Undo, Some((pos, _))) => {
                tx.execute("UPDATE undo_history SET undone = 1 WHERE pos = ?1", [pos])?;
            }
            (Origin::Redo, Some((pos, _))) => {
                tx.execute("UPDATE undo_history SET undone = 0 WHERE pos = ?1", [pos])?;
            }
            _ => unreachable!("undo and redo always name a history entry"),
        }
        tx.commit()?;
        Ok(Applied { seq, label: label.to_owned(), origin, command: cmd.clone(), inverse: inverse.clone() })
    }
}

fn not_a_project(e: rusqlite::Error) -> StoreError {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase => StoreError::NotAProject,
        e => StoreError::Sqlite(e),
    }
}

/// Write-ahead logging plus full sync: a crash never corrupts the file or loses a
/// committed command.
fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > CURRENT_VERSION {
        return Err(StoreError::NewerVersion { found: version, supported: CURRENT_VERSION });
    }
    for (k, sql) in schema::MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", k as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
