//! Task reminders (iteration 4, automation): a macOS notification when a task in an open
//! project is late or due within the next working day. Each is sent once (late ones once a
//! day); in a team project only for tasks assigned to you (matched by name). Window > Task
//! Reminders turns them off.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use pm_core::reminders::{reminders, Reminder, ReminderKind};
use pm_core::{Moment, Project, Schedule};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::actions::AppState;

/// How often open projects are checked.
const EVERY: Duration = Duration::from_secs(15 * 60);
/// Sent reminders are forgotten after this many days.
const KEEP_DAYS: i64 = 30;

#[derive(Serialize, Deserialize)]
struct Saved {
    enabled: bool,
    /// Reminder key → the day it was sent.
    sent: BTreeMap<String, i64>,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { enabled: true, sent: BTreeMap::new() }
    }
}

fn file(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::hosting::profiled(app.path().app_config_dir().ok()?).join("reminders.json"))
}

fn saved(app: &AppHandle) -> Saved {
    file(app).and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(app: &AppHandle, s: &Saved) {
    if let Some(f) = file(app) {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, serde_json::to_vec_pretty(s).expect("reminders serialize"));
    }
}

pub fn enabled(app: &AppHandle) -> bool {
    saved(app).enabled
}

/// Window > Task Reminders.
pub fn set_enabled(app: &AppHandle, on: bool) {
    let mut s = saved(app);
    s.enabled = on;
    save(app, &s);
}

/// One notification.
#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

fn key(p: &Project, r: &Reminder, today: i64) -> String {
    // Late: again each day it stays late. Due soon: once per due date.
    let (kind, day) = match r.kind {
        ReminderKind::Late => ("late", today),
        ReminderKind::DueSoon => ("soon", r.due.day()),
    };
    format!("{}/{}/{kind}/{day}", p.id.0, r.task.0)
}

fn when(due: Moment, now: Moment) -> String {
    let d = due.to_datetime();
    if due.day() == now.day() {
        format!("today at {}", d.strftime("%H:%M"))
    } else if due.day() == now.day() + 1 {
        format!("tomorrow at {}", d.strftime("%H:%M"))
    } else {
        d.strftime("%a %-d %b").to_string()
    }
}

/// What to tell about one project now, and the keys that marks as sent. `me`: in a team
/// project, the person's name (only tasks assigned to them count).
pub fn pick(p: &Project, s: &Schedule, now: Moment, me: Option<&str>, sent: &BTreeMap<String, i64>) -> (Option<Note>, Vec<String>) {
    let mine = |r: &Reminder| match me {
        None => true,
        Some(name) => r.people.iter().any(|id| p.resources.get(id).is_some_and(|res| res.name.trim().eq_ignore_ascii_case(name.trim()))),
    };
    let fresh: Vec<(Reminder, String)> = reminders(p, s, now).into_iter().filter(mine).map(|r| {
        let k = key(p, &r, now.day());
        (r, k)
    }).filter(|(_, k)| !sent.contains_key(k)).collect();
    if fresh.is_empty() {
        return (None, Vec::new());
    }
    let keys = fresh.iter().map(|(_, k)| k.clone()).collect();
    let note = if let [(r, _)] = fresh.as_slice() {
        match r.kind {
            ReminderKind::Late => Note { title: format!("“{}” is late", r.name), body: format!("{} · it was due {}", p.name, when(r.due, now)) },
            ReminderKind::DueSoon => Note { title: format!("“{}” is due {}", r.name, when(r.due, now)), body: p.name.clone() },
        }
    } else {
        let late = fresh.iter().filter(|(r, _)| r.kind == ReminderKind::Late).count();
        let soon = fresh.len() - late;
        let mut parts = Vec::new();
        if late > 0 {
            parts.push(format!("{late} task{} late", if late == 1 { "" } else { "s" }));
        }
        if soon > 0 {
            parts.push(format!("{soon} due soon"));
        }
        let mut names: Vec<String> = fresh.iter().take(3).map(|(r, _)| format!("“{}”", r.name)).collect();
        if fresh.len() > 3 {
            names.push(format!("and {} more", fresh.len() - 3));
        }
        Note { title: format!("{}: {}", p.name, parts.join(", ")), body: names.join(", ") }
    };
    (Some(note), keys)
}

/// Checks every open project once and sends what is new.
fn check(app: &AppHandle) {
    let mut s = saved(app);
    if !s.enabled {
        return;
    }
    let now = Moment::from_datetime(jiff::Zoned::now().datetime());
    let me = crate::team::my_name(app);
    let mut notes = Vec::new();
    {
        let state = app.state::<AppState>();
        let docs = state.docs.lock().expect("lock");
        for d in docs.values() {
            let Ok(schedule) = &d.schedule else { continue };
            let who = if d.team.is_some() { Some(me.as_deref().unwrap_or("")) } else { None };
            let (note, keys) = pick(&d.project, schedule, now, who, &s.sent);
            notes.extend(note);
            for k in keys {
                s.sent.insert(k, now.day());
            }
        }
    }
    if notes.is_empty() {
        return;
    }
    s.sent.retain(|_, day| *day >= now.day() - KEEP_DAYS);
    save(app, &s);
    use tauri_plugin_notification::NotificationExt;
    for n in notes {
        let _ = app.notification().builder().title(n.title).body(n.body).show();
    }
}

/// Starts the background check: shortly after launch, then every 15 minutes.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            check(&app);
            tokio::time::sleep(EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::{plan, Edit};
    use pm_core::{apply, schedule, Units};

    fn at(d: i8, h: i8) -> Moment {
        Moment::from_datetime(jiff::civil::date(2026, 11, d).at(h, 0, 0, 0))
    }

    /// Survey (Mon 2 Nov) → Report (Tue 3 Nov), Survey assigned to Alex.
    fn project() -> Project {
        let mut p = crate::files::empty_project(std::path::Path::new("/Office move.pmproj"));
        p.name = "Office move".into();
        p.start = at(2, 8);
        let u = Units::default();
        let run = |p: &mut Project, e: Edit| {
            let planned = plan(p, None, &u, e).unwrap().unwrap();
            apply(p, &planned.command).unwrap();
            planned.created
        };
        let survey = run(&mut p, Edit::AddTask { after_id: None, name: "Survey".into(), place: Default::default() }).unwrap();
        let report = run(&mut p, Edit::AddTask { after_id: Some(survey.0.to_string()), name: "Report".into(), place: Default::default() }).unwrap();
        run(&mut p, Edit::AddLink { predecessor_id: survey.0.to_string(), successor_id: report.0.to_string() });
        run(&mut p, Edit::SetTaskResources { id: survey.0.to_string(), text: "Alex".into() });
        p
    }

    #[test]
    fn one_reminder_reads_as_a_sentence_and_is_sent_once() {
        let p = project();
        let s = schedule(&p).unwrap();
        let (note, keys) = pick(&p, &s, at(2, 9), None, &BTreeMap::new());
        assert_eq!(note, Some(Note { title: "“Survey” is due today at 17:00".into(), body: "Office move".into() }));
        let sent: BTreeMap<String, i64> = keys.into_iter().map(|k| (k, 0)).collect();
        assert_eq!(pick(&p, &s, at(2, 10), None, &sent).0, None, "not twice");
    }

    #[test]
    fn several_are_grouped_and_late_ones_come_back_each_day() {
        let p = project();
        let s = schedule(&p).unwrap();
        let (note, keys) = pick(&p, &s, at(3, 10), None, &BTreeMap::new());
        assert_eq!(note, Some(Note { title: "Office move: 1 task late, 1 due soon".into(), body: "“Survey”, “Report”".into() }));
        let sent: BTreeMap<String, i64> = keys.into_iter().map(|k| (k, 0)).collect();
        // Wednesday: Survey and Report are both late; Survey's "late" is new for the day.
        let (note, _) = pick(&p, &s, at(4, 10), None, &sent);
        assert_eq!(note.map(|n| n.title), Some("Office move: 2 tasks late".into()));
    }

    #[test]
    fn team_projects_remind_only_about_your_tasks() {
        let p = project();
        let s = schedule(&p).unwrap();
        let (note, _) = pick(&p, &s, at(3, 10), Some("alex"), &BTreeMap::new());
        assert_eq!(note.map(|n| n.title), Some("“Survey” is late".into()));
        assert_eq!(pick(&p, &s, at(3, 10), Some("Sam"), &BTreeMap::new()).0, None);
    }
}
