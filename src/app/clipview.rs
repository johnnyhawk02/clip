//! The Clip View, as in Live: the selected clip's settings (warp and its tempo, loop, start
//! and length, volume, reverse) down the left of the detail view, instead of in menus.

use super::*;

impl App {
    /// The clip the Clip View shows: one selected clip, with no time selection.
    pub(super) fn clip_view_clip(&self) -> Option<(usize, usize)> {
        let p = self.sel_clip.filter(|_| self.sel_extra.is_empty() && self.time_sel.is_none())?;
        self.clip(p).map(|_| p)
    }

    /// Draw the Clip View in `r` and apply what's changed (one undo step per drag or typed value).
    pub(super) fn clip_view(&mut self, ui: &mut egui::Ui, r: Rect) {
        let Some((t, ci)) = self.clip_view_clip() else { return };
        ui.painter().rect_filled(r, 0.0, PANEL);
        let c = self.tracks[t].clips[ci].clone();
        let bf = self.beat_frames().max(1.0);
        let bpb = self.beats_per_bar.max(1) as f64;
        let (mut start, mut length) = (c.trim as f64 / bf, c.len as f64 / bf);
        let mut bpm = c.warp.as_ref().map(|w| w.bpm);
        let mut gain_db = 20.0 * c.gain.max(1e-4).log10();
        let mut toggle = None;
        let mut active = false;
        // Which values the person changed this frame (only those are applied, so rounding never
        // nudges the others).
        let (mut start_set, mut length_set, mut gain_set, mut bpm_set) = (false, false, false, false);
        let inner = r.shrink2(Vec2::new(12.0, 10.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 3.0);
                let small = |ui: &mut egui::Ui, s: &str| ui.label(egui::RichText::new(s).size(11.0).color(MUTED));
                ui.add(egui::Label::new(egui::RichText::new(&c.name).strong().color(INK)).truncate());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                        if ui.add(Button::new("Warp").selected(c.warp.is_some())).on_hover_text("Follow the song tempo").clicked() {
                            toggle = Some(Action::WarpClip);
                        }
                        if let Some(b) = bpm.as_mut() {
                            let resp = ui.add_sized([86.0, ui.spacing().interact_size.y], egui::DragValue::new(b).range(20.0..=400.0).speed(0.05).fixed_decimals(2).suffix(" BPM"));
                            active |= resp.dragged() || resp.has_focus();
                            bpm_set = resp.changed();
                            resp.on_hover_text("The clip's own tempo");
                        }
                });
                ui.horizontal(|ui| {
                    if ui.add(Button::new("Loop").selected(c.looping)).on_hover_text("Repeat the clip as you drag its end out").clicked() {
                        toggle = Some(Action::LoopClip);
                    }
                    if ui.button("Reverse").clicked() {
                        toggle = Some(Action::ReverseClip);
                    }
                });
                // Beats shown as bars.beats.sixteenths, counted from 1 for a position (as Live).
                let fmt = move |b: f64, from_one: bool| {
                    let n = (b * 4.0).round().max(0.0) as i64;
                    let (per_bar, k) = ((bpb * 4.0) as i64, from_one as i64);
                    format!("{}.{}.{}", n / per_bar + k, (n / 4) % bpb as i64 + k, n % 4 + k)
                };
                // Typed values: bars.beats.sixteenths, counted from 1 for a position.
                let parse = move |s: &str, from_one: bool| -> Option<f64> {
                    let parts: Vec<f64> = s.trim().split('.').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
                    let off = if from_one { 1.0 } else { 0.0 };
                    let get = |i: usize| parts.get(i).map_or(0.0, |v| v - off);
                    Some(get(0) * bpb + get(1) + get(2) / 4.0)
                };
                ui.horizontal(|ui| {
                    for (label, value, is_pos) in [("Start", &mut start, true), ("Length", &mut length, false)] {
                        ui.vertical(|ui| {
                            small(ui, label);
                            let resp = ui.add_sized(
                                [72.0, ui.spacing().interact_size.y],
                                egui::DragValue::new(value).speed(0.05).range(0.0..=100_000.0).custom_formatter(move |v, _| fmt(v, is_pos)).custom_parser(move |s| parse(s, is_pos)),
                            );
                            active |= resp.dragged() || resp.has_focus();
                            if resp.changed() {
                                if is_pos { start_set = true } else { length_set = true }
                            }
                        });
                    }
                });
                ui.horizontal(|ui| {
                    small(ui, "Gain");
                    ui.spacing_mut().slider_width = 130.0;
                    let resp = ui.add(egui::Slider::new(&mut gain_db, -24.0..=12.0).show_value(false)).on_hover_text(format!("{gain_db:+.1} dB (double-click for 0 dB)"));
                    if resp.double_clicked() {
                        gain_db = 0.0;
                    }
                    gain_set = resp.changed() || resp.double_clicked();
                    active |= resp.dragged();
                });
            });
        });
        if let Some(a) = toggle {
            let ctx = ui.ctx().clone();
            self.run(a, &ctx);
            return;
        }
        if start_set || length_set || gain_set || bpm_set {
            if !self.clip_view_edit {
                self.push_undo("Clip Settings");
                self.clip_view_edit = true;
            }
            let clip = &mut self.tracks[t].clips[ci];
            if let (true, Some(w), Some(b)) = (bpm_set, clip.warp.as_mut(), bpm) {
                // A new clip tempo: the clip plays faster or slower against the song.
                w.bpm = b;
            }
            let (_, _, _, frames) = clip.picture();
            if start_set {
                let trim = (start * bf).round() as usize;
                clip.trim = if clip.looping { trim } else { trim.min(frames.saturating_sub(1)) };
            }
            if start_set || length_set || bpm_set {
                let len = if length_set { ((length * bf).round() as usize).max(1) } else { clip.len };
                clip.len = if clip.looping { len } else { len.min(frames - clip.trim).max(1) };
            }
            if gain_set {
                clip.gain = 10f32.powf(gain_db / 20.0);
            }
            self.dirty = true;
            self.refresh_playback();
            if bpm_set {
                self.stretch_in_background();
            }
        }
        if !active {
            self.clip_view_edit = false;
        }
    }
}
