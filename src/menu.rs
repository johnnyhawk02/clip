//! The native macOS menu bar (muda), built from `actions::MENUS`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use muda::accelerator::Accelerator;
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::actions::{Action, Item, MENUS, WINDOW_MENU};

enum Entry {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

pub struct NativeMenu {
    _menu: Menu,
    entries: HashMap<Action, Entry>,
    /// Last state pushed to AppKit per item: (enabled, checked, label), to avoid redundant calls.
    state: RefCell<HashMap<Action, (bool, bool, String)>>,
    rx: Receiver<Action>,
}

impl NativeMenu {
    pub fn install(ctx: &eframe::egui::Context) -> NativeMenu {
        let menu = Menu::new();
        let mut entries = HashMap::new();

        let app_menu = Submenu::new("Clip", true);
        let quit = MenuItem::with_id(Action::Quit.id(), Action::Quit.label(), true, accel(Action::Quit));
        let prefs = MenuItem::with_id(Action::Preferences.id(), Action::Preferences.label(), true, accel(Action::Preferences));
        let _ = app_menu.append_items(&[
            &PredefinedMenuItem::about(
                Some("About Clip"),
                Some(AboutMetadata {
                    name: Some("Clip".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &prefs,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some("Hide Clip")),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            // Our own Quit (not the predefined one) so unsaved edits still get a prompt.
            &quit,
        ]);
        entries.insert(Action::Quit, Entry::Plain(quit));
        entries.insert(Action::Preferences, Entry::Plain(prefs));
        let _ = menu.append(&app_menu);

        for (title, items) in MENUS {
            let sub = Submenu::new(*title, true);
            fill(&sub, items, &mut entries);
            let _ = menu.append(&sub);
        }

        let window = Submenu::new("Window", true);
        fill(&window, WINDOW_MENU, &mut entries);
        let _ = window.append_items(&[
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ]);
        let _ = menu.append(&window);

        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        let (tx, rx) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some(a) = Action::from_id(&e.id.0) {
                let _ = tx.send(a);
                ctx.request_repaint();
            }
        }));

        NativeMenu { _menu: menu, entries, state: RefCell::new(HashMap::new()), rx }
    }

    pub fn poll(&self) -> Vec<Action> {
        self.rx.try_iter().collect()
    }

    /// Push enabled/checked state and dynamic labels (e.g. "Undo Fade Out") to the menu.
    pub fn sync(&self, enabled: impl Fn(Action) -> bool, checked: impl Fn(Action) -> bool, label: impl Fn(Action) -> String) {
        let mut state = self.state.borrow_mut();
        for (a, entry) in &self.entries {
            let new = (enabled(*a), checked(*a), label(*a));
            if state.get(a) == Some(&new) {
                continue;
            }
            match entry {
                Entry::Plain(m) => {
                    m.set_enabled(new.0);
                    m.set_text(&new.2);
                }
                Entry::Check(m) => {
                    m.set_enabled(new.0);
                    m.set_checked(new.1);
                }
            }
            state.insert(*a, new);
        }
    }
}

fn accel(a: Action) -> Option<Accelerator> {
    a.accelerator().and_then(|s| s.parse().ok())
}

fn fill(sub: &Submenu, items: &[Item], entries: &mut HashMap<Action, Entry>) {
    for item in items {
        match item {
            Item::Separator => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            Item::Sub(title, inner) => {
                let s = Submenu::new(*title, true);
                fill(&s, inner, entries);
                let _ = sub.append(&s);
            }
            Item::Do(a) if a.is_check() => {
                let m = CheckMenuItem::with_id(a.id(), a.label(), true, false, accel(*a));
                let _ = sub.append(&m);
                entries.insert(*a, Entry::Check(m));
            }
            Item::Do(a) => {
                let m = MenuItem::with_id(a.id(), a.label(), true, accel(*a));
                let _ = sub.append(&m);
                entries.insert(*a, Entry::Plain(m));
            }
        }
    }
}

/// Print the installed menu bar (titles and key equivalents), for the headless self-test.
#[cfg(target_os = "macos")]
pub fn dump_main_menu() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSMenu};
    fn walk(menu: &NSMenu, depth: usize) {
        for item in menu.itemArray().iter() {
            if item.isSeparatorItem() {
                continue;
            }
            let key = item.keyEquivalent().to_string();
            let state = if item.isEnabled() { "" } else { " (disabled)" };
            eprintln!("{}{}{}{}", "  ".repeat(depth), item.title(), if key.is_empty() { String::new() } else { format!("  [⌘{key}]") }, state);
            if let Some(sub) = item.submenu() {
                walk(&sub, depth + 1);
            }
        }
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    match NSApplication::sharedApplication(mtm).mainMenu() {
        Some(m) => walk(&m, 0),
        None => eprintln!("no main menu"),
    }
}

/// Where the mouse is, in the window's own coordinates (points from its top-left). File drops
/// don't carry a position, so the arrangement asks AppKit directly when a file is dropped.
#[cfg(target_os = "macos")]
pub fn mouse_in_window(window_inner: eframe::egui::Rect) -> Option<eframe::egui::Pos2> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen};
    let mtm = MainThreadMarker::new()?;
    let p = NSEvent::mouseLocation();
    // AppKit measures from the bottom of the main screen; egui from the top.
    let screen_h = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let top = screen_h - p.y;
    Some(eframe::egui::pos2(p.x as f32 - window_inner.min.x, top as f32 - window_inner.min.y))
}

/// Files macOS asks the app to open (double-clicking a .sndproj or an audio file in Finder,
/// dropping onto the Dock icon, Open With…). Collected here and picked up by the app each frame.
#[cfg(target_os = "macos")]
static OPEN_REQUESTS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
#[cfg(target_os = "macos")]
static OPEN_CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

/// `-[NSApplication finishLaunching]` as AppKit implemented it, before we wrapped it.
#[cfg(target_os = "macos")]
static ORIGINAL_FINISH_LAUNCHING: std::sync::OnceLock<objc2::runtime::Imp> = std::sync::OnceLock::new();

/// `application:openURLs:` for the app delegate: AppKit calls it for files opened from Finder,
/// both when they launch the app and while it's running.
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn open_urls(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    _app: *mut objc2::runtime::AnyObject,
    urls: *mut objc2::runtime::AnyObject,
) {
    use objc2::msg_send;
    use objc2::rc::Retained;
    if urls.is_null() {
        return;
    }
    unsafe {
        let n: usize = msg_send![urls, count];
        for i in 0..n {
            let url: Option<Retained<objc2_foundation::NSURL>> = msg_send![urls, objectAtIndex: i];
            if let Some(path) = url.and_then(|u| u.path()) {
                OPEN_REQUESTS.lock().unwrap().push(std::path::PathBuf::from(path.to_string()));
            }
        }
    }
    if let Some(ctx) = OPEN_CTX.get() {
        ctx.request_repaint();
    }
}

/// Our wrapper: give the delegate (winit's, which exists by now) an `application:openURLs:`
/// before AppKit finishes launching and delivers any files that launched the app.
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn finish_launching(this: *mut objc2::runtime::AnyObject, cmd: objc2::runtime::Sel) {
    use objc2::{msg_send, sel};
    unsafe {
        let delegate: *mut objc2::runtime::AnyObject = msg_send![this, delegate];
        if !delegate.is_null() {
            let cls = objc2::ffi::object_getClass(delegate) as *mut objc2::runtime::AnyClass;
            let imp: objc2::runtime::Imp = std::mem::transmute(open_urls as unsafe extern "C-unwind" fn(_, _, _, _));
            objc2::ffi::class_addMethod(cls, sel!(application:openURLs:), imp, c"v@:@@".as_ptr());
        }
        if let Some(orig) = ORIGINAL_FINISH_LAUNCHING.get() {
            let orig: unsafe extern "C-unwind" fn(*mut objc2::runtime::AnyObject, objc2::runtime::Sel) = std::mem::transmute(*orig);
            orig(this, cmd);
        }
    }
}

/// Lets Finder open files in the app (double-clicking a project, Open With, the Dock icon).
/// Call once at startup, before the event loop starts: it wraps AppKit's finishLaunching.
#[cfg(target_os = "macos")]
pub fn install_open_event_handler() {
    use objc2::runtime::AnyClass;
    use objc2::sel;
    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else { return };
        let method = objc2::ffi::class_getInstanceMethod(cls, sel!(finishLaunching));
        if method.is_null() || ORIGINAL_FINISH_LAUNCHING.get().is_some() {
            return;
        }
        let imp: objc2::runtime::Imp = std::mem::transmute(finish_launching as unsafe extern "C-unwind" fn(_, _));
        if let Some(orig) = objc2::ffi::method_setImplementation(method, imp) {
            let _ = ORIGINAL_FINISH_LAUNCHING.set(orig);
        }
    }
}

/// Give the open hook the egui context, so files opened later wake the app up.
#[cfg(target_os = "macos")]
pub fn install_open_handler(ctx: &eframe::egui::Context) {
    let _ = OPEN_CTX.set(ctx.clone());
}

#[cfg(target_os = "macos")]
pub fn take_open_requests() -> Vec<std::path::PathBuf> {
    std::mem::take(&mut *OPEN_REQUESTS.lock().unwrap())
}
