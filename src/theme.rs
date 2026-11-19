//! One calm palette for light and dark mode: neutral greys, a single violet
//! accent (close to Obsidian's), soft cards and hairline borders.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Stroke, Vec2};

use crate::icons;

pub const ACCENT: Color32 = Color32::from_rgb(124, 102, 220);

#[derive(Clone, Copy)]
pub struct Pal {
    #[allow(dead_code)]
    pub dark: bool,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub bar: Color32,
    pub side: Color32,
    pub canvas: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub good: Color32,
    pub bad: Color32,
}

pub fn pal_for(dark: bool) -> Pal {
    if dark {
        Pal {
            dark,
            accent: Color32::from_rgb(150, 132, 245),
            accent_soft: Color32::from_rgba_unmultiplied(150, 132, 245, 40),
            bar: Color32::from_rgb(27, 27, 31),
            side: Color32::from_rgb(22, 22, 26),
            canvas: Color32::from_rgb(14, 14, 17),
            card: Color32::from_rgb(33, 33, 38),
            border: Color32::from_rgb(46, 46, 53),
            text: Color32::from_rgb(236, 236, 241),
            weak: Color32::from_rgb(150, 150, 162),
            good: Color32::from_rgb(92, 196, 132),
            bad: Color32::from_rgb(240, 110, 96),
        }
    } else {
        Pal {
            dark,
            accent: ACCENT,
            accent_soft: Color32::from_rgba_unmultiplied(124, 102, 220, 30),
            bar: Color32::from_rgb(255, 255, 255),
            side: Color32::from_rgb(248, 248, 250),
            canvas: Color32::from_rgb(237, 237, 241),
            card: Color32::WHITE,
            border: Color32::from_rgb(229, 229, 235),
            text: Color32::from_rgb(24, 24, 27),
            weak: Color32::from_rgb(113, 113, 122),
            good: Color32::from_rgb(34, 150, 90),
            bad: Color32::from_rgb(210, 70, 50),
        }
    }
}

pub fn pal(ctx: &egui::Context) -> Pal {
    pal_for(ctx.global_style().visuals.dark_mode)
}

/// A soft card: card colour, hairline border, rounded corners.
pub fn card(p: &Pal) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    use egui::{TextStyle, Visuals};
    let p = pal_for(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let (faint, hover, active) = if dark {
        (Color32::from_rgb(38, 38, 44), Color32::from_rgb(48, 48, 56), Color32::from_rgb(60, 60, 70))
    } else {
        (Color32::from_rgb(241, 241, 244), Color32::from_rgb(232, 232, 237), Color32::from_rgb(220, 220, 228))
    };
    v.panel_fill = p.side;
    v.window_fill = p.card;
    v.faint_bg_color = faint;
    v.extreme_bg_color = if dark { Color32::from_rgb(20, 20, 24) } else { Color32::WHITE };
    v.code_bg_color = faint;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.weak);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.45 } else { 0.28 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 38 }) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(if dark { 100 } else { 30 }) };
    for (w, fill) in [(&mut v.widgets.inactive, faint), (&mut v.widgets.hovered, hover), (&mut v.widgets.active, active), (&mut v.widgets.open, hover)] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = CornerRadius::same(8);
        w.fg_stroke.color = p.text;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(8);
    v.widgets.noninteractive.fg_stroke.color = p.text;
    v.striped = true;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, icons::bold())),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.window_margin = Margin::same(18);
        s.spacing.menu_margin = Margin::same(8);
        s.spacing.scroll.bar_width = 6.0;
        s.spacing.scroll.floating = true;
    });
}
