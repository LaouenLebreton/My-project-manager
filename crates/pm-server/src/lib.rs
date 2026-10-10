//! Super PM team server (iteration 3). See docs/iteration-3-plan.md.
//!
//! Step 1: accounts and sessions. The first account, created through `POST /api/setup`, is the
//! server administrator; administrators create the other accounts.

pub mod access;
pub mod auth;
pub mod live;
pub mod permissions;
pub mod sharing;
pub mod storage;

use std::sync::Arc;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use storage::{Storage, StorageError, User};

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub hub: Arc<live::Hub>,
}

impl AppState {
    pub fn new(storage: impl Storage) -> Self {
        AppState { storage: Arc::new(storage), hub: Arc::default() }
    }
}

/// An error the client can show as it is: a status and a plain-words message.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError { status, message: message.into() }
    }
}

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::EmailTaken(_) | StorageError::ProjectExists => ApiError::new(StatusCode::CONFLICT, e.to_string()),
            StorageError::Damaged(_) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            StorageError::Database(_) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "the server could not reach its database"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "message": self.message }))).into_response()
    }
}

pub(crate) type ApiResult<T> = Result<Json<T>, ApiError>;

/// Runs blocking storage calls off the async threads.
pub(crate) async fn db<T: Send + 'static>(state: &AppState, f: impl FnOnce(&dyn Storage) -> Result<T, StorageError> + Send + 'static) -> Result<T, ApiError> {
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || f(&*storage))
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "the server stopped while handling the request"))?
        .map_err(Into::into)
}

/// The signed-in user, from `Authorization: Bearer <token>`.
pub struct Auth(pub User);

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in first"))?;
        let hash = auth::token_hash(&token);
        db(state, move |s| s.session_user(&hash))
            .await?
            .map(Auth)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "your session has ended; sign in again"))
    }
}

fn bearer(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.trim().to_owned())
}

/// Serves the API on `listener` until the task is dropped or the listener fails. Used by the
/// `super-pm-server` binary and by the app when it hosts a team itself.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(|| async { Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })) }))
        .route("/api/setup", post(setup))
        .route("/api/sessions", post(sign_in).delete(sign_out))
        .route("/api/me", get(me))
        .route("/api/users", post(create_user))
        .merge(sharing::routes())
        .merge(live::routes())
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub token: String,
    pub user: User,
}

pub(crate) fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn normalized_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Checks a new account's fields and makes the user record.
fn account(a: &NewAccount, is_admin: bool) -> Result<(User, String), ApiError> {
    let bad = |m: &str| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, m);
    let name = a.name.trim();
    let email = normalized_email(&a.email);
    if name.is_empty() {
        return Err(bad("type a name"));
    }
    if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
        return Err(bad("type an email address such as alex@example.com"));
    }
    if a.password.chars().count() < 10 {
        return Err(bad("choose a password of at least 10 characters"));
    }
    let user = User { id: Uuid::now_v7(), name: name.to_owned(), email, is_admin, created_at: now() };
    Ok((user, auth::hash_password(&a.password)))
}

async fn start_session(state: &AppState, user: User) -> Result<Session, ApiError> {
    let (token, hash) = auth::new_token();
    let id = user.id;
    db(state, move |s| s.create_session(&hash, id, &now())).await?;
    Ok(Session { token, user })
}

/// The very first account: only while the server has none. It becomes the administrator.
async fn setup(State(state): State<AppState>, Json(a): Json<NewAccount>) -> ApiResult<Session> {
    let (user, hash) = account(&a, true)?;
    let created = user.clone();
    let made = db(&state, move |s| s.create_first_user(&created, &hash)).await?;
    if !made {
        return Err(ApiError::new(StatusCode::CONFLICT, "this server is already set up; ask its administrator for an account"));
    }
    Ok(Json(start_session(&state, user).await?))
}

async fn create_user(State(state): State<AppState>, Auth(me): Auth, Json(a): Json<NewAccount>) -> ApiResult<User> {
    if !me.is_admin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the server administrator can create accounts"));
    }
    let (user, hash) = account(&a, false)?;
    let created = user.clone();
    db(&state, move |s| s.create_user(&created, &hash)).await?;
    Ok(Json(user))
}

async fn sign_in(State(state): State<AppState>, Json(c): Json<Credentials>) -> ApiResult<Session> {
    let email = normalized_email(&c.email);
    let found = db(&state, move |s| s.user_by_email(&email)).await?;
    // Same answer whether the email or the password is wrong, so accounts cannot be probed.
    let wrong = || ApiError::new(StatusCode::UNAUTHORIZED, "the email or password is not right");
    let (user, stored) = found.ok_or_else(wrong)?;
    let password = c.password;
    let ok = tokio::task::spawn_blocking(move || auth::verify_password(&password, &stored)).await.unwrap_or(false);
    if !ok {
        return Err(wrong());
    }
    Ok(Json(start_session(&state, user).await?))
}

async fn sign_out(State(state): State<AppState>, parts: axum::http::HeaderMap) -> StatusCode {
    if let Some(token) = parts.get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        let hash = auth::token_hash(token.trim());
        let _ = db(&state, move |s| s.delete_session(&hash)).await;
    }
    StatusCode::NO_CONTENT
}

async fn me(Auth(user): Auth) -> Json<User> {
    Json(user)
}

#[cfg(test)]
mod tests;
