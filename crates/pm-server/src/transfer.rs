//! Moving a team to another server (iteration 3, step 7): "connect it to a server afterwards".
//!
//! The source server exports a team whole: people (by email), projects with their full change
//! history and version numbers, who may change what, and the chat. The target imports it with
//! the same project ids and versions, so everyone's copy (and any unsent edits) carries on
//! syncing once pointed at the new address. The source then marks the team as moved: requests
//! for it get "this team has moved to …", which the app uses to follow.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::{TeamRole, User};
use crate::{db, now, ApiError, ApiResult, AppState, Auth};

/// What the app looks for in refusals to follow a moved team.
pub const MOVED_PREFIX: &str = "this team has moved to ";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamExport {
    pub name: String,
    pub members: Vec<ExMember>,
    pub projects: Vec<ExProject>,
    pub channels: Vec<ExChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExMember {
    pub name: String,
    pub email: String,
    pub role: TeamRole,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExProject {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub created_at: String,
    pub version: i64,
    /// The project at `version`, as JSON.
    pub data: String,
    pub changes: Vec<ExChange>,
    /// Roles and limits the owner set (JSON limits), by email.
    pub members: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExChange {
    pub version: i64,
    pub client_change: Uuid,
    pub author_email: String,
    pub at: String,
    pub label: String,
    pub summary: String,
    pub command: String,
    pub inverse: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExChannel {
    pub id: Uuid,
    pub name: String,
    pub project_id: Option<Uuid>,
    pub created_at: String,
    pub posts: Vec<ExPost>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExPost {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub author_email: String,
    pub at: String,
    pub body: String,
    pub edited_at: Option<String>,
    pub deleted: bool,
    /// (emoji, email)
    pub reactions: Vec<(String, String)>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/teams/{id}/export", get(export))
        .route("/api/teams/{id}/moved", post(moved))
        .route("/api/teams/import", post(import))
}

async fn owner_of(state: &AppState, team: Uuid, user: Uuid) -> Result<(), ApiError> {
    match db(state, move |s| s.team_role(team, user)).await? {
        Some(TeamRole::Owner) => Ok(()),
        Some(TeamRole::Member) => Err(ApiError::new(StatusCode::FORBIDDEN, "only the team's owners can move it")),
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "this team is not on the server, or you are not a member of it")),
    }
}

/// The whole team, each project as it is now.
async fn export(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>) -> ApiResult<TeamExport> {
    owner_of(&state, team, me.id).await?;
    let mut out = db(&state, move |s| s.export_team(team)).await?;
    for p in &mut out.projects {
        // The latest state, whatever the last saved copy is.
        let (project, version) = crate::live::with_live(&state, p.id, |l| (serde_json::to_string(&l.project).expect("projects serialize"), l.version)).await?;
        p.data = project;
        p.version = version;
        p.changes.retain(|c| c.version <= version);
    }
    Ok(Json(out))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub team_id: Uuid,
    /// People who had no account here: their one-time codes, for new invites.
    pub invites: Vec<crate::sharing::InviteResult>,
}

/// A team from another server; the caller becomes one of its owners.
async fn import(State(state): State<AppState>, Auth(me): Auth, Json(bundle): Json<TeamExport>) -> ApiResult<Imported> {
    if bundle.name.trim().is_empty() {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "the team has no name"));
    }
    for p in &bundle.projects {
        serde_json::from_str::<pm_core::Project>(&p.data).map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("“{}” could not be read: {e}", p.name)))?;
    }
    // Everyone mentioned anywhere; those without an account here get one, with a code.
    let mut emails: Vec<String> = bundle.members.iter().map(|m| m.email.clone()).collect();
    emails.extend(bundle.projects.iter().flat_map(|p| p.changes.iter().map(|c| c.author_email.clone()).chain([p.owner_email.clone()])));
    emails.extend(bundle.channels.iter().flat_map(|c| c.posts.iter().flat_map(|p| p.reactions.iter().map(|r| r.1.clone()).chain([p.author_email.clone()]))));
    emails.sort();
    emails.dedup();
    let check = emails.clone();
    let unknown: Vec<String> = db(&state, move |s| Ok(check.into_iter().filter(|e| !matches!(s.user_by_email(e), Ok(Some(_)))).collect::<Vec<_>>())).await?;
    let names: std::collections::HashMap<&str, &str> = bundle.members.iter().map(|m| (m.email.as_str(), m.name.as_str())).collect();
    let mut new_users = Vec::new();
    let mut invites = Vec::new();
    for email in unknown {
        let code = crate::invite_code();
        let name = names.get(email.as_str()).map_or_else(|| crate::name_from_email(&email), |n| (*n).to_owned());
        new_users.push((User { id: Uuid::now_v7(), name, email: email.clone(), is_admin: false, created_at: now() }, crate::auth::hash_password(&code)));
        let member = bundle.members.iter().any(|m| m.email == email);
        if member {
            invites.push(crate::sharing::InviteResult { email, code: Some(code), note: None, server: state.config.public_url.clone() });
        }
    }
    let team_id = db(&state, move |s| s.import_team(&bundle, me.id, &new_users)).await?;
    Ok(Json(Imported { team_id, invites }))
}

#[derive(Deserialize)]
pub struct Moved {
    pub to: String,
}

/// After a move: requests for this team point to its new address from now on.
async fn moved(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>, Json(req): Json<Moved>) -> Result<StatusCode, ApiError> {
    owner_of(&state, team, me.id).await?;
    let to = req.to.trim().trim_end_matches('/').to_owned();
    if !(to.starts_with("https://") || to.starts_with("http://")) {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "give the new server's full address, starting with https://"));
    }
    db(&state, move |s| s.set_team_moved(team, Some(&to))).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Refuses requests for a team that has moved, saying where to.
pub async fn not_moved(state: &AppState, team: Uuid) -> Result<(), ApiError> {
    match db(state, move |s| s.team_moved(team)).await? {
        Some(to) => Err(ApiError::new(StatusCode::GONE, format!("{MOVED_PREFIX}{to}"))),
        None => Ok(()),
    }
}
