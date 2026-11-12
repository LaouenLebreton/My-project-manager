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
        deadline: None,
        elapsed: false,
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
    let (_, mine) = call(&app, "GET", &format!("/api/projects/{id}/access"), Some(&alex), None).await;
    assert_eq!(mine, json!({ "role": "editor", "limits": limits }));
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
    let state = AppState { storage: storage.clone(), hub: Arc::default(), config: Arc::default() };
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
    let app = router(AppState { storage: storage.clone(), hub: Arc::default(), config: Arc::default() });
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
    let restarted = router(AppState { storage, hub: Arc::default(), config: Arc::default() });
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
    let state = AppState { storage, hub: Arc::default(), config: Arc::default() };
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

#[tokio::test]
async fn team_owners_invite_people_and_open_servers_take_sign_ups() {
    let app = server();
    let p = people(&app).await;
    let (_, team) = call(&app, "POST", "/api/teams", Some(&p[1].0), Some(json!({ "name": "Alex's team" }))).await;
    let team_id = team["team"]["id"].as_str().unwrap().to_owned();
    // Alex is not the server administrator, but owns the team.
    let (status, invites) = call(&app, "POST", &format!("/api/teams/{team_id}/invites"), Some(&p[1].0), Some(json!({ "emails": ["new.person@example.com", "sam@example.com", "nope"] }))).await;
    assert_eq!(status, StatusCode::OK, "{invites}");
    let code = invites[0]["code"].as_str().unwrap().to_owned();
    assert!(invites[1]["code"].is_null(), "Sam already has an account");
    assert_eq!(invites[2]["note"], "not an email address");
    let (status, session) = call(&app, "POST", "/api/sessions", None, Some(json!({ "email": "new.person@example.com", "password": code }))).await;
    assert_eq!((status, session["user"]["name"].as_str()), (StatusCode::OK, Some("New Person")));
    let (_, teams) = call(&app, "GET", "/api/teams", Some(session["token"].as_str().unwrap()), None).await;
    assert_eq!(teams[0]["team"]["name"], "Alex's team");
    // Members who do not own the team cannot invite.
    let (status, _) = call(&app, "POST", &format!("/api/teams/{team_id}/invites"), Some(&p[2].0), Some(json!({ "emails": ["x@example.com"] }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Sign-ups: refused here, accepted on an open server.
    let (status, _) = call(&app, "POST", "/api/signup", None, Some(account("Kim", "kim@example.com"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let open = router(AppState::with_config(SqliteStorage::in_memory().unwrap(), Config { public_url: Some("https://pm.example.com".into()), open_signup: true }));
    let (status, s) = call(&open, "POST", "/api/signup", None, Some(account("Kim", "kim@example.com"))).await;
    assert_eq!((status, s["user"]["isAdmin"].as_bool()), (StatusCode::OK, Some(false)));
    let (_, info) = call(&open, "GET", "/api/server", None, None).await;
    assert_eq!(info, json!({ "openSignup": true, "publicUrl": "https://pm.example.com" }));
}

#[tokio::test]
async fn posts_can_be_edited_deleted_and_reacted_to() {
    let app = server();
    let p = people(&app).await;
    let (_, team) = call(&app, "POST", "/api/teams", Some(&p[0].0), Some(json!({ "name": "T" }))).await;
    let team_id = team["team"]["id"].as_str().unwrap().to_owned();
    call(&app, "POST", &format!("/api/teams/{team_id}/members"), Some(&p[0].0), Some(json!({ "email": "alex@example.com" }))).await;
    let (_, channels) = call(&app, "GET", &format!("/api/teams/{team_id}/channels"), Some(&p[1].0), None).await;
    let general = channels[0]["id"].as_str().unwrap().to_owned();
    let (_, post) = call(&app, "POST", &format!("/api/channels/{general}/posts"), Some(&p[1].0), Some(json!({ "body": "Meetng at 9" }))).await;
    let id = post["id"].as_str().unwrap().to_owned();

    let (status, edited) = call(&app, "PUT", &format!("/api/posts/{id}"), Some(&p[1].0), Some(json!({ "body": "Meeting at 9" }))).await;
    assert_eq!((status, edited["body"].as_str()), (StatusCode::OK, Some("Meeting at 9")));
    assert!(edited["editedAt"].is_string());
    let (status, _) = call(&app, "PUT", &format!("/api/posts/{id}"), Some(&p[0].0), Some(json!({ "body": "Hijacked" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "only the author edits");

    let (_, reacted) = call(&app, "POST", &format!("/api/posts/{id}/reactions"), Some(&p[0].0), Some(json!({ "emoji": "👍" }))).await;
    assert_eq!(reacted["reactions"][0]["userName"], "Laouen");
    let (_, again) = call(&app, "POST", &format!("/api/posts/{id}/reactions"), Some(&p[0].0), Some(json!({ "emoji": "👍" }))).await;
    assert_eq!(again["reactions"], json!([]), "a second click takes it back");
    let (status, _) = call(&app, "POST", &format!("/api/posts/{id}/reactions"), Some(&p[0].0), Some(json!({ "emoji": "💩" }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The team's owner may delete anyone's post; it stays as "deleted".
    let (status, deleted) = call(&app, "DELETE", &format!("/api/posts/{id}"), Some(&p[0].0), None).await;
    assert_eq!((status, deleted["deleted"].as_bool(), deleted["body"].as_str()), (StatusCode::OK, Some(true), Some("")));
}

#[tokio::test]
async fn a_team_moves_to_another_server_with_its_history_and_chat() {
    // Source: the team as it lives on someone's Mac.
    let source = server();
    let p = people(&source).await;
    let (id, task) = shared(&source, &p).await;
    let edit = |command: Value| json!({ "clientChange": uuid::Uuid::now_v7().to_string(), "command": command });
    call(&source, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(rename(&task, "Site survey")))).await;
    let (_, teams) = call(&source, "GET", "/api/teams", Some(&p[0].0), None).await;
    let team_id = teams[0]["team"]["id"].as_str().unwrap().to_owned();
    let (_, channels) = call(&source, "GET", &format!("/api/teams/{team_id}/channels"), Some(&p[0].0), None).await;
    let general = channels.as_array().unwrap().iter().find(|c| c["name"] == "General").unwrap()["id"].as_str().unwrap().to_owned();
    let (_, post) = call(&source, "POST", &format!("/api/channels/{general}/posts"), Some(&p[2].0), Some(json!({ "body": "Hello from the Mac" }))).await;
    call(&source, "POST", &format!("/api/posts/{}/reactions", post["id"].as_str().unwrap()), Some(&p[1].0), Some(json!({ "emoji": "🎉" }))).await;

    // Only an owner exports.
    let (status, _) = call(&source, "GET", &format!("/api/teams/{team_id}/export"), Some(&p[1].0), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, bundle) = call(&source, "GET", &format!("/api/teams/{team_id}/export"), Some(&p[0].0), None).await;
    assert_eq!(status, StatusCode::OK);

    // Target: an online server where only the owner has an account so far.
    let target = router(AppState::with_config(SqliteStorage::in_memory().unwrap(), Config { public_url: Some("https://pm.example.com".into()), open_signup: true }));
    let (_, me) = call(&target, "POST", "/api/signup", None, Some(account("Laouen", "laouen@example.com"))).await;
    let me = me["token"].as_str().unwrap().to_owned();
    let (status, imported) = call(&target, "POST", "/api/teams/import", Some(&me), Some(bundle.clone())).await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    let invited: Vec<&str> = imported["invites"].as_array().unwrap().iter().map(|i| i["email"].as_str().unwrap()).collect();
    assert_eq!(invited, ["alex@example.com", "sam@example.com"], "the others get new invites");
    assert_eq!(imported["invites"][0]["server"], "https://pm.example.com");
    let (status, _) = call(&target, "POST", "/api/teams/import", Some(&me), Some(bundle)).await;
    assert_eq!(status, StatusCode::CONFLICT, "imported once");

    // Same project, same version, same history; the chat came along.
    let (_, opened) = call(&target, "GET", &format!("/api/projects/{id}"), Some(&me), None).await;
    assert_eq!((opened["record"]["version"].as_i64(), opened["role"].as_str()), (Some(1), Some("owner")));
    assert_eq!(opened["project"]["tasks"][&task]["name"], "Site survey");
    let (_, history) = call(&target, "GET", &format!("/api/projects/{id}/history"), Some(&me), None).await;
    assert_eq!(history["changes"][0]["authorName"], "Alex");
    let new_team = imported["teamId"].as_str().unwrap();
    let (_, chans) = call(&target, "GET", &format!("/api/teams/{new_team}/channels"), Some(&me), None).await;
    let (_, page) = call(&target, "GET", &format!("/api/channels/{general}/posts"), Some(&me), None).await;
    assert!(chans.as_array().unwrap().len() >= 2);
    assert_eq!(page["threads"][0]["post"]["body"], "Hello from the Mac");
    assert_eq!(page["threads"][0]["post"]["reactions"][0]["emoji"], "🎉");
    // Alex signs in with the invite code and carries on editing at version 2.
    let code = imported["invites"][0]["code"].as_str().unwrap();
    let (_, alex) = call(&target, "POST", "/api/sessions", None, Some(json!({ "email": "alex@example.com", "password": code }))).await;
    let (_, next) = call(&target, "POST", &format!("/api/projects/{id}/changes"), Some(alex["token"].as_str().unwrap()), Some(edit(rename(&task, "Survey (online)")))).await;
    assert_eq!(next["version"], 2);

    // The source says where the team went.
    call(&source, "POST", &format!("/api/teams/{team_id}/moved"), Some(&p[0].0), Some(json!({ "to": "https://pm.example.com/" }))).await;
    let (status, body) = call(&source, "GET", &format!("/api/projects/{id}/access"), Some(&p[1].0), None).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::GONE, Some("this team has moved to https://pm.example.com")));
}

#[tokio::test]
async fn rules_post_when_tasks_are_done_and_send_a_weekly_summary() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let state = AppState { storage: storage.clone(), hub: Arc::default(), config: Arc::default() };
    let app = router(state.clone());
    let p = people(&app).await;
    let (id, task) = shared(&app, &p).await;
    let pid: uuid::Uuid = id.parse().unwrap();
    let rules_path = format!("/api/projects/{id}/rules");

    // No rules yet; only the owner sets them.
    let (status, none) = call(&app, "GET", &rules_path, Some(&p[1].0), None).await;
    assert_eq!((status, none), (StatusCode::OK, json!({ "postWhenDone": false, "weeklySummary": null })));
    let rules = json!({ "postWhenDone": true, "weeklySummary": { "weekday": 0, "hour": 9 } });
    let (status, body) = call(&app, "PUT", &rules_path, Some(&p[1].0), Some(rules.clone())).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::FORBIDDEN, Some("only the project's owner can change its rules")));
    let (status, _) = call(&app, "PUT", &rules_path, Some(&p[0].0), Some(json!({ "postWhenDone": true, "weeklySummary": { "weekday": 7, "hour": 9 } }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, saved) = call(&app, "PUT", &rules_path, Some(&p[0].0), Some(rules.clone())).await;
    assert_eq!((status, saved), (StatusCode::OK, rules));

    let channel = storage.project_channel(pid).unwrap().expect("a team project has a channel");
    let posts = || storage.posts(channel.id, None, 50).unwrap();
    let before = posts().len();

    // Alex renames the task: nothing to say. Alex marks it 100%: one post, from Super PM.
    let edit = |command: Value| json!({ "clientChange": uuid::Uuid::now_v7().to_string(), "command": command });
    call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(rename(&task, "Survey")))).await;
    assert_eq!(posts().len(), before);
    let done = json!({ "type": "SetTask", "id": task, "change": { "field": "PercentComplete", "value": 100 } });
    let (status, change) = call(&app, "POST", &format!("/api/projects/{id}/changes"), Some(&p[1].0), Some(edit(done))).await;
    assert_eq!(status, StatusCode::OK, "{change}");
    let list = posts();
    assert_eq!(list.len(), before + 1);
    assert_eq!((list[0].body.as_str(), list[0].author_name.as_str(), list[0].automatic), ("✅ “Survey” is done (marked by Alex).", "Super PM", true));
    // It counts as new for everyone, the owner who set the rule included.
    let unread = storage.unread(uuid::Uuid::parse_str(&p[0].1).unwrap(), channel.team_id).unwrap();
    assert!(unread.iter().any(|(c, n)| *c == channel.id && *n >= 1));

    // Weekly summary: Monday 9:00 and later that day, once; not on Sunday or at 8:00.
    let at = |d: i8, h: i8| jiff::civil::date(2026, 11, d).at(h, 0, 0, 0);
    rules::tick(&state, at(1, 10)).await; // Sunday
    rules::tick(&state, at(2, 8)).await;
    assert_eq!(posts().len(), before + 1);
    rules::tick(&state, at(2, 9)).await;
    rules::tick(&state, at(2, 15)).await;
    let list = posts();
    assert_eq!(list.len(), before + 2, "once that day");
    assert!(list[0].body.starts_with("📊 Weekly status for Live: Done · 100% done"), "{}", list[0].body);
    rules::tick(&state, at(9, 9)).await;
    assert_eq!(posts().len(), before + 3, "and again the next Monday");
}

// ---------------------------------------------------------------------------------------------
// Security review (2026-11-11)

#[tokio::test]
async fn only_the_administrator_brings_in_a_team_on_a_closed_server() {
    let source = server();
    let p = people(&source).await;
    shared(&source, &p).await;
    let (_, teams) = call(&source, "GET", "/api/teams", Some(&p[0].0), None).await;
    let team_id = teams[0]["team"]["id"].as_str().unwrap().to_owned();
    let (_, bundle) = call(&source, "GET", &format!("/api/teams/{team_id}/export"), Some(&p[0].0), None).await;

    // A closed server: Alex (not the administrator) cannot create accounts by importing.
    let target = server();
    let t = people(&target).await;
    let (status, body) = call(&target, "POST", "/api/teams/import", Some(&t[1].0), Some(bundle.clone())).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::FORBIDDEN, Some("only the server's administrator can bring in a team from another server")));

    // The administrator can; people who already have accounts are not added without an invite.
    let (status, imported) = call(&target, "POST", "/api/teams/import", Some(&t[0].0), Some(bundle)).await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    let (_, alex_teams) = call(&target, "GET", "/api/teams", Some(&t[1].0), None).await;
    assert_eq!(alex_teams, json!([]), "Alex already had an account here: invited, not added");
    let note = imported["invites"].as_array().unwrap().iter().find(|i| i["email"] == "alex@example.com").unwrap();
    assert!(note["code"].is_null());
}

#[tokio::test]
async fn owners_remove_people_from_a_team() {
    let app = server();
    let p = people(&app).await;
    let (id, _) = shared(&app, &p).await;
    let (_, opened) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&p[0].0), None).await;
    let team = opened["record"]["teamId"].as_str().unwrap().to_owned();
    let (alex, sam) = (&p[1].1, &p[2].1);

    let (status, _) = call(&app, "DELETE", &format!("/api/teams/{team}/members/{sam}"), Some(&p[1].0), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "members cannot remove others");
    let (status, body) = call(&app, "DELETE", &format!("/api/teams/{team}/members/{}", p[0].1), Some(&p[0].0), None).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("this person owns projects in the team; they stay until someone else owns them")));

    let (status, _) = call(&app, "DELETE", &format!("/api/teams/{team}/members/{alex}"), Some(&p[0].0), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", &format!("/api/projects/{id}"), Some(&p[1].0), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "Alex no longer sees the project");
    let (_, teams) = call(&app, "GET", "/api/teams", Some(&p[1].0), None).await;
    assert_eq!(teams, json!([]));
    // Members may leave on their own.
    let (status, _) = call(&app, "DELETE", &format!("/api/teams/{team}/members/{sam}"), Some(&p[2].0), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn unused_sessions_end() {
    let storage = SqliteStorage::in_memory().unwrap();
    let user = User { id: uuid::Uuid::now_v7(), name: "A".into(), email: "a@example.com".into(), is_admin: false, created_at: now() };
    storage.create_user(&user, "x").unwrap();
    let old = (jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * 91)).to_string();
    let recent = (jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * 30)).to_string();
    storage.create_session("old", user.id, &old).unwrap();
    storage.create_session("recent", user.id, &recent).unwrap();
    assert!(storage.session_user("old", jiff::Timestamp::now()).unwrap().is_none(), "unused for 91 days");
    assert!(storage.session_user("recent", jiff::Timestamp::now()).unwrap().is_some());
}

#[tokio::test]
async fn repeated_wrong_passwords_are_slowed_down() {
    let app = server();
    people(&app).await;
    let wrong = json!({ "email": "alex@example.com", "password": "not the password" });
    for _ in 0..5 {
        let (status, _) = call(&app, "POST", "/api/sessions", None, Some(wrong.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let right = json!({ "email": "alex@example.com", "password": "a long enough password" });
    let (status, body) = call(&app, "POST", "/api/sessions", None, Some(right)).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too many wrong passwords for this account; try again in 15 minutes")));
    // Unknown emails answer like wrong passwords.
    let (status, _) = call(&app, "POST", "/api/sessions", None, Some(json!({ "email": "nobody@example.com", "password": "whatever it is" }))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invites_are_checked_before_any_work_and_capped() {
    let app = server();
    let p = people(&app).await;
    let (_, team) = call(&app, "POST", "/api/teams", Some(&p[0].0), Some(json!({ "name": "T" }))).await;
    let team = team["team"]["id"].as_str().unwrap().to_owned();
    let many: Vec<String> = (0..101).map(|i| format!("p{i}@example.com")).collect();
    let (status, body) = call(&app, "POST", &format!("/api/teams/{team}/invites"), Some(&p[0].0), Some(json!({ "emails": many }))).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invite at most 100 people at a time")));
    let (status, _) = call(&app, "POST", &format!("/api/teams/{team}/invites"), Some(&p[1].0), Some(json!({ "emails": ["x@example.com"] }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not in the team");
}
