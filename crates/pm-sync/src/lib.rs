//! Live collaboration (iteration 3, step 3), without any networking.
//!
//! The server puts every project's changes in one order and numbers them (versions 1, 2, 3…).
//! Each client keeps a [`Replica`]: the state as of the last change it has heard of
//! (`confirmed`), plus its own edits the server has not confirmed yet (`pending`), applied on
//! top so the user sees them at once. When someone else's change arrives, the pending edits
//! are re-applied on top of it (rebase); an edit that no longer fits is dropped and reported.

use pm_core::{apply, Command, CommandError, Project};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One accepted change, as the server stores and sends it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// 1 for the first change after sharing, then 2, 3…
    pub version: i64,
    /// The id the author's app gave the edit, so it recognises its own changes.
    pub client_change: Uuid,
    pub author: Uuid,
    pub author_name: String,
    /// RFC 3339.
    pub at: String,
    /// What happened, in plain words ("Change Duration").
    pub label: String,
    pub command: Command,
}

/// A local edit waiting for the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub id: Uuid,
    pub command: Command,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SyncError {
    /// Changes `expected..` are missing; fetch them first.
    #[error("missing changes from version {expected}")]
    Gap { expected: i64 },
    /// The server's change does not apply here: the copies differ. Reload the project.
    #[error("this copy of the project no longer matches the server's ({0})")]
    Diverged(CommandError),
}

/// What receiving a change did.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// Already had it.
    Duplicate,
    /// Our own oldest pending edit, now confirmed.
    Confirmed(Uuid),
    /// Someone else's change; these pending edits no longer fit and were dropped.
    Applied { dropped: Vec<(Pending, CommandError)> },
}

#[derive(Clone, Debug)]
pub struct Replica {
    confirmed: Project,
    version: i64,
    pending: Vec<Pending>,
    current: Project,
}

impl Replica {
    pub fn new(project: Project, version: i64) -> Self {
        Replica { current: project.clone(), confirmed: project, version, pending: Vec::new() }
    }

    /// What the user sees: the confirmed state plus their pending edits.
    pub fn current(&self) -> &Project {
        &self.current
    }

    pub fn confirmed(&self) -> &Project {
        &self.confirmed
    }

    /// The last server version this replica has.
    pub fn version(&self) -> i64 {
        self.version
    }

    /// Edits to send (or resend after reconnecting), oldest first.
    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    /// A local edit: applied at once, then waits for the server. Returns its id and inverse.
    pub fn local(&mut self, command: Command) -> Result<(Uuid, Command), CommandError> {
        let inverse = apply(&mut self.current, &command)?;
        let id = Uuid::now_v7();
        self.pending.push(Pending { id, command });
        Ok((id, inverse))
    }

    /// A change from the server. Changes must arrive in version order; duplicates are ignored.
    pub fn receive(&mut self, change: &Change) -> Result<Received, SyncError> {
        if change.version <= self.version {
            return Ok(Received::Duplicate);
        }
        if change.version != self.version + 1 {
            return Err(SyncError::Gap { expected: self.version + 1 });
        }
        apply(&mut self.confirmed, &change.command).map_err(SyncError::Diverged)?;
        self.version = change.version;
        if self.pending.first().is_some_and(|p| p.id == change.client_change) {
            // The server applied our oldest edit to exactly the state we applied it to, so
            // `current` is already right.
            self.pending.remove(0);
            return Ok(Received::Confirmed(change.client_change));
        }
        if let Some(i) = self.pending.iter().position(|p| p.id == change.client_change) {
            // Ours, but out of order (an earlier one was refused): treat like a remote change.
            self.pending.remove(i);
        }
        Ok(Received::Applied { dropped: self.rebase() })
    }

    /// The server refused a pending edit (for example, the member may not change that).
    pub fn refused(&mut self, id: Uuid) -> Vec<(Pending, CommandError)> {
        let before = self.pending.len();
        self.pending.retain(|p| p.id != id);
        if self.pending.len() == before {
            return Vec::new();
        }
        self.rebase()
    }

    /// Rebuilds `current` from `confirmed` and the pending edits, dropping those that fail.
    fn rebase(&mut self) -> Vec<(Pending, CommandError)> {
        self.current = self.confirmed.clone();
        let mut dropped = Vec::new();
        let mut kept = Vec::with_capacity(self.pending.len());
        for p in std::mem::take(&mut self.pending) {
            match apply(&mut self.current, &p.command) {
                Ok(_) => kept.push(p),
                Err(e) => dropped.push((p, e)),
            }
        }
        self.pending = kept;
        dropped
    }
}

#[cfg(test)]
mod tests;
