//! The only place for macOS- or Windows-specific code (spec 4.1).

use tauri::menu::Submenu;
use tauri::{AppHandle, Runtime};

/// The application menu (About, Settings, Services, Hide, Quit). macOS only.
#[cfg(target_os = "macos")]
pub fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Option<Submenu<R>>> {
    use tauri::menu::{AboutMetadata, MenuItemBuilder, SubmenuBuilder};

    let settings = MenuItemBuilder::with_id("app.settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .enabled(false)
        .build(app)?;
    let name = app.package_info().name.clone();
    SubmenuBuilder::new(app, &name)
        .about(Some(AboutMetadata::default()))
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()
        .map(Some)
}

#[cfg(not(target_os = "macos"))]
pub fn app_menu<R: Runtime>(_app: &AppHandle<R>) -> tauri::Result<Option<Submenu<R>>> {
    Ok(None)
}

/// Lets macOS add its own Window list and Help search to these menus.
#[cfg(target_os = "macos")]
pub fn register_standard_menus<R: Runtime>(window: &Submenu<R>, help: &Submenu<R>) -> tauri::Result<()> {
    window.set_as_windows_menu_for_nsapp()?;
    help.set_as_help_menu_for_nsapp()?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn register_standard_menus<R: Runtime>(_window: &Submenu<R>, _help: &Submenu<R>) -> tauri::Result<()> {
    Ok(())
}
