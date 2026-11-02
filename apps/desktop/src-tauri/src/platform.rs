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

/// Windows with this identifier group as tabs of one window (macOS).
pub const TAB_GROUP: &str = "super-pm-projects";

/// Shows `window` as a tab of the window in front, as Safari and Finder do with new tabs.
/// AppKit must be called on the main thread, so this hands the work to it.
#[cfg(target_os = "macos")]
pub fn tab_into<R: Runtime>(app: &AppHandle<R>, front: tauri::WebviewWindow<R>, window: tauri::WebviewWindow<R>) {
    let _ = app.run_on_main_thread(move || {
        use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
        let (Ok(a), Ok(b)) = (front.ns_window(), window.ns_window()) else { return };
        if a == b {
            return;
        }
        // SAFETY: Tauri hands out the window's live NSWindow, and we are on the main thread.
        unsafe {
            let (a, b) = (&*(a as *const NSWindow), &*(b as *const NSWindow));
            a.addTabbedWindow_ordered(b, NSWindowOrderingMode::Above);
            b.makeKeyAndOrderFront(None);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn tab_into<R: Runtime>(_app: &AppHandle<R>, _front: tauri::WebviewWindow<R>, _window: tauri::WebviewWindow<R>) {}
