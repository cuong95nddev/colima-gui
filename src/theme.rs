//! Visual design tokens + egui style setup.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Shadow, Stroke, TextStyle,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const BG: Color32 = rgb(0x17, 0x18, 0x1b);
pub const BAR: Color32 = rgb(0x1c, 0x1d, 0x21);
pub const SURFACE: Color32 = rgb(0x20, 0x22, 0x26);
pub const SURFACE_HI: Color32 = rgb(0x27, 0x2a, 0x2f);
pub const HOVER: Color32 = rgb(0x21, 0x23, 0x28);
pub const BORDER: Color32 = rgb(0x2b, 0x2e, 0x34);
pub const DIVIDER: Color32 = rgb(0x22, 0x24, 0x29);
pub const CODE_BG: Color32 = rgb(0x0f, 0x10, 0x12);
pub const TEXT: Color32 = rgb(0xe9, 0xea, 0xed);
pub const MUTED: Color32 = rgb(0x93, 0x98, 0xa1);
pub const FAINT: Color32 = rgb(0x62, 0x67, 0x70);
pub const ACCENT: Color32 = rgb(0x3d, 0x8b, 0xff);
pub const GREEN: Color32 = rgb(0x32, 0xd1, 0x5d);
pub const ORANGE: Color32 = rgb(0xff, 0xa8, 0x2e);
pub const RED: Color32 = rgb(0xff, 0x5f, 0x57);

pub const BOLD: &str = "bold";

pub fn bold_family() -> FontFamily {
    FontFamily::Name(BOLD.into())
}

pub fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text.into()).family(bold_family()).size(size).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(MUTED)
}

pub fn tint(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

fn load(path: &str, index: u32) -> Option<FontData> {
    let bytes = std::fs::read(path).ok()?;
    let mut f = FontData::from_owned(bytes);
    f.index = index;
    Some(f)
}

fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let add_front = |defs: &mut FontDefinitions, family: FontFamily, name: &str, data: Option<FontData>| {
        if let Some(d) = data {
            defs.font_data.insert(name.into(), d.into());
            defs.families.entry(family).or_default().insert(0, name.into());
        }
    };
    // macOS system UI fonts; egui's bundled fonts stay as fallbacks (symbols/emoji).
    add_front(&mut defs, FontFamily::Proportional, "sf", load("/System/Library/Fonts/SFNS.ttf", 0));
    add_front(&mut defs, FontFamily::Monospace, "sf-mono", load("/System/Library/Fonts/SFNSMono.ttf", 0));
    // Bold family: Helvetica Neue Bold (index 1 of the collection), then the regular stack.
    let fallback = defs.families[&FontFamily::Proportional].clone();
    defs.families.insert(bold_family(), fallback);
    add_front(&mut defs, bold_family(), "hn-bold", load("/System/Library/Fonts/HelveticaNeue.ttc", 1));
    defs
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, bold_family())),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        let sp = &mut s.spacing;
        sp.item_spacing = egui::vec2(8.0, 6.0);
        sp.button_padding = egui::vec2(12.0, 5.0);
        sp.interact_size.y = 28.0;
        sp.menu_margin = Margin::same(8);

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(12);
        v.window_shadow = Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(140) };
        v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) };
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = CODE_BG;
        v.faint_bg_color = HOVER;
        v.code_bg_color = CODE_BG;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = tint(ACCENT, 90);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.error_fg_color = RED;
        v.warn_fg_color = ORANGE;

        let r = CornerRadius::same(7);
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        w.inactive.weak_bg_fill = SURFACE_HI;
        w.inactive.bg_fill = SURFACE_HI;
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;
        w.hovered.weak_bg_fill = rgb(0x31, 0x35, 0x3b);
        w.hovered.bg_fill = rgb(0x31, 0x35, 0x3b);
        w.hovered.bg_stroke = Stroke::new(1.0, rgb(0x3d, 0x42, 0x49));
        w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;
        w.active.weak_bg_fill = rgb(0x3a, 0x3e, 0x45);
        w.active.bg_fill = rgb(0x3a, 0x3e, 0x45);
        w.active.bg_stroke = Stroke::new(1.0, rgb(0x46, 0x4b, 0x53));
        w.active.fg_stroke = Stroke::new(1.0, TEXT);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;
        w.open = w.active;
    });
}
