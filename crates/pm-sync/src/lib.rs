//! Live collaboration (iteration 3, step 3), without any networking.
//!
//! The server puts every project's changes in one order and numbers them (versions 1, 2, 3…).
//! Each app shows the project as the server had it at the last version it heard of, plus its
//! own edits the server has not confirmed yet. The [`Outbox`] keeps those edits, each with its
//! inverse. When someone else's change arrives, the outbox works out one command (a *delta*)
//! that takes the app's copy to the new state: undo the unconfirmed edits, apply the change,
//! re-apply the edits that still fit (rebase). Edits that no longer fit are dropped and
//! reported. The app applies the delta like any other command, so its file stays consistent.

pub mod describe;

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
    /// The change as a sentence, written by the server from the project as it was just before
    /// ("changed the duration of “Survey” from 3 days to 5 days").
    #[serde(default)]
    pub summary: String,
    pub command: Command,
}

/// A local edit waiting for the server, with the command that undoes it on the current copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub id: Uuid,
    pub command: Command,
    pub inverse: Command,
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

/// What receiving a change means for the app's copy.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// Already had it; nothing to do.
    Duplicate,
    /// Our own oldest edit, now confirmed; the copy is already right.
    Confirmed(Uuid),
    /// Apply `delta` to the copy. These unconfirmed edits no longer fit and were dropped.
    Apply { delta: Command, dropped: Vec<(Pending, CommandError)> },
}

/// The app's sync state for one shared project. Saved with the project, so unsent edits
/// survive quitting while offline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outbox {
    /// The last server version the copy includes.
    pub version: i64,
    /// Unconfirmed edits, oldest first.
    pub pending: Vec<Pending>,
}

impl Outbox {
    pub fn new(version: i64) -> Self {
        Outbox { version, pending: Vec::new() }
    }

    /// Records an edit the app has just applied. Returns the id to send it under.
    pub fn push(&mut self, command: Command, inverse: Command) -> Uuid {
        let id = Uuid::now_v7();
        self.pending.push(Pending { id, command, inverse });
        id
    }

    /// A change from the server, for a copy that is `current`. Changes must arrive in version
    /// order; duplicates are ignored. Does not touch `current`: apply the returned delta.
    pub fn receive(&mut self, current: &Project, change: &Change) -> Result<Received, SyncError> {
        if change.version <= self.version {
            return Ok(Received::Duplicate);
        }
        if change.version != self.version + 1 {
            return Err(SyncError::Gap { expected: self.version + 1 });
        }
        if self.pending.first().is_some_and(|p| p.id == change.client_change) {
            // The server applied our oldest edit to exactly the state we applied it to.
            self.version = change.version;
            self.pending.remove(0);
            return Ok(Received::Confirmed(change.client_change));
        }
        let (delta, dropped) = self.rebuild(current, Some(&change.command), Some(change.client_change)).map_err(SyncError::Diverged)?;
        self.version = change.version;
        Ok(Received::Apply { delta, dropped })
    }

    /// The server refused an edit (for example, the member may not change that). Returns the
    /// delta that takes it out of `current`, and any later edits that no longer fit.
    pub fn refused(&mut self, current: &Project, id: Uuid) -> Option<(Command, Vec<(Pending, CommandError)>)> {
        if !self.pending.iter().any(|p| p.id == id) {
            return None;
        }
        self.rebuild(current, None, Some(id)).ok()
    }

    /// Undo every pending edit, apply `remote`, re-apply the pending edits except `skip`.
    /// Works on a scratch copy; returns the steps that worked as one command.
    #[allow(clippy::type_complexity)]
    fn rebuild(&mut self, current: &Project, remote: Option<&Command>, skip: Option<Uuid>) -> Result<(Command, Vec<(Pending, CommandError)>), CommandError> {
        let mut scratch = current.clone();
        let mut steps = Vec::new();
        for p in self.pending.iter().rev() {
            apply(&mut scratch, &p.inverse)?;
            steps.push(p.inverse.clone());
        }
        if let Some(r) = remote {
            apply(&mut scratch, r)?;
            steps.push(r.clone());
        }
        let mut kept = Vec::with_capacity(self.pending.len());
        let mut dropped = Vec::new();
        for p in std::mem::take(&mut self.pending) {
            if Some(p.id) == skip {
                continue;
            }
            match apply(&mut scratch, &p.command) {
                Ok(inverse) => {
                    steps.push(p.command.clone());
                    kept.push(Pending { inverse, ..p });
                }
                Err(e) => dropped.push((p, e)),
            }
        }
        self.pending = kept;
        Ok((Command::Batch { label: "Changes from your team".into(), commands: steps }, dropped))
    }
}

#[cfg(test)]
mod tests;
