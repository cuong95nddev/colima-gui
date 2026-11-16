//! macOS menu-bar icon + menu. Closing the window only hides the app; it keeps running here.

use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSButton, NSColor, NSFont, NSImage, NSLineBreakMode, NSMenu,
    NSMenuItem, NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::backend::{Command, ContainerAction, Shared, State};

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
    menu: Menu,
    /// Container section (rebuilt when the container list or states change), placed right after the status line.
    dynamic: Vec<Retained<NSMenuItem>>,
    /// Button tag → action for the rows in `dynamic`.
    row_acts: Vec<RowAct>,
    target: Option<Retained<RowTarget>>,
    tx: UnboundedSender<Command>,
    last_containers: String,
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
            .with_menu(Box::new(menu.clone()))
            .with_icon_templated(icon)
            .with_tooltip("Colima GUI")
            .build()
            .ok()?;

        let (open_id, start_id, stop_id, quit_id) =
            (open.id().clone(), start.id().clone(), stop.id().clone(), quit.id().clone());
        let tx2 = tx.clone();
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
        ITEMS.with(|i| *i.borrow_mut() = Some(Items {
            status,
            start,
            stop,
            last: String::new(),
            menu,
            dynamic: Vec::new(),
            row_acts: Vec::new(),
            target: None,
            tx: tx2,
            last_containers: String::new(),
        }));
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
        self.update_containers(st);
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

/// What a button in a container row does; indexed by the button's tag.
enum RowAct {
    Container(String, ContainerAction),
    Shell(String),
}

define_class!(
    /// Target for the row buttons (AppKit needs an Objective-C object + selector).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ColimaGuiTrayTarget"]
    struct RowTarget;

    impl RowTarget {
        #[unsafe(method(click:))]
        fn click(&self, sender: &NSButton) {
            // Copy what we need out of ITEMS first: acting may re-enter the tray code.
            let Some((menu, tx, act)) = ITEMS.with(|cell| {
                let guard = cell.borrow();
                let items = guard.as_ref()?;
                let act = match items.row_acts.get(sender.tag() as usize)? {
                    RowAct::Container(id, a) => RowAct::Container(id.clone(), *a),
                    RowAct::Shell(id) => RowAct::Shell(id.clone()),
                };
                Some((items.ns_menu(), items.tx.clone(), act))
            }) else {
                return;
            };
            match act {
                RowAct::Container(id, a) => {
                    let _ = tx.send(Command::Container(id, a));
                }
                RowAct::Shell(id) => {
                    crate::open_shell(&id);
                }
            }
            // Close the menu: it does not redraw while open, so the next open shows the new state.
            menu.cancelTracking();
        }
    }
);

const ROW_W: f64 = 320.0;
const ROW_H: f64 = 26.0;
const BTN: f64 = 24.0;

impl Items {
    fn ns_menu(&self) -> Retained<NSMenu> {
        // SAFETY: muda hands out its live NSMenu*, kept alive by `self.menu`.
        unsafe { Retained::retain(self.menu.ns_menu() as *mut NSMenu) }.expect("tray NSMenu")
    }

    /// One row per container under its compose project: colored status dot, name, and
    /// always-visible icon buttons (stop/start, restart, terminal).
    fn update_containers(&mut self, st: &State) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let running = st.colima.as_ref().is_some_and(|c| c.running);
        let mut groups: BTreeMap<(bool, String), Vec<_>> = BTreeMap::new();
        if running {
            for c in &st.containers {
                let p = crate::compose_project(c);
                groups.entry((p.is_empty(), p)).or_default().push(c);
            }
        }
        for list in groups.values_mut() {
            list.sort_by_key(|c| crate::container_name(c));
        }
        let key: String = groups
            .values()
            .flatten()
            .map(|c| {
                let id = c.id.as_deref().unwrap_or("");
                format!("{id}:{}:{}:{};", crate::container_state(c), crate::compose_project(c), st.busy.contains(id))
            })
            .collect();
        if key == self.last_containers {
            return;
        }
        self.last_containers = key;

        let menu = self.ns_menu();
        for item in self.dynamic.drain(..) {
            menu.removeItem(&item);
        }
        self.row_acts.clear();
        let target = self.target.get_or_insert_with(|| unsafe { msg_send![RowTarget::alloc(mtm), init] }).clone();

        let mut items: Vec<Retained<NSMenuItem>> = Vec::new();
        let only_standalone = groups.len() == 1 && groups.keys().all(|(standalone, _)| *standalone);
        for ((_, project), list) in &groups {
            items.push(NSMenuItem::separatorItem(mtm));
            if !only_standalone {
                let title = if project.is_empty() { "Other" } else { project.as_str() };
                let header = NSMenuItem::new(mtm);
                header.setTitle(&NSString::from_str(title));
                header.setEnabled(false);
                items.push(header);
            }
            for c in list {
                items.push(self.container_row(mtm, &target, c, st.busy.contains(c.id.as_deref().unwrap_or(""))));
            }
        }
        for (i, item) in items.iter().enumerate() {
            menu.insertItem_atIndex(item, 1 + i as isize);
        }
        self.dynamic = items;
    }

    fn container_row(
        &mut self,
        mtm: MainThreadMarker,
        target: &RowTarget,
        c: &bollard::models::ContainerSummary,
        busy: bool,
    ) -> Retained<NSMenuItem> {
        let id = c.id.clone().unwrap_or_default();
        let state = crate::container_state(c);
        let up = state == "running";
        let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(ROW_W, ROW_H)));

        // Status dot.
        let dot = NSTextField::labelWithString(&NSString::from_str("●"), mtm);
        dot.setFrame(NSRect::new(NSPoint::new(14.0, 4.0), NSSize::new(14.0, 18.0)));
        dot.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        let color = match state.as_str() {
            "running" => NSColor::systemGreenColor(),
            "paused" | "restarting" | "created" => NSColor::systemOrangeColor(),
            "dead" => NSColor::systemRedColor(),
            _ => NSColor::tertiaryLabelColor(),
        };
        dot.setTextColor(Some(&color));
        view.addSubview(&dot);

        // Buttons in fixed columns (same order as the main window): stop/start, restart, terminal.
        let mut buttons: Vec<(&str, &str, RowAct, bool)> = Vec::new();
        if up {
            buttons.push(("stop.fill", "Stop", RowAct::Container(id.clone(), ContainerAction::Stop), !busy));
            buttons.push(("arrow.clockwise", "Restart", RowAct::Container(id.clone(), ContainerAction::Restart), !busy));
            buttons.push(("terminal", "Open terminal", RowAct::Shell(id.clone()), true));
        } else {
            buttons.push(("play.fill", "Start", RowAct::Container(id.clone(), ContainerAction::Start), !busy));
        }
        let mut x = ROW_W - 10.0 - BTN * 3.0;
        for (symbol, tip, act, enabled) in buttons {
            let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&NSString::from_str(tip)),
            ) else {
                continue;
            };
            let b = unsafe { NSButton::buttonWithImage_target_action(&img, Some(target), Some(sel!(click:)), mtm) };
            b.setBordered(false);
            b.setFrame(NSRect::new(NSPoint::new(x, 1.0), NSSize::new(BTN, BTN)));
            b.setToolTip(Some(&NSString::from_str(tip)));
            b.setEnabled(enabled);
            b.setTag(self.row_acts.len() as isize);
            self.row_acts.push(act);
            view.addSubview(&b);
            x += BTN;
        }

        // Name, truncated to the space left of the buttons.
        let name = NSTextField::labelWithString(&NSString::from_str(&crate::container_name(c)), mtm);
        name.setFont(Some(&NSFont::menuFontOfSize(0.0)));
        name.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        name.setFrame(NSRect::new(NSPoint::new(32.0, 4.0), NSSize::new(ROW_W - 10.0 - BTN * 3.0 - 40.0, 18.0)));
        name.setToolTip(Some(&NSString::from_str(&crate::short_status(c))));
        view.addSubview(&name);

        let item = NSMenuItem::new(mtm);
        item.setView(Some(&view));
        item
    }
}
