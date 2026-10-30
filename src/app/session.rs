//! The Session view, as in Live: a grid of clip slots, one column per track and one row per
//! scene, for loops to launch. Tab switches between it and the song view.

use super::*;

/// Clip indices from here up mean a Session slot (`SLOT + scene`), so a slot's clip can be
/// edited like any other.
pub(super) const SLOT: usize = 1 << 30;

/// Width of a track's column (with the 1 px gap between columns).
const COL_W: f32 = 116.0;
/// Height of a clip slot (with the gap between rows).
const ROW_H: f32 = 23.0;
/// Height of the track title row.
const HEAD_H: f32 = 22.0;
/// Width of the Main column on the right (scene launch buttons).
const SCENE_W: f32 = 150.0;
/// Height of the mixer along the bottom.
const MIX_H: f32 = 196.0;

/// Live's track activator and Solo colours.
const ACTIVATOR: Color32 = Color32::from_rgb(255, 173, 86);
const SOLO: Color32 = Color32::from_rgb(71, 184, 224);
const PLAYING: Color32 = Color32::from_rgb(40, 200, 60);
const DARK_TEXT: Color32 = Color32::from_gray(10);

/// Fader position (0..1) for a linear gain (0..2, +6 dB at the top), and back.
fn fader_pos(g: f32) -> f32 {
    (g / 2.0).max(0.0).sqrt()
}
fn fader_gain(p: f32) -> f32 {
    2.0 * p.clamp(0.0, 1.0).powi(2)
}

/// A gain in dB as Live writes it.
fn db_text(g: f32) -> String {
    if g < 1e-4 { "-inf".into() } else { format!("{:.1}", 20.0 * g.log10()) }
}

/// A vertical level meter, lit from the bottom on a -60..+6 dB scale.
fn vmeter(p: &egui::Painter, r: Rect, level: f32) {
    p.rect_filled(r, 0.0, gray(205));
    let db = 20.0 * level.max(1e-6).log10();
    let t = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
    if t > 0.0 {
        let lit = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - r.height() * t), r.max);
        let c = if db > 0.0 { Color32::from_rgb(230, 70, 50) } else if db > -6.0 { Color32::from_rgb(230, 200, 60) } else { PLAYING };
        p.rect_filled(lit, 0.0, c);
    }
}

/// A vertical fader for a gain (0..2) with its meter beside it, as in Live's mixer. Drag to
/// change, double-click for 0 dB. Returns (changed, released).
fn fader(ui: &mut egui::Ui, r: Rect, id: egui::Id, gain: &mut f32, meters: &[f32]) -> (bool, bool) {
    let p = ui.painter();
    let mw = 4.0;
    for (k, m) in meters.iter().enumerate() {
        vmeter(p, Rect::from_min_size(Pos2::new(r.left() + k as f32 * (mw + 1.0), r.top()), Vec2::new(mw, r.height())), *m);
    }
    let rail_x = r.left() + meters.len() as f32 * (mw + 1.0) + 10.0;
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
fn pan_knob(ui: &mut egui::Ui, c: Pos2, id: egui::Id, pan: &mut f32) -> bool {
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
    let text = match *pan {
        v if v.abs() < 0.005 => "C".to_string(),
        v if v < 0.0 => format!("{:.0}L", -v * 50.0),
        v => format!("{:.0}R", v * 50.0),
    };
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp.on_hover_text(format!("Pan {text} (double-click for centre)"));
    changed
}

/// What a track plays from the Session view: whether it has taken the track over from the song
/// view, and its launched clips as (slot, from, to) in song frames (`to` None while it plays on).
#[derive(Clone, Default)]
pub(super) struct TrackPlay {
    pub(super) owned: bool,
    pub(super) segments: Vec<(usize, usize, Option<usize>)>,
}

/// What a Session slot carries while it's dragged to another slot.
#[derive(Clone, Copy)]
pub(super) struct SlotDrag(pub(super) usize, pub(super) usize);

/// A clip made ready for a slot: it loops its whole audio, from the start.
pub(super) fn slot_clip(mut c: Clip) -> Clip {
    let (_, _, _, frames) = c.picture();
    (c.start, c.trim, c.looping) = (0, 0, true);
    c.loop_points = None;
    c.len = frames.max(1);
    c
}

impl App {
    /// Every track has one slot per scene (and a Session play state).
    pub(super) fn fit_slots(&mut self) {
        let n = self.scenes.len();
        for t in &mut self.tracks {
            t.slots.resize_with(n, || None);
        }
        self.session_play.resize_with(self.tracks.len(), TrackPlay::default);
    }

    /// The Session view, laid out as Live's: track columns of clip slots under coloured title
    /// bars, the Main column of scene launch buttons on the right, the clip stop buttons, and the
    /// mixer along the bottom (meters, faders, pan, activator and Solo).
    pub(super) fn session(&mut self, ui: &mut egui::Ui) {
        self.fit_slots();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, gray(236));
        let n_tracks = self.tracks.len();
        let n_scenes = self.scenes.len();
        let width = n_tracks as f32 * COL_W + SCENE_W + 16.0;
        let mut changed = false;
        let mut released = false;
        egui::ScrollArea::horizontal().id_salt("session_h").auto_shrink([false, false]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(Vec2::new(width.max(full.width()), full.height()), Sense::hover());
            let x0 = area.left() + 6.0;
            let col_x = |t: usize| x0 + t as f32 * COL_W;
            let main_x = col_x(n_tracks) + 4.0;
            let top = area.top() + 4.0;
            // With little room (the detail view open below), the mixer keeps only its buttons.
            let compact = area.height() < MIX_H + HEAD_H + 6.0 * ROW_H;
            let mix_top = area.bottom() - if compact { 30.0 } else { MIX_H };
            let stop_top = mix_top - ROW_H - 2.0;
            let p = ui.painter().clone();

            // Title bars: each track's name on its colour (click to select the track).
            for t in 0..n_tracks {
                let head = Rect::from_min_size(Pos2::new(col_x(t), top), Vec2::new(COL_W - 1.0, HEAD_H));
                p.rect_filled(head, 0.0, live_color(self.tracks[t].color));
                let mut job = egui::text::LayoutJob::simple_singleline(self.tracks[t].name.clone(), FontId::proportional(11.5), DARK_TEXT);
                job.wrap = egui::text::TextWrapping { max_width: head.width() - 10.0, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let g = p.ctx().fonts_mut(|f| f.layout_job(job));
                p.galley(Pos2::new(head.left() + 5.0, head.center().y - g.size().y / 2.0), g, DARK_TEXT);
                if t == self.active {
                    p.rect_stroke(head, 0.0, Stroke::new(1.5, ink()), egui::StrokeKind::Inside);
                }
                if ui.interact(head, ui.scope_id().with(("shead", t)), Sense::click()).clicked() {
                    self.set_active(t);
                }
            }
            let main_head = Rect::from_min_size(Pos2::new(main_x, top), Vec2::new(SCENE_W, HEAD_H));
            p.rect_filled(main_head, 0.0, gray(200));
            p.text(main_head.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, "Main", FontId::proportional(11.5), ink());

            // The slots and scenes, scrolling up and down between the title bars and the stops.
            let grid = Rect::from_min_max(Pos2::new(area.left(), top + HEAD_H + 1.0), Pos2::new(area.right(), stop_top - 2.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(grid), |ui| {
                egui::ScrollArea::vertical().id_salt("session_v").auto_shrink([false, false]).show(ui, |ui| {
                    let (g, _) = ui.allocate_exact_size(Vec2::new(grid.width(), (n_scenes + 1) as f32 * ROW_H + 4.0), Sense::hover());
                    let row_y = |s: usize| g.top() + s as f32 * ROW_H;
                    for t in 0..n_tracks {
                        let color = live_color(self.tracks[t].color);
                        for s in 0..n_scenes {
                            let r = Rect::from_min_size(Pos2::new(col_x(t), row_y(s)), Vec2::new(COL_W - 1.0, ROW_H - 1.0));
                            self.draw_slot(ui, r, t, s, color);
                        }
                    }
                    // Scene launch buttons: green while any of the scene's clips plays.
                    for s in 0..n_scenes {
                        let r = Rect::from_min_size(Pos2::new(main_x, row_y(s)), Vec2::new(SCENE_W, ROW_H - 1.0));
                        let resp = ui.interact(r, ui.scope_id().with(("scene", s)), Sense::click());
                        let on = (0..n_tracks).any(|t| self.slot_state(t).0 == Some(s));
                        ui.painter().rect_filled(r, 0.0, if resp.hovered() { gray(205) } else { gray(218) });
                        let tri = Rect::from_min_size(r.min, Vec2::new(22.0, r.height()));
                        play_triangle(ui.painter(), tri.center(), if on { PLAYING } else { ink() });
                        ui.painter().text(tri.right_center() + Vec2::new(2.0, 0.0), Align2::LEFT_CENTER, &self.scenes[s], FontId::proportional(11.5), ink());
                        if resp.on_hover_text("Launch every clip in this scene").clicked() {
                            self.launch_scene(s);
                        }
                    }
                    let add = Rect::from_min_size(Pos2::new(main_x, row_y(n_scenes)), Vec2::new(SCENE_W, ROW_H - 1.0));
                    let add_r = ui.interact(add, ui.scope_id().with("addscene"), Sense::click());
                    ui.painter().rect_stroke(add, 0.0, Stroke::new(1.0, if add_r.hovered() { ink() } else { gray(200) }), egui::StrokeKind::Inside);
                    ui.painter().text(add.center(), Align2::CENTER_CENTER, "+ Scene", FontId::proportional(11.0), muted());
                    if add_r.on_hover_text("Add a scene (a row of slots)").clicked() {
                        self.push_undo("Add Scene");
                        let n = self.scenes.len() + 1;
                        self.scenes.push(n.to_string());
                        self.fit_slots();
                        self.dirty = true;
                    }
                });
            });

            // Clip stop buttons, and Stop All Clips under the scenes.
            let p = ui.painter().clone();
            for t in 0..n_tracks {
                let stop = Rect::from_min_size(Pos2::new(col_x(t), stop_top), Vec2::new(COL_W - 1.0, ROW_H - 1.0));
                let r = ui.interact(stop, ui.scope_id().with(("sstop", t)), Sense::click());
                p.rect_filled(stop, 0.0, if r.hovered() { gray(212) } else { gray(224) });
                let playing = self.slot_state(t).0.is_some();
                p.rect_filled(Rect::from_center_size(stop.center(), Vec2::splat(8.0)), 0.0, if playing { ink() } else { gray(150) });
                if r.on_hover_text("Stop this track's clip").clicked() {
                    self.stop_track(t);
                }
            }
            let stop_all = Rect::from_min_size(Pos2::new(main_x, stop_top), Vec2::new(SCENE_W, ROW_H - 1.0));
            let r = ui.interact(stop_all, ui.scope_id().with("stopall"), Sense::click());
            p.rect_filled(stop_all, 0.0, if r.hovered() { gray(205) } else { gray(218) });
            p.rect_filled(Rect::from_center_size(stop_all.left_center() + Vec2::new(11.0, 0.0), Vec2::splat(8.0)), 0.0, ink());
            p.text(stop_all.left_center() + Vec2::new(24.0, 0.0), Align2::LEFT_CENTER, "Stop All Clips", FontId::proportional(11.5), ink());
            if r.clicked() {
                self.stop_all_clips();
            }

            // The mixer: pan knob, meter and fader with its dB, then the activator and Solo.
            p.rect_filled(Rect::from_min_max(Pos2::new(area.left(), mix_top), area.max), 0.0, panel());
            p.line_segment([Pos2::new(area.left(), mix_top), Pos2::new(area.right(), mix_top)], Stroke::new(1.0, rule()));
            for t in 0..n_tracks {
                let c = Rect::from_min_max(Pos2::new(col_x(t), mix_top + 6.0), Pos2::new(col_x(t) + COL_W - 1.0, area.bottom() - 6.0));
                if t > 0 {
                    p.line_segment([Pos2::new(c.left() - 1.0, c.top()), Pos2::new(c.left() - 1.0, c.bottom())], Stroke::new(1.0, rule()));
                }
                let level = self.track_meter.get(t).copied().unwrap_or(0.0);
                let tr = &mut self.tracks[t];
                if !compact {
                    changed |= pan_knob(ui, Pos2::new(c.center().x, c.top() + 16.0), ui.scope_id().with(("span", t)), &mut tr.pan);
                    let fr = Rect::from_min_max(Pos2::new(c.center().x - 22.0, c.top() + 38.0), Pos2::new(c.center().x + 22.0, c.bottom() - 44.0));
                    let (ch, rel) = fader(ui, fr, ui.scope_id().with(("sfader", t)), &mut tr.gain, &[level]);
                    changed |= ch;
                    released |= rel;
                    ui.painter().text(Pos2::new(c.center().x, fr.bottom() + 6.0), Align2::CENTER_TOP, db_text(tr.gain), FontId::proportional(10.5), muted());
                }
                // Track activator (its number, lit while on) and Solo.
                let bw = 30.0;
                let act = Rect::from_min_size(Pos2::new(c.center().x - bw - 2.0, c.bottom() - 18.0), Vec2::new(bw, 18.0));
                let solo = Rect::from_min_size(Pos2::new(c.center().x + 2.0, c.bottom() - 18.0), Vec2::new(bw, 18.0));
                let ar = ui.interact(act, ui.scope_id().with(("sact", t)), Sense::click());
                let sr = ui.interact(solo, ui.scope_id().with(("ssolo", t)), Sense::click());
                let p = ui.painter();
                let on = !tr.mute;
                p.rect(act, 2.0, if on { ACTIVATOR } else { gray(225) }, Stroke::new(1.0, gray(170)), egui::StrokeKind::Inside);
                p.text(act.center(), Align2::CENTER_CENTER, (t + 1).to_string(), FontId::proportional(11.0), if on { DARK_TEXT } else { muted() });
                p.rect(solo, 2.0, if tr.solo { SOLO } else { gray(225) }, Stroke::new(1.0, gray(170)), egui::StrokeKind::Inside);
                p.text(solo.center(), Align2::CENTER_CENTER, "S", FontId::proportional(11.0), if tr.solo { DARK_TEXT } else { muted() });
                if ar.on_hover_text("Track activator: on or muted").clicked() {
                    tr.mute = !tr.mute;
                    changed = true;
                    released = true;
                }
                if sr.on_hover_text("Solo").clicked() {
                    tr.solo = !tr.solo;
                    changed = true;
                    released = true;
                }
            }
            // The Main track's fader, with the output meters.
            if compact {
                return;
            }
            let c = Rect::from_min_max(Pos2::new(main_x, mix_top + 6.0), Pos2::new(main_x + SCENE_W, area.bottom() - 6.0));
            let fr = Rect::from_min_max(Pos2::new(c.center().x - 22.0, c.top() + 38.0), Pos2::new(c.center().x + 22.0, c.bottom() - 44.0));
            let meters = [self.meter.first().copied().unwrap_or(0.0), self.meter.get(1).copied().unwrap_or(0.0)];
            let (ch, rel) = fader(ui, fr, ui.scope_id().with("smain"), &mut self.master_gain, &meters);
            changed |= ch;
            released |= rel;
            ui.painter().text(Pos2::new(c.center().x, fr.bottom() + 6.0), Align2::CENTER_TOP, db_text(self.master_gain), FontId::proportional(10.5), muted());
            ui.painter().text(Pos2::new(c.center().x, c.bottom() - 9.0), Align2::CENTER_CENTER, "Main", FontId::proportional(11.0), muted());
        });
        if changed {
            self.refresh_playback();
        }
        if released {
            self.dirty = true;
        }
    }

    /// One slot: empty, or a clip in its track's colour with a play triangle. Click to select,
    /// drag to move (⌥ copies), and loops from the pool or another slot drop in.
    fn draw_slot(&mut self, ui: &mut egui::Ui, r: Rect, t: usize, s: usize, color: Color32) {
        let id = ui.scope_id().with(("slot", t, s));
        self.slot_rects.push(((t, s), r));
        let selected = self.sel_slot == Some((t, s));
        let (on, queued) = self.slot_state(t);
        let playing = on == Some(s);
        // A launched clip waiting for its bar blinks, as in Live.
        let waiting = queued == Some(s) && (ui.input(|i| i.time) * 4.0) as i64 % 2 == 0;
        if queued.is_some() {
            ui.ctx().request_repaint();
        }
        let clip_name = self.tracks[t].slots[s].as_ref().map(|c| c.name.clone());
        let color = self.tracks[t].slots[s].as_ref().and_then(|c| c.color).map_or(color, live_color);
        // A filled slot can be dragged to another; any slot can be clicked.
        if clip_name.is_some() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                ui.dnd_drag_source(id, SlotDrag(t, s), |ui| ui.allocate_exact_size(r.size(), Sense::hover()));
            });
        }
        let resp = ui.interact(r, id.with("click"), Sense::click());
        if clip_name.is_some() {
            let current = self.tracks[t].slots[s].as_ref().and_then(|c| c.color).unwrap_or(self.tracks[t].color);
            resp.context_menu(|ui| {
                if let Some(i) = color_picker(ui, current) {
                    ui.close();
                    self.push_undo("Clip Colour");
                    if let Some(c) = self.tracks[t].slots[s].as_mut() {
                        c.color = Some(i);
                    }
                    self.dirty = true;
                }
                ui.separator();
                if ui.button("Delete").clicked() {
                    ui.close();
                    self.sel_slot = Some((t, s));
                    self.clear_slot();
                }
            });
        }
        let p = ui.painter();
        match &clip_name {
            Some(name) => {
                // As in Live: the slot in the clip's colour, its triangle green while it plays
                // (blinking while it waits for its bar).
                p.rect_filled(r, 0.0, color);
                let tri = Rect::from_min_size(r.min, Vec2::new(22.0, r.height()));
                let green = Color32::from_rgb(40, 200, 60);
                play_triangle(p, tri.center(), if playing || waiting { green } else { Color32::from_gray(10) });
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), FontId::proportional(11.5), Color32::from_gray(10));
                job.wrap = egui::text::TextWrapping { max_width: r.width() - 28.0, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let g = p.ctx().fonts_mut(|f| f.layout_job(job));
                p.galley(Pos2::new(tri.right(), r.center().y - g.size().y / 2.0), g, Color32::from_gray(10));
            }
            None => {
                // An empty slot shows its clip stop button, as in Live.
                p.rect_filled(r, 0.0, gray(228));
                p.rect_filled(Rect::from_center_size(r.center(), Vec2::splat(6.0)), 0.0, gray(195));
            }
        }
        if selected || self.sel_slots.contains(&(t, s)) {
            p.rect_stroke(r, 0.0, Stroke::new(2.0, ink()), egui::StrokeKind::Inside);
        }
        // Something dragged over the slot: show where it would land.
        if resp.dnd_hover_payload::<SlotDrag>().is_some() || resp.dnd_hover_payload::<PoolDrag>().is_some() {
            p.rect_stroke(r, 0.0, Stroke::new(2.0, ink().gamma_multiply(0.5)), egui::StrokeKind::Inside);
        }
        if let Some(from) = resp.dnd_release_payload::<SlotDrag>() {
            let copy = ui.input(|i| i.modifiers.alt);
            self.move_slot((from.0, from.1), (t, s), copy);
        }
        if let Some(p) = resp.dnd_release_payload::<PoolDrag>() {
            if let Some(clip) = self.pool.get(p.0).map(|l| l.clip.clone()) {
                self.push_undo("Add Loop to Slot");
                self.tracks[t].slots[s] = Some(slot_clip(clip));
                self.sel_slot = Some((t, s));
                self.dirty = true;
            }
        }
        if resp.double_clicked() && clip_name.is_some() {
            // Double-click: the clip opens in full in the audio editor.
            self.sel_slot = Some((t, s));
            self.edit_clip((t, SLOT + s));
            self.full_edit = true;
        } else if resp.clicked() && ui.input(|i| i.modifiers.shift) {
            // ⇧-click: add or remove slots from the selection (to group them, ⌘G).
            match self.sel_slots.iter().position(|&p| p == (t, s)) {
                Some(i) => {
                    self.sel_slots.remove(i);
                }
                None => self.sel_slots.push((t, s)),
            }
            if let Some(prev) = self.sel_slot.filter(|&p| p != (t, s) && !self.sel_slots.contains(&p)) {
                self.sel_slots.push(prev);
            }
            self.sel_slot = Some((t, s));
        } else if resp.clicked() {
            self.sel_slots.clear();
            self.sel_slot = Some((t, s));
            self.set_active(t);
            // A click on a clip's play triangle launches it.
            if clip_name.is_some() && resp.interact_pointer_pos().is_some_and(|q| q.x < r.left() + 22.0) {
                self.launch_slot(t, s);
            }
        }
    }

    /// Move (or with `copy`, copy) the clip in one slot to another.
    pub(super) fn move_slot(&mut self, from: (usize, usize), to: (usize, usize), copy: bool) {
        if from == to {
            return;
        }
        let Some(clip) = self.tracks.get(from.0).and_then(|t| t.slots.get(from.1)).cloned().flatten() else { return };
        self.push_undo(if copy { "Copy Clip" } else { "Move Clip" });
        if !copy {
            self.tracks[from.0].slots[from.1] = None;
        }
        self.tracks[to.0].slots[to.1] = Some(clip);
        self.sel_slot = Some(to);
        self.dirty = true;
    }

    /// Delete the selected slot's clip.
    /// ⌘C / ⌘X in the Session view: the selected slots' clips go on the clipboard, to paste
    /// into other slots or into the song view.
    pub(super) fn copy_slots(&mut self, cut: bool) {
        let mut picked: Vec<(usize, usize)> = self.sel_slots.iter().copied().chain(self.sel_slot).collect();
        picked.sort();
        picked.dedup();
        picked.retain(|&(t, s)| self.tracks.get(t).and_then(|tr| tr.slots.get(s)).is_some_and(|c| c.is_some()));
        let Some(t0) = picked.iter().map(|p| p.0).min() else { return };
        self.clip_board = picked.iter().filter_map(|&(t, s)| Some((t - t0, self.tracks[t].slots[s].clone()?))).collect();
        self.board_from_session = true;
        self.status = format!("Copied {} clip{}", picked.len(), if picked.len() == 1 { "" } else { "s" });
        if cut {
            self.push_undo("Cut");
            for (t, s) in picked {
                self.tracks[t].slots[s] = None;
            }
            self.dirty = true;
        }
    }

    /// ⌘V in the Session view: the copied clips go into the slots from the selected one down
    /// (each copied track's clips one under another, further tracks to the right), making
    /// scenes if needed.
    pub(super) fn paste_slots(&mut self) {
        let Some((t0, s0)) = self.sel_slot else { return };
        if self.clip_board.is_empty() {
            return;
        }
        self.push_undo("Paste");
        let mut row: HashMap<usize, usize> = HashMap::new();
        for (dt, c) in self.clip_board.clone() {
            let t = t0 + dt;
            if t >= self.tracks.len() {
                continue;
            }
            let k = row.entry(dt).or_insert(0);
            let s = s0 + *k;
            *k += 1;
            while self.scenes.len() <= s {
                let n = self.scenes.len() + 1;
                self.scenes.push(n.to_string());
            }
            self.fit_slots();
            self.tracks[t].slots[s] = Some(slot_clip(c));
        }
        self.dirty = true;
        self.refresh_playback();
    }

    /// ⌘V in the song view with whole clips copied: they go in at the insert marker from the
    /// selected track down, keeping their spacing (Session clips one after another).
    pub(super) fn paste_clips(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        self.push_undo("Paste");
        let base = self.clip_board.iter().map(|(_, c)| c.start).min().unwrap_or(0);
        let mut next: HashMap<usize, usize> = HashMap::new();
        let mut last = None;
        for (dt, mut c) in self.clip_board.clone() {
            let t = self.active + dt;
            if t >= self.tracks.len() {
                continue;
            }
            c.start = if self.board_from_session {
                let at = next.entry(dt).or_insert(self.arr_cursor);
                let s = *at;
                *at += c.len;
                s
            } else {
                self.arr_cursor + (c.start - base)
            };
            self.tracks[t].clips.push(c);
            last = Some(self.place_over((t, self.tracks[t].clips.len() - 1)));
        }
        self.sel_clip = last;
        self.dirty = true;
        self.refresh_playback();
    }

    pub(super) fn clear_slot(&mut self) {
        let Some((t, s)) = self.sel_slot else { return };
        if self.tracks.get(t).and_then(|tr| tr.slots.get(s)).is_some_and(|c| c.is_some()) {
            self.push_undo("Delete Clip");
            self.tracks[t].slots[s] = None;
            self.dirty = true;
        }
    }

    /// Where the transport is (the playhead while playing, else the insert marker).
    fn now(&self) -> usize {
        if self.player.is_playing() { self.player.position() } else { self.arr_cursor }
    }

    /// When a launch made now takes effect: the next quantization line while playing (as Live),
    /// straight away when stopped.
    fn launch_time(&self) -> usize {
        let now = self.now();
        let q = self.launch_q * self.bar_frames();
        if !self.player.is_playing() || q < 1.0 {
            return now;
        }
        ((now as f64 / q).ceil() * q).round() as usize
    }

    /// The slot track `t` is playing now, and one waiting to start at the next launch time.
    pub(super) fn slot_state(&self, t: usize) -> (Option<usize>, Option<usize>) {
        let Some(p) = self.session_play.get(t) else { return (None, None) };
        let now = self.now();
        let playing = self.player.is_playing();
        let on = p.segments.iter().find(|&&(_, from, to)| playing && from <= now && to.is_none_or(|to| now < to)).map(|s| s.0);
        let queued = p.segments.iter().find(|&&(_, from, _)| from > now || !playing).map(|s| s.0);
        (on, queued)
    }

    /// End track `t`'s open clip at `at`, dropping clips that would never start.
    fn close_track(&mut self, t: usize, at: usize) {
        let Some(p) = self.session_play.get_mut(t) else { return };
        p.segments.retain(|&(_, from, _)| from < at);
        for seg in &mut p.segments {
            if seg.2.is_none_or(|to| to > at) {
                seg.2 = Some(at);
            }
        }
    }

    /// Start the transport (if stopped) for a launch at `at`, and hand the player the new song.
    fn after_launch(&mut self, at: usize) {
        self.dirty = true;
        self.refresh_playback();
        if !self.player.is_playing() {
            let end = self.arrangement_frames().max(1) + (1 << 35);
            self.play_origin = at;
            self.player.play(at, end, false);
        }
    }

    /// Launch the clip in slot `s` of track `t` (an empty slot stops the track), on the next
    /// launch time. The track then plays from the Session view until Back to Arrangement.
    pub(super) fn launch_slot(&mut self, t: usize, s: usize) {
        self.fit_slots();
        if self.tracks.get(t).and_then(|tr| tr.slots.get(s)).is_none_or(|c| c.is_none()) {
            return self.stop_track(t);
        }
        let at = self.launch_time();
        self.close_track(t, at);
        let p = &mut self.session_play[t];
        p.owned = true;
        p.segments.push((s, at, None));
        self.after_launch(at);
    }

    /// Launch row `s`: each track plays its clip in that scene, or stops if the slot is empty.
    pub(super) fn launch_scene(&mut self, s: usize) {
        self.fit_slots();
        let at = self.launch_time();
        for t in 0..self.tracks.len() {
            self.close_track(t, at);
            if self.tracks[t].slots.get(s).is_some_and(|c| c.is_some()) {
                let p = &mut self.session_play[t];
                p.owned = true;
                p.segments.push((s, at, None));
            }
        }
        self.after_launch(at);
    }

    /// Stop track `t`'s Session clip at the next launch time.
    pub(super) fn stop_track(&mut self, t: usize) {
        let at = self.launch_time();
        self.close_track(t, at);
        if let Some(p) = self.session_play.get_mut(t) {
            p.owned = true;
        }
        self.refresh_playback();
    }

    /// Stop every Session clip at the next launch time.
    pub(super) fn stop_all_clips(&mut self) {
        let at = self.launch_time();
        for t in 0..self.tracks.len() {
            self.close_track(t, at);
        }
        self.refresh_playback();
    }

    /// The transport stopped: launched clips stop with it (the tracks stay with the Session).
    /// With Record to Song on, what was played is written into the song view first.
    pub(super) fn session_stopped(&mut self) {
        if self.session_rec {
            self.record_session();
        }
        for p in &mut self.session_play {
            p.segments.clear();
        }
    }

    /// Write every launched clip, from its launch to where it stopped, into its track in the
    /// song view (over what was there), as Live's Arrangement Record does.
    fn record_session(&mut self) {
        let now = self.stopped_at;
        let played: Vec<(usize, usize, usize, usize)> = self
            .session_play
            .iter()
            .enumerate()
            .flat_map(|(t, p)| p.segments.iter().map(move |&(s, from, to)| (t, s, from, to.unwrap_or(now).min(now))))
            .filter(|&(_, _, from, to)| to > from)
            .collect();
        if played.is_empty() {
            return;
        }
        self.push_undo("Record Session");
        for (t, s, from, to) in played {
            let Some(mut c) = self.clip((t, SLOT + s)).cloned() else { continue };
            (c.start, c.len) = (from, to - from);
            self.tracks[t].clips.push(c);
            let i = self.tracks[t].clips.len() - 1;
            self.place_over((t, i));
        }
        self.sel_clip = None;
        self.dirty = true;
        self.status = "Recorded the session into the song view — Back to Arrangement to hear it there".into();
    }

    /// Back to Arrangement (as Live): every track plays its song-view clips again.
    pub(super) fn back_to_song_view(&mut self) {
        self.session_play.clear();
        self.fit_slots();
        self.refresh_playback();
        self.status = "Back to the song view's clips".into();
    }

    /// Whether any track is playing from the Session view instead of the song view.
    pub(super) fn session_owns_any(&self) -> bool {
        self.session_play.iter().any(|p| p.owned)
    }
}

impl App {
    /// ⌘G in the Session view: the selected slots' clips become one grouped clip, in the first
    /// of them. Each source track gets a track inside the group; clips from one track play one
    /// after another, in scene order.
    pub(super) fn group_slots(&mut self) {
        let mut picked: Vec<(usize, usize)> = self.sel_slots.iter().copied().chain(self.sel_slot).collect();
        picked.sort_by_key(|&(t, s)| (s, t));
        picked.dedup();
        picked.retain(|&(t, s)| self.clip((t, SLOT + s)).is_some());
        let Some(&(t0, s0)) = picked.first() else { return };
        self.push_undo("Group Clips");
        let mut inner: Vec<Track> = Vec::new();
        let mut track_of: Vec<(usize, usize)> = Vec::new();
        for &(t, s) in &picked {
            let i = match track_of.iter().find(|(src, _)| *src == t) {
                Some(&(_, i)) => i,
                None => {
                    let mut nt = self.tracks[t].clone();
                    (nt.clips, nt.slots) = (Vec::new(), Vec::new());
                    inner.push(nt);
                    track_of.push((t, inner.len() - 1));
                    inner.len() - 1
                }
            };
            let mut c = self.tracks[t].slots[s].take().unwrap();
            c.looping = false;
            c.start = inner[i].clips.last().map_or(0, |l| l.end());
            inner[i].clips.push(c);
        }
        let n = (1..).find(|i| !self.tracks.iter().flat_map(|t| t.slots.iter().flatten().chain(&t.clips)).any(|c| c.name == format!("Group {i}"))).unwrap();
        let mut g = Clip::new(format!("Group {n}"), Arc::new(Audio { rate: self.rate(), channels: vec![Vec::new()] }), 0);
        self.set_group_content(&mut g, inner);
        self.tracks[t0].slots[s0] = Some(slot_clip(g));
        self.sel_slots.clear();
        self.sel_slot = Some((t0, s0));
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Grouped {} clips into Group {n} — double-click it to open it", picked.len());
    }

    /// Clip ▸ Copy to Session: the selected song-view clip goes into the first empty slot on
    /// its track.
    pub(super) fn copy_to_session(&mut self) {
        let Some((t, c)) = self.sel_clip.filter(|&(_, c)| c < SLOT) else { return };
        let Some(clip) = self.clip((t, c)).cloned() else { return };
        self.fit_slots();
        let Some(s) = self.tracks[t].slots.iter().position(|s| s.is_none()) else {
            self.status = "No empty slot on that track — add a scene".into();
            return;
        };
        self.push_undo("Copy to Session");
        self.tracks[t].slots[s] = Some(slot_clip(clip));
        self.dirty = true;
        self.status = format!("Copied to the Session view, scene {}", self.scenes[s]);
    }
}

/// A small play triangle centred on `c`.
fn play_triangle(p: &egui::Painter, c: Pos2, color: Color32) {
    p.add(egui::Shape::convex_polygon(vec![c + Vec2::new(-4.0, -5.0), c + Vec2::new(5.0, 0.0), c + Vec2::new(-4.0, 5.0)], color, Stroke::NONE));
}
