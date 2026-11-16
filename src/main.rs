mod backend;
mod icon;
mod icons;
mod theme;
mod titlebar;
mod tray;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::{Command, ContainerAction, Shared, State};
use bollard::models::ContainerSummary;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Painter,
    Pos2, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};
use eframe::egui::text::{LayoutJob, TextWrapping};
use icons::Icon;
use theme::*;
use tokio::sync::mpsc::UnboundedSender;

// ───────────────────────────── formatting helpers ─────────────────────────────

pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else if v >= 100.0 { format!("{v:.0} {}", U[i]) } else { format!("{v:.1} {}", U[i]) }
}

fn fmt_age(ts: i64) -> String {
    let secs = (chrono::Utc::now().timestamp() - ts).max(0);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s if s < 86400 * 30 => format!("{} days ago", s / 86400),
        s if s < 86400 * 365 => format!("{} months ago", s / (86400 * 30)),
        s => format!("{} years ago", s / (86400 * 365)),
    }
}

pub(crate) fn container_name(c: &ContainerSummary) -> String {
    c.names
        .as_ref()
        .and_then(|n| n.first())
        .map(|n| n.trim_start_matches('/').to_string())
        .unwrap_or_else(|| short_id(c.id.as_deref().unwrap_or("")))
}

fn short_id(id: &str) -> String {
    id.trim_start_matches("sha256:").chars().take(12).collect()
}

pub(crate) fn container_state(c: &ContainerSummary) -> String {
    c.state.map(|s| s.to_string()).unwrap_or_default()
}

pub(crate) fn compose_project(c: &ContainerSummary) -> String {
    c.labels.as_ref().and_then(|l| l.get("com.docker.compose.project").cloned()).unwrap_or_default()
}

/// Published ports as (host, container), deduplicated across IPv4/IPv6 bindings.
fn ports(c: &ContainerSummary) -> Vec<(u16, u16)> {
    let mut v: Vec<(u16, u16)> =
        c.ports.iter().flatten().filter_map(|p| p.public_port.map(|pp| (pp, p.private_port))).collect();
    v.sort();
    v.dedup();
    v
}

fn image_label(c: &ContainerSummary) -> String {
    let img = c.image.clone().unwrap_or_default();
    if img.starts_with("sha256:") { short_id(&img) } else { img }
}

/// "Exited (255) 3 minutes ago" → "Exited 3 minutes ago"
pub(crate) fn short_status(c: &ContainerSummary) -> String {
    let s = c.status.clone().unwrap_or_default();
    match (s.find(" ("), s.find(") ")) {
        (Some(a), Some(b)) if a < b => format!("{}{}", &s[..a], &s[b + 1..]),
        _ => s,
    }
}

fn state_color(state: &str) -> Color32 {
    match state {
        "running" => GREEN,
        "paused" | "restarting" | "created" => ORANGE,
        "dead" => RED,
        _ => FAINT,
    }
}


/// "ghcr.io:443/org/app:1.2" → ("ghcr.io:443/org/app", "1.2")
fn split_tag(s: &str) -> (String, String) {
    match s.rfind(':') {
        Some(i) if !s[i..].contains('/') => (s[..i].to_string(), s[i + 1..].to_string()),
        _ => (s.to_string(), "latest".into()),
    }
}


/// Open a shell in the container in Terminal.app. Returns false when the docker CLI is missing.
pub(crate) fn open_shell(id: &str) -> bool {
    let Some(docker) = backend::which("docker") else { return false };
    let home = std::env::var("HOME").unwrap_or_default();
    run_in_terminal(&format!(
        "DOCKER_HOST=unix://{home}/.colima/default/docker.sock {docker} exec -it {id} sh -c 'command -v bash >/dev/null && exec bash || exec sh'"
    ));
    true
}

/// Run a shell command in a new Terminal.app window.
fn run_in_terminal(script: &str) {
    let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("tell application \"Terminal\" to do script \"{escaped}\""),
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .spawn();
}

// ───────────────────────────── drawing primitives ─────────────────────────────

/// Paint single-line text, truncated with "…" to fit `rect`, vertically centred.
fn paint_cell(p: &Painter, rect: Rect, text: &str, font: FontId, color: Color32) {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width(rect.width().max(1.0));
    let g = p.layout_job(job);
    p.galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, color);
}

fn sparkline(p: &Painter, rect: Rect, hist: Option<&VecDeque<f64>>, color: Color32, floor_max: f64) {
    p.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));
    let Some(h) = hist.filter(|h| h.len() >= 2) else { return };
    let max = h.iter().copied().fold(floor_max, f64::max) * 1.15;
    let n = backend::HISTORY_LEN.max(2);
    let off = n - h.len();
    let pts: Vec<Pos2> = h
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + (i + off) as f32 / (n - 1) as f32 * rect.width();
            let y = rect.bottom() - (*v / max) as f32 * rect.height();
            pos2(x, y.clamp(rect.top(), rect.bottom()))
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    let fill = tint(color, 34);
    for w in pts.windows(2) {
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(w[0], fill);
        mesh.colored_vertex(w[1], fill);
        mesh.colored_vertex(pos2(w[1].x, rect.bottom()), tint(color, 4));
        mesh.colored_vertex(pos2(w[0].x, rect.bottom()), tint(color, 4));
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    p.add(mesh);
    p.add(egui::Shape::line(pts, Stroke::new(1.6, color)));
}


/// Square, always-visible icon button. `danger` turns the icon red on hover.
fn icon_btn_ex(ui: &mut Ui, icon: Icon, tip: &str, danger: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let hot = resp.hovered();
    if hot {
        let bg = if danger { tint(RED, 30) } else { SURFACE_HI };
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }
    let color = match (hot, danger) {
        (true, true) => RED,
        (true, false) => TEXT,
        _ => MUTED,
    };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(15.0, 15.0)), icon, color);
    resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand)
}

fn icon_btn(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    icon_btn_ex(ui, icon, tip, false)
}

#[derive(Clone, Copy, PartialEq)]
enum BtnKind {
    Normal,
    Primary,
    Danger,
}

/// Bordered button with an optional leading icon.
fn btn(ui: &mut Ui, icon: Option<Icon>, text: &str, kind: BtnKind) -> Response {
    let font = FontId::new(13.0, FontFamily::Proportional);
    let fg = if kind == BtnKind::Normal { TEXT } else { Color32::WHITE };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let size = vec2(g.size().x + icon_w + 24.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hot = resp.hovered();
    let (fill, stroke) = match kind {
        BtnKind::Normal => (if hot { rgb(0x31, 0x35, 0x3b) } else { SURFACE_HI }, Stroke::new(1.0, BORDER)),
        BtnKind::Primary => (if hot { rgb(0x56, 0x9a, 0xff) } else { ACCENT }, Stroke::NONE),
        BtnKind::Danger => (if hot { rgb(0xff, 0x75, 0x6e) } else { RED }, Stroke::NONE),
    };
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(7), fill, stroke, egui::StrokeKind::Inside);
    let mut x = rect.left() + 12.0;
    if let Some(i) = icon {
        icons::paint(p, Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0)), i, fg);
        x += icon_w;
    }
    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Small rounded label, e.g. "In use".
fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.5), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 14.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), tint(color, 28));
    ui.painter().galley(pos2(rect.left() + 7.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

const ROW_PAD: f32 = 12.0;

/// Allocate a full-width clickable row with hover background and a hairline divider.
fn list_row(ui: &mut Ui, height: f32) -> (Rect, Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let p = ui.painter();
    if resp.contains_pointer() {
        p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::same(8), HOVER);
    } else {
        p.hline(rect.x_range().shrink(ROW_PAD), rect.bottom() - 0.5, Stroke::new(1.0, DIVIDER));
    }
    (rect, resp)
}

/// Two-line text block: title + muted subtitle.
fn paint_two_line(p: &Painter, rect: Rect, title: &str, title_color: Color32, sub: &str) {
    let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.center().y), rect.max);
    paint_cell(p, top.translate(vec2(0.0, 2.5)), title, FontId::new(13.5, bold_family()), title_color);
    paint_cell(p, bottom.translate(vec2(0.0, -1.5)), sub, FontId::proportional(12.0), FAINT);
}

/// Rounded square with a tinted background and an icon, used as a row's leading visual.
fn paint_badge_icon(p: &Painter, center: Pos2, icon: Icon, color: Color32) {
    let r = Rect::from_center_size(center, vec2(30.0, 30.0));
    p.rect_filled(r, CornerRadius::same(8), tint(color, 26));
    icons::paint(p, Rect::from_center_size(center, vec2(15.0, 15.0)), icon, color);
}

/// A child UI placed inside part of a row, for interactive content.
fn cell_ui<R>(ui: &mut Ui, rect: Rect, rtl: bool, salt: impl std::hash::Hash + std::fmt::Debug, f: impl FnOnce(&mut Ui) -> R) -> R {
    let layout = if rtl { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout).id_salt(salt));
    child.spacing_mut().item_spacing.x = 2.0;
    child.spacing_mut().button_padding = vec2(4.0, 4.0);
    f(&mut child)
}

fn section_label(ui: &mut Ui, text: &str, count: usize) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_PAD);
        ui.label(RichText::new(text.to_uppercase()).family(bold_family()).size(10.5).color(MUTED));
        ui.label(RichText::new(count.to_string()).size(11.0).color(FAINT));
    });
    ui.add_space(4.0);
}

fn empty_state(ui: &mut Ui, icon: Icon, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.28).max(20.0));
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(14), SURFACE);
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(26.0, 26.0)), icon, FAINT);
        ui.add_space(12.0);
        ui.label(bold(title, 15.0));
        ui.add_space(2.0);
        ui.label(muted(hint));
    });
}

/// One checklist line on the setup screen: status icon, name, description, state label.
fn setup_item(ui: &mut Ui, ok: bool, name: &str, desc: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let p = ui.painter();
    let c = pos2(rect.left() + 10.0, rect.center().y);
    if ok {
        p.circle_filled(c, 10.0, tint(GREEN, 36));
        icons::paint(p, Rect::from_center_size(c, vec2(12.0, 12.0)), Icon::Check, GREEN);
    } else {
        p.circle_stroke(c, 9.0, Stroke::new(1.4, FAINT));
    }
    let text_r = Rect::from_min_max(pos2(rect.left() + 32.0, rect.top()), pos2(rect.right() - 80.0, rect.bottom()));
    paint_two_line(p, text_r, name, TEXT, desc);
    let (label, color) = if ok { ("Installed", GREEN) } else { ("Missing", ORANGE) };
    let g = p.layout_no_wrap(label.into(), FontId::proportional(12.0), color);
    p.galley(pos2(rect.right() - g.size().x, rect.center().y - g.size().y / 2.0), g, color);
}

/// Search field with a leading magnifier icon.
fn search_field(ui: &mut Ui, text: &mut String, id: Id, hint: &str, width: f32) {
    let focused = ui.memory(|m| m.has_focus(id));
    Frame::new()
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0, if focused { tint(ACCENT, 160) } else { BORDER }))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin { left: 8, right: 4, top: 0, bottom: 0 })
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_height(28.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                icons::show(ui, Icon::Search, 13.0, FAINT);
                let w = ui.available_width() - if text.is_empty() { 0.0 } else { 34.0 };
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .hint_text(RichText::new(hint).color(FAINT))
                        .frame(egui::Frame::NONE)
                        .margin(Margin::ZERO)
                        .min_size(vec2(0.0, 28.0))
                        .align(Align2::LEFT_CENTER)
                        .desired_width(w),
                );
                if !text.is_empty() && icon_btn(ui, Icon::Close, "Clear").clicked() {
                    text.clear();
                }
            });
        });
}

/// Make an area drag the window (the title bar is hidden).
fn drag_strip(ui: &Ui, rect: Rect, salt: &str) {
    let r = ui.interact(rect, Id::unique(("drag", salt)), Sense::click_and_drag());
    if r.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if r.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
}

/// Segmented-control tab: icon + label + count, filled background when selected.
fn tab(ui: &mut Ui, icon: Icon, label: &str, count: &str, selected: bool) -> bool {
    let fg = if selected { TEXT } else { MUTED };
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::new(13.0, bold_family()), fg);
    let c = ui.painter().layout_no_wrap(count.to_string(), FontId::proportional(11.5), if selected { MUTED } else { FAINT });
    let w = 12.0 + 14.0 + 7.0 + g.size().x + 7.0 + c.size().x + 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect(rect, CornerRadius::same(6), SURFACE_HI, Stroke::new(1.0, rgb(0x34, 0x38, 0x3f)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let mut x = rect.left() + 12.0;
    icons::paint(p, Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0)), icon, if selected { ACCENT } else { FAINT });
    x += 14.0 + 7.0;
    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), fg);
    x += g.size().x + 7.0;
    p.galley(pos2(x, rect.center().y - c.size().y / 2.0), c, FAINT);
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// Published ports as small clickable chips that open localhost in the browser.
fn port_links(ui: &mut Ui, c: &ContainerSummary) {
    let ps = ports(c);
    for (host, inner) in ps.iter().take(2) {
        let text = format!(":{host}");
        let g = ui.painter().layout_no_wrap(text, FontId::monospace(11.5), ACCENT);
        let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 22.0), Sense::click());
        let hot = r.hovered();
        ui.painter().rect_filled(rect, CornerRadius::same(6), tint(ACCENT, if hot { 48 } else { 26 }));
        ui.painter().galley(pos2(rect.left() + 7.0, rect.center().y - g.size().y / 2.0), g, ACCENT);
        icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.right() - 16.0, rect.center().y - 5.0), vec2(10.0, 10.0)), Icon::External, ACCENT);
        let r = r
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("Open http://localhost:{host}  (→ container port {inner})"));
        if r.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(format!("http://localhost:{host}")));
        }
        ui.add_space(4.0);
    }
    if ps.len() > 2 {
        let rest: Vec<String> = ps[2..].iter().map(|(h, i)| format!("{h}:{i}")).collect();
        ui.label(RichText::new(format!("+{}", ps.len() - 2)).color(FAINT)).on_hover_text(rest.join(", "));
    }
}

// ───────────────────────────── app ─────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Containers,
    Images,
    Volumes,
}

struct Confirm {
    title: String,
    body: String,
    button: String,
    cmds: Vec<Command>,
}

struct App {
    state: Shared,
    tx: UnboundedSender<Command>,
    page: Page,
    filter: String,
    detail: Option<String>,
    log_filter: String,
    follow: bool,
    confirm: Option<Confirm>,
    toast: Option<(String, f64)>,
    tray: Option<tray::Tray>,
}

const TOP_BAR_H: f32 = 52.0;
const SEARCH_ID: &str = "global-search";

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        cc.egui_ctx.global_style_mut(|s| s.interaction.selectable_labels = false);
        let state: Shared = Arc::new(Mutex::new(State::default()));
        let tx = backend::spawn(state.clone(), cc.egui_ctx.clone());
        let tray = tray::Tray::new(tx.clone(), cc.egui_ctx.clone(), state.clone());
        Self {
            state,
            tx,
            page: Page::Containers,
            filter: String::new(),
            detail: None,
            log_filter: String::new(),
            follow: true,
            confirm: None,
            toast: None,
            tray,
        }
    }

    fn shell(&self, id: &str) {
        if !open_shell(id) {
            self.state.lock().unwrap().notice = Some("Docker CLI not found — install it with: brew install docker".into());
        }
    }

    fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    fn open_detail(&mut self, id: Option<String>) {
        if self.detail != id {
            self.detail = id.clone();
            self.log_filter.clear();
            self.follow = true;
            self.send(Command::FollowLogs(id));
        }
    }

    fn matches(&self, text: &str) -> bool {
        self.filter.is_empty() || text.to_lowercase().contains(&self.filter.to_lowercase())
    }

    fn ask(&mut self, title: impl Into<String>, body: impl Into<String>, button: &str, cmds: Vec<Command>) {
        self.confirm = Some(Confirm { title: title.into(), body: body.into(), button: button.into(), cmds });
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.confirm.is_some() {
            return;
        }
        let (find, refresh, p1, p2, p3, close, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::COMMAND, Key::Num1),
                i.consume_key(Modifiers::COMMAND, Key::Num2),
                i.consume_key(Modifiers::COMMAND, Key::Num3),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.key_pressed(Key::Escape),
            )
        });
        if find {
            ctx.memory_mut(|m| m.request_focus(Id::unique(SEARCH_ID)));
        }
        if refresh {
            self.send(Command::Refresh);
        }
        for (hit, page) in [(p1, Page::Containers), (p2, Page::Images), (p3, Page::Volumes)] {
            if hit {
                self.page = page;
                self.open_detail(None);
            }
        }
        if close {
            self.hide_window(ctx);
        }
        if esc {
            if self.detail.is_some() {
                self.open_detail(None);
            } else if !self.filter.is_empty() {
                self.filter.clear();
            }
        }
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.tray.is_some() {
            tray::hide_app();
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let st = self.state.lock().unwrap().clone();
        tray::sync(&st);

        // Closing the window keeps the app alive in the menu bar.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide_window(&ctx);
        }
        self.handle_keys(&ctx);

        titlebar::center_traffic_lights(TOP_BAR_H as f64, 20.0);
        // Top bar: Colima status + start/stop, next to the traffic lights.
        egui::Panel::top("topbar")
            .exact_size(TOP_BAR_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(BAR).inner_margin(Margin { left: 96, right: 16, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let r = ui.max_rect().expand2(vec2(96.0, 0.0));
                drag_strip(ui, r, "top");
                ui.painter().hline(r.x_range().expand(20.0), r.bottom() - 0.5, Stroke::new(1.0, BORDER));
                ui.horizontal_centered(|ui| self.colima_bar(ui, &st));
            });

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin { left: 20, right: 20, top: 0, bottom: 12 }))
            .show(ui, |ui| {
                self.banner(ui, &st);
                if !st.connected {
                    self.disconnected(ui, &st);
                    return;
                }
                let detail = self
                    .detail
                    .clone()
                    .and_then(|id| st.containers.iter().find(|c| c.id.as_deref() == Some(&id)).cloned());
                match detail {
                    Some(c) if self.page == Page::Containers => self.detail_page(ui, &c, &st),
                    _ => {
                        if self.detail.is_some() {
                            self.open_detail(None);
                        }
                        self.nav(ui, &st);
                        match self.page {
                            Page::Containers => self.containers_page(ui, &st),
                            Page::Images => self.images_page(ui, &st),
                            Page::Volumes => self.volumes_page(ui, &st),
                        }
                    }
                }
            });

        self.confirm_modal(&ctx);
        self.toast(&ctx, &st);
    }
}

// ───────────────────────────── chrome ─────────────────────────────

impl App {
    fn colima_bar(&mut self, ui: &mut Ui, st: &State) {
        let running = st.colima.as_ref().is_some_and(|c| c.running);
        let missing = st.setup.missing_colima();
        let (color, label) = match (&st.colima_busy, &st.colima, running) {
            (Some(b), _, _) => (ORANGE, b.clone()),
            _ if missing => (ORANGE, "Not installed".into()),
            (_, None, _) => (FAINT, "Checking…".into()),
            (_, _, true) => (GREEN, "Running".into()),
            _ => (RED, "Stopped".into()),
        };
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(bold("Colima", 14.0));
        // Status pill
        let g = ui.painter().layout_no_wrap(label, FontId::proportional(12.0), color);
        let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 22.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(11), tint(color, 30));
        ui.painter().circle_filled(pos2(r.left() + 10.0, r.center().y), 3.5, color);
        ui.painter().galley(pos2(r.left() + 18.0, r.center().y - g.size().y / 2.0), g, color);

        if let Some(c) = st.colima.as_ref().filter(|c| c.running) {
            ui.add_space(6.0);
            let resp = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, (v, k)) in [(c.cpu.to_string(), "CPU"), (fmt_bytes(c.memory), "RAM"), (fmt_bytes(c.disk), "Disk")].into_iter().enumerate() {
                        if i > 0 {
                            ui.add_space(8.0);
                        }
                        ui.label(RichText::new(v).color(MUTED));
                        ui.label(RichText::new(k).size(11.5).color(FAINT));
                    }
                })
                .response;
            resp.on_hover_text(format!("{} · {} · {}", c.runtime, c.driver, c.arch));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if st.colima_busy.is_some() {
                ui.spinner();
            } else if running {
                if btn(ui, Some(Icon::Power), "Stop", BtnKind::Normal).clicked() {
                    self.ask(
                        "Stop Colima?",
                        "The VM shuts down and every running container stops.",
                        "Stop Colima",
                        vec![Command::StopColima],
                    );
                }
            } else if st.colima.is_some() && !missing && btn(ui, Some(Icon::Power), "Start", BtnKind::Primary).clicked() {
                self.send(Command::StartColima);
            }
        });
    }

    /// Tabs on the left, search + page action on the right.
    fn nav(&mut self, ui: &mut Ui, st: &State) {
        let up = st.containers.iter().filter(|c| container_state(c) == "running").count();
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            // Segmented control
            Frame::new().fill(BAR).stroke(Stroke::new(1.0, DIVIDER)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (page, icon, label, count) in [
                        (Page::Containers, Icon::Box, "Containers", format!("{up}/{}", st.containers.len())),
                        (Page::Images, Icon::Layers, "Images", st.images.len().to_string()),
                        (Page::Volumes, Icon::Database, "Volumes", st.volumes.len().to_string()),
                    ] {
                        if tab(ui, icon, label, &count, self.page == page) {
                            self.page = page;
                        }
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match self.page {
                    Page::Images if st.busy.contains("prune:images") => {
                        ui.spinner();
                    }
                    Page::Images => {
                        if btn(ui, Some(Icon::Sweep), "Clean up", BtnKind::Normal).on_hover_text("Remove dangling (untagged) images").clicked() {
                            self.ask(
                                "Remove dangling images?",
                                "All untagged (<none>) images are removed.",
                                "Remove",
                                vec![Command::PruneImages],
                            );
                        }
                    }
                    Page::Volumes if st.busy.contains("prune:volumes") => {
                        ui.spinner();
                    }
                    Page::Volumes => {
                        if btn(ui, Some(Icon::Sweep), "Clean up", BtnKind::Normal).on_hover_text("Remove unused volumes").clicked() {
                            self.ask(
                                "Remove unused volumes?",
                                "Every volume not attached to a container is deleted. Their data cannot be recovered.",
                                "Delete volumes",
                                vec![Command::PruneVolumes],
                            );
                        }
                    }
                    Page::Containers => {}
                }
                let hint = match self.page {
                    Page::Containers => "Search containers",
                    Page::Images => "Search images",
                    Page::Volumes => "Search volumes",
                };
                search_field(ui, &mut self.filter, Id::unique(SEARCH_ID), hint, 220.0);
            });
        });
        ui.add_space(6.0);
    }

    fn banner(&mut self, ui: &mut Ui, st: &State) {
        let Some(err) = &st.error else { return };
        let mut dismiss = false;
        ui.add_space(8.0);
        Frame::new()
            .fill(tint(RED, 22))
            .stroke(Stroke::new(1.0, tint(RED, 80)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        dismiss = icon_btn(ui, Icon::Close, "Dismiss").clicked();
                        ui.add(egui::Label::new(RichText::new(err).color(TEXT)).truncate()).on_hover_text(err);
                    });
                });
            });
        if dismiss {
            self.state.lock().unwrap().error = None;
        }
    }

    fn toast(&mut self, ctx: &egui::Context, st: &State) {
        let now = ctx.input(|i| i.time);
        match &st.notice {
            Some(n) if self.toast.as_ref().map(|t| &t.0) != Some(n) => self.toast = Some((n.clone(), now)),
            None => self.toast = None,
            _ => {}
        }
        let Some((msg, t0)) = self.toast.clone() else { return };
        if now - t0 > 3.0 {
            self.state.lock().unwrap().notice = None;
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(300));
        egui::Area::new(Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -20.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(SURFACE_HI)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::symmetric(16, 8))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(TEXT));
                    });
            });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.confirm else { return };
        let (title, body, button) = (c.title.clone(), c.body.clone(), c.button.clone());
        let icon_for = if c.cmds.iter().any(|c| matches!(c, Command::StopColima)) { Icon::Power } else { Icon::Trash };
        let mut decision = None;
        let resp = egui::Modal::new(Id::unique("confirm"))
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(20))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                    paint_badge_icon(ui.painter(), r.center(), icon_for, RED);
                    ui.add_space(4.0);
                    ui.label(bold(title, 15.0));
                });
                ui.add_space(8.0);
                ui.label(muted(body));
                ui.add_space(18.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if btn(ui, None, &button, BtnKind::Danger).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        decision = Some(true);
                    }
                    if btn(ui, None, "Cancel", BtnKind::Normal).clicked() {
                        decision = Some(false);
                    }
                });
            });
        if resp.should_close() && decision.is_none() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                for cmd in self.confirm.take().unwrap().cmds {
                    self.send(cmd);
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    /// First-run screen shown when the colima CLI is not installed.
    fn onboarding(&mut self, ui: &mut Ui, st: &State) {
        let setup = st.setup;
        let mut pkgs = vec!["colima"];
        if !setup.docker {
            pkgs.push("docker");
        }
        let cmd = format!("brew install {}", pkgs.join(" "));
        let width = 460.0_f32.min(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.12).max(16.0));
            ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Center), |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(14), tint(ACCENT, 30));
                icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(26.0, 26.0)), Icon::Box, ACCENT);
                ui.add_space(14.0);
                ui.label(bold("Set up Colima", 18.0));
                ui.add_space(4.0);
                ui.label(muted("Colima runs Docker containers in a lightweight VM. Install it once and this app takes care of the rest."));
                ui.add_space(18.0);

                // Checklist
                Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, DIVIDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let items = [
                            (setup.brew, "Homebrew", "Package manager used to install the tools"),
                            (setup.colima, "Colima", "Container runtime (lightweight VM)"),
                            (setup.docker, "Docker CLI", "Used by the terminal button and your scripts"),
                        ];
                        for (i, (ok, name, desc)) in items.into_iter().enumerate() {
                            if i > 0 {
                                let y = ui.cursor().top();
                                ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, BORDER));
                            }
                            setup_item(ui, ok, name, desc);
                        }
                    });
                ui.add_space(14.0);

                // Command to run
                if !setup.brew {
                    ui.label(RichText::new("Install Homebrew first, then run:").size(12.0).color(MUTED));
                    ui.add_space(4.0);
                }
                Frame::new()
                    .fill(CODE_BG)
                    .stroke(Stroke::new(1.0, DIVIDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin { left: 12, right: 4, top: 2, bottom: 2 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("$").monospace().color(FAINT));
                            ui.label(RichText::new(&cmd).monospace().color(TEXT));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if icon_btn(ui, Icon::Copy, "Copy command").clicked() {
                                    ui.ctx().copy_text(cmd.clone());
                                    self.state.lock().unwrap().notice = Some("Command copied".into());
                                }
                            });
                        });
                    });
                ui.add_space(18.0);

                // Actions
                ui.horizontal(|ui| {
                    let primary_w = if setup.brew { 172.0 } else { 160.0 };
                    ui.add_space(((ui.available_width() - primary_w - 120.0) / 2.0).max(0.0));
                    if setup.brew {
                        if btn(ui, Some(Icon::Download), "Install in Terminal", BtnKind::Primary).clicked() {
                            let brew = backend::find_bin("brew");
                            run_in_terminal(&format!("{brew} install {} && echo && echo 'Done — you can close this window.'", pkgs.join(" ")));
                        }
                    } else if btn(ui, Some(Icon::External), "Get Homebrew", BtnKind::Primary).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab("https://brew.sh"));
                    }
                    if btn(ui, Some(Icon::Restart), "Check again", BtnKind::Normal).clicked() {
                        self.send(Command::Refresh);
                    }
                });
                ui.add_space(14.0);
                ui.label(RichText::new("This screen moves on by itself once the install finishes.").size(12.0).color(FAINT));
                let link = ui
                    .add(egui::Label::new(RichText::new("Other ways to install Colima").size(12.0).color(ACCENT)).sense(Sense::click()))
                    .on_hover_cursor(CursorIcon::PointingHand);
                if link.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/abiosoft/colima#installation"));
                }
            });
        });
    }

    fn disconnected(&mut self, ui: &mut Ui, st: &State) {
        if st.setup.missing_colima() && st.colima_busy.is_none() {
            return self.onboarding(ui, st);
        }
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.3).max(20.0));
            match (&st.colima_busy, &st.colima) {
                (Some(b), _) => {
                    ui.spinner();
                    ui.label(muted(b.clone()));
                }
                (None, None) => {
                    ui.spinner();
                    ui.label(muted("Connecting to Colima…"));
                }
                (None, Some(c)) if c.running => {
                    ui.label(bold("Docker is not reachable", 15.0));
                    ui.label(muted("Colima is running but its Docker socket does not answer yet."));
                }
                _ => {
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(14), SURFACE);
                    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(26.0, 26.0)), Icon::Power, FAINT);
                    ui.add_space(12.0);
                    ui.label(bold("Colima is stopped", 15.0));
                    ui.label(muted("Start it to see your containers."));
                    ui.label(RichText::new("The first start downloads a VM image and can take a few minutes.").size(12.0).color(FAINT));
                    ui.add_space(12.0);
                    if btn(ui, Some(Icon::Power), "Start Colima", BtnKind::Primary).clicked() {
                        self.send(Command::StartColima);
                    }
                }
            }
        });
    }
}

// ───────────────────────────── containers ─────────────────────────────

impl App {
    fn containers_page(&mut self, ui: &mut Ui, st: &State) {
        // Compose projects first (alphabetical), standalone containers last.
        let mut groups: BTreeMap<(bool, String), Vec<&ContainerSummary>> = BTreeMap::new();
        for c in &st.containers {
            let project = compose_project(c);
            let text = format!("{} {} {project}", container_name(c), c.image.as_deref().unwrap_or(""));
            if self.matches(&text) {
                groups.entry((project.is_empty(), project)).or_default().push(c);
            }
        }
        if st.containers.is_empty() {
            return empty_state(ui, Icon::Box, "No containers", "Run `docker run …` or `docker compose up` and they show up here.");
        }
        if groups.is_empty() {
            return empty_state(ui, Icon::Search, "No matches", &format!("Nothing matches “{}”.", self.filter));
        }
        // Hide the header only when everything is standalone; a compose project always gets its name shown.
        let single = groups.len() == 1 && groups.keys().all(|(standalone, _)| *standalone);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for ((_, project), mut list) in groups {
                list.sort_by_key(|c| container_name(c));
                if !single {
                    section_label(ui, if project.is_empty() { "Other" } else { &project }, list.len());
                }
                for c in list {
                    self.container_row(ui, c, st);
                }
            }
        });
    }

    fn container_row(&mut self, ui: &mut Ui, c: &ContainerSummary, st: &State) {
        let id = c.id.clone().unwrap_or_default();
        let state = container_state(c);
        let running = state == "running";
        let busy = st.busy.contains(&id);
        let (rect, resp) = list_row(ui, 56.0);
        let p = ui.painter().clone();

        // [icon] [name / image]  ......  [ports]  [cpu] [mem]  [actions]
        let act_w = 124.0;
        let col_w = 76.0;
        let ports_w = 150.0;
        let act = Rect::from_min_max(pos2(rect.right() - ROW_PAD - act_w, rect.top()), pos2(rect.right() - ROW_PAD, rect.bottom()));
        let mem_r = Rect::from_min_max(pos2(act.left() - 16.0 - col_w, rect.top()), pos2(act.left() - 16.0, rect.bottom()));
        let cpu_r = mem_r.translate(vec2(-col_w - 8.0, 0.0));
        let ports_r = Rect::from_min_max(pos2(cpu_r.left() - 16.0 - ports_w, rect.top()), pos2(cpu_r.left() - 16.0, rect.bottom()));
        let text_r = Rect::from_min_max(pos2(rect.left() + ROW_PAD + 42.0, rect.top()), pos2(ports_r.left() - 16.0, rect.bottom()));

        let color = state_color(&state);
        paint_badge_icon(&p, pos2(rect.left() + ROW_PAD + 15.0, rect.center().y), Icon::Box, if color == FAINT { MUTED } else { color });
        paint_two_line(&p, text_r, &container_name(c), TEXT, &image_label(c));
        cell_ui(ui, ports_r, false, ("ports", &id), |ui| port_links(ui, c));

        match st.stats.get(&id).filter(|_| running) {
            Some(s) => {
                paint_metric(&p, cpu_r, &format!("{:.1}%", s.cpu), "CPU");
                paint_metric(&p, mem_r, &fmt_bytes(s.mem), "MEM");
            }
            None => {
                let r = Rect::from_min_max(cpu_r.min, mem_r.max);
                let text = short_status(c);
                let g = p.layout_no_wrap(text.clone(), FontId::proportional(12.0), FAINT);
                let w = g.size().x.min(r.width());
                paint_cell(&p, Rect::from_min_max(pos2(r.right() - w, r.top()), r.max), &text, FontId::proportional(12.0), FAINT);
            }
        }

        cell_ui(ui, act, true, ("act", &id), |ui| self.container_actions(ui, c, &id, running, busy));

        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            self.open_detail(Some(id));
        }
    }

    /// Start/stop, restart, terminal, delete — always visible, right-to-left.
    fn container_actions(&mut self, ui: &mut Ui, c: &ContainerSummary, id: &str, running: bool, busy: bool) {
        if busy {
            ui.add_space(6.0);
            ui.spinner();
            return;
        }
        if icon_btn_ex(ui, Icon::Trash, "Delete", true).clicked() {
            self.ask(
                format!("Delete {}?", container_name(c)),
                "The container is removed. Its named volumes are kept.",
                "Delete",
                vec![Command::Container(id.to_string(), ContainerAction::Remove)],
            );
        }
        if running {
            if icon_btn(ui, Icon::Terminal, "Open terminal").clicked() {
                self.shell(id);
            }
            if icon_btn(ui, Icon::Restart, "Restart").clicked() {
                self.send(Command::Container(id.to_string(), ContainerAction::Restart));
            }
            if icon_btn(ui, Icon::Stop, "Stop").clicked() {
                self.send(Command::Container(id.to_string(), ContainerAction::Stop));
            }
        } else if icon_btn(ui, Icon::Play, "Start").clicked() {
            self.send(Command::Container(id.to_string(), ContainerAction::Start));
        }
    }

    fn detail_page(&mut self, ui: &mut Ui, c: &ContainerSummary, st: &State) {
        let id = c.id.clone().unwrap_or_default();
        let name = container_name(c);
        let state = container_state(c);
        let running = state == "running";
        let busy = st.busy.contains(&id);
        let color = state_color(&state);

        // Back + title + actions
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if icon_btn(ui, Icon::Back, "Back (Esc)").clicked() {
                self.open_detail(None);
            }
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 40.0), Sense::hover());
            paint_badge_icon(ui.painter(), r.center(), Icon::Box, if color == FAINT { MUTED } else { color });
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(bold(&name, 16.0));
                ui.label(RichText::new(format!("{}  ·  {}", image_label(c), c.status.clone().unwrap_or_default())).size(12.0).color(FAINT));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if busy {
                    ui.spinner();
                    return;
                }
                if icon_btn_ex(ui, Icon::Trash, "Delete", true).clicked() {
                    self.ask(
                        format!("Delete {name}?"),
                        "The container is removed. Its named volumes are kept.",
                        "Delete",
                        vec![Command::Container(id.clone(), ContainerAction::Remove)],
                    );
                }
                if running {
                    if icon_btn(ui, Icon::Terminal, "Open terminal").clicked() {
                        self.shell(&id);
                    }
                    if icon_btn(ui, Icon::Restart, "Restart").clicked() {
                        self.send(Command::Container(id.clone(), ContainerAction::Restart));
                    }
                    ui.add_space(4.0);
                    if btn(ui, Some(Icon::Stop), "Stop", BtnKind::Normal).clicked() {
                        self.send(Command::Container(id.clone(), ContainerAction::Stop));
                    }
                } else if btn(ui, Some(Icon::Play), "Start", BtnKind::Primary).clicked() {
                    self.send(Command::Container(id.clone(), ContainerAction::Start));
                }
            });
        });
        ui.add_space(12.0);

        // Ports + live stats
        let stats = st.stats.get(&id).filter(|_| running);
        if stats.is_some() || !ports(c).is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(s) = stats {
                    stat_card(ui, "CPU", &format!("{:.1}%", s.cpu), &s.cpu_hist, GREEN, 5.0);
                    let mem = if s.mem_limit > 0 { format!("{} / {}", fmt_bytes(s.mem), fmt_bytes(s.mem_limit)) } else { fmt_bytes(s.mem) };
                    stat_card(ui, "Memory", &mem, &s.mem_hist, ACCENT, 16.0);
                }
                if !ports(c).is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Ports").size(12.0).color(FAINT));
                    port_links(ui, c);
                }
            });
            ui.add_space(12.0);
        }

        // Logs toolbar
        ui.horizontal(|ui| {
            ui.label(bold("Logs", 13.0));
            ui.label(RichText::new(st.logs.len().to_string()).size(11.5).color(FAINT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if icon_btn(ui, Icon::Copy, "Copy logs").clicked() {
                    let text: Vec<&str> = st.logs.iter().filter(|l| self.log_line_visible(l)).map(String::as_str).collect();
                    ui.ctx().copy_text(text.join("\n"));
                    self.state.lock().unwrap().notice = Some("Logs copied".into());
                }
                if toggle(ui, "Follow", self.follow).on_hover_text("Auto-scroll to new lines").clicked() {
                    self.follow = !self.follow;
                }
                search_field(ui, &mut self.log_filter, Id::unique("log-filter"), "Filter logs", 180.0);
            });
        });
        ui.add_space(6.0);
        self.logs_view(ui, st);
    }

    fn log_line_visible(&self, line: &str) -> bool {
        self.log_filter.is_empty() || line.to_lowercase().contains(&self.log_filter.to_lowercase())
    }

    fn logs_view(&mut self, ui: &mut Ui, st: &State) {
        Frame::new()
            .fill(CODE_BG)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let lines: Vec<&String> = st.logs.iter().filter(|l| self.log_line_visible(l)).collect();
                if lines.is_empty() {
                    ui.label(RichText::new(if st.logs.is_empty() { "No log output yet." } else { "No lines match the filter." }).color(FAINT));
                    return;
                }
                let font = FontId::monospace(12.0);
                let mut job = LayoutJob::default();
                for l in lines {
                    let lower = l.to_lowercase();
                    let color = if lower.contains("error") || lower.contains("fatal") || lower.contains("panic") {
                        RED
                    } else if lower.contains("warn") {
                        ORANGE
                    } else {
                        Color32::from_rgb(0xc9, 0xcd, 0xd4)
                    };
                    job.append(l, 0.0, egui::TextFormat::simple(font.clone(), color));
                    job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), color));
                }
                egui::ScrollArea::both().auto_shrink(false).stick_to_bottom(self.follow).show(ui, |ui| {
                    ui.add(egui::Label::new(job).selectable(true).extend());
                });
            });
    }
}

/// Right-aligned value with a small caption under it (a table-like numeric column).
fn paint_metric(p: &Painter, rect: Rect, value: &str, caption: &str) {
    let v = p.layout_no_wrap(value.to_string(), FontId::proportional(13.0), TEXT);
    let c = p.layout_no_wrap(caption.to_string(), FontId::new(9.5, bold_family()), FAINT);
    let total = v.size().y + c.size().y;
    let top = rect.center().y - total / 2.0;
    p.galley(pos2(rect.right() - v.size().x, top), v.clone(), TEXT);
    p.galley(pos2(rect.right() - c.size().x, top + v.size().y), c, FAINT);
}

/// Small card: caption, value and a sparkline of recent history.
fn stat_card(ui: &mut Ui, caption: &str, value: &str, hist: &VecDeque<f64>, color: Color32, floor: f64) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, DIVIDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(caption.to_uppercase()).family(bold_family()).size(9.5).color(FAINT));
                    ui.label(RichText::new(value).size(14.0).color(TEXT));
                });
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(vec2(110.0, 32.0), Sense::hover());
                sparkline(ui.painter(), r, Some(hist), color, floor);
            });
        });
}

/// Pill-shaped on/off toggle button.
fn toggle(ui: &mut Ui, label: &str, on: bool) -> Response {
    let fg = if on { ACCENT } else { MUTED };
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), fg);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 30.0, 28.0), Sense::click());
    let p = ui.painter();
    let fill = if on { tint(ACCENT, 34) } else if resp.hovered() { SURFACE_HI } else { Color32::TRANSPARENT };
    p.rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, if on { tint(ACCENT, 90) } else { BORDER }), egui::StrokeKind::Inside);
    p.circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.0, if on { ACCENT } else { FAINT });
    p.galley(pos2(rect.left() + 20.0, rect.center().y - g.size().y / 2.0), g, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ───────────────────────────── images & volumes ─────────────────────────────

impl App {
    /// Shared simple row: icon, two-line text, optional badge and an always-visible delete button.
    #[allow(clippy::too_many_arguments)]
    fn simple_row(&mut self, ui: &mut Ui, key: &str, icon: Icon, title: &str, sub: &str, badge: Option<(&str, Color32)>, delete: impl FnOnce(&mut Self)) {
        let (rect, resp) = list_row(ui, 52.0);
        let p = ui.painter().clone();
        let right = Rect::from_min_max(pos2(rect.right() - ROW_PAD - 110.0, rect.top()), pos2(rect.right() - ROW_PAD, rect.bottom()));
        let text_r = Rect::from_min_max(pos2(rect.left() + ROW_PAD + 42.0, rect.top()), pos2(right.left() - 16.0, rect.bottom()));
        paint_badge_icon(&p, pos2(rect.left() + ROW_PAD + 15.0, rect.center().y), icon, if badge.is_some() { GREEN } else { MUTED });
        paint_two_line(&p, text_r, title, TEXT, sub);
        resp.on_hover_text(title);
        let busy = self.state.lock().unwrap().busy.contains(key);
        cell_ui(ui, right, true, ("del", key), |ui| {
            if busy {
                ui.spinner();
            } else if icon_btn_ex(ui, Icon::Trash, "Delete", true).clicked() {
                delete(self);
            }
            if let Some((text, color)) = badge {
                ui.add_space(8.0);
                pill(ui, text, color);
            }
        });
    }

    fn images_page(&mut self, ui: &mut Ui, st: &State) {
        let mut imgs: Vec<_> = st.images.iter().filter(|i| self.matches(&i.repo_tags.join(" "))).collect();
        imgs.sort_by_key(|i| std::cmp::Reverse(i.created));
        if st.images.is_empty() {
            return empty_state(ui, Icon::Layers, "No images", "Pulled and built images show up here.");
        }
        if imgs.is_empty() {
            return empty_state(ui, Icon::Search, "No matches", &format!("Nothing matches “{}”.", self.filter));
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for i in imgs {
                let title = match i.repo_tags.first() {
                    Some(t) if t != "<none>:<none>" => {
                        let (repo, tag) = split_tag(t);
                        format!("{repo}:{tag}")
                    }
                    _ => format!("<none>  {}", short_id(&i.id)),
                };
                let sub = format!("{}  ·  {}", fmt_bytes(i.size.max(0) as u64), fmt_age(i.created));
                let badge = st.images_in_use.contains(&i.id).then_some(("In use", GREEN));
                let (id, t2) = (i.id.clone(), title.clone());
                self.simple_row(ui, &i.id, Icon::Layers, &title, &sub, badge, move |app| {
                    app.ask(format!("Delete {t2}?"), "Containers using this image must be removed first.", "Delete", vec![Command::RemoveImage(id)]);
                });
            }
        });
    }

    fn volumes_page(&mut self, ui: &mut Ui, st: &State) {
        let mut vols: Vec<_> = st
            .volumes
            .iter()
            .filter(|v| self.matches(&format!("{} {}", v.name, v.labels.get("com.docker.compose.project").cloned().unwrap_or_default())))
            .collect();
        vols.sort_by(|a, b| a.name.cmp(&b.name));
        if st.volumes.is_empty() {
            return empty_state(ui, Icon::Database, "No volumes", "Named volumes created by containers show up here.");
        }
        if vols.is_empty() {
            return empty_state(ui, Icon::Search, "No matches", &format!("Nothing matches “{}”.", self.filter));
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for v in vols {
                let mut sub = vec![v.driver.clone()];
                if let Some(p) = v.labels.get("com.docker.compose.project") {
                    sub.insert(0, p.clone());
                }
                if let Some(d) = v.created_at.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
                    sub.push(fmt_age(d.timestamp()));
                }
                let badge = st.volumes_in_use.contains(&v.name).then_some(("In use", GREEN));
                let name = v.name.clone();
                self.simple_row(ui, &v.name, Icon::Database, &v.name, &sub.join("  ·  "), badge, move |app| {
                    app.ask(
                        format!("Delete volume {name}?"),
                        "All data stored in this volume is permanently lost.",
                        "Delete",
                        vec![Command::RemoveVolume(name)],
                    );
                });
            }
        });
    }
}

fn main() -> eframe::Result {
    let icon = egui::IconData { rgba: icon::app_rgba(256), width: 256, height: 256 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Colima")
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([680.0, 420.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native("Colima", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
