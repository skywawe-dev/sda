//! The palette, the bundled font and the base egui style.

use eframe::egui::{self, Color32, FontId, RichText, Stroke};
use std::sync::Arc;
// ---------------- Palette ----------------

// Austere and near-monochrome, with a single muted accent.
pub(crate) const BG: Color32 = Color32::from_rgb(13, 14, 17);
pub(crate) const SIDEBAR: Color32 = Color32::from_rgb(16, 17, 21);
pub(crate) const SURFACE: Color32 = Color32::from_rgb(20, 22, 26);
pub(crate) const FIELD: Color32 = Color32::from_rgb(26, 28, 33);
pub(crate) const HOVER: Color32 = Color32::from_rgb(33, 36, 42);
pub(crate) const BORDER: Color32 = Color32::from_rgb(30, 33, 39);
pub(crate) const TEXT: Color32 = Color32::from_rgb(223, 226, 231);
pub(crate) const MUTED: Color32 = Color32::from_rgb(130, 136, 148);
pub(crate) const FAINT: Color32 = Color32::from_rgb(72, 77, 88);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(118, 158, 196); // muted steel blue
pub(crate) const GREEN: Color32 = Color32::from_rgb(124, 180, 138);
pub(crate) const RED: Color32 = Color32::from_rgb(202, 108, 110);
pub(crate) const YELLOW: Color32 = Color32::from_rgb(200, 174, 116);

pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// JetBrains Mono is embedded in the binary, so there are no runtime asset files.
fn install_fonts(ctx: &egui::Context) {
    use egui::FontFamily::{Monospace, Proportional};
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("jb".to_owned(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/JetBrainsMono-Regular.ttf"))));
    fonts.font_data.insert("jbm".to_owned(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/JetBrainsMono-Medium.ttf"))));
    // First in priority, with the built-in fonts kept behind it for emoji and symbols.
    fonts.families.entry(Proportional).or_default().insert(0, "jb".to_owned());
    fonts.families.entry(Monospace).or_default().insert(0, "jbm".to_owned());
    ctx.set_fonts(fonts);
}

pub(crate) fn apply_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = 10.into();
    v.window_shadow = egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(120) };
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(110) };
    v.menu_corner_radius = 8.into();
    v.extreme_bg_color = FIELD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    let w = &mut v.widgets;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = 6.into();
        s.expansion = 0.0;
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.weak_bg_fill = FIELD;
    w.inactive.bg_fill = FIELD;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    w.active.weak_bg_fill = HOVER;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        // JetBrains Mono runs wide, so the base sizes come down a little.
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(13.0));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(13.0));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
    });
}

/// A section heading: upper case, letter-spaced, muted.
pub(crate) fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.5).color(MUTED).extra_letter_spacing(1.5));
}
