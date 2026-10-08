//! Native menu bar. Custom items are disabled until the feature behind them exists,
//! so the menu never offers an action that does nothing.

use tauri::menu::{Menu, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Runtime, Wry};

use crate::platform;

/// Stable ids shared with the web UI.
pub mod id {
    pub const NEW: &str = "file.new";
    pub const OPEN: &str = "file.open";
    pub const CLEAR_RECENT: &str = "file.recent.clear";
    pub const SAVE_COPY: &str = "file.save_copy";
    pub const IMPORT: &str = "file.import";
    pub const EXPORT: &str = "file.export";
    pub const PRINT: &str = "file.print";
    pub const UNDO: &str = "edit.undo";
    pub const REDO: &str = "edit.redo";
    pub const FIND: &str = "edit.find";
    pub const COMMAND_BAR: &str = "view.command_bar";
    pub const HELP: &str = "help.contents";
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
}

impl MenuHandles {
    /// "Undo Change Duration", or a disabled "Undo" when there is nothing to undo.
    pub fn update_undo(&self, undo: Option<String>, redo: Option<String>) {
        for (item, verb, label) in [(&self.undo, "Undo", undo), (&self.redo, "Redo", redo)] {
            let _ = item.set_text(label.as_deref().map_or_else(|| verb.to_owned(), |l| format!("{verb} {l}")));
            let _ = item.set_enabled(label.is_some());
        }
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<(Menu<Wry>, MenuHandles)> {
    let recent = SubmenuBuilder::new(app, "Open Recent")
        .item(&item(app, id::CLEAR_RECENT, "Clear Menu", None)?)
        .build()?;

    let file = SubmenuBuilder::new(app, "File")
        .item(&item(app, id::NEW, "New", Some("CmdOrCtrl+N"))?)
        .item(&item(app, id::OPEN, "Open…", Some("CmdOrCtrl+O"))?)
        .item(&recent)
        .separator()
        .close_window()
        .item(&item(app, id::SAVE_COPY, "Save a Copy…", Some("CmdOrCtrl+Shift+S"))?)
        .separator()
        .item(&item(app, id::IMPORT, "Import Microsoft Project XML…", None)?)
        .item(&item(app, id::EXPORT, "Export as Microsoft Project XML…", None)?)
        .separator()
        .item(&item(app, id::PRINT, "Print…", Some("CmdOrCtrl+P"))?)
        .build()?;

    // Undo and redo go through the project's command log, not the web view's
    // text undo, so they are custom items. Clipboard items stay predefined so
    // they work in text fields.
    let undo = item(app, id::UNDO, "Undo", Some("CmdOrCtrl+Z"))?;
    let redo = item(app, id::REDO, "Redo", Some("CmdOrCtrl+Shift+Z"))?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&undo)
        .item(&redo)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(&item(app, id::FIND, "Find…", Some("CmdOrCtrl+F"))?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item(app, id::COMMAND_BAR, "Show Command Bar", Some("CmdOrCtrl+K"))?)
        .separator()
        .fullscreen()
        .build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let mut help = SubmenuBuilder::new(app, "Help").item(&item(app, id::HELP, "Super PM Help", None)?);
    if cfg!(debug_assertions) {
        // Performance checks (spec section 3); not in release builds.
        let dev = |id, text| MenuItemBuilder::with_id(id, text).build(app);
        help = help
            .separator()
            .item(&dev(id::OPEN_5K, "Open Test Project (5,000 Tasks)")?)
            .item(&dev(id::OPEN_20K, "Open Test Project (20,000 Tasks)")?);
    }
    let help = help.build()?;

    let menu = Menu::new(app)?;
    if let Some(app_menu) = platform::app_menu(app)? {
        menu.append(&app_menu)?;
    }
    for sub in [&file, &edit, &view, &window, &help] {
        menu.append(sub)?;
    }
    platform::register_standard_menus(&window, &help)?;
    Ok((menu, MenuHandles { undo, redo }))
}
