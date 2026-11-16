//! Vector line icons painted with egui shapes, so every icon has the same weight and
//! alignment regardless of which fonts are installed (unicode glyphs rendered unevenly).

use eframe::egui::{self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Stop,
    Restart,
    Trash,
    Terminal,
    Back,
    Copy,
    Search,
    Close,
    Sweep,
    Power,
    Box,
    Layers,
    Database,
    External,
    Check,
    Download,
}

/// Points on a circular arc (angles in radians, 0 = +x, clockwise on screen since y points down).
fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = 20;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// Paint `icon` inside `rect` (drawn on a 16×16 grid, scaled to fit).
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let o = rect.center() - vec2(8.0 * s, 8.0 * s);
    let at = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let st = Stroke::new(1.4 * s.max(0.85), color);
    let line = |pts: Vec<Pos2>| p.add(Shape::line(pts, st));
    let seg = |a: Pos2, b: Pos2| p.line_segment([a, b], st);
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        p.rect_stroke(Rect::from_min_max(at(x0, y0), at(x1, y1)), CornerRadius::same((r * s) as u8), st, StrokeKind::Middle)
    };

    match icon {
        Icon::Play => {
            p.add(Shape::convex_polygon(vec![at(4.5, 2.8), at(13.0, 8.0), at(4.5, 13.2)], color, Stroke::new(1.0 * s, color)));
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_min_max(at(3.0, 3.0), at(13.0, 13.0)), CornerRadius::same((2.0 * s) as u8), color);
        }
        Icon::Restart => {
            use std::f32::consts::PI;
            line(arc(at(8.0, 8.0), 5.2 * s, -PI * 0.35, PI * 1.45));
            // arrow head at the arc's start (top-right)
            line(vec![at(9.6, 1.6), at(12.1, 3.6), at(9.4, 5.3)]);
        }
        Icon::Trash => {
            seg(at(2.5, 4.0), at(13.5, 4.0));
            line(vec![at(6.0, 4.0), at(6.0, 2.2), at(10.0, 2.2), at(10.0, 4.0)]);
            line(vec![at(3.8, 4.0), at(4.6, 13.8), at(11.4, 13.8), at(12.2, 4.0)]);
            seg(at(6.6, 6.8), at(6.8, 11.2));
            seg(at(9.4, 6.8), at(9.2, 11.2));
        }
        Icon::Terminal => {
            rrect(1.8, 2.8, 14.2, 13.2, 2.0);
            line(vec![at(4.6, 6.0), at(6.8, 8.0), at(4.6, 10.0)]);
            seg(at(8.4, 10.2), at(11.4, 10.2));
        }
        Icon::Back => {
            line(vec![at(10.0, 3.0), at(5.0, 8.0), at(10.0, 13.0)]);
        }
        Icon::Copy => {
            rrect(5.2, 5.2, 13.8, 13.8, 1.8);
            line(vec![at(10.8, 2.2), at(3.8, 2.2), at(2.2, 3.8), at(2.2, 10.8)]);
        }
        Icon::Search => {
            p.circle_stroke(at(7.0, 7.0), 4.6 * s, st);
            seg(at(10.4, 10.4), at(14.0, 14.0));
        }
        Icon::Close => {
            seg(at(4.0, 4.0), at(12.0, 12.0));
            seg(at(12.0, 4.0), at(4.0, 12.0));
        }
        Icon::Sweep => {
            // Sparkles: "clean up"
            let star = |cx: f32, cy: f32, r: f32| {
                let k = r * 0.28;
                p.add(Shape::convex_polygon(
                    vec![
                        at(cx, cy - r),
                        at(cx + k, cy - k),
                        at(cx + r, cy),
                        at(cx + k, cy + k),
                        at(cx, cy + r),
                        at(cx - k, cy + k),
                        at(cx - r, cy),
                        at(cx - k, cy - k),
                    ],
                    color,
                    Stroke::NONE,
                ));
            };
            star(6.5, 9.0, 5.2);
            star(12.3, 3.8, 2.6);
        }
        Icon::Power => {
            use std::f32::consts::PI;
            line(arc(at(8.0, 8.7), 5.4 * s, -PI * 0.28, PI * 1.28));
            seg(at(8.0, 1.8), at(8.0, 7.6));
        }
        Icon::Box => {
            line(vec![at(8.0, 1.8), at(14.0, 4.8), at(14.0, 11.2), at(8.0, 14.2), at(2.0, 11.2), at(2.0, 4.8), at(8.0, 1.8)]);
            line(vec![at(2.0, 4.8), at(8.0, 7.8), at(14.0, 4.8)]);
            seg(at(8.0, 7.8), at(8.0, 14.2));
        }
        Icon::Layers => {
            line(vec![at(8.0, 2.0), at(14.2, 5.2), at(8.0, 8.4), at(1.8, 5.2), at(8.0, 2.0)]);
            line(vec![at(1.8, 8.2), at(8.0, 11.4), at(14.2, 8.2)]);
            line(vec![at(1.8, 11.2), at(8.0, 14.4), at(14.2, 11.2)]);
        }
        Icon::Database => {
            let e = |y: f32, full: bool| {
                let c = at(8.0, y);
                let n = 24;
                let (a0, a1) = if full { (0.0, std::f32::consts::TAU) } else { (0.0, std::f32::consts::PI) };
                let pts = (0..=n)
                    .map(|i| {
                        let a = a0 + (a1 - a0) * i as f32 / n as f32;
                        pos2(c.x + 5.6 * s * a.cos(), c.y + 2.2 * s * a.sin())
                    })
                    .collect();
                line(pts);
            };
            e(3.8, true);
            e(8.0, false);
            e(12.0, false);
            seg(at(2.4, 3.8), at(2.4, 12.0));
            seg(at(13.6, 3.8), at(13.6, 12.0));
        }
        Icon::Check => {
            line(vec![at(3.2, 8.4), at(6.6, 11.6), at(12.8, 4.6)]);
        }
        Icon::Download => {
            seg(at(8.0, 2.0), at(8.0, 10.4));
            line(vec![at(4.6, 7.2), at(8.0, 10.6), at(11.4, 7.2)]);
            line(vec![at(2.5, 11.0), at(2.5, 13.6), at(13.5, 13.6), at(13.5, 11.0)]);
        }
        Icon::External => {
            line(vec![at(7.0, 3.0), at(3.0, 3.0), at(3.0, 13.0), at(13.0, 13.0), at(13.0, 9.0)]);
            seg(at(8.0, 8.0), at(13.5, 2.5));
            line(vec![at(9.5, 2.5), at(13.5, 2.5), at(13.5, 6.5)]);
        }
    }
}

/// Allocate a square and paint an icon in it (non-interactive).
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), r, icon, color);
    resp
}
