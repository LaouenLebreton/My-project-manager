//! HTTP-level tests: requests go through the real router, with storage in memory.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::storage::SqliteStorage;

fn server() -> Router {
    router(AppState { storage: Arc::new(SqliteStorage::in_memory().unwrap()) })
}

async fn call(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path).header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string()))).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn account(name: &str, email: &str) -> Value {
    json!({ "name": name, "email": email, "password": "a long enough password" })
}

#[tokio::test]
async fn first_account_sets_up_the_server_and_becomes_administrator() {
    let app = server();
    let (status, body) = call(&app, "POST", "/api/setup", None, Some(account("Laouen", " Laouen@Example.com "))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["email"], "laouen@example.com");
    assert_eq!(body["user"]["isAdmin"], true);
    let token = body["token"].as_str().unwrap().to_owned();

    let (status, me) = call(&app, "GET", "/api/me", Some(&token), None).await;
    assert_eq!((status, me["name"].as_str()), (StatusCode::OK, Some("Laouen")));

    let (status, body) = call(&app, "POST", "/api/setup", None, Some(account("Mallory", "m@example.com"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], "this server is already set up; ask its administrator for an account");
}

#[tokio::test]
async fn administrators_create_accounts_and_people_sign_in() {
    let app = server();
    let (_, admin) = call(&app, "POST", "/api/setup", None, Some(account("Laouen", "laouen@example.com"))).await;
    let admin = admin["token"].as_str().unwrap().to_owned();

    let (status, alex) = call(&app, "POST", "/api/users", Some(&admin), Some(account("Alex", "alex@example.com"))).await;
    assert_eq!((status, alex["isAdmin"].as_bool()), (StatusCode::OK, Some(false)));
    let (status, body) = call(&app, "POST", "/api/users", Some(&admin), Some(account("Alex 2", "ALEX@example.com"))).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::CONFLICT, Some("there is already an account for alex@example.com")));

    // Wrong password and unknown email get the same answer.
    let (s1, b1) = call(&app, "POST", "/api/sessions", None, Some(json!({ "email": "alex@example.com", "password": "nope" }))).await;
    let (s2, b2) = call(&app, "POST", "/api/sessions", None, Some(json!({ "email": "who@example.com", "password": "nope" }))).await;
    assert_eq!((s1, &b1), (s2, &b2));
    assert_eq!(s1, StatusCode::UNAUTHORIZED);

    let (status, session) = call(&app, "POST", "/api/sessions", None, Some(json!({ "email": "Alex@Example.com", "password": "a long enough password" }))).await;
    assert_eq!(status, StatusCode::OK);
    let token = session["token"].as_str().unwrap().to_owned();

    // Only administrators create accounts.
    let (status, _) = call(&app, "POST", "/api/users", Some(&token), Some(account("Sam", "sam@example.com"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Signing out ends the session.
    let (status, _) = call(&app, "DELETE", "/api/sessions", Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, "GET", "/api/me", Some(&token), None).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::UNAUTHORIZED, Some("your session has ended; sign in again")));
}

#[tokio::test]
async fn new_accounts_are_checked() {
    let app = server();
    let bad = |name: &str, email: &str, password: &str| json!({ "name": name, "email": email, "password": password });
    for (body, message) in [
        (bad(" ", "a@b.c", "a long enough password"), "type a name"),
        (bad("A", "not an email", "a long enough password"), "type an email address such as alex@example.com"),
        (bad("A", "a@b.c", "short"), "choose a password of at least 10 characters"),
    ] {
        let (status, res) = call(&app, "POST", "/api/setup", None, Some(body)).await;
        assert_eq!((status, res["message"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some(message)));
    }
    let (status, _) = call(&app, "GET", "/api/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
