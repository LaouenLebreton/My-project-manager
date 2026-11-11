//! Teams and shared projects (iteration 3, step 2).
//!
//! A project is shared with a team; every member of the team can open it. The project's owner
//! gives each member a role (Editor by default, or Viewer) and, for Editors, limits.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::access::{Limits, Role};
use crate::storage::{ProjectRecord, Team, TeamRole, User};
use crate::{db, now, ApiError, ApiResult, AppState, Auth};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/teams", get(my_teams).post(create_team))
        .route("/api/teams/{id}/members", get(team_members).post(add_team_member))
        .route("/api/teams/{id}/members/{user}", axum::routing::delete(remove_team_member))
        .route("/api/teams/{id}/invites", axum::routing::post(invite))
        .route("/api/projects", get(my_projects).post(share_project))
        .route("/api/projects/{id}", get(open_project))
        .route("/api/projects/{id}/members", get(project_members))
        .route("/api/projects/{id}/access", get(my_access))
        .route("/api/projects/{id}/members/{user}", put(set_project_member))
}

fn not_found(what: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, format!("this {what} is not on the server, or you are not a member of it"))
}

fn forbidden(message: &str) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, message)
}

/// The caller's place in a project: its record, their role and limits. Projects of teams they
/// are not in look the same as projects that do not exist.
pub async fn access(state: &AppState, project: Uuid, user: Uuid) -> Result<(ProjectRecord, Role, Limits), ApiError> {
    let found = db(state, move |s| {
        let Some(record) = s.project_record(project)? else { return Ok(None) };
        if s.team_role(record.team_id, user)?.is_none() {
            return Ok(None);
        }
        let (role, limits) = s.project_member(project, user)?.unwrap_or((Role::Editor, Limits::default()));
        // The owner is always the owner, whatever a stored row says.
        let role = if record.owner_id == user { Role::Owner } else { role };
        Ok(Some((record, role, limits)))
    })
    .await?
    .ok_or_else(|| not_found("project"))?;
    // A team that moved to another server is refused, saying where it went.
    crate::transfer::not_moved(state, found.0.team_id).await?;
    Ok(found)
}

#[derive(Deserialize)]
pub struct NewTeam {
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub team: Team,
    pub role: TeamRole,
}

async fn create_team(State(state): State<AppState>, Auth(me): Auth, Json(t): Json<NewTeam>) -> ApiResult<TeamView> {
    let name = t.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "type a name for the team"));
    }
    let team = Team { id: Uuid::now_v7(), name, created_at: now() };
    let made = team.clone();
    db(&state, move |s| s.create_team(&made, me.id)).await?;
    Ok(Json(TeamView { team, role: TeamRole::Owner }))
}

async fn my_teams(State(state): State<AppState>, Auth(me): Auth) -> ApiResult<Vec<TeamView>> {
    let teams = db(&state, move |s| s.teams_of(me.id)).await?;
    Ok(Json(teams.into_iter().map(|(team, role)| TeamView { team, role }).collect()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberView {
    pub user: User,
    pub role: TeamRole,
}

async fn team_members(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>) -> ApiResult<Vec<TeamMemberView>> {
    let members = db(&state, move |s| {
        if s.team_role(team, me.id)?.is_none() {
            return Ok(None);
        }
        s.team_members(team).map(Some)
    })
    .await?
    .ok_or_else(|| not_found("team"))?;
    Ok(Json(members.into_iter().map(|(user, role)| TeamMemberView { user, role }).collect()))
}

#[derive(Deserialize)]
pub struct NewTeamMember {
    pub email: String,
    #[serde(default)]
    pub role: Option<TeamRole>,
}

async fn add_team_member(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>, Json(m): Json<NewTeamMember>) -> ApiResult<TeamMemberView> {
    let email = m.email.trim().to_lowercase();
    let role = m.role.unwrap_or(TeamRole::Member);
    enum Outcome {
        NotMember,
        NotOwner,
        NoAccount,
        Added(User),
    }
    let outcome = db(&state, move |s| {
        match s.team_role(team, me.id)? {
            None => return Ok(Outcome::NotMember),
            Some(TeamRole::Member) => return Ok(Outcome::NotOwner),
            Some(TeamRole::Owner) => {}
        }
        let Some((user, _)) = s.user_by_email(&email)? else { return Ok(Outcome::NoAccount) };
        s.set_team_member(team, user.id, role)?;
        Ok(Outcome::Added(user))
    })
    .await?;
    match outcome {
        Outcome::NotMember => Err(not_found("team")),
        Outcome::NotOwner => Err(forbidden("only the team's owners can add people")),
        Outcome::NoAccount => Err(ApiError::new(StatusCode::NOT_FOUND, "there is no account with that email on this server; ask the administrator to create one")),
        Outcome::Added(user) => Ok(Json(TeamMemberView { user, role })),
    }
}

/// Owners take someone out of the team (and its projects); anyone may leave. The owner of a
/// project stays until someone else owns it, and a team keeps at least one owner.
async fn remove_team_member(State(state): State<AppState>, Auth(me): Auth, Path((team, user)): Path<(Uuid, Uuid)>) -> Result<StatusCode, ApiError> {
    enum Outcome {
        NotMember,
        NotOwner,
        NotThere,
        OwnsProjects,
        LastOwner,
        Removed,
    }
    let outcome = db(&state, move |s| {
        let my_role = s.team_role(team, me.id)?;
        match my_role {
            None => return Ok(Outcome::NotMember),
            Some(TeamRole::Member) if user != me.id => return Ok(Outcome::NotOwner),
            _ => {}
        }
        let Some(their_role) = s.team_role(team, user)? else { return Ok(Outcome::NotThere) };
        if s.owns_projects_in(team, user)? {
            return Ok(Outcome::OwnsProjects);
        }
        if their_role == TeamRole::Owner && s.team_members(team)?.iter().filter(|(_, r)| *r == TeamRole::Owner).count() <= 1 {
            return Ok(Outcome::LastOwner);
        }
        s.remove_team_member(team, user)?;
        Ok(Outcome::Removed)
    })
    .await?;
    match outcome {
        Outcome::NotMember => Err(not_found("team")),
        Outcome::NotOwner => Err(forbidden("only the team's owners can remove people")),
        Outcome::NotThere => Err(ApiError::new(StatusCode::NOT_FOUND, "that person is not in the team")),
        Outcome::OwnsProjects => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "this person owns projects in the team; they stay until someone else owns them")),
        Outcome::LastOwner => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "a team needs at least one owner")),
        Outcome::Removed => Ok(StatusCode::NO_CONTENT),
    }
}

/// Most people in one invite request.
const MAX_INVITES: usize = 100;

#[derive(Deserialize)]
pub struct InviteRequest {
    pub emails: Vec<String>,
}

/// One person invited to a team.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteResult {
    pub email: String,
    /// For someone new to the server: the one-time code to sign in with. None if they already
    /// had an account (they were just added to the team).
    pub code: Option<String>,
    /// Anything to tell the person inviting, in plain words.
    pub note: Option<String>,
    /// The server address to put in the invite, when the server knows it.
    pub server: Option<String>,
}

/// A team's owners invite people by email. New people get an account with a one-time code;
/// everyone is added to the team.
async fn invite(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>, Json(req): Json<InviteRequest>) -> ApiResult<Vec<InviteResult>> {
    let emails: Vec<String> = req.emails.iter().map(|e| crate::normalized_email(e)).filter(|e| !e.is_empty()).collect();
    if emails.len() > MAX_INVITES {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invite at most 100 people at a time"));
    }
    // Who may invite is checked before any work (codes are hashed like passwords: slow).
    match db(&state, move |s| s.team_role(team, me.id)).await? {
        Some(TeamRole::Owner) => {}
        Some(TeamRole::Member) => return Err(forbidden("only the team's owners can invite people")),
        None => return Err(not_found("team")),
    }
    let server = state.config.public_url.clone();
    let prepared: Vec<(String, String, String)> = crate::password_work(move || {
        emails
            .into_iter()
            .map(|e| {
                let code = crate::invite_code();
                let hash = crate::auth::hash_password(&code);
                (e, code, hash)
            })
            .collect()
    })
    .await?;
    let results = db(&state, move |s| {
        // Checked again, in case the role changed meanwhile.
        match s.team_role(team, me.id)? {
            Some(TeamRole::Owner) => {}
            Some(TeamRole::Member) => return Ok(Err("only the team's owners can invite people")),
            None => return Ok(Err("")),
        }
        let mut out = Vec::new();
        for (email, code, hash) in prepared {
            if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
                out.push(InviteResult { email, code: None, note: Some("not an email address".into()), server: server.clone() });
                continue;
            }
            let (user, code) = match s.user_by_email(&email)? {
                Some((u, _)) => (u, None),
                None => {
                    let u = User { id: Uuid::now_v7(), name: crate::name_from_email(&email), email: email.clone(), is_admin: false, created_at: now() };
                    s.create_user(&u, &hash)?;
                    (u, Some(code))
                }
            };
            s.set_team_member(team, user.id, s.team_role(team, user.id)?.unwrap_or(TeamRole::Member))?;
            let note = code.is_none().then(|| "already has an account; added to the team".to_owned());
            out.push(InviteResult { email, code, note, server: server.clone() });
        }
        Ok(Ok(out))
    })
    .await?;
    match results {
        Ok(list) => Ok(Json(list)),
        Err("") => Err(not_found("team")),
        Err(m) => Err(forbidden(m)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareProject {
    pub team_id: Uuid,
    pub project: pm_core::Project,
}

async fn share_project(State(state): State<AppState>, Auth(me): Auth, Json(req): Json<ShareProject>) -> ApiResult<ProjectRecord> {
    // The server keeps only projects its own engine can schedule or explain.
    if let Err(e) = pm_core::check(&req.project) {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("this project cannot be shared as it is: {e}")));
    }
    let at = now();
    let record = ProjectRecord {
        id: req.project.id.0,
        team_id: req.team_id,
        name: req.project.name.clone(),
        owner_id: me.id,
        version: 0,
        created_at: at.clone(),
        updated_at: at,
    };
    let data = serde_json::to_string(&req.project).map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    let stored = record.clone();
    let shared = db(&state, move |s| {
        if s.team_role(stored.team_id, me.id)?.is_none() {
            return Ok(None);
        }
        s.create_project(&stored, &data)?;
        // Each team project gets its own channel, as in Teams.
        crate::chat::make_channel(s, stored.team_id, &stored.name, Some(stored.id)).map(Some)
    })
    .await?;
    let Some(channel) = shared else { return Err(not_found("team")) };
    let _ = state.hub.chat.send(std::sync::Arc::new(crate::chat::ChatEvent::Channel { team_id: record.team_id, channel }));
    Ok(Json(record))
}

async fn my_projects(State(state): State<AppState>, Auth(me): Auth) -> ApiResult<Vec<ProjectRecord>> {
    Ok(Json(db(&state, move |s| s.projects_of(me.id)).await?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedProject {
    pub record: ProjectRecord,
    pub project: pm_core::Project,
    pub role: Role,
    pub limits: Limits,
}

async fn open_project(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>) -> ApiResult<OpenedProject> {
    let (mut record, role, limits) = access(&state, id, me.id).await?;
    // The latest state, with every change applied, and the version it is at.
    let (project, version) = crate::live::with_live(&state, id, |l| (l.project.clone(), l.version)).await?;
    record.version = version;
    Ok(Json(OpenedProject { record, project, role, limits }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessView {
    pub role: Role,
    pub limits: Limits,
}

/// What the caller may change in a project, so the app can grey out the rest.
async fn my_access(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>) -> ApiResult<AccessView> {
    let (_, role, limits) = access(&state, id, me.id).await?;
    Ok(Json(AccessView { role, limits }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemberView {
    pub user: User,
    pub role: Role,
    pub limits: Limits,
}

async fn project_members(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>) -> ApiResult<Vec<ProjectMemberView>> {
    let (record, _, _) = access(&state, id, me.id).await?;
    let members = db(&state, move |s| {
        let mut out = Vec::new();
        for (user, _) in s.team_members(record.team_id)? {
            let (role, limits) = s.project_member(id, user.id)?.unwrap_or((Role::Editor, Limits::default()));
            let role = if user.id == record.owner_id { Role::Owner } else { role };
            out.push(ProjectMemberView { user, role, limits });
        }
        Ok(out)
    })
    .await?;
    Ok(Json(members))
}

#[derive(Deserialize)]
pub struct MemberChange {
    pub role: Role,
    #[serde(default)]
    pub limits: Limits,
}

async fn set_project_member(
    State(state): State<AppState>,
    Auth(me): Auth,
    Path((id, user)): Path<(Uuid, Uuid)>,
    Json(change): Json<MemberChange>,
) -> ApiResult<ProjectMemberView> {
    let (record, role, _) = access(&state, id, me.id).await?;
    if role != Role::Owner {
        return Err(forbidden("only the project's owner can change what members may do"));
    }
    if user == record.owner_id || change.role == Role::Owner {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "a project has one owner, who can always change everything"));
    }
    let limits = change.limits.clone();
    let member = db(&state, move |s| {
        if s.team_role(record.team_id, user)?.is_none() {
            return Ok(None);
        }
        s.set_project_member(id, user, change.role, &limits)?;
        s.user(user)
    })
    .await?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "that person is not in the project's team"))?;
    Ok(Json(ProjectMemberView { user: member, role: change.role, limits: change.limits }))
}
