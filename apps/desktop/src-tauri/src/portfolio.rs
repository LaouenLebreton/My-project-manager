//! The Portfolio window (iteration 4): every project at a glance, a combined timeline, and
//! each person's work across all projects.
//!
//! Projects shown: recent projects, team projects on this Mac, and any added by hand, minus
//! any taken out. Open projects are read from their window (so unsaved-looking edits show);
//! the others from their files, read-only.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use pm_core::portfolio::{summarize, Status};
use pm_core::{loads, schedule, Moment, Project, ResourceKind};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};
use ts_rs::TS;

use crate::actions::AppState;
use crate::ipc::IpcError;

pub const WINDOW: &str = "portfolio";

fn err(message: impl Into<String>) -> IpcError {
    IpcError { message: message.into() }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    pub name: String,
    pub date: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRow {
    pub path: String,
    pub name: String,
    /// A team project (shared through a team).
    pub shared: bool,
    /// The team's name, when the team server is reachable.
    pub team: Option<String>,
    /// False for a team project that has no copy on this Mac yet (opening it downloads it).
    pub on_this_mac: bool,
    /// The group the owner put it in ("Clients", "Internal"), if any.
    pub group: Option<String>,
    /// "onTrack", "atRisk", "late", "done", or "problem" (could not be read or scheduled).
    pub status: String,
    pub reasons: Vec<String>,
    pub start: Option<String>,
    pub finish: Option<String>,
    pub baseline_finish: Option<String>,
    #[ts(type = "number | null")]
    pub finish_variance_minutes: Option<i64>,
    pub percent_complete: u8,
    #[ts(type = "number")]
    pub cost_cents: i64,
    #[ts(type = "number | null")]
    pub baseline_cost_cents: Option<i64>,
    pub currency: String,
    pub overallocated: u32,
    pub overdue: u32,
    pub next_milestone: Option<Milestone>,
    pub milestones: Vec<Milestone>,
}

/// One person's work in one week, across projects.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PersonWeek {
    #[ts(type = "number")]
    pub minutes: i64,
    /// What they have that week (working time × max units, from their first project).
    #[ts(type = "number")]
    pub capacity: i64,
    /// Project names with minutes, largest first.
    #[ts(type = "Array<[string, number]>")]
    pub projects: Vec<(String, i64)>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioPerson {
    pub name: String,
    /// Same length as `PortfolioView::weeks`.
    pub weeks: Vec<PersonWeek>,
    /// In how many projects they have work.
    pub project_count: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioView {
    pub projects: Vec<PortfolioRow>,
    /// Mondays (`YYYY-MM-DD`) of the weeks shown for people.
    pub weeks: Vec<String>,
    pub people: Vec<PortfolioPerson>,
    pub today: String,
    /// Every group name in use, sorted.
    pub groups: Vec<String>,
    /// Why the team's other projects could not be listed (offline), if so.
    pub team_note: Option<String>,
}

/// Paths added by hand and taken out, and each project's group, kept between launches.
#[derive(Default, Serialize, Deserialize)]
struct Choices {
    added: Vec<PathBuf>,
    removed: Vec<PathBuf>,
    #[serde(default)]
    groups: BTreeMap<PathBuf, String>,
}

impl Choices {
    /// Puts a project in a group; a blank name takes it out of its group.
    fn set_group(&mut self, path: PathBuf, group: &str) {
        let group = group.trim();
        if group.is_empty() {
            self.groups.remove(&path);
        } else {
            self.groups.insert(path, group.to_owned());
        }
    }

    /// The group names in use, sorted, without duplicates.
    fn group_names(&self) -> Vec<String> {
        let names: std::collections::BTreeSet<&String> = self.groups.values().collect();
        names.into_iter().cloned().collect()
    }
}

fn choices_file(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::hosting::profiled(app.path().app_config_dir().ok()?).join("portfolio.json"))
}

fn choices(app: &AppHandle) -> Choices {
    choices_file(app).and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_choices(app: &AppHandle, c: &Choices) {
    if let Some(f) = choices_file(app) {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, serde_json::to_vec_pretty(c).expect("choices serialize"));
    }
}

/// The projects in the portfolio, without duplicates.
fn paths(app: &AppHandle) -> Vec<PathBuf> {
    let c = choices(app);
    let shared = app
        .path()
        .app_data_dir()
        .ok()
        .map(|d| crate::hosting::profiled(d).join("Shared"))
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "pmproj")).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut out: Vec<PathBuf> = Vec::new();
    for p in crate::files::recent(app).into_iter().chain(c.added).chain(shared) {
        if p.is_file() && !c.removed.contains(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn iso(m: Moment) -> String {
    m.to_datetime().to_string()
}

fn date(day: i64) -> String {
    pm_core::time::date_from_day_number(day).to_string()
}

/// A project as it is now: from its window if open, otherwise from its file.
fn load(app: &AppHandle, path: &Path) -> Result<(Project, bool), String> {
    let state = app.state::<AppState>();
    if let Some(d) = state.docs.lock().expect("lock").values().find(|d| d.path == path) {
        return Ok((d.project.clone(), d.team.is_some()));
    }
    let (store, project) = pm_store::Store::open(path).map_err(|e| e.to_string())?;
    let shared = store.meta(pm_store::TEAM_SERVER).ok().flatten().is_some();
    Ok((project, shared))
}

fn status_code(s: Status) -> &'static str {
    match s {
        Status::OnTrack => "onTrack",
        Status::AtRisk => "atRisk",
        Status::Late => "late",
        Status::Done => "done",
    }
}

/// A project the portfolio shows: its key (a file path, or `team:<id>` for a team project not
/// on this Mac), name if it cannot be read, and the project.
struct Source {
    key: PathBuf,
    fallback_name: String,
    loaded: Result<(Project, bool), String>,
    team: Option<String>,
    on_this_mac: bool,
}

const TEAM_KEY: &str = "team:";

/// Adds the team's projects: one already listed (its copy on this Mac) only gets its team's
/// name; the others come in from the server unless they were taken out.
fn add_team(sources: &mut Vec<Source>, entries: Vec<crate::team::TeamEntry>, removed: &[PathBuf]) {
    for e in entries {
        if let Some(s) = sources.iter_mut().find(|s| s.key == e.local) {
            s.team = Some(e.team);
            continue;
        }
        let key = PathBuf::from(format!("{TEAM_KEY}{}", e.id));
        if removed.contains(&key) || removed.contains(&e.local) {
            continue;
        }
        let Some(project) = e.project else { continue };
        sources.push(Source { key, fallback_name: project.name.clone(), loaded: Ok((project, true)), team: Some(e.team), on_this_mac: false });
    }
}

/// Window > Portfolio: everything it shows. Projects on this Mac, plus (when signed in to a
/// team) the team's projects not opened here yet.
#[tauri::command]
pub async fn portfolio_data(app: AppHandle) -> PortfolioView {
    let remote = crate::team::portfolio_entries(&app).await;
    let chosen = choices(&app);
    let mut sources: Vec<Source> = paths(&app)
        .into_iter()
        .map(|path| Source {
            fallback_name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            loaded: load(&app, &path),
            key: path,
            team: None,
            on_this_mac: true,
        })
        .collect();
    let team_note = match remote {
        Ok(entries) => {
            add_team(&mut sources, entries, &chosen.removed);
            None
        }
        Err(note) => Some(note),
    };
    let today = Moment::from_datetime(jiff::Zoned::now().datetime());
    let mut projects = Vec::new();
    // Per person (lower-case name): display name, capacity source, day → (project → minutes).
    struct Person {
        name: String,
        capacity: Box<dyn Fn(i64) -> i64>,
        days: BTreeMap<i64, HashMap<String, i64>>,
    }
    let mut people: BTreeMap<String, Person> = BTreeMap::new();
    let (mut first_day, mut last_day) = (i64::MAX, i64::MIN);

    for Source { key: path, fallback_name: file_name, loaded, team, on_this_mac } in sources {
        let blank = |name: String, status: &str, reason: String| PortfolioRow {
            path: path.display().to_string(),
            name,
            shared: false,
            team: team.clone(),
            on_this_mac,
            group: chosen.groups.get(&path).cloned(),
            status: status.into(),
            reasons: vec![reason],
            start: None,
            finish: None,
            baseline_finish: None,
            finish_variance_minutes: None,
            percent_complete: 0,
            cost_cents: 0,
            baseline_cost_cents: None,
            currency: String::new(),
            overallocated: 0,
            overdue: 0,
            next_milestone: None,
            milestones: Vec::new(),
        };
        let (p, shared) = match loaded {
            Ok(x) => x,
            Err(e) => {
                projects.push(blank(file_name, "problem", format!("could not be read: {e}")));
                continue;
            }
        };
        let s = match schedule(&p) {
            Ok(s) => s,
            Err(e) => {
                let mut row = blank(p.name.clone(), "problem", format!("dates cannot be calculated: {e}"));
                row.shared = shared;
                projects.push(row);
                continue;
            }
        };
        let l = loads(&p, &s);
        let sum = summarize(&p, &s, &l, today);
        first_day = first_day.min(sum.start.day());
        last_day = last_day.max(sum.finish.day());
        let parents: std::collections::BTreeSet<_> = p.tasks.values().filter_map(|t| t.parent).collect();
        let mut milestones: Vec<(Moment, String)> =
            p.tasks.values().filter(|t| t.is_milestone && !parents.contains(&t.id)).filter_map(|t| s.tasks.get(&t.id).map(|ts| (ts.early_start, t.name.clone()))).collect();
        milestones.sort();
        projects.push(PortfolioRow {
            path: path.display().to_string(),
            name: p.name.clone(),
            shared,
            team: team.clone(),
            on_this_mac,
            group: chosen.groups.get(&path).cloned(),
            status: status_code(sum.status).into(),
            reasons: sum.reasons,
            start: Some(iso(sum.start)),
            finish: Some(iso(sum.finish)),
            baseline_finish: sum.baseline_finish.map(iso),
            finish_variance_minutes: sum.finish_variance,
            percent_complete: sum.percent_complete,
            cost_cents: sum.cost,
            baseline_cost_cents: sum.baseline_cost,
            currency: p.currency.clone(),
            overallocated: sum.overallocated,
            overdue: sum.overdue,
            next_milestone: sum.next_milestone.map(|(_, name, at)| Milestone { name, date: iso(at) }),
            milestones: milestones.into_iter().map(|(at, name)| Milestone { name, date: iso(at) }).collect(),
        });
        // People: work per day in this project.
        for a in p.assignments.values() {
            let Some(r) = p.resources.get(&a.resource).filter(|r| r.kind == ResourceKind::Work) else { continue };
            let key = r.name.trim().to_lowercase();
            let person = people.entry(key).or_insert_with(|| {
                let (cal, units) = (p.calendar.clone(), i64::from(r.max_units.0));
                Person { name: r.name.trim().to_owned(), capacity: Box::new(move |day| cal.pattern(day).working_minutes() * units / 100), days: BTreeMap::new() }
            });
            for (day, minutes) in pm_core::assignment_days(&p, &s, a) {
                *person.days.entry(day).or_default().entry(p.name.clone()).or_default() += minutes;
            }
        }
    }

    // Weeks from the earliest start (or this week) for at most a year.
    let monday = |d: i64| d - pm_core::time::weekday_index(d) as i64;
    let mut weeks = Vec::new();
    if first_day <= last_day {
        let from = monday(first_day.max(today.day() - 28));
        let to = last_day.min(from + 52 * 7);
        let mut w = from;
        while w <= to {
            weeks.push(w);
            w += 7;
        }
    }
    let people = people
        .into_values()
        .filter(|p| !p.days.is_empty())
        .map(|p| {
            let mut projects_seen = std::collections::BTreeSet::new();
            let weeks_out = weeks
                .iter()
                .map(|&w| {
                    let mut by: HashMap<String, i64> = HashMap::new();
                    for (_, per) in p.days.range(w..w + 7) {
                        for (name, m) in per {
                            *by.entry(name.clone()).or_default() += m;
                            projects_seen.insert(name.clone());
                        }
                    }
                    let mut list: Vec<(String, i64)> = by.into_iter().collect();
                    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    PersonWeek { minutes: list.iter().map(|x| x.1).sum(), capacity: (w..w + 7).map(|d| (p.capacity)(d)).sum(), projects: list }
                })
                .collect();
            PortfolioPerson { name: p.name, weeks: weeks_out, project_count: projects_seen.len() as u32 }
        })
        .collect();
    PortfolioView { projects, weeks: weeks.into_iter().map(date).collect(), people, today: iso(today), groups: chosen.group_names(), team_note }
}

/// Adds a project file chosen in a dialog.
#[tauri::command]
pub async fn portfolio_add(app: AppHandle, window: WebviewWindow) -> Result<bool, IpcError> {
    let d = crate::actions::dialog(&app, Some(window.label())).set_title("Add to Portfolio").add_filter("Super PM Project", &[crate::files::EXTENSION]);
    let Some(choice) = d.blocking_pick_file() else { return Ok(false) };
    let path = choice.into_path().map_err(|e| err(e.to_string()))?;
    let mut c = choices(&app);
    c.removed.retain(|p| p != &path);
    if !c.added.contains(&path) {
        c.added.push(path);
    }
    save_choices(&app, &c);
    Ok(true)
}

/// Takes a project out of the portfolio (the file is not touched).
#[tauri::command]
pub fn portfolio_remove(app: AppHandle, path: String) {
    let path = PathBuf::from(path);
    let mut c = choices(&app);
    c.added.retain(|p| p != &path);
    if !c.removed.contains(&path) {
        c.removed.push(path);
    }
    save_choices(&app, &c);
}

/// Puts a project in a group ("Clients"); a blank name takes it out.
#[tauri::command]
pub fn portfolio_set_group(app: AppHandle, path: String, group: String) {
    let mut c = choices(&app);
    c.set_group(PathBuf::from(path), &group);
    save_choices(&app, &c);
}

/// Opens a project from the portfolio in its own tab; a team project not on this Mac is
/// downloaded first.
#[tauri::command]
pub async fn portfolio_open(app: AppHandle, path: String) -> Result<(), IpcError> {
    let file = match path.strip_prefix(TEAM_KEY) {
        Some(id) => crate::team::download(&app, id).await?,
        None => PathBuf::from(path),
    };
    crate::actions::open_path(&app, None, &file).map(|_| ())
}

/// Window > Portfolio: shows the portfolio as a tab beside the projects, creating it the first
/// time. (Projects never load into it: `window_for_new_project` skips it.)
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        crate::actions::show(&w);
        return;
    }
    let builder = tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::App("index.html".into()));
    #[cfg(target_os = "macos")]
    let builder = builder.tabbing_identifier(crate::platform::TAB_GROUP);
    let built = builder
        .title("Portfolio")
        .inner_size(1200.0, 800.0)
        .min_inner_size(720.0, 480.0)
        .visible(false)
        .build();
    match built {
        Ok(w) => {
            crate::actions::tab_into_front(app, &w);
            crate::actions::show(&w);
        }
        Err(e) => eprintln!("could not open the portfolio: {e}"),
    }
}

/// The Home button (`newTab` false: go to the start page tab) and the + (`newTab` true: a new
/// one).
#[tauri::command]
pub fn show_home(app: AppHandle, new_tab: bool) {
    crate::actions::show_home(&app, new_tab);
}

/// The start screen's and command bar's way to Window > Portfolio.
#[tauri::command]
pub fn open_portfolio(app: AppHandle) {
    show(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, local: &str, project: Option<Project>) -> crate::team::TeamEntry {
        crate::team::TeamEntry { id: id.into(), team: "Office".into(), local: local.into(), project }
    }

    #[test]
    fn team_projects_join_without_duplicates() {
        let mut sources = vec![Source { key: "/Shared/a.pmproj".into(), fallback_name: "a".into(), loaded: Err("x".into()), team: None, on_this_mac: true }];
        let named = |n: &str| {
            let mut p = crate::files::empty_project(Path::new("/x.pmproj"));
            p.name = n.into();
            p
        };
        let entries = vec![
            entry("a", "/Shared/a.pmproj", None),
            entry("b", "/Shared/b.pmproj", Some(named("Bravo"))),
            entry("c", "/Shared/c.pmproj", Some(named("Charlie"))),
        ];
        add_team(&mut sources, entries, &[PathBuf::from("team:c")]);
        assert_eq!(sources.len(), 2, "a is not added twice; c was taken out");
        assert_eq!(sources[0].team.as_deref(), Some("Office"));
        assert!(sources[0].on_this_mac);
        assert_eq!(sources[1].key, PathBuf::from("team:b"));
        assert_eq!(sources[1].fallback_name, "Bravo");
        assert!(!sources[1].on_this_mac);
    }

    #[test]
    fn choices_saved_before_groups_still_read() {
        let c: Choices = serde_json::from_str(r#"{"added":["/a.pmproj"],"removed":[]}"#).unwrap();
        assert_eq!(c.added, vec![PathBuf::from("/a.pmproj")]);
        assert!(c.groups.is_empty());
    }

    #[test]
    fn groups_are_set_trimmed_and_cleared() {
        let mut c = Choices::default();
        c.set_group("/a.pmproj".into(), "  Clients ");
        c.set_group("/b.pmproj".into(), "Internal");
        c.set_group("/c.pmproj".into(), "Clients");
        assert_eq!(c.groups.get(Path::new("/a.pmproj")).map(String::as_str), Some("Clients"));
        assert_eq!(c.group_names(), vec!["Clients".to_owned(), "Internal".to_owned()]);
        c.set_group("/b.pmproj".into(), "   ");
        assert_eq!(c.group_names(), vec!["Clients".to_owned()]);
        let back: Choices = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(back.groups, c.groups);
    }
}
