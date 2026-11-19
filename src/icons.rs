//! Toolbar icons drawn as simple vector shapes, so they look the same on every
//! Mac and don't depend on which symbols a font happens to have.

use eframe::egui::{self, epaint::PathShape, Color32, FontFamily, FontId, Pos2, Rect, Response, Sense, Stroke, Vec2};

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Select,
    Strike,
    Underline,
    Highlight,
    Note,
    Image,
    SidebarLeft,
    SidebarRight,
    Folder,
    Prev,
    Next,
    Info,
    Share,
    Gear,
    Moon,
    Sun,
    Copy,
    Trash,
    Close,
    Search,
    Split,
    More,
    Gem,
    Sync,
    External,
    Code,
    Page,
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Paint `icon` centred in `r` with colour `c` (`bg` for knock-out details).
pub fn paint(p: &egui::Painter, r: Rect, icon: Icon, c: Color32, bg: Color32) {
    let ctr = r.center();
    let s = r.height().min(r.width()) / 20.0; // icon unit: 20 units across
    let pt = |x: f32, y: f32| Pos2::new(ctr.x + x * s, ctr.y + y * s);
    let st = Stroke::new(1.5 * s.max(0.8), c);
    match icon {
        Icon::Select => {
            let pts = vec![pt(-4.0, -7.5), pt(-4.0, 6.0), pt(-0.8, 3.0), pt(1.6, 8.0), pt(3.6, 7.0), pt(1.2, 2.2), pt(5.5, 2.0)];
            p.add(PathShape::closed_line(pts, st));
        }
        Icon::Strike => {
            p.text(ctr + Vec2::new(0.0, -0.5 * s), egui::Align2::CENTER_CENTER, "S", FontId::new(15.0 * s, bold()), c);
            p.line_segment([pt(-6.5, 0.0), pt(6.5, 0.0)], st);
        }
        Icon::Underline => {
            p.text(ctr + Vec2::new(0.0, -1.5 * s), egui::Align2::CENTER_CENTER, "U", FontId::new(14.0 * s, bold()), c);
            p.line_segment([pt(-5.5, 7.0), pt(5.5, 7.0)], st);
        }
        Icon::Highlight => {
            let rr = Rect::from_center_size(ctr, Vec2::splat(15.0 * s));
            p.rect_filled(rr, 3.0 * s, c);
            p.text(ctr + Vec2::new(0.0, 0.3 * s), egui::Align2::CENTER_CENTER, "A", FontId::new(12.5 * s, bold()), bg);
        }
        Icon::Note => {
            let rr = Rect::from_min_max(pt(-7.5, -6.5), pt(7.5, 4.5));
            p.rect_stroke(rr, 3.0 * s, st, egui::StrokeKind::Middle);
            p.add(PathShape::line(vec![pt(-3.0, 4.5), pt(-4.5, 8.0), pt(0.5, 4.5)], st));
            p.line_segment([pt(-4.0, -2.5), pt(4.0, -2.5)], Stroke::new(1.2 * s.max(0.8), c));
            p.line_segment([pt(-4.0, 0.8), pt(2.0, 0.8)], Stroke::new(1.2 * s.max(0.8), c));
        }
        Icon::Image => {
            let rr = Rect::from_min_max(pt(-8.0, -6.5), pt(8.0, 6.5));
            p.rect_stroke(rr, 2.5 * s, st, egui::StrokeKind::Middle);
            p.add(PathShape::line(vec![pt(-6.5, 5.0), pt(-2.0, -0.5), pt(1.5, 3.0), pt(3.5, 1.0), pt(6.5, 5.0)], st));
            p.circle_filled(pt(3.5, -3.0), 1.5 * s, c);
        }
        Icon::SidebarLeft | Icon::SidebarRight => {
            let rr = Rect::from_min_max(pt(-8.5, -6.5), pt(8.5, 6.5));
            p.rect_stroke(rr, 2.5 * s, st, egui::StrokeKind::Middle);
            let x = if icon == Icon::SidebarLeft { -3.0 } else { 3.0 };
            p.line_segment([pt(x, -6.5), pt(x, 6.5)], st);
            let (a, b) = if icon == Icon::SidebarLeft { (-6.8, -4.6) } else { (4.6, 6.8) };
            for y in [-3.5, -1.0, 1.5] {
                p.line_segment([pt(a, y), pt(b, y)], Stroke::new(1.1 * s.max(0.8), c));
            }
        }
        Icon::Folder => {
            p.add(PathShape::closed_line(
                vec![pt(-8.0, -5.5), pt(-3.0, -5.5), pt(-1.5, -3.5), pt(8.0, -3.5), pt(8.0, 6.0), pt(-8.0, 6.0)],
                st,
            ));
            p.line_segment([pt(-8.0, -1.0), pt(8.0, -1.0)], st);
        }
        Icon::Prev => {
            p.add(PathShape::line(vec![pt(2.5, -6.0), pt(-3.5, 0.0), pt(2.5, 6.0)], Stroke::new(1.9 * s.max(0.8), c)));
        }
        Icon::Next => {
            p.add(PathShape::line(vec![pt(-2.5, -6.0), pt(3.5, 0.0), pt(-2.5, 6.0)], Stroke::new(1.9 * s.max(0.8), c)));
        }
        Icon::Info => {
            p.circle_stroke(ctr, 8.0 * s, st);
            p.circle_filled(pt(0.0, -3.8), 1.1 * s, c);
            p.line_segment([pt(0.0, -1.0), pt(0.0, 4.5)], Stroke::new(1.8 * s.max(0.8), c));
        }
        Icon::Share => {
            p.add(PathShape::line(vec![pt(-3.0, -2.0), pt(-6.5, -2.0), pt(-6.5, 8.0), pt(6.5, 8.0), pt(6.5, -2.0), pt(3.0, -2.0)], st));
            p.line_segment([pt(0.0, 3.0), pt(0.0, -8.5)], st);
            p.add(PathShape::line(vec![pt(-3.5, -5.0), pt(0.0, -8.5), pt(3.5, -5.0)], st));
        }
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (sn, cs) = a.sin_cos();
                p.line_segment([pt(cs * 5.0, sn * 5.0), pt(cs * 8.3, sn * 8.3)], Stroke::new(2.4 * s.max(0.8), c));
            }
            p.circle_stroke(ctr, 5.6 * s, st);
            p.circle_stroke(ctr, 2.2 * s, st);
        }
        Icon::Moon => {
            p.circle_filled(ctr, 7.5 * s, c);
            p.circle_filled(pt(3.8, -3.2), 6.3 * s, bg);
        }
        Icon::Sun => {
            p.circle_filled(ctr, 3.8 * s, c);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (sn, cs) = a.sin_cos();
                p.line_segment([pt(cs * 6.0, sn * 6.0), pt(cs * 8.5, sn * 8.5)], st);
            }
        }
        Icon::Copy => {
            p.rect_stroke(Rect::from_min_max(pt(-6.5, -4.0), pt(3.5, 8.0)), 2.0 * s, st, egui::StrokeKind::Middle);
            p.add(PathShape::line(vec![pt(-3.0, -6.0), pt(-3.0, -8.0), pt(6.5, -8.0), pt(6.5, 4.5), pt(5.0, 4.5)], st));
        }
        Icon::Trash => {
            p.line_segment([pt(-7.0, -5.0), pt(7.0, -5.0)], st);
            p.add(PathShape::line(vec![pt(-2.5, -5.0), pt(-2.5, -7.5), pt(2.5, -7.5), pt(2.5, -5.0)], st));
            p.add(PathShape::line(vec![pt(-5.5, -5.0), pt(-4.5, 7.5), pt(4.5, 7.5), pt(5.5, -5.0)], st));
        }
        Icon::Close => {
            p.line_segment([pt(-5.0, -5.0), pt(5.0, 5.0)], st);
            p.line_segment([pt(5.0, -5.0), pt(-5.0, 5.0)], st);
        }
        Icon::Split => {
            let rr = Rect::from_min_max(pt(-8.5, -6.5), pt(8.5, 6.5));
            p.rect_stroke(rr, 2.5 * s, st, egui::StrokeKind::Middle);
            p.line_segment([pt(0.0, -6.5), pt(0.0, 6.5)], st);
        }
        Icon::Gem => {
            let (top, left, right, bottom, mid) = (pt(0.0, -8.0), pt(-5.5, -1.0), pt(5.5, -1.0), pt(0.0, 8.5), pt(0.0, 0.5));
            let f = |pts: Vec<Pos2>, col: Color32| p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            f(vec![top, mid, left], c.gamma_multiply(0.75));
            f(vec![top, right, mid], c);
            f(vec![left, mid, bottom], c.gamma_multiply(0.9));
            f(vec![mid, right, bottom], c.gamma_multiply(0.6));
        }
        Icon::Sync => {
            let arc = |a0: f32, a1: f32| -> Vec<Pos2> {
                (0..=12).map(|k| {
                    let a = a0 + (a1 - a0) * k as f32 / 12.0;
                    pt(a.cos() * 6.5, a.sin() * 6.5)
                }).collect()
            };
            use std::f32::consts::PI;
            p.add(PathShape::line(arc(PI * 1.05, PI * 1.85), st));
            p.add(PathShape::line(arc(PI * 0.05, PI * 0.85), st));
            p.add(PathShape::line(vec![pt(3.0, -7.5), pt(5.3, -4.3), pt(1.7, -3.4)], st));
            p.add(PathShape::line(vec![pt(-3.0, 7.5), pt(-5.3, 4.3), pt(-1.7, 3.4)], st));
        }
        Icon::External => {
            p.add(PathShape::line(vec![pt(-1.0, -6.5), pt(-6.5, -6.5), pt(-6.5, 6.5), pt(6.5, 6.5), pt(6.5, 1.0)], st));
            p.line_segment([pt(-0.5, 0.5), pt(7.0, -7.0)], st);
            p.add(PathShape::line(vec![pt(2.0, -7.0), pt(7.0, -7.0), pt(7.0, -2.0)], st));
        }
        Icon::Code => {
            p.add(PathShape::line(vec![pt(-3.0, -5.0), pt(-8.0, 0.0), pt(-3.0, 5.0)], st));
            p.add(PathShape::line(vec![pt(3.0, -5.0), pt(8.0, 0.0), pt(3.0, 5.0)], st));
            p.line_segment([pt(1.5, -7.0), pt(-1.5, 7.0)], st);
        }
        Icon::Page => {
            p.add(PathShape::closed_line(vec![pt(-5.5, -7.5), pt(2.0, -7.5), pt(5.5, -4.0), pt(5.5, 7.5), pt(-5.5, 7.5)], st));
            p.add(PathShape::line(vec![pt(2.0, -7.5), pt(2.0, -4.0), pt(5.5, -4.0)], st));
        }
        Icon::More => {
            for x in [-5.5, 0.0, 5.5] {
                p.circle_filled(pt(x, 0.0), 1.6 * s, c);
            }
        }
        Icon::Search => {
            p.circle_stroke(pt(-1.5, -1.5), 5.0 * s, st);
            p.line_segment([pt(2.2, 2.2), pt(6.5, 6.5)], Stroke::new(2.0 * s.max(0.8), c));
        }
    }
}

/// A square icon button. `selected` fills it with `accent`.
pub fn button(ui: &mut egui::Ui, icon: Icon, selected: bool, accent: Color32, tip: &str) -> Response {
    button_hint(ui, icon, selected, accent, tip, "")
}

/// Like [`button`], with its keyboard shortcut shown in the tooltip.
pub fn button_hint(ui: &mut egui::Ui, icon: Icon, selected: bool, accent: Color32, tip: &str, key: &str) -> Response {
    let size = Vec2::new(32.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.visuals();
    let enabled = ui.is_enabled();
    // Selected: a soft tint of the accent with the icon in the accent colour.
    let bg = if selected {
        accent.gamma_multiply(if v.dark_mode { 0.28 } else { 0.16 })
    } else if resp.is_pointer_button_down_on() {
        v.widgets.active.weak_bg_fill
    } else if resp.hovered() && enabled {
        v.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let fg = if selected {
        accent
    } else if enabled {
        v.text_color()
    } else {
        v.weak_text_color()
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, bg);
    let knock = if bg == Color32::TRANSPARENT { v.panel_fill } else { v.panel_fill };
    paint(painter, rect.shrink2(Vec2::new(7.0, 5.0)), icon, fg, knock);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
    if key.is_empty() { resp.on_hover_text(tip) } else { resp.on_hover_text(format!("{tip}    단축키 {key}")) }
}

// ------------------------------------------------------------------ logo

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// A convex polygon with its corners rounded by `r` (quadratic curves).
fn rounded_poly(pts: &[Pos2], r: f32) -> Vec<Pos2> {
    let n = pts.len();
    let mut out = Vec::new();
    for i in 0..n {
        let v = pts[i];
        let a = pts[(i + n - 1) % n];
        let b = pts[(i + 1) % n];
        let ra = r.min((a - v).length() / 2.0);
        let rb = r.min((b - v).length() / 2.0);
        let p1 = v + (a - v).normalized() * ra;
        let p2 = v + (b - v).normalized() * rb;
        for k in 0..=10 {
            let t = k as f32 / 10.0;
            let q = p1.to_vec2() * (1.0 - t) * (1.0 - t) + v.to_vec2() * 2.0 * t * (1.0 - t) + p2.to_vec2() * t * t;
            out.push(q.to_pos2());
        }
    }
    out
}

/// Fill a convex outline with a colour that varies by position.
fn fill_shaded(p: &egui::Painter, outline: &[Pos2], color_at: impl Fn(Pos2) -> Color32) {
    let mut mesh = egui::Mesh::default();
    let c = outline.iter().fold(Vec2::ZERO, |acc, q| acc + q.to_vec2()) / outline.len() as f32;
    let c = c.to_pos2();
    mesh.colored_vertex(c, color_at(c));
    for q in outline {
        mesh.colored_vertex(*q, color_at(*q));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(egui::Shape::mesh(mesh));
}

fn rect_pts(r: Rect) -> [Pos2; 4] {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
}

/// The app icon: a violet tile, a page with a highlighted line, and a gem.
pub fn paint_logo(p: &egui::Painter, r: Rect, with_tile: bool) {
    let w = r.width();
    let u = |x: f32, y: f32| Pos2::new(r.left() + x * w, r.top() + y * w);
    if with_tile {
        // Apple's icon grid: the tile is 824/1024 of the canvas.
        let tile = Rect::from_center_size(r.center(), Vec2::splat(w * 0.805));
        let shadow = rounded_poly(&rect_pts(tile.translate(Vec2::new(0.0, w * 0.012))), w * 0.19);
        p.add(egui::Shape::convex_polygon(shadow, Color32::from_black_alpha(46), Stroke::NONE));
        let outline = rounded_poly(&rect_pts(tile), w * 0.185);
        let (c0, c1) = (Color32::from_rgb(160, 132, 255), Color32::from_rgb(82, 52, 214));
        fill_shaded(p, &outline, |q| {
            let t = ((q.x - tile.left()) * 0.35 + (q.y - tile.top())) / (tile.width() * 1.35);
            lerp_color(c0, c1, t)
        });
        // Soft light from the top.
        let top = Rect::from_min_max(tile.left_top(), Pos2::new(tile.right(), tile.top() + tile.height() * 0.5));
        fill_shaded(p, &rounded_poly(&rect_pts(top), w * 0.185), |q| {
            Color32::from_white_alpha(((1.0 - (q.y - top.top()) / top.height()) * 34.0) as u8)
        });
    }
    // The page, with its top-right corner folded.
    let (l, t, rr, b, fold) = (0.30, 0.22, 0.66, 0.74, 0.10);
    let page = [u(l, t), u(rr - fold, t), u(rr, t + fold), u(rr, b), u(l, b)];
    let shadow: Vec<Pos2> = rounded_poly(&page, w * 0.03).into_iter().map(|q| q + Vec2::new(w * 0.012, w * 0.022)).collect();
    p.add(egui::Shape::convex_polygon(shadow, Color32::from_black_alpha(40), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(rounded_poly(&page, w * 0.03), Color32::WHITE, Stroke::NONE));
    let fold_tri = [u(rr - fold, t), u(rr - fold, t + fold), u(rr, t + fold)];
    p.add(egui::Shape::convex_polygon(rounded_poly(&fold_tri, w * 0.012), Color32::from_rgb(222, 214, 255), Stroke::NONE));
    // Lines of text, the second one highlighted.
    let line = Color32::from_rgb(208, 203, 230);
    let lines = [(0.34, 0.36, 0.52), (0.34, 0.45, 0.62), (0.34, 0.54, 0.60), (0.34, 0.63, 0.50)];
    for (i, (x0, y, x1)) in lines.iter().enumerate() {
        if i == 1 {
            let band = Rect::from_min_max(u(x0 - 0.02, y - 0.032), u(x1 + 0.025, y + 0.032));
            p.rect_filled(band, w * 0.012, Color32::from_rgb(255, 214, 64));
            p.rect_filled(Rect::from_min_max(u(*x0, y - 0.011), u(*x1, y + 0.011)), w * 0.011, Color32::from_rgb(120, 96, 40).gamma_multiply(0.55));
        } else {
            p.rect_filled(Rect::from_min_max(u(*x0, y - 0.011), u(*x1, y + 0.011)), w * 0.011, line);
        }
    }
    // The gem: a white badge with a faceted violet crystal.
    let gc = u(0.665, 0.70);
    p.circle_filled(gc + Vec2::new(0.0, w * 0.01), w * 0.125, Color32::from_black_alpha(45));
    p.circle_filled(gc, w * 0.12, Color32::WHITE);
    let g = |x: f32, y: f32| gc + Vec2::new(x * w, y * w);
    let (top, left, right, bottom, mid) = (g(0.0, -0.085), g(-0.055, -0.01), g(0.055, -0.01), g(0.0, 0.085), g(0.0, 0.0));
    let facet = |pts: Vec<Pos2>, c: Color32| p.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
    facet(vec![top, mid, left], Color32::from_rgb(170, 146, 255));
    facet(vec![top, right, mid], Color32::from_rgb(132, 104, 245));
    facet(vec![left, mid, bottom], Color32::from_rgb(108, 78, 230));
    facet(vec![mid, right, bottom], Color32::from_rgb(80, 50, 200));
}

/// A small (24 pt) quiet icon button for panels.
pub fn small(ui: &mut egui::Ui, icon: Icon, selected: bool, accent: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let v = ui.visuals();
    let bg = if selected {
        accent.gamma_multiply(if v.dark_mode { 0.28 } else { 0.16 })
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, bg);
    let fg = if selected { accent } else if resp.hovered() { v.text_color() } else { v.weak_text_color() };
    paint(ui.painter(), rect.shrink(5.0), icon, fg, v.panel_fill);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    resp.on_hover_text(tip)
}
