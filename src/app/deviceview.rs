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

/// The device view's height: fixed, as Live's, however big the window is.
pub(super) const DEVICE_VIEW_H: f32 = 252.0;

// The shared look of every device, as Live's devices have one: columns of labelled controls a
// fixed width apart, small muted labels, one slider length and text size, and any graph or
// meter in a column of its own beside them.
const TITLE_H: f32 = 22.0;
/// A folded device's width: just its title, on end.
const FOLDED_W: f32 = 26.0;
const PAD: f32 = 10.0;
const DEV_LABEL_W: f32 = 80.0;
const SLIDER_W: f32 = 92.0;
const COMBO_W: f32 = 92.0;
/// A control the full width of a column (Spacer's track menu).
const FIELD_W: f32 = SLIDER_W + 58.0;
const COL_W: f32 = 252.0;
const GAP: f32 = 16.0;
const GRAPH_W: f32 = 200.0;
const METER_W: f32 = 56.0;
const KNOB: f32 = 84.0;
/// Auto Filter's step strip: the G/S/A labels, then 16 steps.
const SEQ_LABEL: f32 = 14.0;
const SEQ_STEP: f32 = 18.0;
const SEQ_W: f32 = SEQ_LABEL + SEQ_STEP * autofilter::STEPS as f32;
/// On switches, the playing step and Riser's arc: Live's orange, toned down.
const ACCENT: Color32 = Color32::from_rgb(214, 152, 92);

/// The width of `n` columns of controls.
const fn cols(n: usize) -> f32 {
    n as f32 * COL_W + (n as f32 - 1.0) * GAP
}

/// The width of device `d`'s panel (its controls, graphs and padding), in the device view and in its print dialog.
pub(super) fn device_width(d: &Device) -> f32 {
    let body = match d {
        Device::Spacer(_) | Device::Tame(_) => cols(1) + GAP + GRAPH_W,
        Device::Opto(_) => cols(1) + GAP + METER_W,
        Device::Tape(_) => cols(3),
        Device::Pultec(_) => cols(2),
        Device::Riser(_) => KNOB + 50.0,
        Device::AutoFilter(_) => cols(4) + GAP + SEQ_W + GAP + GRAPH_W,
        Device::Reverb(_) | Device::Delay(_) | Device::Spring(_) | Device::Phaser(_) | Device::Chorus(_) => cols(1),
    };
    body + 2.0 * PAD
}

/// The text size and spacing every device panel shares.
fn style(ui: &mut egui::Ui) {
    let st = ui.style_mut();
    for t in [egui::TextStyle::Body, egui::TextStyle::Button] {
        st.text_styles.insert(t, FontId::proportional(12.0));
    }
    st.spacing.interact_size.y = 18.0;
    st.spacing.slider_width = SLIDER_W;
    st.spacing.combo_width = COMBO_W;
    st.spacing.item_spacing = Vec2::new(GAP, 4.0);
}

/// A control's name, muted so the values stand out.
fn label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.label(egui::RichText::new(text).color(muted()))
}

/// A column's heading, small and in capitals (only devices with more than one column have them).
fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(9.5).color(gray(140)));
}

/// One column of labelled controls: `add` fills a two-column grid, label then control, a row each.
fn column<R>(ui: &mut egui::Ui, id: &str, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(Vec2::new(COL_W, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(COL_W);
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        if !title.is_empty() {
            heading(ui, title);
        }
        egui::Grid::new(id).num_columns(2).min_col_width(DEV_LABEL_W).spacing(Vec2::new(8.0, 4.0)).show(ui, add).inner
    })
    .inner
}

/// The rect for a graph or meter beside the controls, `w` wide and the rest of the height
/// (under a heading's height when `title` is given, to line up with headed columns).
fn graph_rect(ui: &mut egui::Ui, w: f32, title: Option<&str>, sense: Sense) -> (Rect, egui::Response) {
    ui.allocate_ui_with_layout(Vec2::new(w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        if let Some(t) = title {
            heading(ui, t);
        }
        let h = ui.available_height().max(60.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, h), sense);
        ui.painter().rect_filled(r, 2.0, gray(238));
        (r, resp)
    })
    .inner
}

/// A graph's small print.
fn note(p: &egui::Painter, pos: Pos2, align: Align2, text: impl ToString) {
    p.text(pos, align, text.to_string(), FontId::proportional(9.5), muted());
}

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
        let cutoffs: Vec<_> = (0..chain.len()).map(|i| self.player.filter_view(t, i).filter(|_| playing)).collect();
        let mut remove = None;
        let mut add = None;
        let mut active = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(8.0)), |ui| {
            egui::ScrollArea::horizontal().id_salt("devices").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (i, d) in chain.iter_mut().enumerate() {
                        let h = ui.available_height();
                        let w = if d.folded() { FOLDED_W } else { device_width(d) };
                        let (panel, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                        ui.painter().rect_filled(panel, 3.0, gray(250));
                        ui.painter().rect_stroke(panel, 3.0, Stroke::new(1.0, rule()), egui::StrokeKind::Inside);
                        // Title strip: on/off, name, remove; folded, the whole panel is its title, standing on end.
                        let title = if d.folded() { panel } else { Rect::from_min_size(panel.min, Vec2::new(panel.width(), TITLE_H)) };
                        // Double-clicking the title folds the device down, as in Live (registered first, so the
                        // switch and × above it still take their clicks).
                        let tr = ui.interact(title, ui.scope_id().with(("devtitle", i)), Sense::click());
                        if tr.double_clicked() {
                            d.set_folded(!d.folded());
                        }
                        let fill = if tr.hovered() { gray(226) } else { gray(232) };
                        let corners = if d.folded() { egui::CornerRadius::same(2) } else { egui::CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 } };
                        ui.painter().rect_filled(title.shrink(1.0), corners, fill);
                        let name_col = if d.on() { ink() } else { muted() };
                        let (sw_at, x_at) = if d.folded() {
                            (title.center_top() + Vec2::new(0.0, TITLE_H / 2.0), title.center_bottom() - Vec2::new(0.0, TITLE_H / 2.0))
                        } else {
                            ui.painter().line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, rule()));
                            (title.left_center() + Vec2::new(PAD + 4.0, 0.0), title.right_center() - Vec2::new(PAD + 2.0, 0.0))
                        };
                        let sw = Rect::from_center_size(sw_at, Vec2::splat(10.0));
                        let swr = ui.interact(sw.expand(4.0), ui.scope_id().with(("devon", i)), Sense::click());
                        ui.painter().circle(sw.center(), 4.5, if d.on() { ACCENT } else { gray(210) }, Stroke::new(1.0, gray(160)));
                        if swr.on_hover_text("Device on / off").clicked() {
                            d.set_on(!d.on());
                        }
                        if d.folded() {
                            // The name reads upwards, as Live's folded devices do.
                            let galley = ui.painter().layout_no_wrap(d.name().to_string(), FontId::proportional(12.0), name_col);
                            let at = Pos2::new(title.center().x - galley.size().y / 2.0, sw.bottom() + 10.0 + galley.size().x);
                            ui.painter().add(egui::epaint::TextShape::new(at, galley, name_col).with_angle(-std::f32::consts::FRAC_PI_2));
                        } else {
                            ui.painter().text(title.left_center() + Vec2::new(PAD + 16.0, 0.0), Align2::LEFT_CENTER, d.name(), FontId::proportional(12.0), name_col);
                        }
                        let x = Rect::from_center_size(x_at, Vec2::splat(16.0));
                        let xr = ui.interact(x, ui.scope_id().with(("devx", i)), Sense::click());
                        ui.painter().text(x.center(), Align2::CENTER_CENTER, "×", FontId::proportional(14.0), if xr.hovered() { ink() } else { muted() });
                        if xr.on_hover_text("Remove this effect").clicked() {
                            remove = Some(i);
                        }
                        _ = tr.on_hover_text(if d.folded() { "Double-click to unfold" } else { "Double-click to fold" });
                        if d.folded() {
                            continue;
                        }
                        let body = Rect::from_min_max(Pos2::new(panel.left() + PAD, title.bottom() + 8.0), panel.max - Vec2::new(PAD, 8.0));
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
                            Device::Tame(s) => tame_panel(&mut inner, s, tames[i], true),
                            Device::Opto(s) => opto_panel(&mut inner, s, optos[i], true),
                            Device::AutoFilter(s) => autofilter_panel(&mut inner, s, cutoffs[i], true),
                        };
                    }
                    // + Add Effect, as Live's "Drop Audio Effects Here".
                    let h = ui.available_height();
                    let (slot, resp) = ui.allocate_exact_size(Vec2::new(160.0, h), Sense::click());
                    ui.painter().rect_stroke(slot, 3.0, Stroke::new(1.0, if resp.hovered() { muted() } else { rule() }), egui::StrokeKind::Inside);
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
        // Folding only changes the look: saved with the project, but no undo step.
        let unfolded = |c: &[Device]| c.iter().map(|d| { let mut d = *d; d.set_folded(false); d }).collect::<Vec<_>>();
        if chain != before && remove.is_none() && unfolded(&chain) == unfolded(&before) {
            if let Some(tr) = self.dev_track_mut(t) {
                tr.devices = chain.clone();
            }
            self.dirty = true;
        } else if chain != before {
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
    style(ui);
    let mut active = false;
    ui.horizontal_top(|ui| {
        column(ui, "spacergrid", "", |ui| {
            label(ui, "Make room for");
            let current = others.iter().find(|o| o.0 == s.source).map_or("(choose a track)", |o| o.1.as_str());
            egui::ComboBox::from_id_salt("spacer_src").width(FIELD_W).selected_text(current).show_ui(ui, |ui| {
                for (i, n) in others {
                    ui.selectable_value(&mut s.source, *i, n);
                }
            });
            ui.end_row();
            label(ui, "Amount");
            active |= pct(ui, &mut s.amount);
            ui.end_row();
            label(ui, "Low / High Cut");
            ui.horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut s.low_cut).range(20.0..=2000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
                let b = ui.add(egui::DragValue::new(&mut s.high_cut).range(500.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
                active |= busy(&a) || busy(&b);
            });
            ui.end_row();
            label(ui, "Attack / Release");
            ui.horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut s.attack).range(0.5..=100.0).speed(0.2).suffix(" ms").fixed_decimals(1));
                let b = ui.add(egui::DragValue::new(&mut s.release).range(10.0..=1000.0).speed(2.0).suffix(" ms").fixed_decimals(0));
                active |= busy(&a) || busy(&b);
            });
            ui.end_row();
            label(ui, "Stereo");
            ui.horizontal(|ui| {
                for (i, n) in ["Linked", "L / R", "M / S"].iter().enumerate() {
                    ui.radio_value(&mut s.stereo, i as u8, *n);
                }
            });
            ui.end_row();
            label(ui, "Output");
            let w = ui.add(egui::Slider::new(&mut s.output, -12.0..=12.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
        });
        // The cut in each band while playing: 30 Hz to 18 kHz, 0 to -30 dB.
        let (r, _) = graph_rect(ui, GRAPH_W, None, Sense::hover());
        let p = ui.painter();
        let bw = r.width() / spacer::BANDS as f32;
        let rows = if s.stereo != 0 { 2 } else { 1 };
        for b in 0..spacer::BANDS {
            for row in 0..rows {
                let cut = cuts.map_or(0.0, |c| c[row][b]).clamp(0.0, 30.0);
                let w = (bw - 2.0) / rows as f32;
                let bar = Rect::from_min_size(Pos2::new(r.left() + b as f32 * bw + 1.0 + row as f32 * w, r.top() + 2.0), Vec2::new(w, cut / 30.0 * (r.height() - 4.0)));
                p.rect_filled(bar, 1.0, if row == 0 { gray(90) } else { gray(150) });
            }
        }
        let deepest = cuts.map_or(0.0, |c| c.iter().flatten().fold(0.0f32, |a, &v| a.max(v)));
        let text = if cuts.is_some() { format!("-{deepest:.1} dB") } else { "play to see the cuts".into() };
        note(p, r.center_bottom() - Vec2::new(0.0, 4.0), Align2::CENTER_BOTTOM, text);
        note(p, r.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, "cut");
    });
    active
}

/// Tame's controls, and beside them (in the device view, `live`) the cut it's making across
/// the spectrum while playing (`view`).
pub(super) fn tame_panel(ui: &mut egui::Ui, s: &mut tame::Settings, view: Option<([f32; tame::CURVE], [f32; tame::CURVE])>, live: bool) -> bool {
    style(ui);
    let mut active = false;
    ui.horizontal_top(|ui| {
        column(ui, "tamegrid", "", |ui| {
            label(ui, "Depth");
            active |= pct(ui, &mut s.depth);
            ui.end_row();
            label(ui, "Detail").on_hover_text("Low: broad, gentle cuts. High: only narrow, sharp resonances");
            active |= pct(ui, &mut s.detail);
            ui.end_row();
            label(ui, "Speed").on_hover_text("How fast the cuts come and go");
            active |= pct(ui, &mut s.speed);
            ui.end_row();
            label(ui, "Low / High");
            ui.horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut s.low).range(20.0..=5000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
                let b = ui.add(egui::DragValue::new(&mut s.high).range(200.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
                active |= busy(&a) || busy(&b);
            });
            ui.end_row();
            label(ui, "Mode");
            ui.horizontal(|ui| {
                ui.radio_value(&mut s.hard, false, "Soft").on_hover_text("Resonances at any level");
                ui.radio_value(&mut s.hard, true, "Hard").on_hover_text("Only resonances louder than the threshold");
                ui.checkbox(&mut s.delta, "Delta").on_hover_text("Hear only what's being taken away");
            });
            ui.end_row();
            if s.hard {
                label(ui, "Threshold");
                let w = ui.add(egui::Slider::new(&mut s.threshold, -60.0..=0.0).suffix(" dB").fixed_decimals(1));
                active |= busy(&w);
                ui.end_row();
            }
            label(ui, "Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        if !live {
            return;
        }
        // The cut across the spectrum, 20 Hz to 20 kHz (log): the level in grey, the cut hanging
        // from the top in black, 0 to -12 dB; outside Low-High shaded.
        let (r, _) = graph_rect(ui, GRAPH_W, None, Sense::hover());
        let p = ui.painter();
        let x_of = |hz: f32| r.left() + r.width() * ((hz.max(20.0) / 20.0).log10() / 3.0).clamp(0.0, 1.0);
        let (lo, hi) = (x_of(s.low), x_of(s.high));
        p.rect_filled(Rect::from_min_max(r.min, Pos2::new(lo, r.bottom())), 2.0, gray(230));
        p.rect_filled(Rect::from_min_max(Pos2::new(hi, r.top()), r.max), 2.0, gray(230));
        for hz in [100.0, 1000.0, 10000.0] {
            let x = x_of(hz);
            p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(1.0, gray(226)));
            let text = if hz >= 1000.0 { format!("{}k", hz as u32 / 1000) } else { format!("{}", hz as u32) };
            note(p, Pos2::new(x + 2.0, r.bottom() - 2.0), Align2::LEFT_BOTTOM, text);
        }
        let x_at = |j: usize| r.left() + r.width() * j as f32 / (tame::CURVE - 1) as f32;
        if let Some((curve, level)) = view {
            // Level: -90 to 0 dB up the height.
            let pts: Vec<Pos2> = (0..tame::CURVE).map(|j| Pos2::new(x_at(j), r.bottom() - r.height() * ((level[j] + 90.0) / 90.0).clamp(0.0, 1.0))).collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.0, gray(175))));
            let depth = |c: f32| r.top() + 2.0 + (c / tame::MAX_CUT).clamp(0.0, 1.0) * (r.height() * 0.6);
            let mut fill = vec![Pos2::new(x_at(0), r.top() + 2.0)];
            fill.extend((0..tame::CURVE).map(|j| Pos2::new(x_at(j), depth(curve[j]))));
            fill.push(Pos2::new(x_at(tame::CURVE - 1), r.top() + 2.0));
            for w in fill.windows(2) {
                let (a, b) = (w[0], w[1]);
                p.add(egui::Shape::convex_polygon(vec![Pos2::new(a.x, r.top() + 2.0), a, b, Pos2::new(b.x, r.top() + 2.0)], gray(140), Stroke::NONE));
            }
            p.add(egui::Shape::line(fill[1..fill.len() - 1].to_vec(), Stroke::new(1.5, gray(60))));
            let deepest = curve.iter().fold(0.0f32, |a, &v| a.max(v));
            note(p, r.right_bottom() - Vec2::new(4.0, 16.0), Align2::RIGHT_BOTTOM, format!("-{deepest:.1} dB"));
        } else {
            note(p, r.center(), Align2::CENTER_CENTER, "play to see the cuts");
        }
        note(p, r.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, "cut");
    });
    active
}

/// Opto's controls, and beside them (in the device view, `live`) its gain reduction meter.
pub(super) fn opto_panel(ui: &mut egui::Ui, s: &mut opto::Settings, gr: Option<f32>, live: bool) -> bool {
    style(ui);
    let mut active = false;
    ui.horizontal_top(|ui| {
        column(ui, "optogrid", "", |ui| {
            label(ui, "Peak Reduction").on_hover_text("How much it levels: turn up until the meter moves the way you want");
            active |= pct(ui, &mut s.peak_reduction);
            ui.end_row();
            label(ui, "Gain").on_hover_text("Make-up gain, after the levelling");
            let w = ui.add(egui::Slider::new(&mut s.gain, -12.0..=24.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            label(ui, "Mode");
            ui.horizontal(|ui| {
                ui.radio_value(&mut s.limit, false, "Compress").on_hover_text("Gentle, about 3:1");
                ui.radio_value(&mut s.limit, true, "Limit").on_hover_text("Firmer, about 12:1, with a harder knee");
            });
            ui.end_row();
            label(ui, "Colour").on_hover_text("Valve and transformer warmth after the levelling");
            active |= pct(ui, &mut s.colour);
            ui.end_row();
            label(ui, "Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        if !live {
            return;
        }
        // Gain reduction, 0 to -20 dB down the height, as the hardware's meter shows it.
        let (side, _) = ui.allocate_exact_size(Vec2::new(METER_W, ui.available_height()), Sense::hover());
        let p = ui.painter();
        let bar = Rect::from_min_max(Pos2::new(side.right() - 14.0, side.top() + 14.0), Pos2::new(side.right(), side.bottom() - 16.0));
        p.rect_filled(bar, 2.0, gray(238));
        let y_of = |db: f32| bar.top() + bar.height() * (db / 20.0).clamp(0.0, 1.0);
        for db in [0.0, 3.0, 6.0, 10.0, 20.0] {
            let y = y_of(db);
            p.line_segment([Pos2::new(bar.left() - 1.0, y), Pos2::new(bar.left() - 4.0, y)], Stroke::new(1.0, gray(180)));
            note(p, Pos2::new(bar.left() - 6.0, y), Align2::RIGHT_CENTER, -(db as i32));
        }
        if let Some(g) = gr {
            p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.right(), y_of(g))), 2.0, gray(90));
        }
        note(p, Pos2::new(bar.center().x, side.top()), Align2::CENTER_TOP, "GR");
        let text = gr.map_or("–".into(), |g| format!("-{g:.1}"));
        note(p, Pos2::new(bar.center().x, side.bottom()), Align2::CENTER_BOTTOM, text);
    });
    active
}

/// Auto Filter's controls, as Live's: the filter, its envelope follower, its envelope and its
/// LFO, its step sequencer, and its curve (drag it: across for Frequency, up and down for
/// Resonance); while playing (`view`), the cutoff where the modulation has it now and the step
/// it's on. `live` is the device view (the print dialog flows to its own height).
pub(super) fn autofilter_panel(ui: &mut egui::Ui, s: &mut autofilter::Settings, view: Option<(f32, Option<usize>)>, live: bool) -> bool {
    style(ui);
    let cutoff = view.map(|v| v.0);
    let mut active = false;
    let hz = |v: f64, _: std::ops::RangeInclusive<usize>| if v >= 1000.0 { format!("{:.2} kHz", v / 1000.0) } else { format!("{v:.0} Hz") };
    ui.horizontal_top(|ui| {
        column(ui, "afgrid", "Filter", |ui| {
            label(ui, "Circuit");
            let name = autofilter::CIRCUITS.get(s.circuit as usize).copied().unwrap_or("MS2");
            egui::ComboBox::from_id_salt("af_circuit").selected_text(name).show_ui(ui, |ui| {
                for (i, n) in autofilter::CIRCUITS.iter().enumerate() {
                    ui.selectable_value(&mut s.circuit, i as u8, *n).on_hover_text("After the Korg MS-20's filter: dirty, and screaming when pushed");
                }
            });
            ui.end_row();
            label(ui, "Type");
            ui.horizontal(|ui| {
                for (i, n) in autofilter::KINDS.iter().enumerate() {
                    ui.radio_value(&mut s.kind, i as u8, *n);
                }
            });
            ui.end_row();
            label(ui, "Frequency");
            let w = ui.add(egui::Slider::new(&mut s.freq, 20.0..=20000.0).logarithmic(true).custom_formatter(hz));
            active |= busy(&w);
            ui.end_row();
            label(ui, "Resonance").on_hover_text("Past 90% it oscillates on its own, and screams");
            active |= pct(ui, &mut s.res);
            ui.end_row();
            label(ui, "Drive").on_hover_text("Gain into the filter: pushed, the sound and the resonance fight and tear");
            let w = ui.add(egui::Slider::new(&mut s.drive, 0.0..=24.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            label(ui, "Output");
            let w = ui.add(egui::Slider::new(&mut s.output, -24.0..=24.0).suffix(" dB").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            label(ui, "Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        column(ui, "afgrid2", "Follower", |ui| {
            label(ui, "Amount").on_hover_text("How far the sound's own level moves the cutoff, up or down (±5 octaves)");
            active |= signed_pct(ui, &mut s.env_amount);
            ui.end_row();
            label(ui, "Attack");
            let w = ui.add(egui::Slider::new(&mut s.env_attack, 0.1..=1000.0).logarithmic(true).suffix(" ms").fixed_decimals(1));
            active |= busy(&w);
            ui.end_row();
            label(ui, "Release");
            let w = ui.add(egui::Slider::new(&mut s.env_release, 1.0..=5000.0).logarithmic(true).suffix(" ms").fixed_decimals(0));
            active |= busy(&w);
            ui.end_row();
        });
        column(ui, "afgrid3", "Envelope", |ui| {
            label(ui, "Amount").on_hover_text("An ADSR on the cutoff, up or down (±5 octaves)");
            active |= signed_pct(ui, &mut s.gen_amount);
            ui.end_row();
            label(ui, "Trigger");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let name = autofilter::TRIGGERS.get(s.gen_trigger as usize).copied().unwrap_or("Off");
                egui::ComboBox::from_id_salt("af_trig").width(64.0).selected_text(name).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.gen_trigger, 0, "Off");
                    ui.selectable_value(&mut s.gen_trigger, 1, "Sync").on_hover_text("On a grid of the song's beats (Drift moves it too)");
                    ui.selectable_value(&mut s.gen_trigger, 2, "Input").on_hover_text("On each hit in the sound, held while it stays loud");
                    ui.selectable_value(&mut s.gen_trigger, 3, "Seq").on_hover_text("On each gated step of the sequencer, held through slides");
                });
                if s.gen_trigger == 1 {
                    sync_rate(ui, "af_genrate", &mut s.gen_beats);
                }
            });
            ui.end_row();
            label(ui, "A / D");
            ui.horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut s.gen_attack).range(0.1..=5000.0).speed(1.0).suffix(" ms").fixed_decimals(1)).on_hover_text("Attack");
                let d = ui.add(egui::DragValue::new(&mut s.gen_decay).range(1.0..=10000.0).speed(4.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Decay");
                active |= busy(&a) || busy(&d);
            });
            ui.end_row();
            label(ui, "S / R");
            ui.horizontal(|ui| {
                let mut p = s.gen_sustain * 100.0;
                let a = ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).speed(0.5).suffix(" %").fixed_decimals(0)).on_hover_text("Sustain, while the gate is held");
                s.gen_sustain = p / 100.0;
                let r = ui.add(egui::DragValue::new(&mut s.gen_release).range(1.0..=10000.0).speed(4.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Release");
                active |= busy(&a) || busy(&r);
            });
            ui.end_row();
        });
        column(ui, "afgrid4", "LFO", |ui| {
            label(ui, "Amount").on_hover_text("How far the LFO swings the cutoff each way (up to 4 octaves)");
            active |= pct(ui, &mut s.lfo_amount);
            ui.end_row();
            label(ui, "Rate");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.selectable_value(&mut s.lfo_sync, false, "Hz");
                ui.selectable_value(&mut s.lfo_sync, true, "Sync").on_hover_text("In time with the song");
                if s.lfo_sync {
                    sync_rate(ui, "af_lforate", &mut s.lfo_beats);
                } else {
                    ui.spacing_mut().slider_width = 44.0;
                    let w = ui.add(egui::Slider::new(&mut s.lfo_rate, 0.01..=40.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
                    active |= busy(&w);
                }
            });
            ui.end_row();
            label(ui, "Shape");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let shape = tape::LFO_SHAPES.get(s.lfo_shape as usize).copied().unwrap_or("Sine");
                egui::ComboBox::from_id_salt("af_lfo").width(80.0).selected_text(shape).show_ui(ui, |ui| {
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
            label(ui, "Drift").on_hover_text("Lets the LFO wander out of time and back in: locked for two bars, drifting out, drifting back (in Hz: four seconds each)");
            active |= pct(ui, &mut s.lfo_drift);
            ui.end_row();
            label(ui, "Phase").on_hover_text("The right side's LFO ahead of the left's, for movement across the stereo field");
            let w = ui.add(egui::Slider::new(&mut s.lfo_phase, 0.0..=360.0).suffix("°").fixed_decimals(0));
            active |= busy(&w);
            ui.end_row();
        });
        active |= autofilter_seq(ui, s, view.and_then(|v| v.1), live);
        active |= autofilter_curve(ui, s, cutoff, live);
    });
    active
}

/// Auto Filter's step sequencer: a strip of steps (drag in the bars to draw values; click the
/// Gate, Slide and Accent rows to toggle them), then its settings in three rows. `playing` is
/// the step it's on.
fn autofilter_seq(ui: &mut egui::Ui, s: &mut autofilter::Settings, playing: Option<usize>, live: bool) -> bool {
    const ROW: f32 = 11.0;
    let mut active = false;
    ui.allocate_ui_with_layout(Vec2::new(SEQ_W, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        heading(ui, "Sequencer");
        let h = if live { (ui.available_height() - 3.0 * 26.0).clamp(54.0, 120.0) } else { 110.0 };
        let (r, resp) = ui.allocate_exact_size(Vec2::new(SEQ_W, h), Sense::click_and_drag());
        let bars = Rect::from_min_max(Pos2::new(r.left() + SEQ_LABEL, r.top()), Pos2::new(r.right(), r.bottom() - 3.0 * ROW - 2.0));
        let rows = |k: usize| Rect::from_min_max(Pos2::new(r.left() + SEQ_LABEL, bars.bottom() + 2.0 + ROW * k as f32), Pos2::new(r.right(), bars.bottom() + 2.0 + ROW * (k + 1) as f32));
        let len = (s.seq_len as usize).clamp(1, autofilter::STEPS);
        if let Some(pos) = resp.interact_pointer_pos() {
            let i = (((pos.x - bars.left()) / SEQ_STEP).floor().max(0.0) as usize).min(autofilter::STEPS - 1);
            // Drawing values: anywhere a drag passes over the bars.
            if (resp.dragged() || resp.clicked()) && bars.expand2(Vec2::new(0.0, 4.0)).contains(pos) {
                s.seq_steps[i] = (1.0 - 2.0 * (pos.y - bars.top()) / bars.height()).clamp(-1.0, 1.0);
                active = true;
            } else if resp.clicked() {
                for (k, mask) in [&mut s.seq_gate, &mut s.seq_slide, &mut s.seq_accent].into_iter().enumerate() {
                    if rows(k).contains(pos) {
                        *mask ^= 1 << i;
                    }
                }
            }
        }
        let resp = resp.on_hover_text("Drag in the bars to draw each step's cutoff. Below: G gate (off holds the step before), S slide (glides into the step), A accent (a kick of cutoff and resonance)");
        let p = ui.painter_at(r.expand(1.0));
        p.rect_filled(bars, 2.0, gray(238));
        p.line_segment([Pos2::new(bars.left(), bars.center().y), Pos2::new(bars.right(), bars.center().y)], Stroke::new(1.0, gray(218)));
        for (k, name) in ["G", "S", "A"].iter().enumerate() {
            p.text(Pos2::new(r.left() + SEQ_LABEL / 2.0, rows(k).center().y), Align2::CENTER_CENTER, *name, FontId::proportional(8.5), muted());
        }
        for i in 0..autofilter::STEPS {
            let x = bars.left() + SEQ_STEP * i as f32;
            let used = i < len;
            let gate = s.seq_gate >> i & 1 == 1;
            let col = if !used { gray(210) } else if gate { gray(80) } else { gray(170) };
            let v = s.seq_steps[i];
            let y = bars.center().y - v * bars.height() / 2.0;
            let bar = Rect::from_min_max(Pos2::new(x + 3.0, y.min(bars.center().y)), Pos2::new(x + SEQ_STEP - 3.0, y.max(bars.center().y) + 1.0));
            p.rect_filled(bar, 1.0, col);
            if playing == Some(i) {
                p.rect_stroke(Rect::from_min_max(Pos2::new(x + 1.0, bars.top() + 1.0), Pos2::new(x + SEQ_STEP - 1.0, bars.bottom() - 1.0)), 2.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
            }
            if i % 4 == 0 && i > 0 {
                p.line_segment([Pos2::new(x, bars.top()), Pos2::new(x, rows(2).bottom())], Stroke::new(1.0, gray(218)));
            }
            for (k, mask) in [s.seq_gate, s.seq_slide, s.seq_accent].into_iter().enumerate() {
                let cell = Rect::from_center_size(Pos2::new(x + SEQ_STEP / 2.0, rows(k).center().y), Vec2::splat(ROW - 3.0));
                if mask >> i & 1 == 1 {
                    p.rect_filled(cell, 1.5, if used { gray(80) } else { gray(190) });
                } else {
                    p.rect_stroke(cell, 1.5, Stroke::new(1.0, gray(190)), egui::StrokeKind::Inside);
                }
            }
        }
        _ = resp;
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.horizontal(|ui| {
            ui.toggle_value(&mut s.seq_on, "Seq").on_hover_text("Turn the step sequencer on");
            let mut p = s.seq_amount * 100.0;
            let w = ui.add(egui::DragValue::new(&mut p).range(-100.0..=100.0).speed(0.5).prefix("Amount ").suffix(" %").fixed_decimals(0)).on_hover_text("How far the steps move the cutoff, up or down (±4 octaves)");
            s.seq_amount = p / 100.0;
            active |= busy(&w);
            sync_rate(ui, "af_seqrate", &mut s.seq_beats);
        });
        ui.horizontal(|ui| {
            let mut n = s.seq_len as u32;
            let w = ui.add(egui::DragValue::new(&mut n).range(1..=autofilter::STEPS as u32).prefix("Length ")).on_hover_text("Steps used");
            s.seq_len = n as u8;
            active |= busy(&w);
            let w = ui.add(egui::DragValue::new(&mut s.seq_glide).range(1.0..=2000.0).speed(2.0).prefix("Glide ").suffix(" ms").fixed_decimals(0)).on_hover_text("How long a slide takes");
            active |= busy(&w);
            let dir = autofilter::DIRECTIONS.get(s.seq_dir as usize).copied().unwrap_or("Forward");
            egui::ComboBox::from_id_salt("af_seqdir").width(78.0).selected_text(dir).show_ui(ui, |ui| {
                for (i, n) in autofilter::DIRECTIONS.iter().enumerate() {
                    ui.selectable_value(&mut s.seq_dir, i as u8, *n);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.seq_drift, "Drift").on_hover_text("The LFO's Drift moves the steps in and out of time too");
            if ui.small_button("Randomise").on_hover_text("New values, and a few slides, accents and rests").clicked() {
                randomise_steps(s);
            }
            if ui.small_button("Clear").on_hover_text("All steps to the middle, gated, no slides or accents").clicked() {
                s.seq_steps = [0.0; autofilter::STEPS];
                s.seq_gate = u16::MAX;
                s.seq_slide = 0;
                s.seq_accent = 0;
            }
        });
    });
    active
}

/// New random step values, with about one in five sliding, one in four accented and one in eight resting.
fn randomise_steps(s: &mut autofilter::Settings) {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
    let mut rand = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    let (mut gate, mut slide, mut accent) = (0u16, 0u16, 0u16);
    for i in 0..autofilter::STEPS {
        s.seq_steps[i] = (rand() * 2.0 - 1.0).clamp(-1.0, 1.0);
        gate |= ((rand() > 0.125) as u16) << i;
        slide |= ((rand() < 0.2) as u16) << i;
        accent |= ((rand() < 0.25) as u16) << i;
    }
    (s.seq_gate, s.seq_slide, s.seq_accent) = (gate, slide, accent);
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
fn autofilter_curve(ui: &mut egui::Ui, s: &mut autofilter::Settings, cutoff: Option<f32>, live: bool) -> bool {
    let (r, resp) = if live {
        graph_rect(ui, GRAPH_W, Some("Response"), Sense::drag())
    } else {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(GRAPH_W, 140.0), Sense::drag());
        ui.painter().rect_filled(r, 2.0, gray(238));
        (r, resp)
    };
    if resp.dragged() {
        let d = resp.drag_delta();
        s.freq = (s.freq * (d.x / r.width() * 10.0).exp2()).clamp(20.0, 20000.0);
        s.res = (s.res - d.y / r.height()).clamp(0.0, 1.0);
    }
    let resp = resp.on_hover_text("Drag across for Frequency, up and down for Resonance");
    let p = ui.painter_at(r);
    let x_of = |f: f32| r.left() + r.width() * (f / 20.0).log2() / 1000f32.log2();
    let y_of = |db: f32| r.top() + r.height() * ((24.0 - db) / 60.0).clamp(0.0, 1.0);
    for f in [100.0, 1000.0, 10000.0] {
        p.line_segment([Pos2::new(x_of(f), r.top()), Pos2::new(x_of(f), r.bottom())], Stroke::new(1.0, gray(226)));
    }
    p.line_segment([Pos2::new(r.left(), y_of(0.0)), Pos2::new(r.right(), y_of(0.0))], Stroke::new(1.0, gray(218)));
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
            p.add(egui::Shape::line(line(fc), Stroke::new(1.5, gray(60))));
        }
        None => _ = p.add(egui::Shape::line(line(s.freq), Stroke::new(1.5, gray(60)))),
    }
    let at = cutoff.unwrap_or(s.freq);
    let text = if at >= 1000.0 { format!("{:.2} kHz", at / 1000.0) } else { format!("{at:.0} Hz") };
    note(&p, r.right_top() + Vec2::new(-5.0, 4.0), Align2::RIGHT_TOP, text);
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
    style(ui);
    let mut active = false;
    column(ui, "reverbgrid", "", |ui| {
        label(ui, "Size");
        active |= pct(ui, &mut s.size);
        ui.end_row();
        label(ui, "Damping");
        active |= pct(ui, &mut s.damping);
        ui.end_row();
        label(ui, "Width");
        active |= pct(ui, &mut s.width);
        ui.end_row();
        label(ui, "Pre-delay");
        let w = ui.add(egui::Slider::new(&mut s.predelay, 0.0..=200.0).suffix(" ms").fixed_decimals(0));
        active |= busy(&w);
        ui.end_row();
        label(ui, "Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// The note lengths Delay and Tape's echo times come in.
const TIMES: [(&str, f32); 8] = [("1/16", 0.25), ("1/8", 0.5), ("3/16", 0.75), ("1/4", 1.0), ("3/8", 1.5), ("1/2", 2.0), ("3/4", 3.0), ("1 bar", 4.0)];

/// An echo time in notes (synced to the tempo).
fn echo_time(ui: &mut egui::Ui, id: &str, beats: &mut f32) {
    let current = TIMES.iter().find(|t| (t.1 - *beats).abs() < 1e-3).map_or("custom", |t| t.0);
    egui::ComboBox::from_id_salt(id).selected_text(current).show_ui(ui, |ui| {
        for (n, b) in TIMES {
            ui.selectable_value(beats, b, n);
        }
    });
}

/// Delay's controls: the time in notes (synced to the tempo).
pub(super) fn delay_panel(ui: &mut egui::Ui, s: &mut fx::DelaySettings) -> bool {
    style(ui);
    let mut active = false;
    column(ui, "delaygrid", "", |ui| {
        label(ui, "Time");
        echo_time(ui, "delay_time", &mut s.beats);
        ui.end_row();
        label(ui, "Feedback");
        let mut p = s.feedback * 100.0;
        let w = ui.add(egui::Slider::new(&mut p, 0.0..=95.0).suffix(" %").fixed_decimals(0));
        s.feedback = p / 100.0;
        active |= busy(&w);
        ui.end_row();
        label(ui, "Ping Pong");
        ui.checkbox(&mut s.ping_pong, "");
        ui.end_row();
        label(ui, "Filter");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let a = ui.add(egui::DragValue::new(&mut s.low_cut).range(20.0..=2000.0).speed(2.0).suffix(" Hz").fixed_decimals(0));
            let b = ui.add(egui::DragValue::new(&mut s.high_cut).range(500.0..=20000.0).speed(20.0).suffix(" Hz").fixed_decimals(0));
            active |= busy(&a) || busy(&b);
        });
        ui.end_row();
        label(ui, "Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Spring's controls.
pub(super) fn spring_panel(ui: &mut egui::Ui, s: &mut fx::SpringSettings) -> bool {
    style(ui);
    let mut active = false;
    column(ui, "springgrid", "", |ui| {
        label(ui, "Length");
        let w = ui.add(egui::Slider::new(&mut s.length, 20.0..=120.0).suffix(" ms").fixed_decimals(0)).on_hover_text("Time between echoes");
        active |= busy(&w);
        ui.end_row();
        label(ui, "Decay");
        active |= pct(ui, &mut s.decay);
        ui.end_row();
        label(ui, "Tone");
        let w = ui.add(egui::Slider::new(&mut s.tone, 1500.0..=7000.0).logarithmic(true).suffix(" Hz").fixed_decimals(0)).on_hover_text("The spring's transition frequency: the boing sits below it (applies when you let go)");
        active |= busy(&w);
        ui.end_row();
        label(ui, "Chirp");
        active |= pct(ui, &mut s.chirp);
        ui.end_row();
        label(ui, "Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Tape's controls in three columns: the echo (time, feedback, wow, mix), the tape machine, and
/// the LFO on the delay time.
pub(super) fn tape_panel(ui: &mut egui::Ui, s: &mut tape::Settings) -> bool {
    const SPEEDS: [(&str, f32); 4] = [("3¾ ips", 3.75), ("7½ ips", 7.5), ("15 ips", 15.0), ("30 ips", 30.0)];
    style(ui);
    let mut active = false;
    ui.horizontal_top(|ui| {
        column(ui, "tapegrid", "Echo", |ui| {
            label(ui, "Time");
            echo_time(ui, "tape_time", &mut s.beats);
            ui.end_row();
            label(ui, "Feedback");
            let mut p = s.feedback * 100.0;
            let w = ui.add(egui::Slider::new(&mut p, 0.0..=105.0).suffix(" %").fixed_decimals(0)).on_hover_text("Past 100% the echoes run away into the tape's saturation");
            s.feedback = p / 100.0;
            active |= busy(&w);
            ui.end_row();
            label(ui, "Wow");
            active |= pct(ui, &mut s.wow);
            ui.end_row();
            label(ui, "Dry / Wet");
            active |= pct(ui, &mut s.mix);
            ui.end_row();
        });
        column(ui, "tapegrid2", "Tape", |ui| {
            label(ui, "Speed");
            let current = SPEEDS.iter().find(|t| (t.1 - s.speed).abs() < 1e-3).map_or("custom", |t| t.0);
            egui::ComboBox::from_id_salt("tape_speed").selected_text(current).show_ui(ui, |ui| {
                for (n, v) in SPEEDS {
                    ui.selectable_value(&mut s.speed, v, n);
                }
            }).response.on_hover_text("Slower tape loses more top");
            ui.end_row();
            label(ui, "Drive");
            active |= pct(ui, &mut s.drive);
            ui.end_row();
            label(ui, "Bias");
            let mut p = s.bias * 100.0;
            let w = ui.add(egui::Slider::new(&mut p, 0.0..=100.0).suffix(" %").fixed_decimals(0)).on_hover_text("Low leaves the tape's deadzone: quiet sounds crunch");
            s.bias = p / 100.0;
            active |= busy(&w);
            ui.end_row();
            label(ui, "Spacing");
            let w = ui.add(egui::Slider::new(&mut s.spacing, 0.0..=30.0).suffix(" µm").fixed_decimals(0)).on_hover_text("Play head to tape: further is darker");
            active |= busy(&w);
            ui.end_row();
        });
        column(ui, "tapegrid3", "LFO", |ui| {
            label(ui, "Shape");
            let shape = tape::LFO_SHAPES.get(s.lfo_shape as usize).copied().unwrap_or("Sine");
            egui::ComboBox::from_id_salt("tape_lfo").selected_text(shape).show_ui(ui, |ui| {
                for (i, n) in tape::LFO_SHAPES.iter().enumerate() {
                    ui.selectable_value(&mut s.lfo_shape, i as u8, *n);
                }
            });
            ui.end_row();
            label(ui, "Rate");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().slider_width = 40.0;
                let unit = if s.lfo_slow { " cHz" } else { " Hz" };
                let w = ui.add(egui::Slider::new(&mut s.lfo_rate, 0.01..=20.0).logarithmic(true).suffix(unit).fixed_decimals(2));
                active |= busy(&w);
                ui.checkbox(&mut s.lfo_slow, "×0.01").on_hover_text("A hundredth of the rate, for very slow drifts (cHz: hundredths of a hertz)");
            });
            ui.end_row();
            label(ui, "Depth");
            let w = ui.add(egui::Slider::new(&mut s.lfo_depth, 0.0..=50.0).suffix(" ms").fixed_decimals(1)).on_hover_text("How far the delay time swings each way");
            active |= busy(&w);
            ui.end_row();
            label(ui, "Slew");
            active |= pct(ui, &mut s.lfo_slew);
            ui.end_row();
        });
    });
    active
}

/// Phaser's controls: the pedal's speed knob, and how much of it is heard.
pub(super) fn phaser_panel(ui: &mut egui::Ui, s: &mut phaser::Settings) -> bool {
    style(ui);
    let mut active = false;
    column(ui, "phasergrid", "", |ui| {
        label(ui, "Speed");
        let w = ui.add(egui::Slider::new(&mut s.speed, 0.05..=10.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
        active |= busy(&w);
        ui.end_row();
        label(ui, "Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
    });
    active
}

/// Chorus's controls: the mode (which sets its rate), rate, amount, dry/wet, and hiss (Tape: drive).
pub(super) fn chorus_panel(ui: &mut egui::Ui, s: &mut chorus::Settings) -> bool {
    style(ui);
    let mut active = false;
    column(ui, "chorusgrid", "", |ui| {
        label(ui, "Mode");
        let mut m = s.mode;
        egui::ComboBox::from_id_salt("chorus_mode").selected_text(chorus::MODES.get(m as usize).copied().unwrap_or("Juno I")).show_ui(ui, |ui| {
            for (i, n) in chorus::MODES.iter().enumerate() {
                ui.selectable_value(&mut m, i as u8, *n);
            }
        });
        if m != s.mode {
            s.set_mode(m);
        }
        ui.end_row();
        label(ui, "Rate");
        let w = ui.add(egui::Slider::new(&mut s.rate, 0.01..=20.0).logarithmic(true).suffix(" Hz").fixed_decimals(2));
        active |= busy(&w);
        ui.end_row();
        label(ui, "Amount");
        let mut p = s.amount * 100.0;
        let w = ui.add(egui::Slider::new(&mut p, 0.0..=200.0).suffix(" %").fixed_decimals(0)).on_hover_text("Depth: 100% is the original's");
        s.amount = p / 100.0;
        active |= busy(&w);
        ui.end_row();
        label(ui, "Dry / Wet");
        active |= pct(ui, &mut s.mix);
        ui.end_row();
        if chorus::MODES.get(s.mode as usize) != Some(&"Clean") {
            label(ui, "Noise");
            active |= pct(ui, &mut s.noise);
            ui.end_row();
        }
        if chorus::MODES.get(s.mode as usize) == Some(&"Tape") {
            label(ui, "Drive");
            active |= pct(ui, &mut s.drive);
            ui.end_row();
        }
    });
    active
}

/// Pultec's front panel: the low section (frequency, boost, attenuation), then the high
/// boost (frequency, boost, bandwidth) and high attenuation (selector, amount). Knobs go 0 to 10.
pub(super) fn pultec_panel(ui: &mut egui::Ui, s: &mut pultec::Settings) -> bool {
    style(ui);
    let mut active = false;
    let knob = |ui: &mut egui::Ui, v: &mut f32| busy(&ui.add(egui::Slider::new(v, 0.0..=10.0).fixed_decimals(1)));
    let pick = |ui: &mut egui::Ui, id: &str, v: &mut f32, options: &[f32], unit: &str| {
        egui::ComboBox::from_id_salt(id).selected_text(format!("{} {unit}", *v)).show_ui(ui, |ui| {
            for &o in options {
                ui.selectable_value(v, o, format!("{o} {unit}"));
            }
        });
    };
    ui.horizontal_top(|ui| {
        column(ui, "pultec_low", "Low", |ui| {
            label(ui, "Frequency");
            pick(ui, "pultec_lf", &mut s.low_freq, &pultec::LOW_FREQS, "Hz");
            ui.end_row();
            label(ui, "Boost");
            active |= knob(ui, &mut s.low_boost);
            ui.end_row();
            label(ui, "Atten");
            active |= knob(ui, &mut s.low_atten);
            ui.end_row();
        });
        column(ui, "pultec_high", "High", |ui| {
            label(ui, "Frequency");
            pick(ui, "pultec_hf", &mut s.high_freq, &pultec::HIGH_FREQS, "kHz");
            ui.end_row();
            label(ui, "Boost");
            active |= knob(ui, &mut s.high_boost);
            ui.end_row();
            label(ui, "Bandwidth");
            active |= knob(ui, &mut s.bandwidth);
            ui.end_row();
            label(ui, "Atten Sel");
            pick(ui, "pultec_as", &mut s.atten_sel, &pultec::ATTEN_SELS, "kHz");
            ui.end_row();
            label(ui, "Atten");
            active |= knob(ui, &mut s.high_atten);
            ui.end_row();
        });
    });
    active
}

/// Riser's one big knob (drag up and down; double-click for none), and, printed offline, whether
/// it rises across the selection.
pub(super) fn riser_panel(ui: &mut egui::Ui, s: &mut riser::Settings, offline: bool) -> bool {
    style(ui);
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.vertical_centered(|ui| riser_knob(ui, s, offline)).inner
}

fn riser_knob(ui: &mut egui::Ui, s: &mut riser::Settings, offline: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(KNOB), Sense::click_and_drag());
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
    let rad = KNOB * 0.42;
    // 300° of travel, from lower left round to lower right.
    let (a0, a1) = (std::f32::consts::PI * 0.75, std::f32::consts::PI * 2.25);
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 48;
        (0..=n).map(|i| { let a = from + (to - from) * i as f32 / n as f32; c + Vec2::angled(a) * rad }).collect()
    };
    p.add(egui::Shape::line(arc(a0, a1), Stroke::new(4.0, gray(222))));
    let at = a0 + (a1 - a0) * s.amount;
    if s.amount > 0.0 {
        p.add(egui::Shape::line(arc(a0, at), Stroke::new(4.0, ACCENT)));
    }
    p.circle_filled(c, rad - 7.0, gray(if resp.hovered() || resp.dragged() { 236 } else { 242 }));
    p.line_segment([c + Vec2::angled(at) * (rad - 20.0), c + Vec2::angled(at) * (rad - 8.0)], Stroke::new(2.0, gray(60)));
    p.text(c, Align2::CENTER_CENTER, format!("{:.0} %", s.amount * 100.0), FontId::proportional(12.0), ink());
    resp.on_hover_text("Amount: drag up to build. Shift-drag for fine, double-click for none");
    label(ui, "Amount");
    if offline {
        ui.add_space(6.0);
        ui.checkbox(&mut s.ramp, "Rise from nothing to Amount across it");
    }
    active
}
