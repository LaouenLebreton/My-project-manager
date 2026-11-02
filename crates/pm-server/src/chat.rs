//! Chat (iteration 3, step 6), modelled on Microsoft Teams: each team has channels, a channel
//! has posts, and a post has replies. Every team has a General channel; each team project
//! gets its own channel when it is shared. New posts reach everyone in the team at once.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::storage::{Channel, Post, StorageError};
use crate::{db, now, ApiError, ApiResult, AppState, Auth};

pub const GENERAL: &str = "General";
const PAGE: u32 = 50;

/// What the live stream sends.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatEvent {
    #[serde(rename_all = "camelCase")]
    Post { team_id: Uuid, post: Post },
    #[serde(rename_all = "camelCase")]
    Channel { team_id: Uuid, channel: Channel },
}

impl ChatEvent {
    fn team(&self) -> Uuid {
        match self {
            ChatEvent::Post { team_id, .. } | ChatEvent::Channel { team_id, .. } => *team_id,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/teams/{id}/channels", get(channels).post(create_channel))
        .route("/api/channels/{id}/posts", get(posts).post(add_post))
        .route("/api/channels/{id}/read", post(mark_read))
        .route("/api/chat/live", get(live))
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "this team or channel is not on the server, or you are not a member of it")
}

/// Creates a channel, adjusting the name if the team already has one like it.
pub fn make_channel(s: &dyn crate::storage::Storage, team: Uuid, name: &str, project: Option<Uuid>) -> Result<Channel, StorageError> {
    let taken: HashSet<String> = s.channels(team)?.into_iter().map(|c| c.name.to_lowercase()).collect();
    let mut chosen = name.trim().to_owned();
    let mut n = 2;
    while taken.contains(&chosen.to_lowercase()) {
        chosen = format!("{} {n}", name.trim());
        n += 1;
    }
    let channel = Channel { id: Uuid::now_v7(), team_id: team, name: chosen, project_id: project, created_at: now(), unread: 0 };
    s.create_channel(&channel)?;
    Ok(channel)
}

/// The team's channels with the caller's unread counts; General first, created if missing.
async fn channels(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>) -> ApiResult<Vec<Channel>> {
    let list = db(&state, move |s| {
        if s.team_role(team, me.id)?.is_none() {
            return Ok(None);
        }
        if !s.channels(team)?.iter().any(|c| c.name == GENERAL) {
            make_channel(s, team, GENERAL, None)?;
        }
        let mut list = s.channels(team)?;
        let unread = s.unread(me.id, team)?;
        for c in &mut list {
            c.unread = unread.iter().find(|(id, _)| *id == c.id).map_or(0, |(_, n)| *n);
        }
        list.sort_by_key(|c| (c.name != GENERAL, c.project_id.is_none(), c.name.to_lowercase()));
        Ok(Some(list))
    })
    .await?
    .ok_or_else(not_found)?;
    Ok(Json(list))
}

#[derive(Deserialize)]
pub struct NewChannel {
    pub name: String,
}

async fn create_channel(State(state): State<AppState>, Auth(me): Auth, Path(team): Path<Uuid>, Json(req): Json<NewChannel>) -> ApiResult<Channel> {
    if req.name.trim().is_empty() {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "type a name for the channel"));
    }
    let channel = db(&state, move |s| {
        if s.team_role(team, me.id)?.is_none() {
            return Ok(None);
        }
        make_channel(s, team, &req.name, None).map(Some)
    })
    .await?
    .ok_or_else(not_found)?;
    let _ = state.hub.chat.send(Arc::new(ChatEvent::Channel { team_id: team, channel: channel.clone() }));
    Ok(Json(channel))
}

/// The channel, if the caller is in its team.
async fn channel_for(state: &AppState, channel: Uuid, user: Uuid) -> Result<Channel, ApiError> {
    db(state, move |s| {
        let Some(c) = s.channel(channel)? else { return Ok(None) };
        Ok(s.team_role(c.team_id, user)?.map(|_| c))
    })
    .await?
    .ok_or_else(not_found)
}

#[derive(Deserialize)]
pub struct Before {
    pub before: Option<Uuid>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub post: Post,
    /// Oldest first.
    pub replies: Vec<Post>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// Newest first.
    pub threads: Vec<Thread>,
    pub has_more: bool,
}

/// A page of conversations: posts newest first, each with all its replies.
async fn posts(State(state): State<AppState>, Auth(me): Auth, Path(channel): Path<Uuid>, Query(q): Query<Before>) -> ApiResult<Page> {
    channel_for(&state, channel, me.id).await?;
    let before = q.before;
    let page = db(&state, move |s| {
        let top = s.posts(channel, before, PAGE)?;
        let ids: Vec<Uuid> = top.iter().map(|p| p.id).collect();
        let mut replies = s.replies(&ids)?;
        let has_more = top.len() == PAGE as usize;
        let threads = top
            .into_iter()
            .map(|post| {
                let (mine, rest): (Vec<Post>, Vec<Post>) = std::mem::take(&mut replies).into_iter().partition(|r| r.parent_id == Some(post.id));
                replies = rest;
                Thread { post, replies: mine }
            })
            .collect();
        Ok(Page { threads, has_more })
    })
    .await?;
    Ok(Json(page))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPost {
    pub body: String,
    /// Replying to this post.
    pub parent_id: Option<Uuid>,
}

async fn add_post(State(state): State<AppState>, Auth(me): Auth, Path(channel): Path<Uuid>, Json(req): Json<NewPost>) -> ApiResult<Post> {
    let body = req.body.trim().to_owned();
    if body.is_empty() {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "write something first"));
    }
    if body.chars().count() > 20_000 {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "this post is too long; split it into several"));
    }
    let c = channel_for(&state, channel, me.id).await?;
    let post = Post { id: Uuid::now_v7(), channel_id: channel, parent_id: req.parent_id, author_id: me.id, author_name: me.name.clone(), at: now(), body };
    let stored = post.clone();
    let ok = db(&state, move |s| {
        // Replies go to posts in the same channel, one level deep, as in Teams.
        if let Some(parent) = stored.parent_id {
            match s.post(parent)? {
                Some(p) if p.channel_id == stored.channel_id && p.parent_id.is_none() => {}
                _ => return Ok(false),
            }
        }
        s.add_post(&stored)?;
        s.mark_read(stored.author_id, stored.channel_id, stored.id)?;
        Ok(true)
    })
    .await?;
    if !ok {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "you can only reply to a post in this channel"));
    }
    let _ = state.hub.chat.send(Arc::new(ChatEvent::Post { team_id: c.team_id, post: post.clone() }));
    Ok(Json(post))
}

/// The caller has seen everything in the channel so far.
async fn mark_read(State(state): State<AppState>, Auth(me): Auth, Path(channel): Path<Uuid>) -> Result<StatusCode, ApiError> {
    channel_for(&state, channel, me.id).await?;
    db(&state, move |s| s.mark_read(me.id, channel, Uuid::now_v7())).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A WebSocket with every new post and channel in the caller's teams.
async fn live(State(state): State<AppState>, Auth(me): Auth, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| stream(state, me.id, socket))
}

async fn stream(state: AppState, user: Uuid, mut socket: WebSocket) {
    let mut rx = state.hub.chat.subscribe();
    let teams = |state: AppState| async move { db(&state, move |s| s.teams_of(user)).await.map(|t| t.into_iter().map(|(t, _)| t.id).collect::<HashSet<_>>()).unwrap_or_default() };
    let mut mine = teams(state.clone()).await;
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(e) => {
                    if !mine.contains(&e.team()) {
                        // Perhaps added to a team since connecting.
                        mine = teams(state.clone()).await;
                        if !mine.contains(&e.team()) {
                            continue;
                        }
                    }
                    let text = serde_json::to_string(&*e).expect("chat events serialize");
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
                Err(_) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
