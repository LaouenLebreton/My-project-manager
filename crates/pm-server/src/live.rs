//! Live sync (iteration 3, step 3): the server puts each project's changes in one order.
//!
//! Each open project has a room: the project as of its latest version, kept in memory, and a
//! broadcast channel to everyone watching. A change is applied, stored and broadcast under the
//! room's lock, so versions never interleave. The project is saved whole every
//! [`SNAPSHOT_EVERY`] changes; in between, opening it replays the stored changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use pm_core::{apply, Command, Project};
use pm_sync::Change;
use serde::Deserialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::access::Role;
use crate::sharing::access;
use crate::{db, now, ApiError, ApiResult, AppState, Auth};

pub const SNAPSHOT_EVERY: i64 = 100;
/// Most changes sent in one catch-up reply; clients ask again from where it ended.
const PAGE: u32 = 500;

pub struct Hub {
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
    /// Chat events for everyone connected; each stream keeps those of its user's teams.
    pub chat: broadcast::Sender<Arc<crate::chat::ChatEvent>>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub { rooms: Mutex::default(), chat: broadcast::channel(1024).0 }
    }
}

pub struct Room {
    state: tokio::sync::Mutex<Option<Live>>,
    pub tx: broadcast::Sender<Arc<Change>>,
}

pub struct Live {
    pub project: Project,
    pub version: i64,
}

impl Hub {
    pub fn room(&self, project: Uuid) -> Arc<Room> {
        let mut rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        rooms.entry(project).or_insert_with(|| Arc::new(Room { state: tokio::sync::Mutex::new(None), tx: broadcast::channel(256).0 })).clone()
    }
}

fn damaged(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("the stored project is damaged: {e}"))
}

/// The project as of its latest version: the saved copy plus the changes after it.
async fn load(state: &AppState, id: Uuid) -> Result<Live, ApiError> {
    let (data, mut version) = db(state, move |s| s.project_snapshot(id)).await?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "this project is not on the server"))?;
    let mut project: Project = serde_json::from_str(&data).map_err(damaged)?;
    loop {
        let since = version;
        let changes = db(state, move |s| s.changes_since(id, since, PAGE)).await?;
        if changes.is_empty() {
            return Ok(Live { project, version });
        }
        for c in changes {
            apply(&mut project, &c.command).map_err(damaged)?;
            version = c.version;
        }
    }
}

/// Runs `f` on the room's live project, loading it first if needed.
pub async fn with_live<T>(state: &AppState, id: Uuid, f: impl FnOnce(&Live) -> T) -> Result<T, ApiError> {
    let room = state.hub.room(id);
    let mut guard = room.state.lock().await;
    if guard.is_none() {
        *guard = Some(load(state, id).await?);
    }
    Ok(f(guard.as_ref().expect("loaded")))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/projects/{id}/changes", post(submit).get(changes))
        .route("/api/projects/{id}/live", get(live))
        .route("/api/projects/{id}/history", get(history))
        .route("/api/projects/{id}/revert", post(revert))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Submit {
    /// The id the app gave this edit; sending it again returns the same change.
    pub client_change: Uuid,
    pub command: Command,
}

async fn submit(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Json(req): Json<Submit>) -> ApiResult<Change> {
    let (_, role, limits) = access(&state, id, me.id).await?;
    if role == Role::Viewer {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "you can view this project but not change it"));
    }
    commit(&state, id, &me, role, &limits, req.client_change, req.command, None).await.map(Json)
}

/// Applies, stores and broadcasts one change, in the project's order. `summary` replaces the
/// sentence worked out from the command (for reverts).
#[allow(clippy::too_many_arguments)]
async fn commit(
    state: &AppState,
    id: Uuid,
    me: &crate::storage::User,
    role: Role,
    limits: &crate::access::Limits,
    client_change: Uuid,
    command: Command,
    summary: Option<String>,
) -> Result<Change, ApiError> {
    let room = state.hub.room(id);
    let mut guard = room.state.lock().await;
    if let Some(done) = db(state, move |s| s.change_by_client(id, client_change)).await? {
        return Ok(done);
    }
    if guard.is_none() {
        *guard = Some(load(state, id).await?);
    }
    let live = guard.as_mut().expect("loaded");
    crate::permissions::check(role, limits, me, &live.project, &command).map_err(|m| ApiError::new(StatusCode::FORBIDDEN, m))?;
    let summary = summary.unwrap_or_else(|| pm_sync::describe::summary(&live.project, &command));
    let inverse = apply(&mut live.project, &command).map_err(|e| ApiError::new(StatusCode::CONFLICT, format!("this change no longer fits the project: {e}")))?;
    let change = Change {
        version: live.version + 1,
        client_change,
        author: me.id,
        author_name: me.name.clone(),
        at: now(),
        label: command.label(),
        summary,
        command,
    };
    let snapshot = (change.version % SNAPSHOT_EVERY == 0).then(|| serde_json::to_string(&live.project).expect("projects serialize"));
    let inverse_json = serde_json::to_string(&inverse).expect("commands serialize");
    let stored = change.clone();
    let saved = db(state, move |s| s.append_change(id, &stored, &inverse_json, snapshot.as_deref())).await;
    match saved {
        Ok(true) => {
            live.version = change.version;
            let _ = room.tx.send(Arc::new(change.clone()));
            Ok(change)
        }
        other => {
            // Not stored: undo it here. If another instance wrote first, reload next time.
            if apply(&mut live.project, &inverse).is_err() || matches!(other, Ok(false)) {
                *guard = None;
            }
            match other {
                Err(e) => Err(e),
                _ => Err(ApiError::new(StatusCode::CONFLICT, "someone else changed the project at the same moment; try again")),
            }
        }
    }
}

#[derive(Deserialize)]
pub struct Before {
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// Newest first.
    pub changes: Vec<Change>,
    /// The caller owns the project and may revert changes.
    pub can_revert: bool,
}

/// The History view: changes newest first, a page at a time.
async fn history(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Query(q): Query<Before>) -> ApiResult<History> {
    let (_, role, _) = access(&state, id, me.id).await?;
    let limit = q.limit.unwrap_or(100).clamp(1, PAGE);
    let before = q.before;
    let changes = db(&state, move |s| s.changes_before(id, before, limit)).await?;
    Ok(Json(History { changes, can_revert: role == Role::Owner }))
}

#[derive(Deserialize)]
pub struct Revert {
    pub version: i64,
}

/// The owner reverts a change: its undo is applied now, as a new change, so the history keeps
/// both. Refused if later changes depend on it.
async fn revert(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Json(req): Json<Revert>) -> ApiResult<Change> {
    let (_, role, limits) = access(&state, id, me.id).await?;
    if role != Role::Owner {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the project's owner can revert changes"));
    }
    let version = req.version;
    let (original, inverse) = db(&state, move |s| s.change_with_inverse(id, version))
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "that change is not in this project's history"))?;
    if inverse.is_empty() {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "this change was made before the history could undo changes, so it cannot be reverted"));
    }
    let command: Command = serde_json::from_str(&inverse).map_err(damaged)?;
    let what = if original.summary.is_empty() { original.label.to_lowercase() } else { original.summary.clone() };
    let summary = format!("reverted {}'s change: {what}", original.author_name);
    commit(&state, id, &me, role, &limits, Uuid::now_v7(), command, Some(summary)).await.map_err(|e| {
        if e.status == StatusCode::CONFLICT {
            ApiError::new(StatusCode::CONFLICT, format!("this change cannot be reverted any more: later changes depend on it ({})", e.message))
        } else {
            e
        }
    }).map(Json)
}

#[derive(Deserialize)]
pub struct Since {
    #[serde(default)]
    pub since: i64,
}

/// Changes after a version, oldest first (catch-up, and the change log).
async fn changes(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Query(q): Query<Since>) -> ApiResult<Vec<Change>> {
    access(&state, id, me.id).await?;
    let since = q.since;
    Ok(Json(db(&state, move |s| s.changes_since(id, since, PAGE)).await?))
}

/// A WebSocket that sends every change after `since`, then each new one as it happens.
async fn live(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Query(q): Query<Since>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    access(&state, id, me.id).await?;
    Ok(ws.on_upgrade(move |socket| stream(state, id, q.since, socket)))
}

async fn stream(state: AppState, id: Uuid, mut since: i64, mut socket: WebSocket) {
    // Subscribe before catching up, so nothing falls between the two.
    let mut rx = state.hub.room(id).tx.subscribe();
    loop {
        let from = since;
        let Ok(backlog) = db(&state, move |s| s.changes_since(id, from, PAGE)).await else { return };
        if backlog.is_empty() {
            break;
        }
        for c in backlog {
            since = c.version;
            if socket.send(Message::Text(serde_json::to_string(&c).expect("changes serialize").into())).await.is_err() {
                return;
            }
        }
    }
    loop {
        tokio::select! {
            change = rx.recv() => match change {
                Ok(c) if c.version > since => {
                    since = c.version;
                    if socket.send(Message::Text(serde_json::to_string(&*c).expect("changes serialize").into())).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                // Fell behind: close, and the app reconnects from the version it has.
                Err(_) => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
