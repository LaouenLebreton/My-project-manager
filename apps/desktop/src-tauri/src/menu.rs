//! Native menu bar. Custom items are disabled until the feature behind them exists,
//! so the menu never offers an action that does nothing.

use std::path::PathBuf;

use tauri::menu::{Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Runtime, Wry};

use crate::platform;

/// Stable ids shared with the web UI.
pub mod id {
    pub const NEW: &str = "file.new";
    pub const OPEN: &str = "file.open";
    pub const OPEN_BACKUP: &str = "file.open_backup";
    /// Open Recent items are this prefix followed by the file path.
    pub const RECENT_PREFIX: &str = "recent:";
    pub const CLEAR_RECENT: &str = "file.recent.clear";
    pub const SAVE_COPY: &str = "file.save_copy";
    pub const IMPORT: &str = "file.import";
    pub const EXPORT: &str = "file.export";
    pub const PRINT: &str = "file.print";
    pub const UNDO: &str = "edit.undo";
    pub const REDO: &str = "edit.redo";
    pub const FIND: &str = "edit.find";
    pub const COMMAND_BAR: &str = "view.command_bar";
    pub const VIEW_TASKS: &str = "view.tasks";
    pub const VIEW_RESOURCES: &str = "view.resources";
    pub const VIEW_WBS: &str = "view.wbs";
    pub const VIEW_WORKLOAD: &str = "view.workload";
    pub const VIEW_CALENDAR: &str = "view.calendar";
    pub const VIEW_BOARD: &str = "view.board";
    pub const VIEW_HISTORY: &str = "view.history";
    pub const VIEW_CHAT: &str = "view.chat";
    pub const HELP: &str = "help.contents";
    pub const TOUR: &str = "help.tour";
    pub const TASK_INFO: &str = "task.info";
    pub const REMINDERS: &str = "window.reminders";
    pub const RECURRING: &str = "task.recurring";
    pub const INDENT: &str = "task.indent";
    pub const OUTDENT: &str = "task.outdent";
    pub const PROJECT_INFO: &str = "project.info";
    pub const WORKING_TIME: &str = "project.working_time";
    pub const LEVEL: &str = "project.level";
    pub const CLEAR_LEVELING: &str = "project.clear_leveling";
    pub const SET_BASELINE: &str = "project.set_baseline";
    pub const CLEAR_BASELINE: &str = "project.clear_baseline";
    pub const UPDATE_PROJECT: &str = "project.update";
    pub const STATISTICS: &str = "project.statistics";
    /// `task.percent_0` … `task.percent_100`: Microsoft Project's 0%…100% Complete buttons.
    pub const PERCENT: [(&str, &str); 5] = [
        ("task.percent_0", "0% Complete"),
        ("task.percent_25", "25% Complete"),
        ("task.percent_50", "50% Complete"),
        ("task.percent_75", "75% Complete"),
        ("task.percent_100", "100% Complete"),
    ];
    pub const TEAM_SIGN_IN: &str = "team.sign_in";
    pub const TEAM_OPEN: &str = "team.open";
    pub const TEAM_SHARE: &str = "team.share";
    pub const TEAM_SIGN_OUT: &str = "team.sign_out";
    pub const TEAM_ADD_ACCOUNT: &str = "team.add_account";
    pub const TEAM_JOIN: &str = "team.join";
    pub const TEAM_INVITE: &str = "team.invite";
    pub const TEAM_MEMBERS: &str = "team.members";
    pub const TEAM_RULES: &str = "team.rules";
    pub const TEAM_MOVE: &str = "team.move";
    pub const PORTFOLIO: &str = "window.portfolio";
    pub const NEW_TAB: &str = "file.new_tab";
    pub const SECOND_USER: &str = "dev.second_user";
    pub const OPEN_5K: &str = "dev.open_5000";
    pub const OPEN_20K: &str = "dev.open_20000";
}

fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
) -> tauri::Result<MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(id, text).enabled(false);
    if let Some(a) = accelerator {
        b = b.accelerator(a);
    }
    b.build(app)
}

/// Menu items whose text or state changes while the app runs.
pub struct MenuHandles {
    undo: MenuItem<Wry>,
    redo: MenuItem<Wry>,
    save_copy: MenuItem<Wry>,
    export: MenuItem<Wry>,
    recent: Submenu<Wry>,
    /// Task and Project menu items, which need an open project.
    document_items: Vec<MenuItem<Wry>>,
}

impl MenuHandles {
    /// "Undo Change Duration", or a disabled "Undo" when there is nothing to undo.
    pub fn update_undo(&self, undo: Option<String>, redo: Option<String>) {
        for (item, verb, label) in [(&self.undo, "Undo", undo), (&self.redo, "Redo", redo)] {
            let _ = item.set_text(label.as_deref().map_or_else(|| verb.to_owned(), |l| format!("{verb} {l}")));
            let _ = item.set_enabled(label.is_some());
        }
    }

    /// Items that need an open project.
    pub fn set_document_open(&self, open: bool) {
        let _ = self.save_copy.set_enabled(open);
        let _ = self.export.set_enabled(open);
        for item in &self.document_items {
            let _ = item.set_enabled(open);
        }
    }

    /// Rebuilds File > Open Recent: file names, then Clear Menu.
    pub fn set_recent(&self, app: &AppHandle, paths: &[PathBuf]) {
        if let Ok(items) = self.recent.items() {
            for item in items {
                let _ = self.recent.remove(&item);
            }
        }
        for path in paths {
            let label = crate::files::display_name(path);
            let id = format!("{}{}", id::RECENT_PREFIX, path.display());
            if let Ok(item) = MenuItemBuilder::with_id(id, label).build(app) {
                let _ = self.recent.append(&item);
            }
        }
        if !paths.is_empty()
            && let Ok(sep) = PredefinedMenuItem::separator(app)
        {
            let _ = self.recent.append(&sep);
        }
        if let Ok(clear) = MenuItemBuilder::with_id(id::CLEAR_RECENT, "Clear Menu").enabled(!paths.is_empty()).build(app) {
            let _ = self.recent.append(&clear);
        }
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<(Menu<Wry>, MenuHandles)> {
    let recent = SubmenuBuilder::new(app, "Open Recent")
        .item(&item(app, id::CLEAR_RECENT, "Clear Menu", None)?)
        .build()?;

    let enabled = |id: &str, text: &str, accelerator: Option<&str>| -> tauri::Result<MenuItem<Wry>> {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accelerator {
            b = b.accelerator(a);
        }
        b.build(app)
    };
    let save_copy = item(app, id::SAVE_COPY, "Save a Copy…", Some("CmdOrCtrl+Shift+S"))?;
    let export = item(app, id::EXPORT, "Export as Microsoft Project XML…", Some("CmdOrCtrl+Shift+E"))?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&enabled(id::NEW, "New Project…", Some("CmdOrCtrl+N"))?)
        .item(&enabled(id::NEW_TAB, "New Tab", Some("CmdOrCtrl+T"))?)
        .item(&enabled(id::OPEN, "Open…", Some("CmdOrCtrl+O"))?)
        .item(&recent)
        .separator()
        .close_window()
        .item(&save_copy)
        .item(&enabled(id::OPEN_BACKUP, "Open Backup…", None)?)
        .separator()
        .item(&enabled(id::IMPORT, "Import Microsoft Project File…", Some("CmdOrCtrl+Shift+I"))?)
        .item(&export)
        .separator()
        .item(&item(app, id::PRINT, "Print…", Some("CmdOrCtrl+P"))?)
        .build()?;

    // Undo and redo go through the project's command log, not the web view's
    // text undo, so they are custom items. Clipboard items stay predefined so
    // they work in text fields.
    let undo = item(app, id::UNDO, "Undo", Some("CmdOrCtrl+Z"))?;
    let redo = item(app, id::REDO, "Redo", Some("CmdOrCtrl+Shift+Z"))?;
    let find = item(app, id::FIND, "Find…", Some("CmdOrCtrl+F"))?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&undo)
        .item(&redo)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(&find)
        .build()?;

    let view_tasks = item(app, id::VIEW_TASKS, "Tasks", Some("CmdOrCtrl+1"))?;
    let view_resources = item(app, id::VIEW_RESOURCES, "Resource Sheet", Some("CmdOrCtrl+2"))?;
    let view_wbs = item(app, id::VIEW_WBS, "Work Breakdown Structure", Some("CmdOrCtrl+3"))?;
    let view_workload = item(app, id::VIEW_WORKLOAD, "Workload", Some("CmdOrCtrl+4"))?;
    let view_calendar = item(app, id::VIEW_CALENDAR, "Calendar", Some("CmdOrCtrl+5"))?;
    let view_board = item(app, id::VIEW_BOARD, "Board", Some("CmdOrCtrl+6"))?;
    let view_history = item(app, id::VIEW_HISTORY, "History", Some("CmdOrCtrl+7"))?;
    let view_chat = item(app, id::VIEW_CHAT, "Chat", Some("CmdOrCtrl+8"))?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&view_tasks)
        .item(&view_resources)
        .item(&view_wbs)
        .item(&view_workload)
        .item(&view_calendar)
        .item(&view_board)
        .item(&view_history)
        .item(&view_chat)
        .separator()
        .item(&enabled(id::COMMAND_BAR, "Show Command Bar", Some("CmdOrCtrl+K"))?)
        .separator()
        .fullscreen()
        .build()?;

    // Task and Project menus. Indent and Outdent show Tab and ⇧Tab without owning those
    // keys (the grid handles them), so they have no accelerator here.
    let task_info = item(app, id::TASK_INFO, "Task Info…", Some("CmdOrCtrl+I"))?;
    let recurring = item(app, id::RECURRING, "Recurring Task…", None)?;
    let indent = item(app, id::INDENT, "Indent", None)?;
    let outdent = item(app, id::OUTDENT, "Outdent", None)?;
    let percent = id::PERCENT.iter().map(|(i, label)| item(app, i, label, None)).collect::<Result<Vec<_>, _>>()?;
    let mut task = SubmenuBuilder::new(app, "Task").item(&task_info).item(&recurring).separator().item(&indent).item(&outdent).separator();
    for p in &percent {
        task = task.item(p);
    }
    let task = task.build()?;
    let project_info = item(app, id::PROJECT_INFO, "Project Info…", None)?;
    let working_time = item(app, id::WORKING_TIME, "Working Time…", None)?;
    let level = item(app, id::LEVEL, "Level Resources", None)?;
    let clear_leveling = item(app, id::CLEAR_LEVELING, "Clear Leveling", None)?;
    let set_baseline = item(app, id::SET_BASELINE, "Set Baseline", None)?;
    let clear_baseline = item(app, id::CLEAR_BASELINE, "Clear Baseline", None)?;
    let update_project = item(app, id::UPDATE_PROJECT, "Update Project…", None)?;
    let statistics = item(app, id::STATISTICS, "Project Statistics…", None)?;
    let project = SubmenuBuilder::new(app, "Project")
        .item(&project_info)
        .item(&statistics)
        .item(&working_time)
        .separator()
        .item(&level)
        .item(&clear_leveling)
        .separator()
        .item(&set_baseline)
        .item(&clear_baseline)
        .item(&update_project)
        .build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .item(&enabled(id::PORTFOLIO, "Portfolio", Some("CmdOrCtrl+0"))?)
        .item(&tauri::menu::CheckMenuItemBuilder::with_id(id::REMINDERS, "Task Reminders").checked(crate::reminders::enabled(app)).build(app)?)
        .separator()
        .close_window()
        .build()?;

    // Team (iteration 3). Creating a team needs no server: the app hosts it. Joining and
    // opening work without a project; a separate server is the advanced route.
    let team_share = item(app, id::TEAM_SHARE, "Create Team Project…", None)?;
    let team_members = item(app, id::TEAM_MEMBERS, "Members & Permissions…", None)?;
    let team_rules = item(app, id::TEAM_RULES, "Automation Rules…", None)?;
    let server = SubmenuBuilder::new(app, "Team Server")
        .item(&enabled(id::TEAM_MOVE, "Connect Teams to a Server…", None)?)
        .separator()
        .item(&enabled(id::TEAM_SIGN_IN, "Sign In to a Team Server…", None)?)
        .item(&enabled(id::TEAM_ADD_ACCOUNT, "Add Account to Server…", None)?)
        .build()?;
    let team = SubmenuBuilder::new(app, "Team")
        .item(&team_share)
        .item(&enabled(id::TEAM_INVITE, "Invite People…", None)?)
        .item(&team_members)
        .item(&team_rules)
        .separator()
        .item(&enabled(id::TEAM_JOIN, "Join a Team…", None)?)
        .item(&enabled(id::TEAM_OPEN, "Open Team Project…", None)?)
        .separator()
        .item(&server)
        .item(&enabled(id::TEAM_SIGN_OUT, "Sign Out", None)?)
        .build()?;

    let mut help = SubmenuBuilder::new(app, "Help")
        .item(&enabled(id::HELP, "Super PM Help", Some("CmdOrCtrl+Shift+Slash"))?)
        .item(&enabled(id::TOUR, "Show Tour", None)?);
    if cfg!(debug_assertions) {
        // Performance checks (spec section 3); not in release builds.
        let dev = |id, text| MenuItemBuilder::with_id(id, text).build(app);
        help = help
            .separator()
            .item(&dev(id::SECOND_USER, "Open Second Test User Window")?)
            .item(&dev(id::OPEN_5K, "Open Test Project (5,000 Tasks)")?)
            .item(&dev(id::OPEN_20K, "Open Test Project (20,000 Tasks)")?);
    }
    let help = help.build()?;

    let menu = Menu::new(app)?;
    if let Some(app_menu) = platform::app_menu(app)? {
        menu.append(&app_menu)?;
    }
    for sub in [&file, &edit, &view, &task, &project, &team, &window, &help] {
        menu.append(sub)?;
    }
    platform::register_standard_menus(&window, &help)?;
    let document_items = vec![find, task_info, recurring, indent, outdent, project_info, working_time, view_tasks, view_resources, view_wbs, view_workload, view_calendar, view_board, view_history, view_chat, level, clear_leveling, set_baseline, clear_baseline, update_project, statistics, team_share, team_members, team_rules];
    let document_items = document_items.into_iter().chain(percent).collect::<Vec<_>>();
    Ok((menu, MenuHandles { undo, redo, save_copy, export, recent, document_items }))
}
