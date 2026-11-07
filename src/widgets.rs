//! Small painting helpers shared by the screens and the modal dialogs.

use crate::i18n::t;
use crate::tf;
use crate::theme::*;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};
// ---------------- Small widgets ----------------

pub(crate) fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Btn {
    Primary,
    Plain,
    Ghost,
    Danger,
    Success,
}

pub(crate) fn button(ui: &mut egui::Ui, text: &str, kind: Btn, enabled: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), TEXT);
    let size = Vec2::new(galley.size().x + 26.0, 30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let h = ui.ctx().animate_bool(resp.id, resp.hovered() && enabled);
    let (fill, fg, stroke) = match kind {
        Btn::Primary => (mix(ACCENT.gamma_multiply(0.9), ACCENT, h), BG, Stroke::NONE),
        Btn::Plain => (mix(FIELD, HOVER, h), TEXT, Stroke::new(1.0, BORDER)),
        Btn::Ghost => (HOVER.gamma_multiply(0.8 * h), mix(MUTED, TEXT, h), Stroke::NONE),
        Btn::Danger => (RED.gamma_multiply(0.1 + 0.1 * h), RED, Stroke::new(1.0, RED.gamma_multiply(0.4))),
        Btn::Success => (GREEN.gamma_multiply(0.1 + 0.1 * h), GREEN, Stroke::new(1.0, GREEN.gamma_multiply(0.4))),
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect_filled(rect, 6, fill.gamma_multiply(alpha));
    if stroke != Stroke::NONE {
        ui.painter().rect_stroke(rect, 6, Stroke::new(stroke.width, stroke.color.gamma_multiply(alpha)), egui::StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, fg.gamma_multiply(alpha));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub(crate) fn input(ui: &mut egui::Ui, value: &mut String, hint: &str, password: bool, width: f32) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(RichText::new(hint).color(FAINT))
            .password(password)
            .desired_width(width)
            .margin(Vec2::new(10.0, 7.0)),
    )
}

/// The countdown ring showing how long the current code lasts.
pub(crate) fn timer_ring(ui: &mut egui::Ui, frac_left: f32, secs: i64) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
    let c = rect.center();
    let r = 21.0;
    let color = if secs <= 5 { RED } else { mix(MUTED, ACCENT, frac_left) };
    ui.painter().circle_stroke(c, r, Stroke::new(2.5, FIELD));
    let n = 64;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac_left * i as f32 / n as f32;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.5, color)));
    ui.painter().text(c, Align2::CENTER_CENTER, secs.to_string(), FontId::monospace(14.0), color);
}

/// An avatar: the profile picture when there is one, otherwise a rounded square
/// with the first letter of the name.
pub(crate) fn avatar(ui: &egui::Ui, center: egui::Pos2, radius: f32, name: &str, tex: Option<&egui::TextureHandle>, alpha: f32) {
    let rect = egui::Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    let r = (radius * 0.4) as u8;
    if let Some(tex) = tex {
        // Painting the texture into a rounded RectShape clips the JPEG's square corners.
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let img = egui::epaint::RectShape::filled(rect, r, Color32::WHITE.gamma_multiply(alpha)).with_texture(tex.id(), uv);
        ui.painter().add(img);
        ui.painter().rect_stroke(rect, r, Stroke::new(1.0, BORDER.gamma_multiply(alpha)), egui::StrokeKind::Inside);
        return;
    }
    ui.painter().rect_filled(rect, r, FIELD.gamma_multiply(alpha));
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, BORDER.gamma_multiply(alpha)), egui::StrokeKind::Inside);
    let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(center, Align2::CENTER_CENTER, letter, FontId::monospace(radius * 0.9), mix(MUTED, TEXT, 0.5).gamma_multiply(alpha));
}

/// Draws one account row in the sidebar. An `alpha` below 1 dims it, which marks the
/// gap left behind by a row being dragged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_account_row(
    ui: &egui::Ui,
    rect: egui::Rect,
    primary: &str,
    secondary: &str,
    enrolled: bool,
    badge: Option<usize>,
    tex: Option<&egui::TextureHandle>,
    sel_t: f32,
    hov: f32,
    alpha: f32,
) {
    let p = ui.painter();
    let bg = mix(HOVER.gamma_multiply(hov * 0.7), ACCENT.gamma_multiply(0.13), sel_t);
    p.rect_filled(rect, 8, bg.gamma_multiply(alpha));
    if sel_t > 0.01 {
        let bar = egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 12.0), Vec2::new(3.0, 26.0 * sel_t));
        p.rect_filled(bar, 2, ACCENT.gamma_multiply(alpha));
    }
    avatar(ui, rect.left_center() + Vec2::new(26.0, 0.0), 15.0, primary, tex, alpha);
    p.text(
        rect.left_center() + Vec2::new(50.0, -8.0),
        Align2::LEFT_CENTER,
        primary,
        FontId::proportional(14.0),
        mix(MUTED, TEXT, sel_t.max(hov)).gamma_multiply(alpha),
    );
    p.text(
        rect.left_center() + Vec2::new(50.0, 10.0),
        Align2::LEFT_CENTER,
        secondary,
        FontId::proportional(11.0),
        FAINT.gamma_multiply(alpha),
    );
    if !enrolled {
        p.text(
            rect.right_center() + Vec2::new(-14.0, 0.0),
            Align2::RIGHT_CENTER,
            "●",
            FontId::proportional(11.0),
            YELLOW.gamma_multiply(alpha),
        );
    } else if let Some(n) = badge.filter(|n| *n > 0) {
        let c = egui::pos2(rect.right() - 20.0, rect.center().y);
        p.circle_filled(c, 10.0, ACCENT.gamma_multiply(alpha));
        p.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::monospace(11.0), BG.gamma_multiply(alpha));
    }
}

pub(crate) fn ago(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let d = (crate::steam::unix_now() - ts).max(0);
    match d {
        0..=59 => t("just now", "только что").into(),
        60..=3599 => tf!("{} min ago", "{} мин назад", d / 60),
        3600..=86399 => tf!("{} h ago", "{} ч назад", d / 3600),
        _ => tf!("{} d ago", "{} дн назад", d / 86400),
    }
}

pub(crate) fn banner(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.1))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.4)))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(mix(color, TEXT, 0.25)).size(12.5));
        });
}
