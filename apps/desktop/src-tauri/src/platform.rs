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

/// Shows the "+" button at the right of the tab bar: macOS shows it for windows that answer
/// `newWindowForTab:`, so that answer is added to the windows' class once. Clicking it opens
/// the start page in a new tab (File > New Tab).
#[cfg(target_os = "macos")]
pub fn enable_new_tab_button(app: &AppHandle<tauri::Wry>, window: &tauri::WebviewWindow<tauri::Wry>) {
    use objc2::runtime::{AnyObject, Imp, Sel};
    use std::sync::OnceLock;

    static APP: OnceLock<AppHandle<tauri::Wry>> = OnceLock::new();
    static ADDED: std::sync::Once = std::sync::Once::new();
    let _ = APP.set(app.clone());

    unsafe extern "C-unwind" fn new_window_for_tab(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
        if let Some(app) = APP.get() {
            let app = app.clone();
            // Off the AppKit call: opening a window from inside it is not safe everywhere.
            tauri::async_runtime::spawn(async move { crate::actions::show_home(&app) });
        }
    }

    let Ok(ns) = window.ns_window() else { return };
    ADDED.call_once(|| {
        // SAFETY: `ns` is the window's live NSWindow; adding a method to its class is what the
        // Objective-C runtime is for, and the method's types match `v@:@` (void, self, _cmd, id).
        unsafe {
            // macOS may have swapped in a stand-in class for observing the window
            // ("NSKVONotifying_…"); the method belongs on the real class, shared by every window.
            let mut class: &objc2::runtime::AnyClass = (*(ns as *const AnyObject)).class();
            while class.name().to_string_lossy().starts_with("NSKVONotifying_") {
                match class.superclass() {
                    Some(sup) => class = sup,
                    None => break,
                }
            }
            let class = class as *const objc2::runtime::AnyClass as *mut objc2::runtime::AnyClass;
            let imp: Imp = std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject), Imp>(new_window_for_tab);
            objc2::ffi::class_addMethod(class, objc2::sel!(newWindowForTab:), imp, c"v@:@".as_ptr());
            // macOS looks for the action along the responder chain up to the application, so
            // the application answers too.
            if let Some(app_class) = objc2::runtime::AnyClass::get(c"NSApplication") {
                let app_class = app_class as *const objc2::runtime::AnyClass as *mut objc2::runtime::AnyClass;
                objc2::ffi::class_addMethod(app_class, objc2::sel!(newWindowForTab:), imp, c"v@:@".as_ptr());
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn enable_new_tab_button(_app: &AppHandle<tauri::Wry>, _window: &tauri::WebviewWindow<tauri::Wry>) {}

/// Keeps the tab bar showing even with one tab, so its "+" button is always there (as Safari
/// can). `rebuild`: hide and show it once, for a window whose bar was built before the "+"
/// existed (the start window). Must run on the main thread; this hands the work to it.
#[cfg(target_os = "macos")]
pub fn show_tab_bar(window: &tauri::WebviewWindow<tauri::Wry>, rebuild: bool) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        use objc2_app_kit::NSWindow;
        let Ok(ns) = w.ns_window() else { return };
        // SAFETY: the window's live NSWindow, on the main thread.
        let ns = unsafe { &*(ns as *const NSWindow) };
        let Some(group) = ns.tabGroup() else { return };
        if group.isTabBarVisible() {
            if !rebuild {
                return;
            }
            ns.toggleTabBar(None);
        }
        ns.toggleTabBar(None);
    });
}

#[cfg(not(target_os = "macos"))]
pub fn show_tab_bar(_window: &tauri::WebviewWindow<tauri::Wry>, _rebuild: bool) {}
