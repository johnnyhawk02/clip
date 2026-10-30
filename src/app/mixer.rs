//! Live's mixer controls, shared by the Arrangement's track headers and the Session mixer: the
//! track activator and Solo buttons, the value boxes Live shows volume, pan and sends in, faders,
//! knobs and level meters.

use super::*;

/// Live's track activator and Solo colours.
pub(super) const ACTIVATOR: Color32 = Color32::from_rgb(255, 173, 86);
pub(super) const SOLO: Color32 = Color32::from_rgb(71, 184, 224);
pub(super) const PLAYING: Color32 = Color32::from_rgb(40, 200, 60);
pub(super) const DARK_TEXT: Color32 = Color32::from_gray(10);

/// Fader position (0..1) for a linear gain (0..2, +6 dB at the top), and back.
pub(super) fn fader_pos(g: f32) -> f32 {
    (g / 2.0).max(0.0).sqrt()
}
pub(super) fn fader_gain(p: f32) -> f32 {
    2.0 * p.clamp(0.0, 1.0).powi(2)
}

/// A gain in dB as Live writes it.
pub(super) fn db_text(g: f32) -> String {
    if g < 1e-4 { "-inf".into() } else { format!("{:.1}", 20.0 * g.log10()) }
}

/// A pan as Live writes it: C, or 1L..50L / 1R..50R.
pub(super) fn pan_text(pan: f32) -> String {
    match pan {
        v if v.abs() < 0.005 => "C".to_string(),
        v if v < 0.0 => format!("{:.0}L", -v * 50.0),
        v => format!("{:.0}R", v * 50.0),
    }
}

/// A level meter, lit from the bottom (or, wider than tall, from the left) on a -60..+6 dB scale.
pub(super) fn vmeter(p: &egui::Painter, r: Rect, level: f32) {
    p.rect_filled(r, 0.0, gray(205));
    let db = 20.0 * level.max(1e-6).log10();
    let t = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
    if t > 0.0 {
        let lit = if r.width() > r.height() {
            Rect::from_min_max(r.min, Pos2::new(r.left() + r.width() * t, r.bottom()))
        } else {
            Rect::from_min_max(Pos2::new(r.left(), r.bottom() - r.height() * t), r.max)
        };
        let c = if db > 0.0 { Color32::from_rgb(230, 70, 50) } else if db > -6.0 { Color32::from_rgb(230, 200, 60) } else { PLAYING };
        p.rect_filled(lit, 0.0, c);
    }
}

/// Meters side by side (one per channel) filling `r`.
pub(super) fn meters(p: &egui::Painter, r: Rect, levels: &[f32]) {
    let n = levels.len().max(1) as f32;
    let w = (r.width() - (n - 1.0)) / n;
    for (k, m) in levels.iter().enumerate() {
        vmeter(p, Rect::from_min_size(Pos2::new(r.left() + k as f32 * (w + 1.0), r.top()), Vec2::new(w, r.height())), *m);
    }
}

/// A track activator (its number or letter, lit while the track is on) or a Solo button, as
/// Live draws them. Returns whether it was clicked.
pub(super) fn mix_button(ui: &mut egui::Ui, r: Rect, id: egui::Id, text: &str, on: bool, lit: Color32, tip: &str) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let p = ui.painter();
    let fill = if on { lit } else if resp.hovered() { gray(214) } else { gray(225) };
    p.rect(r, 2.0, fill, Stroke::new(1.0, gray(170)), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.0), if on { DARK_TEXT } else { muted() });
    resp.on_hover_text(tip).clicked()
}

/// What a value box shows and edits.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum BoxKind {
    /// A gain, 0..2 (+6 dB), shown in dB; double-click for 0 dB.
    Volume,
    /// -1..1, shown as Live's C / 50L / 50R; double-click for centre.
    Pan,
    /// A send level, 0..1, shown in dB; double-click for none.
    Send,
}

/// One of Live's value boxes: the value written in a small box, with a bar behind it showing
/// where it is in its range. Drag up and down to change it (Shift for fine). Returns
/// (changed, released).
pub(super) fn value_box(ui: &mut egui::Ui, r: Rect, id: egui::Id, v: &mut f32, kind: BoxKind, tip: &str) -> (bool, bool) {
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let mut changed = false;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
        let d = -resp.drag_delta().y * 0.006 * fine;
        *v = match kind {
            BoxKind::Volume => fader_gain(fader_pos(*v) + d),
            BoxKind::Pan => (*v + 2.0 * d).clamp(-1.0, 1.0),
            BoxKind::Send => (*v + d).clamp(0.0, 1.0),
        };
        changed = true;
    }
    if resp.double_clicked() {
        *v = match kind {
            BoxKind::Volume => 1.0,
            BoxKind::Pan | BoxKind::Send => 0.0,
        };
        changed = true;
    }
    let p = ui.painter();
    let active = resp.hovered() || resp.dragged();
    p.rect(r, 2.0, gray(if active { 232 } else { 238 }), Stroke::new(1.0, gray(if active { 160 } else { 200 })), egui::StrokeKind::Inside);
    let inner = r.shrink(2.0);
    let bar = match kind {
        BoxKind::Volume => Some((0.0f32, fader_pos(*v))),
        BoxKind::Send => Some((0.0, v.sqrt())),
        BoxKind::Pan => (v.abs() >= 0.005).then(|| (0.5, 0.5 + *v / 2.0)),
    };
    if let Some((a, b)) = bar {
        let (a, b) = (a.min(b), a.max(b));
        let fill = Rect::from_min_max(Pos2::new(inner.left() + inner.width() * a, inner.top()), Pos2::new(inner.left() + inner.width() * b, inner.bottom()));
        p.rect_filled(fill, 1.0, gray(212));
    }
    let text = match kind {
        BoxKind::Volume | BoxKind::Send => db_text(*v),
        BoxKind::Pan => pan_text(*v),
    };
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(10.5), ink());
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp.on_hover_text(tip);
    (changed, changed && !ui.input(|i| i.pointer.any_down()))
}

/// A vertical fader for a gain (0..2) with its meter beside it, as in Live's mixer. Drag to
/// change, double-click for 0 dB. Returns (changed, released).
pub(super) fn fader(ui: &mut egui::Ui, r: Rect, id: egui::Id, gain: &mut f32, levels: &[f32]) -> (bool, bool) {
    let p = ui.painter();
    let mw = 4.0;
    for (k, m) in levels.iter().enumerate() {
        vmeter(p, Rect::from_min_size(Pos2::new(r.left() + k as f32 * (mw + 1.0), r.top()), Vec2::new(mw, r.height())), *m);
    }
    let rail_x = r.left() + levels.len() as f32 * (mw + 1.0) + 10.0;
    let rail = Rect::from_min_max(Pos2::new(rail_x, r.top()), Pos2::new(rail_x + 3.0, r.bottom()));
    p.rect_filled(rail, 1.0, gray(190));
    // 0 dB mark.
    let zero_y = r.bottom() - r.height() * fader_pos(1.0);
    p.line_segment([Pos2::new(rail.left() - 4.0, zero_y), Pos2::new(rail.right() + 4.0, zero_y)], Stroke::new(1.0, gray(150)));
    let y = r.bottom() - r.height() * fader_pos(*gain);
    let thumb = Rect::from_center_size(Pos2::new(rail.center().x, y), Vec2::new(16.0, 7.0));
    let hit = Rect::from_min_max(Pos2::new(rail.left() - 10.0, r.top() - 4.0), Pos2::new(rail.right() + 10.0, r.bottom() + 4.0));
    let resp = ui.interact(hit, id, Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(thumb, 1.0, if resp.hovered() || resp.dragged() { ink() } else { gray(90) });
    let mut changed = false;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
        let pos = fader_pos(*gain) - resp.drag_delta().y / r.height() * fine;
        *gain = fader_gain(pos);
        changed = true;
    }
    if resp.double_clicked() {
        *gain = 1.0;
        changed = true;
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp.on_hover_text("Volume (double-click for 0 dB)");
    (changed, changed && !ui.input(|i| i.pointer.any_down()))
}

/// A pan knob, as in Live's mixer: drag up or down, double-click for centre.
pub(super) fn pan_knob(ui: &mut egui::Ui, c: Pos2, id: egui::Id, pan: &mut f32) -> bool {
    let r = 11.0;
    let resp = ui.interact(Rect::from_center_size(c, Vec2::splat(r * 2.0 + 4.0)), id, Sense::click_and_drag());
    let p = ui.painter();
    use std::f32::consts::PI;
    // The arc runs from 7 o'clock to 5 o'clock; the lit part from 12 to the pan.
    let ang = |v: f32| -PI / 2.0 + v * 0.75 * PI;
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        (0..=24).map(|k| from + (to - from) * k as f32 / 24.0).map(|a| c + Vec2::new(a.cos(), a.sin()) * r).collect()
    };
    p.add(egui::Shape::line(arc(ang(-1.0), ang(1.0)), Stroke::new(2.5, gray(200))));
    if pan.abs() > 0.005 {
        p.add(egui::Shape::line(arc(ang(0.0), ang(*pan)), Stroke::new(2.5, ink())));
    }
    let a = ang(*pan);
    p.line_segment([c, c + Vec2::new(a.cos(), a.sin()) * (r - 1.0)], Stroke::new(1.5, ink()));
    let mut changed = false;
    if resp.dragged() {
        *pan = (*pan - resp.drag_delta().y * 0.01).clamp(-1.0, 1.0);
        changed = true;
    }
    if resp.double_clicked() {
        *pan = 0.0;
        changed = true;
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp.on_hover_text(format!("Pan {} (double-click for centre)", pan_text(*pan)));
    changed
}
