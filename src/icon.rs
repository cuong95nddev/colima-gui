//! Procedurally rendered app / menu-bar icon (an isometric shipping container), so we ship
//! no image assets. Style: black macOS squircle, white/silver faceted glyph, one warm accent.

type Rgba = [f32; 4];
type P = (f32, f32);

// ── geometry helpers ──

fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn sd_segment(p: P, a: P, b: P) -> f32 {
    let (pax, pay) = (p.0 - a.0, p.1 - a.1);
    let (bax, bay) = (b.0 - a.0, b.1 - a.1);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    ((pax - bax * h).powi(2) + (pay - bay * h).powi(2)).sqrt()
}

fn in_poly(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn lerp(a: P, b: P, t: f32) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Shrink a polygon towards its centroid (leaves a dark seam between faces).
fn inset(poly: &[P], k: f32) -> Vec<P> {
    let n = poly.len() as f32;
    let c = poly.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
    poly.iter().map(|p| lerp(*p, c, k)).collect()
}

fn over(dst: Rgba, src: Rgba) -> Rgba {
    let a = src[3] + dst[3] * (1.0 - src[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mut o = [0.0; 4];
    for i in 0..3 {
        o[i] = (src[i] * src[3] + dst[i] * dst[3] * (1.0 - src[3])) / a;
    }
    o[3] = a;
    o
}

fn hex(c: u32, a: f32) -> Rgba {
    [((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32, a]
}

// ── the glyph: isometric container ──

struct Cube {
    top: [P; 4],
    left: [P; 4],
    right: [P; 4],
    /// Corrugation ribs on the two side faces.
    ribs: Vec<(P, P)>,
}

fn cube(cx: f32, cy: f32, s: f32) -> Cube {
    let k = 0.866 * s;
    let t = (cx, cy - s);
    let ul = (cx - k, cy - s * 0.5);
    let ur = (cx + k, cy - s * 0.5);
    let c = (cx, cy);
    let ll = (cx - k, cy + s * 0.5);
    let lr = (cx + k, cy + s * 0.5);
    let b = (cx, cy + s);
    let mut ribs = Vec::new();
    for f in [0.25, 0.5, 0.75] {
        // left face spans UL→C horizontally, top edge UL–C, bottom edge LL–B
        let (a0, a1) = (lerp(ul, c, f), lerp(ll, b, f));
        ribs.push((lerp(a0, a1, 0.16), lerp(a0, a1, 0.84)));
        let (b0, b1) = (lerp(c, ur, f), lerp(b, lr, f));
        ribs.push((lerp(b0, b1, 0.16), lerp(b0, b1, 0.84)));
    }
    Cube { top: [t, ur, c, ul], left: [ul, c, b, ll], right: [c, ur, lr, b], ribs }
}

const ACCENT: u32 = 0xffc531;

/// Colour of the full-colour icon at unit coords (u, v); `px` = one pixel in unit coords.
fn app_sample(u: f32, v: f32, px: f32) -> Rgba {
    let mut col: Rgba = [0.0; 4];
    // Squircle body on the macOS 1024 grid (824 pt body, 100 pt margin).
    let d = sd_round_rect(u, v, 0.5, 0.5, 0.4023, 0.4023, 0.18);
    // soft drop shadow
    let sh = sd_round_rect(u, v - 0.012, 0.5, 0.5, 0.4023, 0.4023, 0.18);
    col = over(col, hex(0x000000, (0.35 * (1.0 - (sh / 0.03).clamp(0.0, 1.0))).max(0.0)));
    if d > px {
        return col;
    }
    let cover = (0.5 - d / px).clamp(0.0, 1.0);
    // background: near-black with a faint top highlight
    let g = ((v - 0.1) / 0.8).clamp(0.0, 1.0);
    let bg = [24.0 - 14.0 * g, 24.0 - 14.0 * g, 27.0 - 15.0 * g, 1.0];
    let mut inner = bg;
    // hairline border
    if d > -0.006 {
        inner = over(inner, hex(0x3a3a3e, 1.0));
    }

    let c = cube(0.555, 0.515, 0.215);
    let p = (u, v);
    // speed lines (fade to the left)
    for (y, x0, x1) in [(0.425, 0.205, 0.345), (0.515, 0.155, 0.335), (0.605, 0.225, 0.345)] {
        let dist = sd_segment(p, (x0, y), (x1, y)) - 0.014;
        if dist < px {
            let a = (0.5 - dist / px).clamp(0.0, 1.0) * ((u - x0) / (x1 - x0)).clamp(0.0, 1.0).powf(1.4) * 0.9;
            inner = over(inner, hex(0xd6d9de, a));
        }
    }
    let seam = 0.035;
    if in_poly(p, &inset(&c.top, seam)) {
        inner = over(inner, hex(ACCENT, 1.0));
    } else if in_poly(p, &inset(&c.left, seam)) {
        let t = ((v - c.left[0].1) / (c.left[3].1 - c.left[0].1 + 0.2)).clamp(0.0, 1.0);
        inner = over(inner, [250.0 - 40.0 * t, 250.0 - 38.0 * t, 252.0 - 36.0 * t, 1.0]);
    } else if in_poly(p, &inset(&c.right, seam)) {
        let t = ((v - c.right[0].1) / 0.4).clamp(0.0, 1.0);
        inner = over(inner, [168.0 - 40.0 * t, 172.0 - 40.0 * t, 180.0 - 40.0 * t, 1.0]);
    }
    for (a, b) in &c.ribs {
        let dist = sd_segment(p, *a, *b) - 0.0085;
        if dist < px {
            let a = (0.5 - dist / px).clamp(0.0, 1.0) * 0.55;
            inner = over(inner, hex(0x2a2c31, a));
        }
    }
    inner[3] *= cover;
    over(col, inner)
}

/// Full-colour app icon, `size`×`size` RGBA (4×4 supersampled).
pub fn app_rgba(size: u32) -> Vec<u8> {
    render(size, app_sample)
}

/// Monochrome template icon for the macOS menu bar (only alpha matters): cube outline + ribs.
pub fn tray_rgba(size: u32) -> Vec<u8> {
    render(size, |u, v, px| {
        let c = cube(0.5, 0.5, 0.40);
        let p = (u, v);
        let edges = [
            (c.top[0], c.top[1]),
            (c.top[1], c.right[2]),
            (c.right[2], c.left[2]),
            (c.left[2], c.left[3]),
            (c.left[3], c.top[3]),
            (c.top[3], c.top[0]),
            (c.top[3], c.top[2]),
            (c.top[2], c.top[1]),
            (c.top[2], c.left[2]),
        ];
        let mut d = f32::MAX;
        for (a, b) in edges {
            d = d.min(sd_segment(p, a, b) - 0.045);
        }
        for (i, (a, b)) in c.ribs.iter().enumerate() {
            if (2..4).contains(&i) {
                d = d.min(sd_segment(p, *a, *b) - 0.03);
            }
        }
        [0.0, 0.0, 0.0, (0.5 - d / px).clamp(0.0, 1.0)]
    })
}

fn render(size: u32, f: impl Fn(f32, f32, f32) -> Rgba) -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let px = 1.0 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) * px;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) * px;
                    let c = f(u, v, px);
                    for i in 0..3 {
                        acc[i] += c[i] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let rgb = |i: usize| if acc[3] > 0.0 { (acc[i] / acc[3]).clamp(0.0, 255.0) as u8 } else { 0 };
            out.extend_from_slice(&[rgb(0), rgb(1), rgb(2), (a * 255.0).round() as u8]);
        }
    }
    out
}
