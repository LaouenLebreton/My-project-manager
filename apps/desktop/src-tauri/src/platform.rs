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
#[cfg(target_os = "macos")]
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

/// Answers macOS's "new tab" request (`newWindowForTab:`), which makes it show the "+" at the
/// end of the tab bar; clicking it opens the start page in a new tab (File > New Tab). macOS
/// decides on the "+" when a window and its tab bar are made, so this runs at launch, before
/// any window exists: the method goes on NSWindow (every window inherits it) and on
/// NSApplication (the end of the responder chain macOS searches).
#[cfg(target_os = "macos")]
pub fn install_new_tab_action(app: &AppHandle<tauri::Wry>) {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use std::sync::OnceLock;

    static APP: OnceLock<AppHandle<tauri::Wry>> = OnceLock::new();
    if APP.set(app.clone()).is_err() {
        return;
    }

    unsafe extern "C-unwind" fn new_window_for_tab(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
        if let Some(app) = APP.get() {
            let app = app.clone();
            // Off the AppKit call: opening a window from inside it is not safe everywhere.
            tauri::async_runtime::spawn(async move { crate::actions::show_home(&app, true) });
        }
    }

    for name in [c"NSWindow", c"NSApplication"] {
        let Some(class) = AnyClass::get(name) else { continue };
        // SAFETY: adding a method to a class is what the Objective-C runtime is for; the
        // function's signature matches the types `v@:@` (void; self, _cmd, sender).
        unsafe {
            let imp: Imp = std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject), Imp>(new_window_for_tab);
            objc2::ffi::class_addMethod(class as *const AnyClass as *mut AnyClass, objc2::sel!(newWindowForTab:), imp, c"v@:@".as_ptr());
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install_new_tab_action(_app: &AppHandle<tauri::Wry>) {}

/// Keeps the tab bar showing even with one tab, as Safari can. AppKit must be called on the
/// main thread; this hands the work to it.
#[cfg(target_os = "macos")]
pub fn show_tab_bar(window: &tauri::WebviewWindow<tauri::Wry>) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(ns) = w.ns_window() else { return };
        // SAFETY: the window's live NSWindow, on the main thread.
        let ns = unsafe { &*(ns as *const objc2_app_kit::NSWindow) };
        if ns.tabGroup().is_some_and(|g| !g.isTabBarVisible()) {
            ns.toggleTabBar(None);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn show_tab_bar(_window: &tauri::WebviewWindow<tauri::Wry>) {}
