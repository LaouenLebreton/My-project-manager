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
    router(AppState::new(SqliteStorage::in_memory().unwrap()))
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

fn sample_project(name: &str) -> Value {
    use pm_core::*;
    let id = uuid::Uuid::now_v7();
    let mut p = Project {
        id: ProjectId(id),
        name: name.into(),
        start: Moment::from_datetime(jiff::civil::date(2026, 10, 5).at(8, 0, 0, 0)),
        calendar_id: CalendarId(uuid::Uuid::now_v7()),
        calendar: Calendar::standard(),
        tasks: Default::default(),
        dependencies: Default::default(),
        resources: Default::default(),
        assignments: Default::default(),
        currency: "€".into(),
        baseline: None,
        status_date: None,
    };
    let t = Task {
        id: TaskId(uuid::Uuid::now_v7()),
        parent: None,
        sort_key: SortKey("V".into()),
        name: "Survey".into(),
        duration: Duration::minutes(960).unwrap(),
        is_milestone: false,
        constraint: Constraint::AsSoonAsPossible,
        percent_complete: Percent::default(),
        leveling_delay: Duration::ZERO,
        fixed_cost: Money(0),
        manual_start: None,
        actual: Actual::default(),
        notes: String::new(),
    };
    p.tasks.insert(t.id, t);
    serde_json::to_value(&p).unwrap()
}

/// An administrator and two more accounts, each signed in: (admin, alex, sam) tokens and ids.
async fn people(app: &Router) -> [(String, String); 3] {
    let (_, admin) = call(app, "POST", "/api/setup", None, Some(account("Laouen", "laouen@example.com"))).await;
    let admin_token = admin["token"].as_str().unwrap().to_owned();
    let mut out = vec![(admin_token.clone(), admin["user"]["id"].as_str().unwrap().to_owned())];
    for (name, email) in [("Alex", "alex@example.com"), ("Sam", "sam@example.com")] {
        call(app, "POST", "/api/users", Some(&admin_token), Some(account(name, email))).await;
        let (_, s) = call(app, "POST", "/api/sessions", None, Some(json!({ "email": email, "password": "a long enough password" }))).await;
        out.push((s["token"].as_str().unwrap().to_owned(), s["user"]["id"].as_str().unwrap().to_owned()));
    }
    out.try_into().unwrap()
}

#[tokio::test]
async fn a_project_shared_with_a_team_opens_for_its_members() {
    let app = server();
    let [(laouen, _), (alex, alex_id), (sam, _)] = people(&app).await;

    let (status, team) = call(&app, "POST", "/api/teams", Some(&laouen), Some(json!({ "name": "Office move" }))).await;
    assert_eq!((status, team["role"].as_str()), (StatusCode::OK, Some("owner")));
    let team_id = team["team"]["id"].as_str().unwrap().to_owned();

    // Members are added by email; only owners add them.
    let (status, _) = call(&app, "POST", &format!("/api/teams/{team_id}/members"), Some(&laouen), Some(json!({ "email": "Alex@example.com" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&app, "POST", &format!("/api/teams/{team_id}/members"), Some(&alex), Some(json!({ "email": "sam@example.com" }))).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::FORBIDDEN, Some("only the team's owners can add people")));
    let (status, _) = call(&app, "POST", &format!("/api/teams/{team_id}/members"), Some(&laouen), Some(json!({ "email": "nobody@example.com" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let project = sample_project("Office move");
    let (status, record) = call(&app, "POST", "/api/projects", Some(&laouen), Some(json!({ "teamId": team_id, "project": project }))).await;
    assert_eq!((status, record["version"].as_i64()), (StatusCode::OK, Some(0)));
    let id = record["id"].as_str().unwrap().to_owned();
    let (status, _) = call(&app, "POST", "/api/projects", Some(&laouen), Some(json!({ "teamId": team_id, "project": project }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "shared once");

    // Alex (in the team) sees and opens it as an Editor; Sam (not in the team) cannot.
    let (_, list) = call(&app, "GET", "/api/projects", Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (status, opened) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&alex), None).await;
    assert_eq!((status, opened["role"].as_str()), (StatusCode::OK, Some("editor")));
    assert_eq!(opened["project"], project, "the project comes back exactly as shared");
    let (status, _) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&sam), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = call(&app, "GET", "/api/projects", Some(&sam), None).await;
    assert_eq!(list, json!([]));

    // The owner limits Alex to progress updates on tasks assigned to him.
    let limits = json!({ "schedule": false, "progress": true, "resources": false, "costs": false, "project": false, "tasks": { "kind": "assigned" } });
    let (status, member) = call(&app, "PUT", &format!("/api/projects/{id}/members/{alex_id}"), Some(&laouen), Some(json!({ "role": "editor", "limits": limits }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(member["limits"], limits);
    let (_, opened) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&alex), None).await;
    assert_eq!(opened["limits"], limits);
    let (_, members) = call(&app, "GET", &format!("/api/projects/{id}/members"), Some(&alex), None).await;
    let roles: Vec<(&str, &str)> = members.as_array().unwrap().iter().map(|m| (m["user"]["name"].as_str().unwrap(), m["role"].as_str().unwrap())).collect();
    assert_eq!(roles, [("Alex", "editor"), ("Laouen", "owner")]);

    // Only the owner changes members, and the owner stays the owner.
    let (status, _) = call(&app, "PUT", &format!("/api/projects/{id}/members/{alex_id}"), Some(&alex), Some(json!({ "role": "viewer" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, "PUT", &format!("/api/projects/{id}/members/{alex_id}"), Some(&laouen), Some(json!({ "role": "owner" }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Shares a sample project with a new team holding all three people; returns its id and the
/// id of its one task.
async fn shared(app: &Router, people: &[(String, String); 3]) -> (String, String) {
    let owner = &people[0].0;
    let (_, team) = call(app, "POST", "/api/teams", Some(owner), Some(json!({ "name": "Team" }))).await;
    let team_id = team["team"]["id"].as_str().unwrap().to_owned();
    for email in ["alex@example.com", "sam@example.com"] {
        call(app, "POST", &format!("/api/teams/{team_id}/members"), Some(owner), Some(json!({ "email": email }))).await;
    }
    let project = sample_project("Live");
    let task = project["tasks"].as_object().unwrap().keys().next().unwrap().clone();
    let (_, record) = call(app, "POST", "/api/projects", Some(owner), Some(json!({ "teamId": team_id, "project": project }))).await;
    (record["id"].as_str().unwrap().to_owned(), task)
}

fn rename(task: &str, name: &str) -> Value {
    json!({ "type": "SetTask", "id": task, "change": { "field": "Name", "value": name } })
}

#[tokio::test]
async fn changes_are_ordered_stored_and_broadcast() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let state = AppState { storage: storage.clone(), hub: Arc::default() };
    let app = router(state.clone());
    let p = people(&app).await;
    let (id, task) = shared(&app, &p).await;
    let mut rx = state.hub.room(id.parse().unwrap()).tx.subscribe();

    let edit = |client: &str, command: Value| json!({ "clientChange": client, "command": command });
    let a = uuid::Uuid::now_v7().to_string();
    let (status, first) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(&a, rename(&task, "Alex's name")))).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!((first["version"].as_i64(), first["authorName"].as_str(), first["label"].as_str()), (Some(1), Some("Alex"), Some("Rename Task")));
    assert_eq!(rx.recv().await.unwrap().version, 1, "everyone watching hears it");

    // Sending the same edit again (a lost reply) returns the same change, not a second one.
    let (_, again) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(&a, rename(&task, "Alex's name")))).await;
    assert_eq!(again, first);

    let duration = json!({ "type": "SetTask", "id": task, "change": { "field": "Duration", "value": 2400 } });
    let (_, second) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[0].0), Some(edit(&uuid::Uuid::now_v7().to_string(), duration))).await;
    assert_eq!(second["version"].as_i64(), Some(2));

    // Deleted, then renamed by someone who had not heard: refused in plain words.
    let delete = json!({ "type": "DeleteTasks", "ids": [task] });
    call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[0].0), Some(edit(&uuid::Uuid::now_v7().to_string(), delete))).await;
    let (status, refused) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(&uuid::Uuid::now_v7().to_string(), rename(&task, "Too late")))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(refused["message"].as_str().unwrap().starts_with("this change no longer fits the project"));

    let (_, log) = call(&app, "GET", &format!("/api/projects/{id}/changes?since=0"), Some(&p[2].0), None).await;
    let versions: Vec<i64> = log.as_array().unwrap().iter().map(|c| c["version"].as_i64().unwrap()).collect();
    assert_eq!(versions, [1, 2, 3]);
    let (_, opened) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&p[2].0), None).await;
    assert_eq!(opened["record"]["version"].as_i64(), Some(3));
    assert!(opened["project"]["tasks"].as_object().unwrap().is_empty());

    // Viewers cannot change anything.
    let sam = &p[2].1;
    call(&app, "PUT", &format!("/api/projects/{id}/members/{sam}"), Some(&p[0].0), Some(json!({ "role": "viewer" }))).await;
    let insert = json!({ "type": "InsertTasks", "tasks": [], "dependencies": [] });
    let (status, body) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[2].0), Some(edit(&uuid::Uuid::now_v7().to_string(), insert))).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::FORBIDDEN, Some("you can view this project but not change it")));
}

#[tokio::test]
async fn a_restarted_server_rebuilds_projects_from_snapshots_and_changes() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let app = router(AppState { storage: storage.clone(), hub: Arc::default() });
    let p = people(&app).await;
    let (id, task) = shared(&app, &p).await;
    // Past one saved snapshot (every 100 changes), then a few more.
    for n in 1..=105 {
        let body = json!({ "clientChange": uuid::Uuid::now_v7().to_string(), "command": rename(&task, &format!("Name {n}")) });
        let (status, _) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[0].0), Some(body)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (snapshot, at) = storage.project_snapshot(id.parse().unwrap()).unwrap().unwrap();
    assert_eq!(at, 100);
    assert!(snapshot.contains("Name 100"));

    // A new server process: nothing in memory.
    let restarted = router(AppState { storage, hub: Arc::default() });
    let (_, opened) = call(&restarted, "GET", &format!("/api/projects/{id}"), Some(&p[1].0), None).await;
    assert_eq!(opened["record"]["version"].as_i64(), Some(105));
    assert_eq!(opened["project"]["tasks"][&task]["name"], "Name 105");
}

#[tokio::test]
async fn the_history_reads_in_plain_words_and_the_owner_can_revert() {
    let app = server();
    let p = people(&app).await;
    let (id, task) = shared(&app, &p).await;
    let edit = |command: Value| json!({ "clientChange": uuid::Uuid::now_v7().to_string(), "command": command });
    call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(rename(&task, "Site survey")))).await;
    let duration = json!({ "type": "SetTask", "id": task, "change": { "field": "Duration", "value": 2400 } });
    call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[2].0), Some(edit(duration))).await;

    let (status, history) = call(&app, "GET", &format!("/api/projects/{id}/history"), Some(&p[1].0), None).await;
    assert_eq!(status, StatusCode::OK);
    let lines: Vec<(&str, &str)> = history["changes"].as_array().unwrap().iter().map(|c| (c["authorName"].as_str().unwrap(), c["summary"].as_str().unwrap())).collect();
    assert_eq!(lines, [("Sam", "changed the duration of “Site survey” from 2 days to 5 days"), ("Alex", "renamed “Survey” to “Site survey”")]);
    assert_eq!(history["canRevert"], false, "only the owner reverts");
    let (_, page) = call(&app, "GET", &format!("/api/projects/{id}/history?before=2&limit=10"), Some(&p[1].0), None).await;
    assert_eq!(page["changes"].as_array().unwrap().len(), 1);

    // Alex may not revert; the owner may, and the revert is a new change in the history.
    let (status, _) = call(&app, "POST", &format!("/api/projects/{id}/revert"), Some(&p[1].0), Some(json!({ "version": 2 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, reverted) = call(&app, "POST", &format!("/api/projects/{id}/revert"), Some(&p[0].0), Some(json!({ "version": 2 }))).await;
    assert_eq!(status, StatusCode::OK, "{reverted}");
    assert_eq!(reverted["version"], 3);
    assert_eq!(reverted["summary"], "reverted Sam's change: changed the duration of “Site survey” from 2 days to 5 days");
    let (_, opened) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&p[0].0), None).await;
    assert_eq!(opened["project"]["tasks"][&task]["duration"], 960);

    // A change that later changes depend on cannot be reverted.
    let delete = json!({ "type": "DeleteTasks", "ids": [task] });
    call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[0].0), Some(edit(delete))).await;
    let (status, body) = call(&app, "POST", &format!("/api/projects/{id}/revert"), Some(&p[0].0), Some(json!({ "version": 1 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["message"].as_str().unwrap().starts_with("this change cannot be reverted any more"));
}

#[tokio::test]
async fn teams_have_channels_with_posts_and_replies() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let state = AppState { storage, hub: Arc::default() };
    let app = router(state.clone());
    let p = people(&app).await;
    let (_, team) = call(&app, "POST", "/api/teams", Some(&p[0].0), Some(json!({ "name": "Office move" }))).await;
    let team_id = team["team"]["id"].as_str().unwrap().to_owned();
    call(&app, "POST", &format!("/api/teams/{team_id}/members"), Some(&p[0].0), Some(json!({ "email": "alex@example.com" }))).await;
    let project = sample_project("Office move");
    call(&app, "POST", "/api/projects", Some(&p[0].0), Some(json!({ "teamId": team_id, "project": project }))).await;

    // General first, then the project's own channel.
    let (status, channels) = call(&app, "GET", &format!("/api/teams/{team_id}/channels"), Some(&p[1].0), None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = channels.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["General", "Office move"]);
    assert!(channels[1]["projectId"].is_string());
    let general = channels[0]["id"].as_str().unwrap().to_owned();

    let mut rx = state.hub.chat.subscribe();
    let (status, post) = call(&app, "POST", &format!("/api/channels/{general}/posts"), Some(&p[0].0), Some(json!({ "body": "Kick-off tomorrow at 9, @Alex can you book the room?" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(&*rx.recv().await.unwrap(), chat::ChatEvent::Post { .. }), "everyone in the team hears it");
    let post_id = post["id"].as_str().unwrap().to_owned();
    let (status, reply) = call(&app, "POST", &format!("/api/channels/{general}/posts"), Some(&p[1].0), Some(json!({ "body": "Booked.", "parentId": post_id }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "POST", &format!("/api/channels/{general}/posts"), Some(&p[0].0), Some(json!({ "body": "Thanks", "parentId": reply["id"] }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "replies are one level deep");

    let (_, page) = call(&app, "GET", &format!("/api/channels/{general}/posts"), Some(&p[1].0), None).await;
    assert_eq!(page["threads"][0]["post"]["authorName"], "Laouen");
    assert_eq!(page["threads"][0]["replies"][0]["body"], "Booked.");

    // Unread: Laouen has one reply to read; reading the channel clears it.
    let (_, channels) = call(&app, "GET", &format!("/api/teams/{team_id}/channels"), Some(&p[0].0), None).await;
    assert_eq!(channels[0]["unread"], 1);
    call(&app, "POST", &format!("/api/channels/{general}/read"), Some(&p[0].0), None).await;
    let (_, channels) = call(&app, "GET", &format!("/api/teams/{team_id}/channels"), Some(&p[0].0), None).await;
    assert_eq!(channels[0]["unread"], 0);

    // People outside the team see nothing.
    let (status, _) = call(&app, "GET", &format!("/api/channels/{general}/posts"), Some(&p[2].0), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, channel) = call(&app, "POST", &format!("/api/teams/{team_id}/channels"), Some(&p[1].0), Some(json!({ "name": "general" }))).await;
    assert_eq!((status, channel["name"].as_str()), (StatusCode::OK, Some("general 2")), "names stay distinct");
}
