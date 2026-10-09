//! Who may do what in a shared project (iteration 3).
//!
//! Every member of a project's team can open it. Their role in the project is Owner, Editor or
//! Viewer; the owner can also limit what an Editor may change (enforced from step 5).

use pm_core::TaskId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    pub fn code(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
        }
    }

    pub fn from_code(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }
}

/// Which tasks an Editor may change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskScope {
    #[default]
    All,
    /// Only tasks they are assigned to as a resource (matched by name or email).
    Assigned,
    /// Only tasks inside these phases (summary tasks).
    Phases { ids: Vec<TaskId> },
}

/// What an Editor may change. Owners may change everything; Viewers nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    /// Tasks, durations, links, constraints, outline.
    pub schedule: bool,
    /// % complete and actual dates.
    pub progress: bool,
    /// The resource pool and assignments.
    pub resources: bool,
    /// Rates and fixed costs.
    pub costs: bool,
    /// Project settings: start, calendar, currency, baseline, status date, leveling.
    pub project: bool,
    pub tasks: TaskScope,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { schedule: true, progress: true, resources: true, costs: true, project: true, tasks: TaskScope::All }
    }
}
