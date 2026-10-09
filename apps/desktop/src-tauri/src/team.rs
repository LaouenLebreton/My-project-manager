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
    pub server: Option<String>,
    pub user: Option<TeamUser>,
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
    Some(app.path().app_config_dir().ok()?.join("team.json"))
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
    let mut req = http().request(method, format!("{server}{path}"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.map_err(|_| CallError::Offline(format!("the team server at {server} cannot be reached")))?;
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
    account(app).ok_or_else(|| err("sign in to a team server first (File > Team Server > Sign In)"))
}

fn status_of(app: &AppHandle) -> TeamStatus {
    match account(app) {
        Some(a) => TeamStatus { server: Some(a.server), user: Some(a.user) },
        None => TeamStatus { server: None, user: None },
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

/// Signs in, or with `setupName` sets up a new server and becomes its administrator.
#[tauri::command]
pub async fn team_sign_in(app: AppHandle, server: String, email: String, password: String, setup_name: Option<String>) -> Result<TeamStatus, IpcError> {
    let server = normalized_server(&server)?;
    let reply: SessionReply = match setup_name.filter(|n| !n.trim().is_empty()) {
        Some(name) => call(reqwest::Method::POST, &server, "/api/setup", None, Some(serde_json::json!({ "name": name, "email": email, "password": password }))).await?,
        None => call(reqwest::Method::POST, &server, "/api/sessions", None, Some(serde_json::json!({ "email": email, "password": password }))).await?,
    };
    save_account(&app, Some(&Account { server, token: reply.token, user: reply.user }))?;
    restart_all(&app);
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
    let label = window.label().to_owned();
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
    let dir = app.path().app_data_dir().map_err(|e| err(e.to_string()))?.join("Shared");
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
        emit_status(app, label, "stopped", Some(format!("Sign in to {server} to sync this project with your team.")));
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
    let ws_base = a.server.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    let mut req = format!("{ws_base}/api/projects/{project}/live?since={since}").into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("authorization", format!("Bearer {}", a.token).parse().map_err(|_| "bad token".to_owned())?);
    let (ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|_| format!("Offline: the team server at {} cannot be reached. Your edits are kept and will be sent later.", a.server))?;
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
                emit_status(app, label, "stopped", Some(format!("This copy no longer matches your team's ({why}). Open the project again from File > Team Server > Open Shared Project to get a fresh copy.")));
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
pub fn snapshot_info(doc: &Document) -> Option<(String, u32)> {
    doc.team.as_ref().map(|t| (t.server.clone(), t.outbox.pending.len() as u32))
}
