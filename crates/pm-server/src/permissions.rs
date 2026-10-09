//! What a member may change (iteration 3). Step 5 checks each command against the member's
//! limits; for now Owners and Editors may change everything and Viewers nothing.

use pm_core::{Command, Project};

use crate::access::{Limits, Role};
use crate::storage::User;

/// Ok, or why this member may not make this change, in plain words.
pub fn check(role: Role, _limits: &Limits, _user: &User, _project: &Project, _command: &Command) -> Result<(), String> {
    match role {
        Role::Owner | Role::Editor => Ok(()),
        Role::Viewer => Err("you can view this project but not change it".into()),
    }
}
