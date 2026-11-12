//! Automation rules for team projects (iteration 4). The owner sets them; the server runs
//! them, so each happens once however many people have the project open:
//!
//! - when tasks reach 100%, a post in the project's channel;
//! - every week on a chosen day and hour, a status summary there.
//!
//! Posts written by a rule are marked automatic and shown as from Super PM.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pm_core::{Moment, Project, TaskId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::access::Role;
use crate::chat::ChatEvent;
use crate::sharing::access;
use crate::storage::Post;
use crate::{db, now, ApiError, ApiResult, AppState, Auth};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// Post in the project's channel when tasks reach 100%.
    #[serde(default)]
    pub post_when_done: bool,
    /// A status summary in the project's channel every week.
    #[serde(default)]
    pub weekly_summary: Option<Weekly>,
    /// Post when a task becomes late (due by now and not finished), once per task.
    #[serde(default)]
    pub post_when_late: bool,
    /// Post when a milestone is reached (marked 100%).
    #[serde(default)]
    pub post_when_milestone: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weekly {
    /// Monday = 0 … Sunday = 6.
    pub weekday: u8,
    /// Hour of the day on the server's clock, 0–23.
    pub hour: u8,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/projects/{id}/rules", get(read).put(write))
}

async fn read(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>) -> ApiResult<Rules> {
    access(&state, id, me.id).await?;
    Ok(Json(rules_of(&state, id).await?.map(|(r, _)| r).unwrap_or_default()))
}

async fn write(State(state): State<AppState>, Auth(me): Auth, Path(id): Path<Uuid>, Json(rules): Json<Rules>) -> ApiResult<Rules> {
    let (_, role, _) = access(&state, id, me.id).await?;
    if role != Role::Owner {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the project's owner can change its rules"));
    }
    if rules.weekly_summary.is_some_and(|w| w.weekday > 6 || w.hour > 23) {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "choose a day of the week and an hour from 0 to 23"));
    }
    let json = serde_json::to_string(&rules).expect("rules serialize");
    db(&state, move |s| s.set_rules(id, &json, me.id)).await?;
    Ok(Json(rules))
}

async fn rules_of(state: &AppState, project: Uuid) -> Result<Option<(Rules, Uuid)>, ApiError> {
    let stored = db(state, move |s| s.rules(project)).await?;
    Ok(stored.and_then(|s| serde_json::from_str(&s.rules).ok().map(|r| (r, s.set_by))))
}

/// Writes an automatic post in the project's channel. Quietly does nothing if the project has
/// no channel (it always should).
async fn post(state: &AppState, project: Uuid, set_by: Uuid, body: String) -> Result<(), ApiError> {
    let Some(channel) = db(state, move |s| s.project_channel(project)).await? else { return Ok(()) };
    let post = Post {
        id: Uuid::now_v7(),
        channel_id: channel.id,
        parent_id: None,
        author_id: set_by,
        author_name: "Super PM".into(),
        at: now(),
        body,
        edited_at: None,
        deleted: false,
        reactions: Vec::new(),
        automatic: true,
    };
    let stored = post.clone();
    db(state, move |s| s.add_post(&stored)).await?;
    let _ = state.hub.chat.send(Arc::new(ChatEvent::Post { team_id: channel.team_id, post }));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// When tasks reach 100%

/// Leaf tasks not yet complete, to compare with after a change. Empty (and cheap) unless the
/// project posts when tasks are done.
pub async fn watch_done(state: &AppState, project: Uuid, p: &Project) -> Option<BTreeSet<TaskId>> {
    match rules_of(state, project).await {
        Ok(Some((r, _))) if r.post_when_done || r.post_when_milestone => Some(open_tasks(p)),
        _ => None,
    }
}

fn open_tasks(p: &Project) -> BTreeSet<TaskId> {
    let parents: BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
    p.tasks.values().filter(|t| !parents.contains(&t.id) && t.percent_complete.get() < 100).map(|t| t.id).collect()
}

/// The post for tasks that were open before and are complete now, if any: milestones reached
/// (with `milestones`) and other tasks done (with `tasks`).
pub fn done_text(p: &Project, open_before: &BTreeSet<TaskId>, by: &str, tasks: bool, milestones: bool) -> Option<String> {
    let open_now = open_tasks(p);
    let finished: Vec<&pm_core::Task> = open_before.iter().filter(|id| !open_now.contains(id)).filter_map(|id| p.tasks.get(id)).collect();
    fn names(list: Vec<&pm_core::Task>) -> Vec<&str> {
        let mut v: Vec<&str> = list.into_iter().map(|t| t.name.as_str()).collect();
        v.sort_unstable();
        v
    }
    let (reached, done): (Vec<&pm_core::Task>, Vec<&pm_core::Task>) = finished.into_iter().partition(|t| milestones && t.is_milestone);
    let mut lines = Vec::new();
    match names(reached).as_slice() {
        [] => {}
        [one] => lines.push(format!("◆ Milestone “{one}” reached (marked by {by}).")),
        many => lines.push(format!("◆ {} milestones reached (marked by {by}): {}.", many.len(), quoted(many, 5))),
    }
    if tasks {
        match names(done).as_slice() {
            [] => {}
            [one] => lines.push(format!("✅ “{one}” is done (marked by {by}).")),
            many => lines.push(format!("✅ {} tasks are done (marked by {by}): {}.", many.len(), quoted(many, 5))),
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// After a change: posts about the tasks it completed.
pub async fn after_change(state: &AppState, project: Uuid, p: &Project, open_before: &BTreeSet<TaskId>, by: &str) {
    let Ok(Some((rules, set_by))) = rules_of(state, project).await else { return };
    if let Some(text) = done_text(p, open_before, by, rules.post_when_done, rules.post_when_milestone) {
        let _ = post(state, project, set_by, text).await;
    }
}

fn quoted(names: &[&str], at_most: usize) -> String {
    let mut out: Vec<String> = names.iter().take(at_most).map(|n| format!("“{n}”")).collect();
    if names.len() > at_most {
        out.push(format!("and {} more", names.len() - at_most));
    }
    out.join(", ")
}

// ---------------------------------------------------------------------------------------------
// Weekly summary

fn short_date(m: Moment) -> String {
    m.to_datetime().strftime("%a %-d %b").to_string()
}

/// The weekly status post: status with its reasons, progress and finish, what is late, what
/// is due in the coming week, and the next milestone.
pub fn weekly_text(p: &Project, now: Moment) -> String {
    let Ok(s) = pm_core::schedule(p) else {
        return format!("Weekly status for {}: its dates cannot be calculated right now; open it to see why.", p.name);
    };
    let l = pm_core::loads(p, &s);
    let sum = pm_core::portfolio::summarize(p, &s, &l, now);
    let status = match sum.status {
        pm_core::portfolio::Status::OnTrack => "On track",
        pm_core::portfolio::Status::AtRisk => "At risk",
        pm_core::portfolio::Status::Late => "Late",
        pm_core::portfolio::Status::Done => "Done",
    };
    let mut lines = vec![format!("📊 Weekly status for {}: {status} · {}% done · finishes {}", p.name, sum.percent_complete, short_date(sum.finish))];
    if !sum.reasons.is_empty() {
        lines.push(format!("Why: {}.", sum.reasons.join("; ")));
    }
    let parents: BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
    let week = now.day() + 7;
    let mut late = Vec::new();
    let mut due = Vec::new();
    let mut leaves: Vec<_> = p.tasks.values().filter(|t| !parents.contains(&t.id) && t.percent_complete.get() < 100).filter_map(|t| s.tasks.get(&t.id).map(|ts| (ts.early_finish, t))).collect();
    leaves.sort_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    for (finish, t) in leaves {
        if finish <= now {
            late.push(t.name.as_str());
        } else if finish.day() < week {
            due.push(format!("“{}” ({})", t.name, short_date(finish)));
        }
    }
    if !late.is_empty() {
        lines.push(format!("Late: {}.", quoted(&late, 5)));
    }
    if !due.is_empty() {
        let more = due.len().saturating_sub(5);
        due.truncate(5);
        lines.push(format!("Due this week: {}{}.", due.join(", "), if more > 0 { format!(", and {more} more") } else { String::new() }));
    }
    if let Some((_, name, at)) = sum.next_milestone {
        lines.push(format!("Next milestone: ◆ {name} ({}).", short_date(at)));
    }
    lines.join("\n")
}

/// Sends the weekly summaries due at `now` (server time): on the chosen weekday, from the
/// chosen hour, once that day.
pub async fn tick(state: &AppState, now: jiff::civil::DateTime) {
    let Ok(all) = db(state, |s| s.all_rules()).await else { return };
    let today = now.date().to_string();
    let weekday = now.date().weekday().to_monday_zero_offset() as u8;
    for (project, stored) in all {
        let Ok(rules) = serde_json::from_str::<Rules>(&stored.rules) else { continue };
        if rules.post_when_late {
            late_check(state, project, &stored, now).await;
        }
        let Some(w) = rules.weekly_summary else { continue };
        if w.weekday != weekday || (now.hour() as u8) < w.hour || stored.last_summary.as_deref() == Some(today.as_str()) {
            continue;
        }
        let at = Moment::from_datetime(now);
        let Ok(text) = crate::live::with_live(state, project, |live| weekly_text(&live.project, at)).await else { continue };
        let day = today.clone();
        if db(state, move |s| s.set_last_summary(project, &day)).await.is_ok() {
            let _ = post(state, project, stored.set_by, text).await;
        }
    }
}

/// When a task becomes late: one post for the tasks that are newly late. The tasks already
/// posted about are remembered; one that stops being late (finished, or moved) is forgotten, so
/// it is posted about again if it becomes late again.
async fn late_check(state: &AppState, project: Uuid, stored: &crate::storage::StoredRules, now: jiff::civil::DateTime) {
    let at = Moment::from_datetime(now);
    let Ok(late) = crate::live::with_live(state, project, |live| {
        let p = &live.project;
        let Ok(s) = pm_core::schedule(p) else { return Vec::new() };
        let parents: BTreeSet<TaskId> = p.tasks.values().filter_map(|t| t.parent).collect();
        let mut late: Vec<(String, String, Moment, String)> = p
            .tasks
            .values()
            .filter(|t| !parents.contains(&t.id) && t.percent_complete.get() < 100)
            .filter_map(|t| s.tasks.get(&t.id).filter(|ts| ts.early_finish <= at).map(|ts| (t, ts.early_finish)))
            .map(|(t, finish)| {
                let people: Vec<&str> = p.assignments.values().filter(|a| a.task == t.id).filter_map(|a| p.resources.get(&a.resource)).map(|r| r.name.as_str()).collect();
                (t.id.0.to_string(), t.name.clone(), finish, people.join(", "))
            })
            .collect();
        late.sort_by(|a, b| (a.2, &a.1).cmp(&(b.2, &b.1)));
        late
    })
    .await
    else {
        return;
    };
    let notified: BTreeSet<String> = serde_json::from_str(&stored.late_notified).unwrap_or_default();
    let current: BTreeSet<String> = late.iter().map(|x| x.0.clone()).collect();
    let fresh: Vec<&(String, String, Moment, String)> = late.iter().filter(|x| !notified.contains(&x.0)).collect();
    if current != notified {
        let json = serde_json::to_string(&current).expect("ids serialize");
        if db(state, move |s| s.set_late_notified(project, &json)).await.is_err() {
            return; // not remembered: do not post, or it would be posted again
        }
    }
    let who = |people: &str| if people.is_empty() { String::new() } else { format!(" ({people})") };
    let text = match fresh.as_slice() {
        [] => return,
        [(_, name, finish, people)] => format!("⏰ “{name}” is late: it should have finished on {}{}.", short_date(*finish), who(people)),
        many => {
            let list: Vec<String> = many.iter().take(5).map(|(_, name, finish, _)| format!("“{name}” ({})", short_date(*finish))).collect();
            let more = if many.len() > 5 { format!(", and {} more", many.len() - 5) } else { String::new() };
            format!("⏰ {} tasks are late: {}{more}.", many.len(), list.join(", "))
        }
    };
    let _ = post(state, project, stored.set_by, text).await;
}

/// Runs the weekly summaries: checks once a minute.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        loop {
            tick(&state, jiff::Zoned::now().datetime()).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
