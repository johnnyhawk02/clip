//! The device view, as in Live: the selected track's built-in effects in a row along the
//! bottom (click a track's name to see it), each with its on/off switch, settings and remove
//! button, and + Add Effect at the end.

use super::*;
use crate::devices::{self, Device};
use crate::fx;

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
        let mut remove = None;
        let mut add = None;
        let mut active = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(8.0)), |ui| {
            egui::ScrollArea::horizontal().id_salt("devices").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (i, d) in chain.iter_mut().enumerate() {
                        let h = ui.available_height();
                        let (panel, _) = ui.allocate_exact_size(Vec2::new(DEV_W, h), Sense::hover());
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
        if cuts.iter().any(|c| c.is_some()) {
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
fn reverb_panel(ui: &mut egui::Ui, s: &mut fx::ReverbSettings) -> bool {
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
fn delay_panel(ui: &mut egui::Ui, s: &mut fx::DelaySettings) -> bool {
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
fn spring_panel(ui: &mut egui::Ui, s: &mut fx::SpringSettings) -> bool {
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
