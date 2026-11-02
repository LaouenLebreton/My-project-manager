//! Team server client (iteration 3): signing in, sharing projects, and live sync.
//!
//! One account at a time, saved in the app's settings folder (readable by this user only;
//! moving the token to the macOS Keychain is noted in docs/open-questions.md). Each open shared
//! project gets a sync task: it holds a WebSocket to the server for incoming changes, sends the
//! outbox over HTTP, and reconnects with a growing pause when the network drops. Edits keep
//! working offline; they wait in the outbox, which is saved inside the project file.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use pm_core::Project;
use pm_sync::Change;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use ts_rs::TS;
use uuid::Uuid;

use crate::document::{Document, ReceiveError};
use crate::hosting::{self, LOCAL};
use crate::ipc::{changed, with_doc, IpcError, Snapshot};

/// Sent to a window: whether its shared project is in touch with the server.
pub const STATUS_EVENT: &str = "team-status";
/// Sent to a window: something to tell the user about sync, in plain words.
pub const NOTICE_EVENT: &str = "team-notice";

fn err(message: impl Into<String>) -> IpcError {
    IpcError { message: message.into() }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TeamUser {
    pub id: String,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
}

/// The signed-in account, saved between launches.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Account {
    server: String,
    token: String,
    user: TeamUser,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TeamStatus {
    /// Where the teams live, as shown to the user ("this Mac" or a server address).
    pub server: Option<String>,
    pub user: Option<TeamUser>,
    /// The teams are hosted by this copy of the app (no separate server).
    pub hosted_here: bool,
}

/// An invitation to send to one person.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub email: String,
    /// The text to send them; they paste it into Team > Join a Team. None if they already
    /// have an account (they only needed adding to the team).
    pub invite: Option<String>,
    /// Anything to tell the user about this person, in plain words.
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TeamProject {
    pub snapshot: Snapshot,
    pub invites: Vec<Invite>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// "online", "connecting", "offline" or "stopped".
    pub state: String,
    /// Edits not yet confirmed by the server.
    pub pending: u32,
    /// Why it is offline or stopped, if there is something to do about it.
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub id: String,
    pub name: String,
    /// "owner" or "member".
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SharedProject {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[ts(type = "number")]
    pub version: i64,
    pub updated_at: String,
}

fn account_file(app: &AppHandle) -> Option<PathBuf> {
    Some(hosting::profiled(app.path().app_config_dir().ok()?).join("team.json"))
}

fn account(app: &AppHandle) -> Option<Account> {
    let text = std::fs::read_to_string(account_file(app)?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_account(app: &AppHandle, account: Option<&Account>) -> Result<(), IpcError> {
    let file = account_file(app).ok_or_else(|| err("the settings folder cannot be found"))?;
    match account {
        None => {
            let _ = std::fs::remove_file(&file);
            Ok(())
        }
        Some(a) => {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| err(format!("the settings folder cannot be written: {e}")))?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            use std::io::Write;
            options
                .open(&file)
                .and_then(|mut f| f.write_all(serde_json::to_string_pretty(a).expect("accounts serialize").as_bytes()))
                .map_err(|e| err(format!("the sign-in could not be saved: {e}")))
        }
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("an HTTP client"))
}

/// "pm.example.com" → "https://pm.example.com"; local addresses default to http.
fn normalized_server(server: &str) -> Result<String, IpcError> {
    let s = server.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(err("type the team server's address, such as pm.example.com"));
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Ok(s.to_owned());
    }
    let local = s.starts_with("localhost") || s.starts_with("127.") || s.starts_with("[::1]");
    Ok(format!("{}://{s}", if local { "http" } else { "https" }))
}

/// One request to the server; failures come back as plain words.
async fn call<T: DeserializeOwned>(method: reqwest::Method, server: &str, path: &str, token: Option<&str>, body: Option<serde_json::Value>) -> Result<T, CallError> {
    let mut req = http().request(method, format!("{}{path}", hosting::base(server)));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.map_err(|_| CallError::Offline(if server == LOCAL { "the team on this Mac is not running; quit and reopen Super PM".to_owned() } else { format!("the team server at {server} cannot be reached") }))?;
    let status = res.status();
    if status.is_success() {
        if status == reqwest::StatusCode::NO_CONTENT {
            return serde_json::from_value(serde_json::Value::Null).map_err(|e| CallError::Refused(e.to_string()));
        }
        return res.json::<T>().await.map_err(|e| CallError::Refused(format!("the server's answer was not understood: {e}")));
    }
    let message = res
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("the server refused the request ({status})"));
    Err(CallError::Refused(message))
}

enum CallError {
    /// No answer: try again later.
    Offline(String),
    /// An answer saying no, with the reason.
    Refused(String),
}

impl From<CallError> for IpcError {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Offline(m) | CallError::Refused(m) => err(m),
        }
    }
}

fn signed_in(app: &AppHandle) -> Result<Account, IpcError> {
    account(app).ok_or_else(|| err("create a team (Team > Create Team Project) or join one (Team > Join a Team) first"))
}

fn status_of(app: &AppHandle) -> TeamStatus {
    match account(app) {
        Some(a) => TeamStatus { server: Some(hosting::display(&a.server)), hosted_here: a.server == LOCAL, user: Some(a.user) },
        None => TeamStatus { server: None, user: None, hosted_here: false },
    }
}

#[tauri::command]
pub fn team_status(app: AppHandle) -> TeamStatus {
    status_of(&app)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionReply {
    token: String,
    user: TeamUser,
}

/// Signs in. With `name`: `mode` "setup" sets up a new server (you become its administrator),
/// "signup" creates your account on a server that takes sign-ups.
#[tauri::command]
pub async fn team_sign_in(app: AppHandle, server: String, email: String, password: String, name: Option<String>, mode: Option<String>) -> Result<TeamStatus, IpcError> {
    let server = normalized_server(&server)?;
    let path = match mode.as_deref() {
        Some("setup") => "/api/setup",
        Some("signup") => "/api/signup",
        _ => "/api/sessions",
    };
    let body = serde_json::json!({ "name": name.unwrap_or_default(), "email": email, "password": password });
    let reply: SessionReply = call(reqwest::Method::POST, &server, path, None, Some(body)).await?;
    save_account(&app, Some(&Account { server, token: reply.token, user: reply.user }))?;
    restart_all(&app);
    start_chat(&app);
    Ok(status_of(&app))
}

#[tauri::command]
pub async fn team_sign_out(app: AppHandle) -> Result<TeamStatus, IpcError> {
    if let Some(a) = account(&app) {
        let _ = call::<serde_json::Value>(reqwest::Method::DELETE, &a.server, "/api/sessions", Some(&a.token), None).await;
    }
    save_account(&app, None)?;
    Ok(status_of(&app))
}

/// Administrators only: creates an account for someone else on the server.
#[tauri::command]
pub async fn team_create_account(app: AppHandle, name: String, email: String, password: String) -> Result<TeamUser, IpcError> {
    let a = signed_in(&app)?;
    Ok(call(reqwest::Method::POST, &a.server, "/api/users", Some(&a.token), Some(serde_json::json!({ "name": name, "email": email, "password": password }))).await?)
}

#[tauri::command]
pub async fn team_teams(app: AppHandle) -> Result<Vec<TeamView>, IpcError> {
    let a = signed_in(&app)?;
    #[derive(Deserialize)]
    struct Raw {
        team: RawTeam,
        role: String,
    }
    #[derive(Deserialize)]
    struct RawTeam {
        id: String,
        name: String,
    }
    let list: Vec<Raw> = call(reqwest::Method::GET, &a.server, "/api/teams", Some(&a.token), None).await?;
    Ok(list.into_iter().map(|r| TeamView { id: r.team.id, name: r.team.name, role: r.role }).collect())
}

#[tauri::command]
pub async fn team_create_team(app: AppHandle, name: String) -> Result<TeamView, IpcError> {
    let a = signed_in(&app)?;
    let v: serde_json::Value = call(reqwest::Method::POST, &a.server, "/api/teams", Some(&a.token), Some(serde_json::json!({ "name": name }))).await?;
    Ok(TeamView {
        id: v["team"]["id"].as_str().unwrap_or_default().to_owned(),
        name: v["team"]["name"].as_str().unwrap_or_default().to_owned(),
        role: "owner".into(),
    })
}

#[tauri::command]
pub async fn team_add_member(app: AppHandle, team_id: String, email: String) -> Result<(), IpcError> {
    let a = signed_in(&app)?;
    let _: serde_json::Value = call(reqwest::Method::POST, &a.server, &format!("/api/teams/{team_id}/members"), Some(&a.token), Some(serde_json::json!({ "email": email }))).await?;
    Ok(())
}

#[tauri::command]
pub async fn team_projects(app: AppHandle) -> Result<Vec<SharedProject>, IpcError> {
    let a = signed_in(&app)?;
    Ok(call(reqwest::Method::GET, &a.server, "/api/projects", Some(&a.token), None).await?)
}

/// Shares the project in this window with a team; from then on it syncs.
#[tauri::command]
pub async fn team_share(app: AppHandle, window: WebviewWindow, team_id: String) -> Result<Snapshot, IpcError> {
    let a = signed_in(&app)?;
    share(&app, window.label(), &a, &team_id).await
}

const INVITE_PREFIX: &str = "SuperPM-Invite";

/// Adds people to a team by email. Whoever hosts the team (or administers the server) can
/// create their accounts on the spot; each new person gets an invite to paste into Join a Team.
async fn invite_people(a: &Account, team_id: &str, emails: &[String]) -> Vec<Invite> {
    #[derive(Deserialize)]
    struct Raw {
        email: String,
        code: Option<String>,
        note: Option<String>,
        server: Option<String>,
    }
    let fallback = if a.server == LOCAL { hosting::invite_address() } else { a.server.clone() };
    let body = serde_json::json!({ "emails": emails });
    match call::<Vec<Raw>>(reqwest::Method::POST, &a.server, &format!("/api/teams/{team_id}/invites"), Some(&a.token), Some(body)).await {
        Ok(list) => list
            .into_iter()
            .map(|r| {
                // The server's public address when it has one; otherwise how we reach it.
                let address = r.server.unwrap_or_else(|| fallback.clone());
                Invite { invite: r.code.map(|code| format!("{INVITE_PREFIX}|{address}|{}|{code}", r.email)), note: r.note.map(|n| capitalized(&n)), email: r.email }
            })
            .collect(),
        Err(CallError::Refused(m) | CallError::Offline(m)) => {
            emails.iter().map(|e| Invite { email: e.trim().to_lowercase(), invite: None, note: Some(format!("Could not be invited: {m}.")) }).collect()
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str() + if s.ends_with('.') { "" } else { "." }).unwrap_or_default()
}

/// Team > Create Team Project: makes the project in this window a team project. The first time,
/// the app starts hosting teams itself (no server to set up) with the given name and email.
#[tauri::command]
pub async fn team_create_project(
    app: AppHandle,
    window: WebviewWindow,
    my_name: Option<String>,
    my_email: Option<String>,
    team_id: Option<String>,
    team_name: String,
    emails: Vec<String>,
) -> Result<TeamProject, IpcError> {
    let a = match account(&app) {
        Some(a) => a,
        None => {
            let (name, email) = (my_name.unwrap_or_default(), my_email.unwrap_or_default());
            if name.trim().is_empty() || !email.contains('@') {
                return Err(err("type your name and email, so your team knows who changed what"));
            }
            hosting::ensure(&app).await?;
            // Nobody types this password: the app keeps the session, and invites carry codes.
            let body = serde_json::json!({ "name": name, "email": email, "password": Uuid::new_v4().simple().to_string() });
            let reply: SessionReply = call(reqwest::Method::POST, LOCAL, "/api/setup", None, Some(body)).await.map_err(|e| match e {
                CallError::Refused(m) if m.starts_with("this server is already set up") => err("teams were already created on this Mac under another sign-in; choose Team > Join a Team with an invite, or ask for help restoring the sign-in"),
                other => other.into(),
            })?;
            let a = Account { server: LOCAL.to_owned(), token: reply.token, user: reply.user };
            save_account(&app, Some(&a))?;
            start_chat(&app);
            a
        }
    };
    if a.server == LOCAL {
        hosting::ensure(&app).await?;
    }
    let team_id = match team_id.filter(|t| !t.is_empty()) {
        Some(id) => id,
        None => {
            let v: serde_json::Value = call(reqwest::Method::POST, &a.server, "/api/teams", Some(&a.token), Some(serde_json::json!({ "name": team_name }))).await?;
            v["team"]["id"].as_str().unwrap_or_default().to_owned()
        }
    };
    let invites = invite_people(&a, &team_id, &emails).await;
    let snapshot = share(&app, window.label(), &a, &team_id).await?;
    Ok(TeamProject { snapshot, invites })
}

/// One line of the History view.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[ts(type = "number")]
    pub version: i64,
    pub author_name: String,
    /// RFC 3339.
    pub at: String,
    /// What they did, in plain words ("changed the duration of “Survey” from 3 days to 5 days").
    pub summary: String,
    /// Tasks the change touched, to show them.
    pub task_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
    /// The user owns the project and may revert changes.
    pub can_revert: bool,
    /// Older changes can be loaded.
    pub has_more: bool,
}

const HISTORY_PAGE: usize = 100;

/// The shared project in this window, and the account to reach it with.
fn shared_in(app: &AppHandle, label: &str) -> Result<(Account, Uuid), IpcError> {
    let (server, project) = with_doc(app, label, |d| Ok(d.team.as_ref().map(|t| (t.server.clone(), d.project.id.0))))?.ok_or_else(|| err("this project is not a team project"))?;
    let a = account(app).filter(|a| a.server == server).ok_or_else(|| err("join the team again (Team > Join a Team) to see this project's history"))?;
    Ok((a, project))
}

/// The History view: changes newest first, a page at a time (`before`: the oldest version
/// already shown).
#[tauri::command]
pub async fn team_history(app: AppHandle, window: WebviewWindow, before: Option<i64>) -> Result<HistoryPage, IpcError> {
    let (a, project) = shared_in(&app, window.label())?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Raw {
        changes: Vec<Change>,
        can_revert: bool,
    }
    let query = match before {
        Some(b) => format!("?limit={HISTORY_PAGE}&before={b}"),
        None => format!("?limit={HISTORY_PAGE}"),
    };
    let raw: Raw = call(reqwest::Method::GET, &a.server, &format!("/api/projects/{project}/history{query}"), Some(&a.token), None).await?;
    let has_more = raw.changes.len() == HISTORY_PAGE;
    let entries = raw
        .changes
        .into_iter()
        .map(|c| HistoryEntry {
            version: c.version,
            author_name: c.author_name,
            at: c.at,
            summary: if c.summary.is_empty() { c.label.to_lowercase() } else { c.summary },
            task_ids: pm_sync::describe::tasks(&c.command).into_iter().map(|t| t.0.to_string()).collect(),
        })
        .collect();
    Ok(HistoryPage { entries, can_revert: raw.can_revert, has_more })
}

/// Which tasks an Editor may change (same shape as the server's).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskScopeView {
    All,
    /// Only tasks they are assigned to (as a resource with their name or email).
    Assigned,
    /// Only tasks inside these phases.
    Phases { ids: Vec<String> },
}

/// What an Editor may change.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MemberLimits {
    pub schedule: bool,
    pub progress: bool,
    pub resources: bool,
    pub costs: bool,
    pub project: bool,
    pub tasks: TaskScopeView,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub email: String,
    /// "owner", "editor" or "viewer".
    pub role: String,
    pub limits: MemberLimits,
    /// This is the person using the app.
    pub is_me: bool,
}

/// Team > Members & Permissions: everyone in the project's team and what they may change.
#[tauri::command]
pub async fn team_members(app: AppHandle, window: WebviewWindow) -> Result<Vec<Member>, IpcError> {
    let (a, project) = shared_in(&app, window.label())?;
    #[derive(Deserialize)]
    struct Raw {
        user: TeamUser,
        role: String,
        limits: MemberLimits,
    }
    let list: Vec<Raw> = call(reqwest::Method::GET, &a.server, &format!("/api/projects/{project}/members"), Some(&a.token), None).await?;
    Ok(list
        .into_iter()
        .map(|m| Member { is_me: m.user.id == a.user.id, user_id: m.user.id, name: m.user.name, email: m.user.email, role: m.role, limits: m.limits })
        .collect())
}

/// The owner sets one member's role ("editor" or "viewer") and limits.
#[tauri::command]
pub async fn team_set_member(app: AppHandle, window: WebviewWindow, user_id: String, role: String, limits: MemberLimits) -> Result<(), IpcError> {
    let (a, project) = shared_in(&app, window.label())?;
    let _: serde_json::Value = call(
        reqwest::Method::PUT,
        &a.server,
        &format!("/api/projects/{project}/members/{user_id}"),
        Some(&a.token),
        Some(serde_json::json!({ "role": role, "limits": limits })),
    )
    .await?;
    Ok(())
}

/// The owner reverts a change; the result arrives like any other change.
#[tauri::command]
pub async fn team_revert(app: AppHandle, window: WebviewWindow, version: i64) -> Result<(), IpcError> {
    let (a, project) = shared_in(&app, window.label())?;
    let _: Change = call(reqwest::Method::POST, &a.server, &format!("/api/projects/{project}/revert"), Some(&a.token), Some(serde_json::json!({ "version": version }))).await?;
    Ok(())
}

/// Team > Invite People: adds more people to a team.
#[tauri::command]
pub async fn team_invite(app: AppHandle, team_id: String, emails: Vec<String>) -> Result<Vec<Invite>, IpcError> {
    let a = signed_in(&app)?;
    if a.server == LOCAL {
        hosting::ensure(&app).await?;
    }
    Ok(invite_people(&a, &team_id, &emails).await)
}

/// Team > Join a Team: signs in with an invite someone sent.
#[tauri::command]
pub async fn team_join(app: AppHandle, invite: String) -> Result<TeamStatus, IpcError> {
    let parts: Vec<&str> = invite.trim().split('|').map(str::trim).collect();
    let [INVITE_PREFIX, address, email, code] = parts[..] else {
        return Err(err("that does not look like a Super PM invite; copy the whole text you were sent, starting with SuperPM-Invite"));
    };
    let server = normalized_server(address)?;
    let reply: SessionReply = call(reqwest::Method::POST, &server, "/api/sessions", None, Some(serde_json::json!({ "email": email, "password": code })))
        .await
        .map_err(|e| match e {
            CallError::Offline(_) => err("the team cannot be reached right now. The person who invited you needs Super PM open, and you need to be on the same network."),
            CallError::Refused(m) => err(format!("the invite was not accepted: {m}")),
        })?;
    save_account(&app, Some(&Account { server, token: reply.token, user: reply.user }))?;
    restart_all(&app);
    start_chat(&app);
    Ok(status_of(&app))
}

/// When the app starts: bring the teams hosted here back up, then sync open shared projects.
pub fn on_launch(app: &AppHandle) {
    start_chat(app);
    if !account(app).is_some_and(|a| a.server == LOCAL) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = hosting::ensure(&app).await {
            eprintln!("{}", e.message);
        }
        restart_all(&app);
    });
}

/// Development only: a second copy of the app acting as another person, for trying teams on
/// one Mac.
#[tauri::command]
pub fn open_second_user() -> Result<(), IpcError> {
    let exe = std::env::current_exe().map_err(|e| err(e.to_string()))?;
    std::process::Command::new(exe).env("SUPER_PM_PROFILE", "second-user").spawn().map_err(|e| err(format!("the second copy could not be started: {e}")))?;
    Ok(())
}

async fn share(app: &AppHandle, label: &str, a: &Account, team_id: &str) -> Result<Snapshot, IpcError> {
    let app = app.clone();
    let label = label.to_owned();
    let project = with_doc(&app, &label, |doc| {
        if doc.team.is_some() {
            return Err(err("this project is already shared"));
        }
        Ok(doc.project.clone())
    })?;
    let _: serde_json::Value = call(reqwest::Method::POST, &a.server, "/api/projects", Some(&a.token), Some(serde_json::json!({ "teamId": team_id, "project": project }))).await?;
    let snap = with_doc(&app, &label, |doc| {
        doc.link_team(&a.server, 0).map_err(|e| err(format!("the project is shared, but this file could not be updated: {e}")))?;
        Ok(changed(&app, &label, doc))
    })?;
    start_sync(&app, &label);
    Ok(snap)
}

#[derive(Deserialize)]
struct Opened {
    record: SharedProject,
    project: Project,
}

/// Downloads a shared project into the app's Shared folder (or reuses the copy already
/// there) and opens it.
#[tauri::command]
pub async fn team_open(app: AppHandle, window: WebviewWindow, project_id: String) -> Result<Option<Snapshot>, IpcError> {
    let a = signed_in(&app)?;
    let dir = hosting::profiled(app.path().app_data_dir().map_err(|e| err(e.to_string()))?).join("Shared");
    std::fs::create_dir_all(&dir).map_err(|e| err(format!("the Shared folder cannot be created: {e}")))?;
    let path = dir.join(format!("{project_id}.pmproj"));
    if path.exists() {
        return crate::actions::open_path(&app, Some(window.label()), &path);
    }
    let opened: Opened = call(reqwest::Method::GET, &a.server, &format!("/api/projects/{project_id}"), Some(&a.token), None).await?;
    let mut doc = Document::create(&path, opened.project).map_err(|e| err(format!("the project could not be saved on this Mac: {e}")))?;
    doc.link_team(&a.server, opened.record.version).map_err(|e| err(e.to_string()))?;
    Ok(crate::actions::install(&app, Some(window.label()), doc))
}

// ---------------------------------------------------------------------------------------------
// Live sync

fn running() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(Default::default)
}

fn emit_status(app: &AppHandle, label: &str, state: &str, message: Option<String>) {
    let pending = with_doc(app, label, |d| Ok(d.team.as_ref().map_or(0, |t| t.outbox.pending.len() as u32))).unwrap_or(0);
    let _ = app.emit_to(label, STATUS_EVENT, SyncStatus { state: state.into(), pending, message });
}

fn notice(app: &AppHandle, label: &str, message: String) {
    let _ = app.emit_to(label, NOTICE_EVENT, message);
}

/// Starts syncing every open shared project (after signing in).
fn restart_all(app: &AppHandle) {
    let labels: Vec<String> = app.state::<crate::actions::AppState>().docs.lock().expect("lock").iter().filter(|(_, d)| d.team.is_some()).map(|(l, _)| l.clone()).collect();
    for l in labels {
        start_sync(app, &l);
    }
}

/// Starts the sync task for the shared project in `label`, unless one is running.
pub fn start_sync(app: &AppHandle, label: &str) {
    let Ok(Some((server, project, wake))) = with_doc(app, label, |d| Ok(d.team.as_ref().map(|t| (t.server.clone(), d.project.id.0, t.wake.clone())))) else { return };
    let Some(a) = account(app).filter(|a| a.server == server) else {
        emit_status(app, label, "stopped", Some(format!("Join the team again (Team > Join a Team) to sync this project. It is shared through {}.", hosting::display(&server))));
        return;
    };
    if !running().lock().expect("lock").insert(label.to_owned()) {
        return;
    }
    let (app, label) = (app.clone(), label.to_owned());
    tauri::async_runtime::spawn(async move {
        sync_loop(&app, &label, &a, project, wake).await;
        running().lock().expect("lock").remove(&label);
    });
}

fn open(app: &AppHandle, label: &str) -> bool {
    with_doc(app, label, |d| Ok(d.team.is_some())).unwrap_or(false)
}

async fn sync_loop(app: &AppHandle, label: &str, a: &Account, project: Uuid, wake: Arc<tokio::sync::Notify>) {
    let mut pause = Duration::from_secs(1);
    while open(app, label) {
        emit_status(app, label, "connecting", None);
        match session(app, label, a, project, &wake).await {
            Ok(()) => return, // the window closed or the copy needs downloading again
            Err(message) => {
                emit_status(app, label, "offline", Some(message));
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// One connection: catch up, send the outbox, then follow changes until the connection drops.
async fn session(app: &AppHandle, label: &str, a: &Account, project: Uuid, wake: &tokio::sync::Notify) -> Result<(), String> {
    let since = with_doc(app, label, |d| Ok(d.team.as_ref().map_or(0, |t| t.outbox.version))).map_err(|e| e.message)?;
    let ws_base = hosting::base(&a.server).replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    let mut req = format!("{ws_base}/api/projects/{project}/live?since={since}").into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("authorization", format!("Bearer {}", a.token).parse().map_err(|_| "bad token".to_owned())?);
    let (ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|_| format!("Offline: the team on {} cannot be reached. Your edits are kept and will be sent later.", hosting::display(&a.server)))?;
    let (mut write, mut read) = ws.split();
    emit_status(app, label, "online", None);
    let mut sent = HashSet::new();
    send_outbox(app, label, a, project, &mut sent).await?;
    loop {
        tokio::select! {
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let Ok(change) = serde_json::from_str::<Change>(&text) else { continue };
                    if !take_in(app, label, a, project, change).await {
                        return Ok(());
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Err("The connection to the team server was lost; reconnecting.".into()),
                Some(Ok(_)) => {}
            },
            _ = wake.notified() => send_outbox(app, label, a, project, &mut sent).await?,
            _ = tokio::time::sleep(Duration::from_secs(25)) => {
                if write.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return Err("The connection to the team server was lost; reconnecting.".into());
                }
                if !open(app, label) {
                    return Ok(());
                }
            }
        }
    }
}

/// Applies one change from the server, fetching missing ones first. False: stop syncing.
async fn take_in(app: &AppHandle, label: &str, a: &Account, project: Uuid, change: Change) -> bool {
    let mut queue = vec![change];
    while let Some(c) = queue.pop() {
        let outcome = with_doc(app, label, |doc| {
            let r = doc.receive(&c);
            if matches!(&r, Ok(u) if u.changed) {
                let _ = app.emit_to(label, crate::CHANGED_EVENT, changed(app, label, doc));
            }
            Ok(r)
        });
        match outcome {
            Err(_) => return false, // window closed
            Ok(Ok(update)) => {
                if !update.dropped.is_empty() {
                    notice(app, label, format!("{} could not be kept: a teammate changed the same part of the plan first.", update.dropped.join(", ")));
                }
            }
            Ok(Err(ReceiveError::Gap(from))) => {
                let missing: Result<Vec<Change>, _> = call(reqwest::Method::GET, &a.server, &format!("/api/projects/{project}/changes?since={}", from - 1), Some(&a.token), None).await;
                match missing {
                    Ok(list) if !list.is_empty() => {
                        queue.push(c);
                        queue.extend(list.into_iter().rev());
                    }
                    _ => return true, // try again on the next change or reconnect
                }
            }
            Ok(Err(ReceiveError::Diverged(why))) => {
                emit_status(app, label, "stopped", Some(format!("This copy no longer matches your team's ({why}). Open the project again from Team > Open Shared Project to get a fresh copy.")));
                return false;
            }
        }
    }
    emit_status(app, label, "online", None);
    true
}

/// Sends every edit not yet sent on this connection, oldest first.
async fn send_outbox(app: &AppHandle, label: &str, a: &Account, project: Uuid, sent: &mut HashSet<Uuid>) -> Result<(), String> {
    loop {
        let next = with_doc(app, label, |d| Ok(d.team.as_ref().and_then(|t| t.outbox.pending.iter().find(|p| !sent.contains(&p.id)).cloned()))).map_err(|e| e.message)?;
        let Some(p) = next else { break };
        let body = serde_json::json!({ "clientChange": p.id, "command": p.command });
        match call::<Change>(reqwest::Method::POST, &a.server, &format!("/api/projects/{project}/changes"), Some(&a.token), Some(body)).await {
            Ok(_) => {
                sent.insert(p.id);
            }
            Err(CallError::Offline(m)) => return Err(m),
            Err(CallError::Refused(reason)) => {
                let label_text = p.command.label();
                let _ = with_doc(app, label, |doc| {
                    let (changed_copy, also) = doc.refused(p.id);
                    if changed_copy {
                        let _ = app.emit_to(label, crate::CHANGED_EVENT, changed(app, label, doc));
                    }
                    let extra = if also.is_empty() { String::new() } else { format!(" {} no longer fit either.", also.join(", ")) };
                    notice(app, label, format!("“{label_text}” was not accepted by the team server: {reason}.{extra}"));
                    Ok(())
                });
            }
        }
    }
    emit_status(app, label, "online", None);
    Ok(())
}

/// For the snapshot: where this project syncs.
pub fn snapshot_info(doc: &Document) -> Option<(String, u32, String)> {
    doc.team.as_ref().map(|t| (t.server.clone(), t.outbox.pending.len() as u32, doc.project.id.0.to_string()))
}

// ---------------------------------------------------------------------------------------------
// Chat (step 6): teams, channels, posts and replies, as in Microsoft Teams.

/// Sent to every window when a post or channel appears.
pub const CHAT_EVENT: &str = "chat-event";

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatChannel {
    pub id: String,
    pub team_id: String,
    pub name: String,
    /// The team project this channel belongs to, if any.
    pub project_id: Option<String>,
    /// Posts by others not seen yet.
    pub unread: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatTeam {
    pub id: String,
    pub name: String,
    /// General first, then project channels, then the others.
    pub channels: Vec<ChatChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatPost {
    pub id: String,
    pub channel_id: String,
    /// Set for a reply.
    pub parent_id: Option<String>,
    pub author_id: String,
    pub author_name: String,
    pub at: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatThread {
    pub post: ChatPost,
    /// Oldest first.
    pub replies: Vec<ChatPost>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    /// Newest first.
    pub threads: Vec<ChatThread>,
    pub has_more: bool,
}

/// Who the signed-in person is, for telling their own posts apart.
#[tauri::command]
pub fn chat_me(app: AppHandle) -> Option<String> {
    account(&app).map(|a| a.user.id)
}

/// Every team the person is in, with its channels.
#[tauri::command]
pub async fn chat_teams(app: AppHandle) -> Result<Vec<ChatTeam>, IpcError> {
    let a = signed_in(&app)?;
    if a.server == LOCAL {
        hosting::ensure(&app).await?;
    }
    let teams = team_teams(app.clone()).await?;
    let mut out = Vec::new();
    for t in teams {
        let channels: Vec<ChatChannel> = call(reqwest::Method::GET, &a.server, &format!("/api/teams/{}/channels", t.id), Some(&a.token), None).await?;
        out.push(ChatTeam { id: t.id, name: t.name, channels });
    }
    Ok(out)
}

#[tauri::command]
pub async fn chat_posts(app: AppHandle, channel_id: String, before: Option<String>) -> Result<ChatPage, IpcError> {
    let a = signed_in(&app)?;
    let query = before.map(|b| format!("?before={b}")).unwrap_or_default();
    Ok(call(reqwest::Method::GET, &a.server, &format!("/api/channels/{channel_id}/posts{query}"), Some(&a.token), None).await?)
}

/// A new post, or with `parentId` a reply.
#[tauri::command]
pub async fn chat_send(app: AppHandle, channel_id: String, body: String, parent_id: Option<String>) -> Result<ChatPost, IpcError> {
    let a = signed_in(&app)?;
    Ok(call(reqwest::Method::POST, &a.server, &format!("/api/channels/{channel_id}/posts"), Some(&a.token), Some(serde_json::json!({ "body": body, "parentId": parent_id }))).await?)
}

#[tauri::command]
pub async fn chat_read(app: AppHandle, channel_id: String) -> Result<(), IpcError> {
    let a = signed_in(&app)?;
    let _: serde_json::Value = call(reqwest::Method::POST, &a.server, &format!("/api/channels/{channel_id}/read"), Some(&a.token), None).await?;
    Ok(())
}

#[tauri::command]
pub async fn chat_create_channel(app: AppHandle, team_id: String, name: String) -> Result<ChatChannel, IpcError> {
    let a = signed_in(&app)?;
    Ok(call(reqwest::Method::POST, &a.server, &format!("/api/teams/{team_id}/channels"), Some(&a.token), Some(serde_json::json!({ "name": name }))).await?)
}

/// Keeps one live chat connection for the signed-in account and passes every event to all
/// windows. Restarted when the account changes.
pub fn start_chat(app: &AppHandle) {
    static RUNNING: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    let Some(a) = account(app) else { return };
    let key = format!("{}|{}", a.server, a.token);
    {
        let mut running = RUNNING.get_or_init(Default::default).lock().expect("lock");
        if running.as_deref() == Some(key.as_str()) {
            return;
        }
        *running = Some(key.clone());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut pause = Duration::from_secs(1);
        loop {
            // Stop when someone else signs in (a new task takes over).
            let current = RUNNING.get_or_init(Default::default).lock().expect("lock").clone();
            if current.as_deref() != Some(key.as_str()) {
                return;
            }
            let base = hosting::base(&a.server).replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
            let Ok(mut req) = format!("{base}/api/chat/live").into_client_request() else { return };
            let Ok(auth) = format!("Bearer {}", a.token).parse() else { return };
            req.headers_mut().insert("authorization", auth);
            if let Ok((ws, _)) = tokio_tungstenite::connect_async(req).await {
                pause = Duration::from_secs(1);
                let (mut write, mut read) = ws.split();
                loop {
                    tokio::select! {
                        msg = read.next() => match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                    let _ = app.emit(CHAT_EVENT, v);
                                }
                            }
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                            Some(Ok(_)) => {}
                        },
                        _ = tokio::time::sleep(Duration::from_secs(25)) => {
                            if write.send(Message::Ping(Vec::new().into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_secs(30));
        }
    });
}
