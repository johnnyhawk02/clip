//! The device view, as in Live: the selected track's built-in effects in a row along the
//! bottom (click a track's name to see it), each with its on/off switch, settings and remove
//! button, and + Add Effect at the end.

use super::*;
use crate::devices::{self, Device};
use crate::fx;
use crate::tape;
use crate::phaser;
use crate::chorus;
use crate::pultec;
use crate::riser;
use crate::tame;
use crate::opto;
use crate::autofilter;

/// Width of one device panel.
const DEV_W: f32 = 560.0;

/// Return track `r` is device target `RETURN + r`; a plain number is a track.
use crate::engine::RETURN;

impl App {
    /// The track (or return track) device target `k` names.
    pub(super) fn dev_track(&self, k: usize) -> Option<&Track> {
        if k >= RETURN { self.returns.get(k - RETURN) } else { self.tracks.get(k) }
    }

    fn dev_track_mut(&mut self, k: usize) -> Option<&mut Track> {
        if k >= RETURN { self.returns.get_mut(k - RETURN) } else { self.tracks.get_mut(k) }
    }

    /// Show track `t`'s effects in the detail view (clicking its name, as in Live).
    pub(super) fn show_track_devices(&mut self, t: usize) {
        self.set_active(t);
        self.device_target = t;
        self.show_devices = true;
        self.devices_over = (self.sel_clip, self.sel_slot);
    }

    /// Show return track `r`'s effects in the detail view.
    pub(super) fn show_return_devices(&mut self, r: usize) {
        self.device_target = RETURN + r;
        self.show_devices = true;
        self.devices_over = (self.sel_clip, self.sel_slot);
    }

    /// Add effect `name` to track (or return track) `k` and show its chain.
    pub(super) fn add_device(&mut self, k: usize, name: &str) {
        let Some(d) = Device::new(name, k, self.tracks.len()) else { return };
        if self.dev_track(k).is_none() {
            return;
        }
        self.push_undo("Add Effect");
        let t = self.dev_track_mut(k).unwrap();
        t.devices.push(d);
        let tn = t.name.clone();
        if k >= RETURN { self.show_return_devices(k - RETURN) } else { self.show_track_devices(k) }
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Added {name} to {tn}");
    }

    /// The menu of effects to add to track `t`.
    pub(super) fn add_effect_menu(&mut self, ui: &mut egui::Ui, t: usize) {
        for (name, about) in devices::ALL {
            if ui.button(*name).on_hover_text(*about).clicked() {
                ui.close();
                self.add_device(t, name);
            }
        }
    }

    /// The selected track's device chain in `r`.
    pub(super) fn device_view(&mut self, ui: &mut egui::Ui, r: Rect) {
        let t = if self.device_target >= RETURN { self.device_target } else { self.active.min(self.tracks.len().saturating_sub(1)) };
        let Some(track) = self.dev_track(t) else { return };
        ui.painter().rect_filled(r, 0.0, gray(236));
        let before = track.devices.clone();
        let mut chain = before.clone();
        let name = track.name.clone();
        let others: Vec<(usize, String)> = self.tracks.iter().enumerate().filter(|(i, _)| *i != t).map(|(i, tr)| (i, tr.name.clone())).collect();
        let playing = self.player.is_playing();
        let cuts: Vec<_> = (0..chain.len()).map(|i| self.player.spacer_cuts(t, i).filter(|_| playing)).collect();
        let tames: Vec<_> = (0..chain.len()).map(|i| self.player.tame_view(t, i).filter(|_| playing)).collect();
        let optos: Vec<_> = (0..chain.len()).map(|i| self.player.opto_gr(t, i).filter(|_| playing)).collect();
        let cutoffs: Vec<_> = (0..chain.len()).map(|i| self.player.filter_cutoff(t, i).filter(|_| playing)).collect();
        let mut remove = None;
        let mut add = None;
        let mut active = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(8.0)), |ui| {
            egui::ScrollArea::horizontal().id_salt("devices").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (i, d) in chain.iter_mut().enumerate() {
                        let h = ui.available_height();
                        // Tape has a third column, for its LFO.
                        let w = match d {
                            Device::Tape(_) => DEV_W + 300.0,
                            // Riser is one knob.
                            Device::Riser(_) => 180.0,
                            Device::Opto(_) => 520.0,
                            Device::AutoFilter(_) => AUTOFILTER_W,
                            _ => DEV_W,
                        };
                        let (panel, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                        ui.painter().rect_filled(panel, 3.0, gray(250));
                        ui.painter().rect_stroke(panel, 3.0, Stroke::new(1.0, rule()), egui::StrokeKind::Inside);
                        // Title strip: on/off, name, remove.
                        let title = Rect::from_min_size(panel.min, Vec2::new(panel.width(), 24.0));
                        ui.painter().rect_filled(title, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, gray(225));
                        let sw = Rect::from_center_size(title.left_center() + Vec2::new(14.0, 0.0), Vec2::splat(12.0));
                        let swr = ui.interact(sw.expand(3.0), ui.scope_id().with(("devon", i)), Sense::click());
                        ui.painter().circle(sw.center(), 5.5, if d.on() { Color32::from_rgb(255, 173, 86) } else { gray(205) }, Stroke::new(1.0, gray(150)));
                        if swr.on_hover_text("Device on / off").clicked() {
                            d.set_on(!d.on());
                        }
                        ui.painter().text(title.left_center() + Vec2::new(28.0, 0.0), Align2::LEFT_CENTER, d.name(), FontId::proportional(12.0), ink());
                        let x = Rect::from_center_size(title.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(16.0));
                        let xr = ui.interact(x, ui.scope_id().with(("devx", i)), Sense::click());
                        ui.painter().text(x.center(), Align2::CENTER_CENTER, "×", FontId::proportional(15.0), if xr.hovered() { ink() } else { muted() });
                        if xr.on_hover_text("Remove this effect").clicked() {
                            remove = Some(i);
                        }
                        let body = Rect::from_min_max(Pos2::new(panel.left() + 10.0, title.bottom() + 4.0), panel.max - Vec2::splat(6.0));
                        // A child that doesn't move the row along (the panel already did).
                        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("device", i)));
                        active |= match d {
                            Device::Spacer(s) => spacer_panel(&mut inner, s, &others, cuts[i]),
                            Device::Reverb(s) => reverb_panel(&mut inner, s),
                            Device::Delay(s) => delay_panel(&mut inner, s),
                            Device::Spring(s) => spring_panel(&mut inner, s),
                            Device::Tape(s) => tape_panel(&mut inner, s),
                            Device::Phaser(s) => phaser_panel(&mut inner, s),
                            Device::Chorus(s) => chorus_panel(&mut inner, s),
                            Device::Pultec(s) => pultec_panel(&mut inner, s),
                            Device::Riser(s) => riser_panel(&mut inner, s, false),
                            Device::Tame(s) => tame_panel(&mut inner, s, tames[i]),
                            Device::Opto(s) => opto_panel(&mut inner, s, optos[i]),
                            Device::AutoFilter(s) => autofilter_panel(&mut inner, s, cutoffs[i]),
                        };
                    }
                    // + Add Effect, as Live's "Drop Audio Effects Here".
                    let h = ui.available_height();
                    let (slot, resp) = ui.allocate_exact_size(Vec2::new(200.0, h), Sense::click());
                    ui.painter().rect_stroke(slot, 3.0, Stroke::new(1.0, if resp.hovered() { ink() } else { gray(200) }), egui::StrokeKind::Inside);
                    let text = if chain.is_empty() { format!("No effects on {name}\n\n+ Add Effect") } else { "+ Add Effect".into() };
                    ui.painter().text(slot.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.0), muted());
                    egui::Popup::menu(&resp).show(|ui| {
                        for (n, about) in devices::ALL {
                            if ui.button(*n).on_hover_text(*about).clicked() {
                                ui.close();
                                add = Some(*n);
                            }
                        }
                    });
                });
            });
        });
        if let Some(i) = remove {
            chain.remove(i);
        }
        if let Some(n) = add {
            return self.add_device(t, n);
        }
        if chain != before {
            if !self.device_edit {
                self.push_undo(if remove.is_some() { "Remove Effect" } else { "Effect Settings" });
                self.device_edit = true;
            }
            if let Some(tr) = self.dev_track_mut(t) {
                tr.devices = chain;
            }
            self.dirty = true;
            self.refresh_playback();
        }
        if !active {
            self.device_edit = false;
        }
        if cuts.iter().any(|c| c.is_some()) || tames.iter().any(|c| c.is_some()) || optos.iter().any(|c| c.is_some()) || cutoffs.iter().any(|c| c.is_some()) {
            ui.ctx().request_repaint();
        }
    }
}

/// Spacer's controls: the track to make room for, its settings and the live cut per band.
/// Returns whether a control is being dragged or typed in.
fn spacer_panel(ui: &mut egui::Ui, s: &mut spacer::Settings, others: &[(usize, String)], cuts: Option<[[f32; spacer::BANDS]; 2]>) -> bool {
    let mut active = false;
    let full = ui.max_rect();
    let side = Rect::from_min_max(Pos2::new(full.left() + 340.0, full.top()), full.max);
    ui.spacing_mut().slider_width = 110.0;
    ui.spacing_mut().interact_size.y = 18.0;
    ui.spacing_mut().item_spacing.y = 2.0;
    let left = Rect::from_min_max(full.min, Pos2::new(side.left() - 10.0, full.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| egui::Grid::new("spacergrid").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
        ui.label("Make room for");
        let current = others.iter().find(|o| o.0 == s.source).map_or("(choose a track)", |o| o.1.as_str());
        egui::ComboBox::from_id_salt("spacer_src").width(190.0).selected_text(current).show_ui(ui, |ui| {
            for (i, n) in others {
                ui.selectable_value(&mut s.source, *i, n);
            }
        });
        ui.end_row();
        ui.label("Amount");
        let mut pct = s.amount * 100.0;
        let w = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").fixed_decimals(0));
        s.amount = pct / 100.0;
        active |= busy(&w);
        ui.end_row();
        ui.label("Low / High Cut");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut s.low_cut).range(20.0..=2000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
            let b = ui.add(egui::DragValue::new(&mut s.high_cut).range(500.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
            active |= busy(&a) || busy(&b);
        });
        ui.end_row();
        ui.label("Attack / Release");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut s.attack).range(0.5..=100.0).speed(0.2).suffix(" ms").fixed_decimals(1));
            let b = ui.add(egui::DragValue::new(&mut s.release).range(10.0..=1000.0).speed(2.0).suffix(" ms").fixed_decimals(0));
            active |= busy(&a) || busy(&b);
        });
        ui.end_row();
        ui.label("Stereo");
        ui.horizontal(|ui| {
            for (i, n) in ["Linked", "L / R", "M / S"].iter().enumerate() {
                ui.radio_value(&mut s.stereo, i as u8, *n);
            }
        });
        ui.end_row();
        ui.label("Output");
        let w = ui.add(egui::Slider::new(&mut s.output, -12.0..=12.0).suffix(" dB").fixed_decimals(1));
        active |= busy(&w);
        ui.end_row();
    }));
    // The cut in each band while playing, beside the controls: 30 Hz to 18 kHz, 0 to -30 dB.
    if side.width() > 60.0 && side.height() > 30.0 {
        let r = side;
        let p = ui.painter();
        p.rect_filled(r, 3.0, gray(238));
        let bw = r.width() / spacer::BANDS as f32;
        let rows = if s.stereo != 0 { 2 } else { 1 };
        for b in 0..spacer::BANDS {
            for row in 0..rows {
                let cut = cuts.map_or(0.0, |c| c[row][b]).clamp(0.0, 30.0);
                let w = (bw - 2.0) / rows as f32;
                let bar = Rect::from_min_size(Pos2::new(r.left() + b as f32 * bw + 1.0 + row as f32 * w, r.top() + 2.0), Vec2::new(w, cut / 30.0 * (r.height() - 4.0)));
                p.rect_filled(bar, 1.0, if row == 0 { ink() } else { muted() });
            }
        }
        let deepest = cuts.map_or(0.0, |c| c.iter().flatten().fold(0.0f32, |a, &v| a.max(v)));
        let label = if cuts.is_some() { format!("-{deepest:.1} dB") } else { "play to see\nthe cuts".into() };
        p.text(r.center_bottom() - Vec2::new(0.0, 6.0), Align2::CENTER_BOTTOM, label, FontId::proportional(9.5), muted());
        p.text(r.left_top() + Vec2::new(3.0, 2.0), Align2::LEFT_TOP, "cut", FontId::proportional(9.0), muted());
    }
    active
}

/// Tame's controls, and beside them the cut it's making across the spectrum (while playing; in
/// the print dialog, `view` is `None` and the controls take the whole width).
pub(super) fn tame_panel(ui: &mut egui::Ui, s: &mut tame::Settings, view: Option<([f32; tame::CURVE], [f32; tame::CURVE])>) -> bool {
    let mut active = false;
    let full = ui.max_rect();
    let live = full.width() > 400.0 && full.height() < 400.0;
    let side = Rect::from_min_max(Pos2::new(full.left() + 330.0, full.top()), full.max);
    ui.spacing_mut().slider_width = 120.0;
    ui.spacing_mut().interact_size.y = 18.0;
    ui.spacing_mut().item_spacing.y = 2.0;
    let left = Rect::from_min_max(full.min, Pos2::new(side.left() - 10.0, full.bottom()));
    let mut controls = |ui: &mut egui::Ui| egui::Grid::new("tamegrid").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
        ui.label("Depth");
        active |= pct(ui, &mut s.depth);
        ui.end_row();
        ui.label("Detail").on_hover_text("Low: broad, gentle cuts. High: only narrow, sharp resonances");
        active |= pct(ui, &mut s.detail);
        ui.end_row();
        ui.label("Speed").on_hover_text("How fast the cuts come and go");
        active |= pct(ui, &mut s.speed);
        ui.end_row();
        ui.label("Low / High");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut s.low).range(20.0..=5000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
            let b = ui.add(egui::DragValue::new(&mut s.high).range(200.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
            active |= busy(&a) || busy(&b);
        });
        ui.end_row();
        ui.label("Mode");
        ui.horizontal(|ui| {
            ui.radio_value(&mut s.hard, false, "Soft").on_hover_text("Resonances at any level");
            ui.radio_value(&mut s.hard, true, "Hard").on_hover_text("Only resonances louder than the threshold");
            ui.add_space(12.0);
            ui.checkbox(&mut s.delta, "Delta").on_hover_text("Hear only what's being taken away");
        });
        ui.end_row();
        if s.hard {
            ui.label("Threshold");
            let w = ui.add(egui::Slider::new(&mut s.threshold, -60.0..=0.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
        }
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    // In the print dialog the controls flow on; in the device view the curve sits beside them.
    if live {
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| controls(ui));
    } else {
        controls(ui);
    }
    // The cut across the spectrum, 20 Hz to 20 kHz (log): the level in grey, the cut hanging
    // from the top in black, 0 to -12 dB; outside Low-High shaded.
    if live && side.width() > 60.0 && side.height() > 30.0 {
        let r = side;
        let p = ui.painter();
        p.rect_filled(r, 3.0, gray(238));
        let x_of = |hz: f32| r.left() + r.width() * ((hz.max(20.0) / 20.0).log10() / 3.0).clamp(0.0, 1.0);
        let (lo, hi) = (x_of(s.low), x_of(s.high));
        p.rect_filled(Rect::from_min_max(r.min, Pos2::new(lo, r.bottom())), 3.0, gray(228));
        p.rect_filled(Rect::from_min_max(Pos2::new(hi, r.top()), r.max), 3.0, gray(228));
        for hz in [100.0, 1000.0, 10000.0] {
            let x = x_of(hz);
            p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(1.0, gray(225)));
            let label = if hz >= 1000.0 { format!("{}k", hz as u32 / 1000) } else { format!("{}", hz as u32) };
            p.text(Pos2::new(x + 2.0, r.bottom() - 2.0), Align2::LEFT_BOTTOM, label, FontId::proportional(9.0), muted());
        }
        let x_at = |j: usize| r.left() + r.width() * j as f32 / (tame::CURVE - 1) as f32;
        if let Some((curve, level)) = view {
            // Level: -90 to 0 dB up the height.
            let pts: Vec<Pos2> = (0..tame::CURVE).map(|j| Pos2::new(x_at(j), r.bottom() - r.height() * ((level[j] + 90.0) / 90.0).clamp(0.0, 1.0))).collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.0, gray(170))));
            let depth = |c: f32| r.top() + 2.0 + (c / tame::MAX_CUT).clamp(0.0, 1.0) * (r.height() * 0.6);
            let mut fill = vec![Pos2::new(x_at(0), r.top() + 2.0)];
            fill.extend((0..tame::CURVE).map(|j| Pos2::new(x_at(j), depth(curve[j]))));
            fill.push(Pos2::new(x_at(tame::CURVE - 1), r.top() + 2.0));
            for w in fill.windows(2) {
                let (a, b) = (w[0], w[1]);
                p.add(egui::Shape::convex_polygon(vec![Pos2::new(a.x, r.top() + 2.0), a, b, Pos2::new(b.x, r.top() + 2.0)], gray(120), Stroke::NONE));
            }
            p.add(egui::Shape::line(fill[1..fill.len() - 1].to_vec(), Stroke::new(1.5, ink())));
            let deepest = curve.iter().fold(0.0f32, |a, &v| a.max(v));
            p.text(r.right_bottom() - Vec2::new(4.0, 16.0), Align2::RIGHT_BOTTOM, format!("-{deepest:.1} dB"), FontId::proportional(9.5), muted());
        } else {
            p.text(r.center(), Align2::CENTER_CENTER, "play to see the cuts", FontId::proportional(9.5), muted());
        }
        p.text(r.left_top() + Vec2::new(3.0, 2.0), Align2::LEFT_TOP, "cut", FontId::proportional(9.0), muted());
    }
    active
}

/// Opto's controls, and beside them its gain reduction meter (while playing; in the print
/// dialog, `gr` is `None` and there is no meter).
pub(super) fn opto_panel(ui: &mut egui::Ui, s: &mut opto::Settings, gr: Option<f32>) -> bool {
    let mut active = false;
    let full = ui.max_rect();
    let live = full.width() > 300.0 && full.height() < 400.0;
    ui.spacing_mut().slider_width = 140.0;
    let mut controls = |ui: &mut egui::Ui| egui::Grid::new("optogrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Peak Reduction").on_hover_text("How much it levels: turn up until the meter moves the way you want");
        active |= pct(ui, &mut s.peak_reduction);
        ui.end_row();
        ui.label("Gain").on_hover_text("Make-up gain, after the levelling");
        let w = ui.add(egui::Slider::new(&mut s.gain, -12.0..=24.0).suffix(" dB").fixed_decimals(1));
        active |= busy(&w);
        ui.end_row();
        ui.label("Mode");
        ui.horizontal(|ui| {
            ui.radio_value(&mut s.limit, false, "Compress").on_hover_text("Gentle, about 3:1");
            ui.radio_value(&mut s.limit, true, "Limit").on_hover_text("Firmer, about 12:1, with a harder knee");
        });
        ui.end_row();
        ui.label("Colour").on_hover_text("Valve and transformer warmth after the levelling");
        active |= pct(ui, &mut s.colour);
        ui.end_row();
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    let side = Rect::from_min_max(Pos2::new(full.left() + 380.0, full.top()), full.max);
    if live {
        let left = Rect::from_min_max(full.min, Pos2::new(side.left() - 10.0, full.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| controls(ui));
    } else {
        controls(ui);
    }
    // Gain reduction, 0 to -20 dB down the height, as the hardware's meter shows it.
    if live && side.width() > 40.0 && side.height() > 40.0 {
        let p = ui.painter();
        let r = Rect::from_min_max(side.min + Vec2::new(0.0, 14.0), side.max - Vec2::new(0.0, 16.0));
        let bar = Rect::from_center_size(r.center(), Vec2::new(r.width().min(36.0), r.height()));
        p.rect_filled(bar, 3.0, gray(238));
        let y_of = |db: f32| bar.top() + bar.height() * (db / 20.0).clamp(0.0, 1.0);
        for db in [0.0, 3.0, 6.0, 10.0, 20.0] {
            let y = y_of(db);
            p.line_segment([Pos2::new(bar.left(), y), Pos2::new(bar.left() - 4.0, y)], Stroke::new(1.0, gray(180)));
            p.text(Pos2::new(bar.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{}", -(db as i32)), FontId::proportional(9.0), muted());
        }
        if let Some(g) = gr {
            p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.right(), y_of(g))), 3.0, ink());
        }
        p.text(Pos2::new(bar.center().x, side.top()), Align2::CENTER_TOP, "GR", FontId::proportional(9.0), muted());
        let label = gr.map_or("play to see".into(), |g| format!("-{g:.1} dB"));
        p.text(Pos2::new(bar.center().x, side.bottom()), Align2::CENTER_BOTTOM, label, FontId::proportional(9.5), muted());
    }
    active
}

/// Auto Filter's panel width (three columns of controls and its curve).
pub(super) const AUTOFILTER_W: f32 = 1300.0;

/// Auto Filter's controls, as Live's: the filter, its envelope follower, its LFO, and its curve
/// (drag it: across for Frequency, up and down for Resonance), with the cutoff where the
/// modulation has it now while playing (`cutoff`).
pub(super) fn autofilter_panel(ui: &mut egui::Ui, s: &mut autofilter::Settings, cutoff: Option<f32>) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 90.0;
    ui.spacing_mut().interact_size.y = 18.0;
    ui.spacing_mut().item_spacing.y = 2.0;
    let hz = |v: f64, _: std::ops::RangeInclusive<usize>| if v >= 1000.0 { format!("{:.2} kHz", v / 1000.0) } else { format!("{v:.0} Hz") };
    ui.horizontal_top(|ui| {
        egui::Grid::new("afgrid").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
            ui.label("Circuit");
            let name = autofilter::CIRCUITS.get(s.circuit as usize).copied().unwrap_or("MS2");
            egui::ComboBox::from_id_salt("af_circuit").width(90.0).selected_text(name).show_ui(ui, |ui| {
                for (i, n) in autofilter::CIRCUITS.iter().enumerate() {
                    ui.selectable_value(&mut s.circuit, i as u8, *n).on_hover_text("After the Korg MS-20's filter: dirty, and screaming when pushed");
                }
            });
            ui.end_row();
            ui.label("Type");
            ui.horizontal(|ui| {
                for (i, n) in autofilter::KINDS.iter().enumerate() {
                    ui.radio_value(&mut s.kind, i as u8, *n);
                }
            });
            ui.end_row();
            ui.label("Frequency");
            let w = ui.add(egui::Slider::new(&mut s.freq, 20.0..=20000.0).logarithmic(true).custom_formatter(hz));
            active |= busy(&w);
            ui.end_row();
            ui.label("Resonance").on_hover_text("Past 90% it oscillates on its own, and screams");
            active |= pct(ui, &mut s.res);
            ui.end_row();
            ui.label("Drive").on_hover_text("Gain into the filter: pushed, the sound and the resonance fight and tear");
            let w = ui.add(egui::Slider::new(&mut s.drive, 0.0..=24.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
        });
        ui.add_space(14.0);
        egui::Grid::new("afgrid2").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
            ui.label("Follower").on_hover_text("How far the sound's own level moves the cutoff, up or down (±5 octaves)");
            active |= signed_pct(ui, &mut s.env_amount);
            ui.end_row();
            ui.label("Attack");
            let w = ui.add(egui::Slider::new(&mut s.env_attack, 0.1..=1000.0).logarithmic(true).suffix(" ms").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            ui.label("Release");
            let w = ui.add(egui::Slider::new(&mut s.env_release, 1.0..=5000.0).logarithmic(true).suffix(" ms").fixed_decimals(0));
            active |= busy(&w);
            ui.end_row();
            ui.label("Output");
            let w = ui.add(egui::Slider::new(&mut s.output, -24.0..=24.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            ui.label("Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        ui.add_space(14.0);
        egui::Grid::new("afgrid3").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
            ui.label("Envelope").on_hover_text("An ADSR on the cutoff, up or down (±5 octaves)");
            active |= signed_pct(ui, &mut s.gen_amount);
            ui.end_row();
            ui.label("Trigger");
            ui.horizontal(|ui| {
                let name = autofilter::TRIGGERS.get(s.gen_trigger as usize).copied().unwrap_or("Off");
                egui::ComboBox::from_id_salt("af_trig").width(64.0).selected_text(name).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.gen_trigger, 0, "Off");
                    ui.selectable_value(&mut s.gen_trigger, 1, "Sync").on_hover_text("On a grid of the song's beats (Drift moves it too)");
                    ui.selectable_value(&mut s.gen_trigger, 2, "Input").on_hover_text("On each hit in the sound, held while it stays loud");
                });
                if s.gen_trigger == 1 {
                    sync_rate(ui, "af_genrate", &mut s.gen_beats);
                }
            });
            ui.end_row();
            ui.label("A / D");
            ui.horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut s.gen_attack).range(0.1..=5000.0).speed(1.0).suffix(" ms").fixed_decimals(1)).on_hover_text("Attack");
                let d = ui.add(egui::DragValue::new(&mut s.gen_decay).range(1.0..=10000.0).speed(4.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Decay");
                active |= busy(&a) || busy(&d);
            });
            ui.end_row();
            ui.label("S / R");
            ui.horizontal(|ui| {
                let mut p = s.gen_sustain * 100.0;
                let a = ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).speed(0.5).suffix(" %").fixed_decimals(0)).on_hover_text("Sustain, while the gate is held");
                s.gen_sustain = p / 100.0;
                let r = ui.add(egui::DragValue::new(&mut s.gen_release).range(1.0..=10000.0).speed(4.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Release");
                active |= busy(&a) || busy(&r);
            });
            ui.end_row();
        });
        ui.add_space(14.0);
        egui::Grid::new("afgrid4").num_columns(2).spacing(Vec2::new(10.0, 1.0)).show(ui, |ui| {
            ui.label("LFO").on_hover_text("How far the LFO swings the cutoff each way (up to 4 octaves)");
            active |= pct(ui, &mut s.lfo_amount);
            ui.end_row();
            ui.label("Rate");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.lfo_sync, false, "Hz");
                ui.selectable_value(&mut s.lfo_sync, true, "Sync").on_hover_text("In time with the song");
                if s.lfo_sync {
                    sync_rate(ui, "af_lforate", &mut s.lfo_beats);
                } else {
                    ui.spacing_mut().slider_width = 70.0;
                    let w = ui.add(egui::Slider::new(&mut s.lfo_rate, 0.01..=40.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
                    active |= busy(&w);
                }
            });
            ui.end_row();
            ui.label("Shape");
            ui.horizontal(|ui| {
                let shape = tape::LFO_SHAPES.get(s.lfo_shape as usize).copied().unwrap_or("Sine");
                egui::ComboBox::from_id_salt("af_lfo").width(100.0).selected_text(shape).show_ui(ui, |ui| {
                    for (i, n) in tape::LFO_SHAPES.iter().enumerate() {
                        ui.selectable_value(&mut s.lfo_shape, i as u8, *n);
                    }
                });
                let mut p = s.lfo_slew * 100.0;
                let w = ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).speed(0.5).prefix("Slew ").suffix(" %").fixed_decimals(0)).on_hover_text("How slowly the LFO follows its shape");
                s.lfo_slew = p / 100.0;
                active |= busy(&w);
            });
            ui.end_row();
            ui.label("Drift").on_hover_text("Lets the LFO wander out of time and back in: locked for two bars, drifting out, drifting back (in Hz: four seconds each)");
            active |= pct(ui, &mut s.lfo_drift);
            ui.end_row();
            ui.label("Phase").on_hover_text("The right side's LFO ahead of the left's, for movement across the stereo field");
            let w = ui.add(egui::Slider::new(&mut s.lfo_phase, 0.0..=360.0).suffix("°").fixed_decimals(0));
            active |= busy(&w);
            ui.end_row();
        });
        ui.add_space(14.0);
        active |= autofilter_curve(ui, s, cutoff);
    });
    active
}

/// A −100..100 % slider for a −1..1 setting.
fn signed_pct(ui: &mut egui::Ui, v: &mut f32) -> bool {
    let mut p = *v * 100.0;
    let w = ui.add(egui::Slider::new(&mut p, -100.0..=100.0).suffix(" %").fixed_decimals(0));
    *v = p / 100.0;
    busy(&w)
}

/// A synced length (beats), from Auto Filter's list.
fn sync_rate(ui: &mut egui::Ui, id: &str, beats: &mut f32) {
    let name = autofilter::SYNC_RATES.iter().find(|r| (r.1 - *beats).abs() < 1e-4).map_or("1/4", |r| r.0);
    egui::ComboBox::from_id_salt(id).width(64.0).selected_text(name).show_ui(ui, |ui| {
        for (n, b) in autofilter::SYNC_RATES {
            ui.selectable_value(beats, b, n);
        }
    });
}

/// Auto Filter's response, 20 Hz to 20 kHz across, +24 to −36 dB down; drag it to tune.
fn autofilter_curve(ui: &mut egui::Ui, s: &mut autofilter::Settings, cutoff: Option<f32>) -> bool {
    let h = ui.available_height().clamp(90.0, 140.0);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(210.0, h), Sense::drag());
    if resp.dragged() {
        let d = resp.drag_delta();
        s.freq = (s.freq * (d.x / r.width() * 10.0).exp2()).clamp(20.0, 20000.0);
        s.res = (s.res - d.y / r.height()).clamp(0.0, 1.0);
    }
    let resp = resp.on_hover_text("Drag across for Frequency, up and down for Resonance");
    let p = ui.painter_at(r);
    p.rect_filled(r, 3.0, gray(238));
    let x_of = |f: f32| r.left() + r.width() * (f / 20.0).log2() / 1000f32.log2();
    let y_of = |db: f32| r.top() + r.height() * ((24.0 - db) / 60.0).clamp(0.0, 1.0);
    for f in [100.0, 1000.0, 10000.0] {
        p.line_segment([Pos2::new(x_of(f), r.top()), Pos2::new(x_of(f), r.bottom())], Stroke::new(1.0, gray(222)));
    }
    p.line_segment([Pos2::new(r.left(), y_of(0.0)), Pos2::new(r.right(), y_of(0.0))], Stroke::new(1.0, gray(215)));
    let line = |fc: f32| -> Vec<Pos2> {
        (0..=100).map(|i| {
            let f = 20.0 * 1000f32.powf(i as f32 / 100.0);
            Pos2::new(x_of(f), y_of(autofilter::response(s, fc, f)))
        }).collect()
    };
    match cutoff {
        // Playing: the set cutoff faint, where the modulation has it now in ink.
        Some(fc) => {
            p.add(egui::Shape::line(line(s.freq), Stroke::new(1.0, gray(190))));
            p.add(egui::Shape::line(line(fc), Stroke::new(1.5, ink())));
        }
        None => _ = p.add(egui::Shape::line(line(s.freq), Stroke::new(1.5, ink()))),
    }
    let label = cutoff.unwrap_or(s.freq);
    let text = if label >= 1000.0 { format!("{:.2} kHz", label / 1000.0) } else { format!("{label:.0} Hz") };
    p.text(r.right_top() + Vec2::new(-5.0, 4.0), Align2::RIGHT_TOP, text, FontId::proportional(9.5), muted());
    resp.dragged()
}

/// Whether a control is being dragged or typed in.
fn busy(w: &egui::Response) -> bool {
    w.dragged() || w.has_focus()
}

/// A 0..1 setting as a percentage slider.
fn pct(ui: &mut egui::Ui, v: &mut f32) -> bool {
    let mut p = *v * 100.0;
    let w = ui.add(egui::Slider::new(&mut p, 0.0..=100.0).suffix(" %").fixed_decimals(0));
    *v = p / 100.0;
    busy(&w)
}

/// Reverb's controls.
pub(super) fn reverb_panel(ui: &mut egui::Ui, s: &mut fx::ReverbSettings) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 160.0;
    egui::Grid::new("reverbgrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Size");
        active |= pct(ui, &mut s.size);
        ui.end_row();
        ui.label("Damping");
        active |= pct(ui, &mut s.damping);
        ui.end_row();
        ui.label("Width");
        active |= pct(ui, &mut s.width);
        ui.end_row();
        ui.label("Pre-delay");
        let w = ui.add(egui::Slider::new(&mut s.predelay, 0.0..=200.0).suffix(" ms").fixed_decimals(0));
        active |= busy(&w);
        ui.end_row();
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Delay's controls: the time in notes (synced to the tempo).
pub(super) fn delay_panel(ui: &mut egui::Ui, s: &mut fx::DelaySettings) -> bool {
    const TIMES: [(&str, f32); 8] = [("1/16", 0.25), ("1/8", 0.5), ("3/16", 0.75), ("1/4", 1.0), ("3/8", 1.5), ("1/2", 2.0), ("3/4", 3.0), ("1 bar", 4.0)];
    let mut active = false;
    ui.spacing_mut().slider_width = 160.0;
    egui::Grid::new("delaygrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Time");
        let current = TIMES.iter().find(|t| (t.1 - s.beats).abs() < 1e-3).map_or("custom", |t| t.0);
        egui::ComboBox::from_id_salt("delay_time").width(90.0).selected_text(current).show_ui(ui, |ui| {
            for (n, b) in TIMES {
                ui.selectable_value(&mut s.beats, b, n);
            }
        });
        ui.end_row();
        ui.label("Feedback");
        let mut p = s.feedback * 100.0;
        let w = ui.add(egui::Slider::new(&mut p, 0.0..=95.0).suffix(" %").fixed_decimals(0));
        s.feedback = p / 100.0;
        active |= busy(&w);
        ui.end_row();
        ui.label("Ping Pong");
        ui.checkbox(&mut s.ping_pong, "");
        ui.end_row();
        ui.label("Filter");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut s.low_cut).range(20.0..=2000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
            let b = ui.add(egui::DragValue::new(&mut s.high_cut).range(500.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
            active |= busy(&a) || busy(&b);
        });
        ui.end_row();
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Spring's controls.
pub(super) fn spring_panel(ui: &mut egui::Ui, s: &mut fx::SpringSettings) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 160.0;
    egui::Grid::new("springgrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Length");
        let w = ui.add(egui::Slider::new(&mut s.length, 20.0..=120.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Time between echoes");
        active |= busy(&w);
        ui.end_row();
        ui.label("Decay");
        active |= pct(ui, &mut s.decay);
        ui.end_row();
        ui.label("Tone");
        let w = ui.add(egui::Slider::new(&mut s.tone, 1500.0..=7000.0).logarithmic(true).suffix(" Hz").fixed_decimals(0)).on_hover_text("The spring's transition frequency: the boing sits below it (applies when you let go)");
        active |= busy(&w);
        ui.end_row();
        ui.label("Chirp");
        active |= pct(ui, &mut s.chirp);
        ui.end_row();
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Tape's controls in two columns: the echo (time, feedback, wow, mix), then the tape machine.
pub(super) fn tape_panel(ui: &mut egui::Ui, s: &mut tape::Settings) -> bool {
    const TIMES: [(&str, f32); 8] = [("1/16", 0.25), ("1/8", 0.5), ("3/16", 0.75), ("1/4", 1.0), ("3/8", 1.5), ("1/2", 2.0), ("3/4", 3.0), ("1 bar", 4.0)];
    const SPEEDS: [(&str, f32); 4] = [("3¾ ips", 3.75), ("7½ ips", 7.5), ("15 ips", 15.0), ("30 ips", 30.0)];
    let mut active = false;
    ui.spacing_mut().slider_width = 100.0;
    ui.horizontal_top(|ui| {
        egui::Grid::new("tapegrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
            ui.label("Time");
            let current = TIMES.iter().find(|t| (t.1 - s.beats).abs() < 1e-3).map_or("custom", |t| t.0);
            egui::ComboBox::from_id_salt("tape_time").width(90.0).selected_text(current).show_ui(ui, |ui| {
                for (n, b) in TIMES {
                    ui.selectable_value(&mut s.beats, b, n);
                }
            });
            ui.end_row();
            ui.label("Feedback");
            let mut p = s.feedback * 100.0;
            let w = ui.add(egui::Slider::new(&mut p, 0.0..=105.0).suffix(" %").fixed_decimals(0)).on_hover_text("Past 100% the echoes run away into the tape's saturation");
            s.feedback = p / 100.0;
            active |= busy(&w);
            ui.end_row();
            ui.label("Wow");
            active |= pct(ui, &mut s.wow);
            ui.end_row();
            ui.label("Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        ui.add_space(16.0);
        egui::Grid::new("tapegrid2").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
            ui.label("Speed");
            let current = SPEEDS.iter().find(|t| (t.1 - s.speed).abs() < 1e-3).map_or("custom", |t| t.0);
            egui::ComboBox::from_id_salt("tape_speed").width(90.0).selected_text(current).show_ui(ui, |ui| {
                for (n, v) in SPEEDS {
                    ui.selectable_value(&mut s.speed, v, n);
                }
            }).response.on_hover_text("Slower tape loses more top");
            ui.end_row();
            ui.label("Drive");
            active |= pct(ui, &mut s.drive);
            ui.end_row();
            ui.label("Bias");
            let w = { let mut p = s.bias * 100.0; let w = ui.add(egui::Slider::new(&mut p, 0.0..=100.0).suffix(" %").fixed_decimals(0)).on_hover_text("Low leaves the tape's deadzone: quiet sounds crunch"); s.bias = p / 100.0; w };
            active |= busy(&w);
            ui.end_row();
            ui.label("Spacing");
            let w = ui.add(egui::Slider::new(&mut s.spacing, 0.0..=30.0).suffix(" µm").fixed_decimals(0)).on_hover_text("Play head to tape: further is darker");
            active |= busy(&w);
            ui.end_row();
        });
        ui.add_space(16.0);
        // The delay-time LFO.
        egui::Grid::new("tapegrid3").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
            ui.label("LFO");
            let shape = tape::LFO_SHAPES.get(s.lfo_shape as usize).copied().unwrap_or("Sine");
            egui::ComboBox::from_id_salt("tape_lfo").width(110.0).selected_text(shape).show_ui(ui, |ui| {
                for (i, n) in tape::LFO_SHAPES.iter().enumerate() {
                    ui.selectable_value(&mut s.lfo_shape, i as u8, *n);
                }
            });
            ui.end_row();
            ui.label("Rate");
            ui.horizontal(|ui| {
                let unit = if s.lfo_slow { " cHz" } else { " Hz" };
                let w = ui.add(egui::Slider::new(&mut s.lfo_rate, 0.01..=20.0).logarithmic(true).suffix(unit).fixed_decimals(2));
                active |= busy(&w);
                ui.checkbox(&mut s.lfo_slow, "×0.01").on_hover_text("A hundredth of the rate, for very slow drifts (cHz: hundredths of a hertz)");
            });
            ui.end_row();
            ui.label("Depth");
            let w = ui.add(egui::Slider::new(&mut s.lfo_depth, 0.0..=50.0).suffix(" ms").fixed_decimals(1)).on_hover_text("How far the delay time swings each way");
            active |= busy(&w);
            ui.end_row();
            ui.label("Slew");
            active |= pct(ui, &mut s.lfo_slew);
            ui.end_row();
        });
    });
    active
}

/// Phaser's controls: the pedal's speed knob, and how much of it is heard.
pub(super) fn phaser_panel(ui: &mut egui::Ui, s: &mut phaser::Settings) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 160.0;
    egui::Grid::new("phasergrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Speed");
        let w = ui.add(egui::Slider::new(&mut s.speed, 0.05..=10.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
        active |= busy(&w);
        ui.end_row();
        ui.label("Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Chorus's controls: the mode (which sets its rate), rate, amount, dry/wet, and hiss (Tape: drive).
pub(super) fn chorus_panel(ui: &mut egui::Ui, s: &mut chorus::Settings) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 160.0;
    egui::Grid::new("chorusgrid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label("Mode");
        let mut m = s.mode;
        egui::ComboBox::from_id_salt("chorus_mode").width(120.0).selected_text(chorus::MODES.get(m as usize).copied().unwrap_or("Juno I")).show_ui(ui, |ui| {
            for (i, n) in chorus::MODES.iter().enumerate() {
                ui.selectable_value(&mut m, i as u8, *n);
            }
        });
        if m != s.mode {
            s.set_mode(m);
        }
        ui.end_row();
        ui.label("Rate");
        let w = ui.add(egui::Slider::new(&mut s.rate, 0.01..=20.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
        active |= busy(&w);
        ui.end_row();
        ui.label("Amount");
        let w = { let mut p = s.amount * 100.0; let w = ui.add(egui::Slider::new(&mut p, 0.0..=200.0).suffix(" %").fixed_decimals(0)).on_hover_text("Depth: 100% is the original's"); s.amount = p / 100.0; w };
        active |= busy(&w);
        ui.end_row();
        ui.label("Dry / Wet");
        let w = pct(ui, &mut s.mix);
        active |= w;
        ui.end_row();
        if chorus::MODES.get(s.mode as usize) != Some(&"Clean") {
            ui.label("Noise");
            active |= pct(ui, &mut s.noise);
            ui.end_row();
        }
        if chorus::MODES.get(s.mode as usize) == Some(&"Tape") {
            ui.label("Drive");
            active |= pct(ui, &mut s.drive);
            ui.end_row();
        }
    });
    active
}

/// Pultec's front panel: the low section (frequency, boost, attenuation), then the high
/// boost (frequency, boost, bandwidth) and high attenuation (selector, amount). Knobs go 0 to 10.
pub(super) fn pultec_panel(ui: &mut egui::Ui, s: &mut pultec::Settings) -> bool {
    let mut active = false;
    ui.spacing_mut().slider_width = 100.0;
    let knob = |ui: &mut egui::Ui, v: &mut f32| busy(&ui.add(egui::Slider::new(v, 0.0..=10.0).fixed_decimals(1)));
    let pick = |ui: &mut egui::Ui, id: &str, v: &mut f32, options: &[f32], unit: &str| {
        egui::ComboBox::from_id_salt(id).width(80.0).selected_text(format!("{} {unit}", *v)).show_ui(ui, |ui| {
            for &o in options {
                ui.selectable_value(v, o, format!("{o} {unit}"));
            }
        });
    };
    ui.horizontal_top(|ui| {
        egui::Grid::new("pultec_low").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
            ui.label("Low");
            pick(ui, "pultec_lf", &mut s.low_freq, &pultec::LOW_FREQS, "Hz");
            ui.end_row();
            ui.label("Boost");
            active |= knob(ui, &mut s.low_boost);
            ui.end_row();
            ui.label("Atten");
            active |= knob(ui, &mut s.low_atten);
            ui.end_row();
        });
        ui.add_space(16.0);
        egui::Grid::new("pultec_high").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
            ui.label("High");
            pick(ui, "pultec_hf", &mut s.high_freq, &pultec::HIGH_FREQS, "kHz");
            ui.end_row();
            ui.label("Boost");
            active |= knob(ui, &mut s.high_boost);
            ui.end_row();
            ui.label("Bandwidth");
            active |= knob(ui, &mut s.bandwidth);
            ui.end_row();
            ui.label("Atten Sel");
            pick(ui, "pultec_as", &mut s.atten_sel, &pultec::ATTEN_SELS, "kHz");
            ui.end_row();
            ui.label("Atten");
            active |= knob(ui, &mut s.high_atten);
            ui.end_row();
        });
    });
    active
}

/// Riser's one big knob (drag up and down; double-click for none), and, printed offline, whether
/// it rises across the selection.
pub(super) fn riser_panel(ui: &mut egui::Ui, s: &mut riser::Settings, offline: bool) -> bool {
    ui.vertical_centered(|ui| riser_knob(ui, s, offline)).inner
}

fn riser_knob(ui: &mut egui::Ui, s: &mut riser::Settings, offline: bool) -> bool {
    let size = 96.0;
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    if resp.dragged() {
        // A full turn over 200 points; Shift for fine.
        let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
        s.amount = (s.amount - resp.drag_delta().y / 200.0 * fine).clamp(0.0, 1.0);
    }
    if resp.double_clicked() {
        s.amount = 0.0;
    }
    let active = resp.dragged();
    let p = ui.painter();
    let c = r.center();
    let rad = size * 0.42;
    // 300° of travel, from lower left round to lower right.
    let (a0, a1) = (std::f32::consts::PI * 0.75, std::f32::consts::PI * 2.25);
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 48;
        (0..=n).map(|i| { let a = from + (to - from) * i as f32 / n as f32; c + Vec2::angled(a) * rad }).collect()
    };
    p.add(egui::Shape::line(arc(a0, a1), Stroke::new(6.0, gray(215))));
    let at = a0 + (a1 - a0) * s.amount;
    if s.amount > 0.0 {
        p.add(egui::Shape::line(arc(a0, at), Stroke::new(6.0, Color32::from_rgb(255, 173, 86))));
    }
    p.circle_filled(c, rad - 9.0, gray(if resp.hovered() || resp.dragged() { 238 } else { 244 }));
    p.line_segment([c + Vec2::angled(at) * (rad - 24.0), c + Vec2::angled(at) * (rad - 10.0)], Stroke::new(2.5, ink()));
    p.text(c, Align2::CENTER_CENTER, format!("{:.0} %", s.amount * 100.0), FontId::proportional(14.0), ink());
    resp.on_hover_text("Amount: drag up to build. Shift-drag for fine, double-click for none");
    ui.label("Amount");
    if offline {
        ui.add_space(6.0);
        ui.checkbox(&mut s.ramp, "Rise from nothing to Amount across it");
    }
    active
}
