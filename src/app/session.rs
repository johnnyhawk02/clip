//! The Session view, as in Live: a grid of clip slots, one column per track and one row per
//! scene, for loops to launch. Tab switches between it and the song view.

use super::*;

/// Clip indices from here up mean a Session slot (`SLOT + scene`), so a slot's clip can be
/// edited like any other.
pub(super) const SLOT: usize = 1 << 30;

/// Width of a track's column.
const COL_W: f32 = 150.0;
/// Height of a clip slot.
const ROW_H: f32 = 26.0;
/// Height of the track title row.
const HEAD_H: f32 = 30.0;
/// Width of the scene column on the right.
const SCENE_W: f32 = 150.0;

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

    /// The Session view: track columns of clip slots, stop buttons and mixer, and the scenes on
    /// the right.
    pub(super) fn session(&mut self, ui: &mut egui::Ui) {
        self.fit_slots();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, gray(255));
        let n_tracks = self.tracks.len();
        let n_scenes = self.scenes.len();
        let grid_h = HEAD_H + n_scenes as f32 * ROW_H + ROW_H;
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(Vec2::new(n_tracks as f32 * COL_W + SCENE_W + 24.0, grid_h + 90.0), Sense::hover());
            let origin = area.min + Vec2::new(12.0, 8.0);
            let col = |t: usize| Rect::from_min_size(origin + Vec2::new(t as f32 * COL_W, 0.0), Vec2::new(COL_W - 6.0, grid_h + 80.0));
            let slot_rect = |t: usize, s: usize| Rect::from_min_size(origin + Vec2::new(t as f32 * COL_W, HEAD_H + s as f32 * ROW_H), Vec2::new(COL_W - 6.0, ROW_H - 3.0));
            let mut changed = false;
            for t in 0..n_tracks {
                let c = col(t);
                let color = live_color(self.tracks[t].color);
                // Title: the track's name in its colour (click to select the track).
                let head = Rect::from_min_size(c.min, Vec2::new(c.width(), HEAD_H - 6.0));
                ui.painter().rect_filled(head, 3.0, color);
                ui.painter().text(head.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, &self.tracks[t].name, FontId::proportional(12.0), Color32::from_gray(10));
                if t == self.active {
                    ui.painter().rect_stroke(head, 3.0, Stroke::new(1.5, ink()), egui::StrokeKind::Inside);
                }
                if ui.interact(head, ui.scope_id().with(("shead", t)), Sense::click()).clicked() {
                    self.set_active(t);
                }
                for s in 0..n_scenes {
                    let r = slot_rect(t, s);
                    self.draw_slot(ui, r, t, s, color);
                }
                // The track's stop button, under its slots.
                let stop = Rect::from_min_size(slot_rect(t, n_scenes).min, Vec2::new(c.width(), ROW_H - 3.0));
                let stop_r = ui.interact(stop, ui.scope_id().with(("sstop", t)), Sense::click());
                ui.painter().rect_filled(stop, 3.0, if stop_r.hovered() { gray(225) } else { panel() });
                ui.painter().rect_filled(Rect::from_center_size(stop.center(), Vec2::splat(8.0)), 1.0, ink());
                if stop_r.on_hover_text("Stop this track's clip").clicked() {
                    self.stop_track(t);
                }
                // The track's mixer: activator, Solo, volume and pan, as in the song view.
                let mix = Rect::from_min_size(stop.left_bottom() + Vec2::new(0.0, 8.0), Vec2::new(c.width(), 60.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(mix), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
                    let tr = &mut self.tracks[t];
                    ui.horizontal(|ui| {
                        let (act, act_r) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
                        let on = !tr.mute;
                        ui.painter().rect(act, 3.0, if on { ink() } else { Color32::TRANSPARENT }, Stroke::new(1.0, ink()), egui::StrokeKind::Inside);
                        ui.painter().text(act.center(), Align2::CENTER_CENTER, (t + 1).to_string(), FontId::proportional(11.0), if on { gray(255) } else { ink() });
                        if act_r.on_hover_text("Track on / mute").clicked() {
                            tr.mute = !tr.mute;
                            changed = true;
                        }
                        if ui.add(Button::new("S").selected(tr.solo).min_size(Vec2::new(22.0, 20.0))).on_hover_text("Solo").clicked() {
                            tr.solo = !tr.solo;
                            changed = true;
                        }
                        ui.spacing_mut().slider_width = (mix.width() - 60.0).max(30.0);
                        let v = ui.add(egui::Slider::new(&mut tr.gain, 0.0..=2.0).show_value(false)).on_hover_text(format!("Volume {:.0}%", tr.gain * 100.0));
                        changed |= v.drag_stopped() || (v.changed() && !v.dragged());
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Pan").size(11.0).color(muted()));
                        ui.spacing_mut().slider_width = (mix.width() - 40.0).max(30.0);
                        let p = ui.add(egui::Slider::new(&mut tr.pan, -1.0..=1.0).show_value(false));
                        if p.double_clicked() {
                            tr.pan = 0.0;
                        }
                        changed |= p.drag_stopped() || p.double_clicked() || (p.changed() && !p.dragged());
                    });
                });
            }
            // Scenes: name and launch button on the right, then Stop All.
            let sx = origin.x + n_tracks as f32 * COL_W + 6.0;
            let shead = Rect::from_min_size(Pos2::new(sx, origin.y), Vec2::new(SCENE_W, HEAD_H - 6.0));
            ui.painter().text(shead.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, "Scenes", FontId::proportional(12.0), muted());
            for s in 0..n_scenes {
                let r = Rect::from_min_size(Pos2::new(sx, origin.y + HEAD_H + s as f32 * ROW_H), Vec2::new(SCENE_W, ROW_H - 3.0));
                let resp = ui.interact(r, ui.scope_id().with(("scene", s)), Sense::click());
                ui.painter().rect_filled(r, 3.0, if resp.hovered() { gray(225) } else { panel() });
                let tri = Rect::from_min_size(r.min, Vec2::new(22.0, r.height()));
                play_triangle(ui.painter(), tri.center(), ink());
                ui.painter().text(tri.right_center() + Vec2::new(2.0, 0.0), Align2::LEFT_CENTER, &self.scenes[s], FontId::proportional(12.0), ink());
                if resp.on_hover_text("Launch every clip in this scene").clicked() {
                    self.launch_scene(s);
                }
            }
            let stop_all = Rect::from_min_size(Pos2::new(sx, origin.y + HEAD_H + n_scenes as f32 * ROW_H), Vec2::new(SCENE_W, ROW_H - 3.0));
            if ui.put(stop_all, Button::new("Stop All Clips")).clicked() {
                self.stop_all_clips();
            }
            let add = Rect::from_min_size(stop_all.left_bottom() + Vec2::new(0.0, 8.0), Vec2::new(SCENE_W, ROW_H - 3.0));
            if ui.put(add, Button::new("+ Scene")).on_hover_text("Add a scene (a row of slots)").clicked() {
                self.push_undo("Add Scene");
                let n = self.scenes.len() + 1;
                self.scenes.push(n.to_string());
                self.fit_slots();
                self.dirty = true;
            }
            if changed {
                self.dirty = true;
                self.refresh_playback();
            }
        });
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
                p.rect_filled(r, 3.0, color);
                let tri = Rect::from_min_size(r.min, Vec2::new(22.0, r.height()));
                let green = Color32::from_rgb(40, 200, 60);
                play_triangle(p, tri.center(), if playing || waiting { green } else { Color32::from_gray(10) });
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), FontId::proportional(11.5), Color32::from_gray(10));
                job.wrap = egui::text::TextWrapping { max_width: r.width() - 28.0, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let g = p.ctx().fonts_mut(|f| f.layout_job(job));
                p.galley(Pos2::new(tri.right(), r.center().y - g.size().y / 2.0), g, Color32::from_gray(10));
            }
            None => {
                p.rect_filled(r, 3.0, gray(238));
                p.rect_stroke(Rect::from_center_size(r.center(), Vec2::splat(7.0)), 1.0, Stroke::new(1.0, gray(200)), egui::StrokeKind::Inside);
            }
        }
        if selected || self.sel_slots.contains(&(t, s)) {
            p.rect_stroke(r, 3.0, Stroke::new(2.0, ink()), egui::StrokeKind::Inside);
        }
        // Something dragged over the slot: show where it would land.
        if resp.dnd_hover_payload::<SlotDrag>().is_some() || resp.dnd_hover_payload::<PoolDrag>().is_some() {
            p.rect_stroke(r, 3.0, Stroke::new(2.0, ink().gamma_multiply(0.5)), egui::StrokeKind::Inside);
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
