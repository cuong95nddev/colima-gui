//! macOS menu-bar icon + menu. Closing the window only hides the app; it keeps running here.

use eframe::egui;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use std::cell::RefCell;

use crate::backend::{Command, Shared, State};

/// Hide the window and the Dock icon; the app keeps running in the menu bar.
pub fn hide_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.hide(None);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// Bring the window (and the Dock icon) back.
pub fn show_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        app.unhide(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

/// Menu items are !Send, so they live in a main-thread-local slot that both the UI and the
/// tray hover handler (also main thread) can update.
struct Items {
    status: MenuItem,
    start: MenuItem,
    stop: MenuItem,
    last: String,
}

thread_local! {
    static ITEMS: RefCell<Option<Items>> = const { RefCell::new(None) };
}

pub struct Tray {
    _icon: TrayIcon,
}

impl Tray {
    /// Must be called on the main thread after the event loop has started (eframe app creation).
    pub fn new(tx: UnboundedSender<Command>, ctx: egui::Context, state: Shared) -> Option<Self> {
        let status = MenuItem::new("Colima: checking…", false, None);
        let open = MenuItem::new("Open Colima GUI", true, None);
        let start = MenuItem::new("Start Colima", true, None);
        let stop = MenuItem::new("Stop Colima", true, None);
        let quit = MenuItem::new("Quit Colima GUI", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &open,
            &PredefinedMenuItem::separator(),
            &start,
            &stop,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .ok()?;
        let icon = Icon::from_rgba(crate::icon::tray_rgba(44), 44, 44).ok()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon_templated(icon)
            .with_tooltip("Colima GUI")
            .build()
            .ok()?;

        let (open_id, start_id, stop_id, quit_id) =
            (open.id().clone(), start.id().clone(), stop.id().clone(), quit.id().clone());
        // Menu actions arrive on the main thread, even while the window is hidden.
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == open_id {
                show_app();
                ctx.request_repaint();
            } else if e.id == start_id {
                let _ = tx.send(Command::StartColima);
            } else if e.id == stop_id {
                let _ = tx.send(Command::StopColima);
            } else if e.id == quit_id {
                std::process::exit(0);
            }
        }));

        // Refresh the menu as soon as the pointer reaches the icon, before the menu opens —
        // the egui UI does not run while the window is hidden.
        tray_icon::TrayIconEvent::set_event_handler(Some(move |_e| {
            let st = state.lock().unwrap().clone();
            sync(&st);
        }));
        ITEMS.with(|i| *i.borrow_mut() = Some(Items { status, start, stop, last: String::new() }));
        Some(Self { _icon: tray })
    }
}

/// Reflect the latest state in the menu (cheap: only touches AppKit when something changed).
pub fn sync(st: &State) {
    ITEMS.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(items) = guard.as_mut() else { return };
        items.update(st);
    });
}

impl Items {
    fn update(&mut self, st: &State) {
        let running = st.colima.as_ref().is_some_and(|c| c.running);
        let text = match (&st.colima_busy, running) {
            (Some(b), _) => format!("Colima: {b}"),
            (None, true) => {
                let n = st.containers.len();
                let up = st.containers.iter().filter(|c| crate::container_state(c) == "running").count();
                format!("Colima: Running · {up}/{n} containers up")
            }
            (None, false) if st.setup.missing_colima() => "Colima: Not installed".into(),
            (None, false) if st.colima.is_none() => "Colima: checking…".into(),
            (None, false) => "Colima: Stopped".into(),
        };
        let key = format!("{text}|{running}|{}", st.colima_busy.is_some());
        if key == self.last {
            return;
        }
        self.last = key;
        self.status.set_text(&text);
        let idle = st.colima_busy.is_none();
        self.start.set_enabled(idle && !running && !st.setup.missing_colima());
        self.stop.set_enabled(idle && running);
    }
}
