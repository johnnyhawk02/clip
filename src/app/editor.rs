//! The single-clip editor: the waveform view and the edits made on a selection.

use super::*;

/// What a drag in the clip editor's ruler is doing.
#[derive(Clone, Copy)]
pub(super) enum EdRuler {
    /// Zooming: the frame grabbed, the pointer's height then and the view length then.
    Zoom(f64, f32, f64),
    /// Drawing a new loop from this bar line.
    Draw(usize),
    /// Moving or resizing the loop: where it was grabbed, the range then, which part.
    Loop(f64, Sel, LoopGrab),
    /// Dragging the start flag (1.1.1) to this frame.
    Start(usize),
}

/// Where the parts of the editor view sit this frame, and the input state its tools share.
#[derive(Clone, Copy)]
pub(super) struct WaveArea {
    pub(super) full: Rect,
    /// The loop strip above the ruler (zero height outside the clip editor).
    pub(super) strip: Rect,
    pub(super) ruler: Rect,
    pub(super) wave: Rect,
    pub(super) scroll: Rect,
    pub(super) rate: f64,
    pub(super) n_ch: usize,
    pub(super) lane_h: f32,
    /// Frames per pixel.
    pub(super) spp: f64,
    /// Frames in the sound being edited.
    pub(super) n: usize,
    /// Where the button went down (see `waveform`).
    pub(super) press: Option<Pos2>,
    pub(super) shift: bool,
    pub(super) alt: bool,
    pub(super) command: bool,
}

impl WaveArea {
    fn x_to_frame(&self, x: f32, vs: f64, vl: f64) -> usize {
        let t = ((x - self.wave.left()) / self.wave.width()) as f64;
        (vs + t * vl).round().clamp(0.0, self.n as f64) as usize
    }
}

impl App {
    pub(super) fn apply(&mut self, label: &'static str, new: Audio, sel: Option<Sel>) {
        self.apply_edit(label, new, sel, &[]);
    }

    /// Like `apply`, for edits that change the length: each (at, removed, inserted) moves the
    /// cue points to follow the audio.
    pub(super) fn apply_edit(&mut self, label: &'static str, new: Audio, sel: Option<Sel>, shifts: &[(usize, usize, usize)]) {
        self.push_undo(label);
        for &(at, removed, inserted) in shifts {
            self.shift_cues(at, removed, inserted);
        }
        self.sel = sel;
        let was_playing = self.player.is_playing();
        self.set_audio(Arc::new(new));
        if was_playing && self.player.position() >= self.frames() {
            self.player.stop();
        }
        self.dirty = true;
        self.status = format!("{label} — ⌘Z to undo");
    }

    /// `removed` frames at `at` were replaced by `inserted` frames. Cues inside the replaced
    /// part scale into the new part (or go, if nothing replaced it); later cues move along.
    fn shift_cues(&mut self, at: usize, removed: usize, inserted: usize) {
        if removed == inserted {
            return;
        }
        let moved = |f: usize| -> Option<usize> {
            if f < at {
                Some(f)
            } else if f < at + removed {
                (inserted > 0).then(|| at + ((f - at) as f64 * inserted as f64 / removed as f64) as usize)
            } else {
                Some(f + inserted - removed)
            }
        };
        self.regions.retain_mut(|r| {
            let s = moved(r.start).unwrap_or(at);
            let e = moved(r.end).unwrap_or(at);
            r.start = s;
            r.end = e;
            e > s
        });
        self.loop_points = self.loop_points.and_then(|(s, e)| {
            let (s, e) = (moved(s).unwrap_or(at), moved(e).unwrap_or(at));
            (e > s).then_some((s, e))
        });
        self.cues.retain_mut(|c| {
            if c.frame < at {
                true
            } else if c.frame < at + removed {
                if inserted == 0 {
                    return false;
                }
                c.frame = at + ((c.frame - at) as f64 * inserted as f64 / removed as f64) as usize;
                true
            } else {
                c.frame = c.frame + inserted - removed;
                true
            }
        });
    }

    pub(super) fn trim(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.cursor = 0;
        let n = a.frames();
        self.apply_edit("Trim to selection", a.crop(s, e), None, &[(e, n - e, 0), (0, s, 0)]);
        self.zoom_fit();
    }

    pub(super) fn delete(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        if e - s >= a.frames() {
            self.status = "Can't delete everything — use Trim instead".into();
            return;
        }
        self.cursor = s;
        self.apply_edit("Delete selection", a.delete(s, e), None, &[(s, e - s, 0)]);
    }

    pub(super) fn copy(&mut self, ctx: &egui::Context) -> bool {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return false };
        let clip = a.slice(s, e);
        // Put a text marker on the system clipboard so ⌘V reliably produces a paste event.
        ctx.copy_text(format!("[audio clip, {}]", fmt_time(clip.duration_secs())));
        self.status = format!("Copied {}", fmt_time(clip.duration_secs()));
        self.clipboard = Some(clip);
        self.clip_board.clear();
        true
    }

    pub(super) fn cut(&mut self, ctx: &egui::Context) {
        if self.copy(ctx) {
            let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
            self.cursor = s;
            if e - s >= a.frames() {
                self.apply_edit("Cut", a.empty_like(), None, &[(s, e - s, 0)]);
            } else {
                self.apply_edit("Cut", a.delete(s, e), None, &[(s, e - s, 0)]);
            }
        }
    }

    pub(super) fn paste(&mut self) {
        let (Some(a), Some(clip)) = (self.audio.clone(), self.clipboard.clone()) else { return };
        let (at, end) = self.sel.unwrap_or((self.cursor, self.cursor));
        let new = a.insert(at, end, &clip);
        let pasted_len = new.frames() + (end - at) - a.frames();
        self.cursor = at;
        self.apply_edit("Paste", new, Some((at, at + pasted_len)), &[(at, end - at, pasted_len)]);
    }

    pub(super) fn fade(&mut self, fade_in: bool) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        if fade_in {
            self.apply("Fade in", a.fade_in(s, e), self.sel);
        } else {
            self.apply("Fade out", a.fade_out(s, e), self.sel);
        }
    }

    pub(super) fn normalize(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        match a.normalize(s, e, self.param("norm_db")) {
            Some(n) => self.apply("Normalize", n, self.sel),
            None => self.status = "Nothing to normalize — range is silent".into(),
        }
    }

    pub(super) fn gain(&mut self, db: f32) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Amplify", a.gain(s, e, 10f32.powf(db / 20.0)), self.sel);
    }

    pub(super) fn silence(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.apply("Silence", a.silence(s, e), self.sel);
    }

    pub(super) fn reverse(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Backwards", a.reverse(s, e), self.sel);
    }

    pub(super) fn to_mono(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        if a.num_channels() > 1 {
            self.apply("Convert to mono", a.to_mono(), self.sel);
        }
    }

    pub(super) fn param(&self, key: &str) -> f32 {
        self.params.get(key).copied().unwrap_or(0.0)
    }

    pub(super) fn split_gap(&self) -> usize {
        self.param("gap").max(1.0) as usize
    }

    /// Run an effect on the selection (or whole sound) and splice the result back in.
    /// Effects may change the length (tempo, speed); the selection follows.
    pub(super) fn process(&mut self, label: &'static str, f: impl FnOnce(&Audio) -> Audio) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        if e <= s {
            return;
        }
        let out = f(&a.slice(s, e));
        let len = out.frames();
        let sel = self.sel.map(|_| (s, s + len));
        self.apply_edit(label, a.insert(s, e, &out), sel, &[(s, e - s, len)]);
    }

    /// Insert a generated clip at the cursor, replacing the selection if there is one.
    pub(super) fn insert_clip(&mut self, label: &'static str, clip: Audio) {
        let Some(a) = self.audio.clone() else { return };
        let (at, end) = self.sel.unwrap_or((self.cursor, self.cursor));
        let new = a.insert(at, end, &clip);
        let len = new.frames() + (end - at) - a.frames();
        self.cursor = at;
        self.apply_edit(label, new, Some((at, at + len)), &[(at, end - at, len)]);
    }

    pub(super) fn mix_paste(&mut self) {
        let (Some(a), Some(clip)) = (self.audio.clone(), self.clipboard.clone()) else { return };
        let at = self.sel.map_or(self.cursor, |(s, _)| s);
        let level = 10f32.powf(self.param("mix_db") / 20.0);
        let len = clip.resampled(a.rate).frames();
        self.apply("Mix Paste", effects::mix_into(&a, at, &clip, level), Some((at, at + len)));
    }

    /// Convert the whole sound's format (0 = keep that property).
    pub(super) fn convert_format(&mut self, rate: u32, channels: usize, bits: u32) {
        let Some(a) = self.audio.clone() else { return };
        let mut out = (*a).clone();
        if channels > 0 {
            out = effects::convert_channels(&out, channels);
        }
        if rate > 0 && rate != out.rate {
            out = effects::convert_rate(&out, rate);
        }
        if bits > 0 {
            out = effects::quantize(&out, bits);
            match bits {
                8 => self.wav_format = WavFormat::Pcm8,
                16 => self.wav_format = WavFormat::Pcm16,
                _ => {}
            }
        }
        // Positions are in frames, so cues, cursor, selection and view follow the new rate.
        let k = out.rate as f64 / a.rate as f64;
        let scale = |f: usize| (f as f64 * k) as usize;
        let (cursor, view_start, view_len) = (self.cursor, self.view_start, self.view_len);
        let sel = self.sel.map(|(s, e)| (scale(s), scale(e)));
        self.apply("Convert Format", out, sel);
        for c in &mut self.cues {
            c.frame = scale(c.frame);
        }
        self.cursor = scale(cursor);
        self.view_start = view_start * k;
        self.view_len = view_len * k;
        self.clamp_view();
        let a = self.audio.as_ref().unwrap();
        self.status = format!("Converted to {} Hz, {} ch — ⌘Z to undo", a.rate, a.num_channels());
    }

    pub(super) fn apply_curve(&mut self, kind: CurveKind) {
        match kind {
            CurveKind::Envelope => {
                let pts = self.env_points.clone();
                self.process("Envelope", |c| effects::envelope(c, &pts));
            }
            CurveKind::Bender => {
                let pts = self.bend_points.clone();
                self.process("Bender", |c| effects::bender(c, &pts));
            }
        }
    }

    pub(super) fn add_cue(&mut self) {
        let at = self.sel.map_or(self.cursor, |(s, _)| s);
        if self.cues.iter().any(|c| c.frame == at) {
            return;
        }
        let n = (1..).find(|i| !self.cues.iter().any(|c| c.name == format!("Cue {i}"))).unwrap();
        self.push_undo("Cue Point");
        self.cues.push(Cue { frame: at, name: format!("Cue {n}") });
        self.cues.sort_by_key(|c| c.frame);
        self.dirty = true;
        self.status = format!("Added Cue {n} — rename it in the Cues panel (⌘3)");
    }

    pub(super) fn jump_cue(&mut self, forward: bool) {
        let from = self.sel.map_or(self.cursor, |(s, _)| s);
        let target = if forward {
            self.cues.iter().map(|c| c.frame).find(|&f| f > from)
        } else {
            self.cues.iter().rev().map(|c| c.frame).find(|&f| f < from)
        };
        if let Some(f) = target {
            self.go_to(f);
        }
    }

    /// ⌘-drag snapping: to a cue point within 8 pixels, otherwise to the nearest zero crossing.
    pub(super) fn snap(&self, f: usize, spp: f64) -> usize {
        let radius = (8.0 * spp).max(1.0) as usize;
        if let Some(c) = self.cues.iter().map(|c| c.frame).filter(|c| c.abs_diff(f) <= radius).min_by_key(|c| c.abs_diff(f)) {
            return c;
        }
        self.snap_zero(f, spp)
    }

    /// In the clip editor with Snap on, a selection edge goes to the nearest grid line (the song
    /// view's grid, counted from the clip's 1.1.1); ⌘ snaps to cues and zero crossings instead.
    fn snap_edge(&self, f: usize, spp: f64, command: bool) -> usize {
        if command {
            return self.snap(f, spp);
        }
        match (matches!(self.view, View::Editor { .. }) && self.snap, self.grid_frames(spp)) {
            (true, Some(g)) => {
                let o = self.ed_markers().1;
                ((((f as f64 - o) / g).round() * g + o).max(0.0) as usize).min(self.frames())
            }
            _ => f,
        }
    }

    /// Nearest zero crossing (of all channels summed) within 8 pixels, or `f` if there is none.
    fn snap_zero(&self, f: usize, spp: f64) -> usize {
        let Some(a) = &self.audio else { return f };
        a.nearest_zero_crossing(f, ((8.0 * spp) as usize).clamp(32, 50_000)).unwrap_or(f)
    }

    /// Move the insertion point to `f` and scroll it into view.
    pub(super) fn go_to(&mut self, f: usize) {
        self.sel = None;
        self.cursor = f;
        self.play_origin = f;
        if (f as f64) < self.view_start || (f as f64) > self.view_start + self.view_len {
            self.view_start = f as f64 - self.view_len * 0.1;
            self.clamp_view();
        }
    }

    pub(super) fn set_loop(&mut self) {
        if let Some(sel) = self.sel {
            self.push_undo("Set Loop");
            self.loop_points = Some(sel);
            self.dirty = true;
            self.status = "Loop set — turn on Loop (⌘L) and play to hear it repeat".into();
        }
    }

    pub(super) fn add_region(&mut self) {
        let Some((s, e)) = self.sel else { return };
        let n = (1..).find(|i| !self.regions.iter().any(|r| r.name == format!("Region {i}"))).unwrap();
        self.push_undo("Region");
        self.regions.push(Region { start: s, end: e, name: format!("Region {n}") });
        self.regions.sort_by_key(|r| r.start);
        self.show_regions = true;
        self.dirty = true;
        self.status = format!("Added Region {n} — rename it in the Regions panel");
    }

    pub(super) fn waveform(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let Some(_) = self.audio.as_ref() else {
            ui.painter().rect_filled(full, 0.0, gray(255));
            ui.painter().text(
                full.center(),
                Align2::CENTER_CENTER,
                if self.loading.is_some() { "Loading…" } else { "Drop a sound here, or choose File ▸ Open\n(WAV, MP3, FLAC, OGG, M4A, AIFF…)" },
                FontId::proportional(16.0),
                gray(120),
            );
            return;
        };
        let rate = self.audio.as_ref().unwrap().rate as f64;
        let n_ch = self.audio.as_ref().unwrap().num_channels().max(1);

        let total = self.timeline_frames();
        // Editing a clip, the ruler works like the song view's: a loop strip on top, bars below.
        let clip_mode = matches!(self.view, View::Editor { .. }) || self.split;
        let strip = Rect::from_min_max(full.min, Pos2::new(full.right(), full.top() + if clip_mode { LOOP_H } else { 0.0 }));
        let ruler = Rect::from_min_max(Pos2::new(full.left(), strip.bottom()), Pos2::new(full.right(), strip.bottom() + RULER_H));
        let scroll = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - 15.0), full.max);
        let wave = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.right(), scroll.top()));
        let lane_h = wave.height() / n_ch as f32;
        let spp = self.view_len / wave.width() as f64; // frames per pixel

        // --- interaction on the waveform ---
        let resp = ui.allocate_rect(Rect::from_min_max(strip.min, wave.max), Sense::click_and_drag());
        let n = self.frames();

        if resp.hovered() {
            let (zoom, scroll_delta, hover) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zoom != 1.0 {
                let anchor = hover.map_or(self.view_start + self.view_len / 2.0, |p| {
                    self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len
                });
                self.zoom(1.0 / zoom as f64, anchor);
            }
            let alt_scroll = ui.input(|i| i.modifiers.alt) && scroll_delta.y != 0.0;
            if alt_scroll {
                // ⌥-scroll zooms around the mouse.
                let anchor = hover.map_or(self.view_start + self.view_len / 2.0, |p| {
                    self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len
                });
                self.zoom((-scroll_delta.y as f64 / 150.0).exp(), anchor);
            }
            let d = if scroll_delta.x.abs() > scroll_delta.y.abs() { scroll_delta.x } else { scroll_delta.y };
            if d != 0.0 && !alt_scroll {
                self.view_start -= d as f64 / wave.width() as f64 * self.view_len;
                self.clamp_view();
            }
        }

        let (shift, alt, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        // Where the button went down: egui only reports a drag after the pointer has moved a
        // few pixels, and anchoring there would shift the start of the drag.
        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        // Holding Space switches to the Hand tool for as long as it is held.
        let tool = match (self.space_down, self.tool) {
            (true, _) => Tool::Hand,
            // The split tool is for the arrangement; in the editor it selects.
            (false, Tool::Split) => Tool::Select,
            (false, t) => t,
        };
        let a = WaveArea { full, strip, ruler, wave, scroll, rate, n_ch, lane_h, spp, n, press, shift, alt, command };
        if clip_mode && self.clip_ruler(ui, &resp, &a) {
            // The ruler has this drag.
        } else {
        match tool {
            Tool::Select | Tool::Split => self.select_tool(ui, &resp, &a),
            Tool::Hand => {
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                }
                if resp.dragged() {
                    self.view_start -= resp.drag_delta().x as f64 / wave.width() as f64 * self.view_len;
                    self.clamp_view();
                    if self.space_down {
                        self.space_used = true;
                    }
                }
            }
            Tool::Zoom => self.zoom_tool(ui, &resp, &a),
            Tool::Pencil => self.pencil_tool(ui, &resp, &a),
        }
        }

        let thumb = self.wave_scrollbar(ui, scroll, total);


        // Follow the playhead while playing.
        // Song playback isn't shown here: the detail view only follows its own.
        let playing = self.player.is_playing() && self.view != View::Arrangement;
        let play_pos = self.player.heard_position();
        if playing {
            ui.ctx().request_repaint();
            if self.drag_anchor.is_none() && !self.scrollbar_drag && self.tool != Tool::Hand {
                let p = play_pos as f64;
                if p > self.view_start + self.view_len || p < self.view_start {
                    self.view_start = p - self.view_len * 0.05;
                    self.clamp_view();
                }
            }
        }
        self.paint_waveform(ui, &a, thumb, resp.hover_pos(), playing, play_pos);
    }







    /// Select tool: click to place the cursor, drag to select, drag cue flags, double-click to select
    /// between cues.
    fn select_tool(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &WaveArea) {
        let WaveArea { ruler, wave, spp, n, press, shift, alt, command, .. } = *a;
        let x_to_frame = |x: f32, vs: f64, vl: f64| a.x_to_frame(x, vs, vl);
        // Cue flags in the ruler can be dragged along (⌥-drag duplicates), or clicked to jump there.
        let flag_at = |q: Pos2, cues: &[Cue], vs: f64, vl: f64| {
            (q.y <= ruler.top() + 14.0).then(|| {
                cues.iter().position(|c| (wave.left() + ((c.frame as f64 - vs) / vl) as f32 * wave.width() - q.x).abs() < 6.0)
            })?
        };
        let hover_flag = resp.hover_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len));
        if hover_flag.is_some() || self.cue_drag.is_some() {
            ui.ctx().set_cursor_icon(if alt && self.cue_drag.is_none() { egui::CursorIcon::Copy } else { egui::CursorIcon::ResizeHorizontal });
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let clicked_flag = if resp.clicked() { resp.interact_pointer_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)) } else { None };
        // A click on a region's bar in the ruler selects that region.
        let clicked_region = if resp.clicked() && clicked_flag.is_none() {
            resp.interact_pointer_pos().filter(|q| q.y >= ruler.top() + 15.0 && q.y <= ruler.top() + 23.0).and_then(|q| {
                let f = x_to_frame(q.x, self.view_start, self.view_len);
                self.regions.iter().find(|r| f >= r.start && f < r.end).map(|r| (r.start, r.end))
            })
        } else {
            None
        };
        if resp.drag_started() {
            if let Some(i) = press.and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)) {
                if alt {
                    self.push_undo("Duplicate Cue");
                    let copy = Cue { frame: self.cues[i].frame, name: format!("{} copy", self.cues[i].name) };
                    self.cues.push(copy);
                    self.cue_drag = Some(self.cues.len() - 1);
                } else {
                    self.push_undo("Move Cue");
                    self.cue_drag = Some(i);
                }
            }
        }
        if let Some(i) = self.cue_drag {
            if let Some(q) = resp.interact_pointer_pos() {
                let mut f = x_to_frame(q.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                if command {
                    f = self.snap_zero(f, spp);
                }
                if let Some(c) = self.cues.get_mut(i) {
                    c.frame = f;
                }
            }
            if resp.drag_stopped() {
                self.cue_drag = None;
                self.cues.sort_by_key(|c| c.frame);
                self.dirty = true;
            }
        } else if let Some(i) = clicked_flag {
            let f = self.cues[i].frame;
            self.go_to(f);
        } else if let Some((rs, re)) = clicked_region {
            self.sel = Some((rs.min(n), re.min(n))).filter(|(s, e)| e > s);
            self.cursor = rs.min(n);
        } else {
            if resp.drag_started() {
                if let Some(p) = press {
                    let f = x_to_frame(p.x, self.view_start, self.view_len);
                    match (alt && !shift, self.sel) {
                        // ⌥-drag moves the nearest selection edge at a tenth of the speed.
                        (true, Some((s, e))) => {
                            let is_end = f.abs_diff(e) < f.abs_diff(s);
                            self.fine_edge = Some((is_end, if is_end { e } else { s } as f64));
                            self.status = "Fine adjust: the edge moves at 1/10 speed".into();
                        }
                        _ => {
                            let f = self.snap_edge(f, spp, command);
                            self.drag_anchor = Some(match (shift, self.sel) {
                                // Shift-drag extends the existing selection from its far end.
                                (true, Some((s, e))) => {
                                    if f.abs_diff(s) < f.abs_diff(e) { e } else { s }
                                }
                                (true, None) => self.cursor,
                                _ => f,
                            });
                        }
                    }
                }
            }
            if resp.dragged() {
                if let (Some((is_end, pos)), Some((s, e))) = (self.fine_edge, self.sel) {
                    let pos = (pos + resp.drag_delta().x as f64 * spp * 0.1).clamp(0.0, n as f64);
                    self.fine_edge = Some((is_end, pos));
                    let other = if is_end { s } else { e };
                    let f = pos.round() as usize;
                    if f != other {
                        self.sel = Some((f.min(other), f.max(other)));
                        self.cursor = f.min(other);
                    }
                } else if let (Some(anchor), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos()) {
                    // Auto-scroll when dragging past the edges.
                    if p.x < wave.left() {
                        self.view_start -= self.view_len * 0.02;
                        self.clamp_view();
                    } else if p.x > wave.right() {
                        self.view_start += self.view_len * 0.02;
                        self.clamp_view();
                    }
                    let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                    let f = self.snap_edge(f, spp, command);
                    let (s, e) = (anchor.min(f), anchor.max(f));
                    if e > s {
                        self.sel = Some((s, e));
                        self.cursor = s;
                        if !self.player.is_playing() {
                            self.play_origin = s;
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_anchor = None;
                self.fine_edge = None;
                if self.player.is_playing() {
                    if let Some((s, e)) = self.sel {
                        self.player.play(s, e, self.looping);
                        self.play_origin = s;
                    }
                }
            }
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let f = x_to_frame(p.x, self.view_start, self.view_len);
                    let f = if command { self.snap(f, spp) } else { f };
                    if shift {
                        let (a, b) = match self.sel {
                            Some((s, e)) => {
                                if f.abs_diff(s) < f.abs_diff(e) { (f, e) } else { (s, f) }
                            }
                            None => (self.cursor, f),
                        };
                        if a.max(b) > a.min(b) {
                            self.sel = Some((a.min(b), a.max(b)));
                        }
                    } else {
                        self.sel = None;
                        self.cursor = f;
                        self.play_origin = f;
                        // ⌥-click plays from the mouse; while playing, any click jumps there.
                        if alt || self.player.is_playing() {
                            self.player.play(f, n, false);
                        }
                    }
                }
            }
            if resp.double_clicked() && n > 0 {
                // Double-click selects between the surrounding cues; again selects everything.
                let f = resp.interact_pointer_pos().map_or(0, |p| x_to_frame(p.x, self.view_start, self.view_len));
                let a = self.cues.iter().map(|c| c.frame).filter(|&c| c <= f).max().unwrap_or(0);
                let b = self.cues.iter().map(|c| c.frame).filter(|&c| c > f).min().unwrap_or(n);
                let region = (a, b);
                self.sel = Some(if self.last_double == Some(region) && (a, b) != (0, n) { (0, n) } else { region });
                self.last_double = self.sel.filter(|r| *r == region);
                self.cursor = self.sel.map_or(f, |(s, _)| s);
            }
        }
    }

    /// Zoom tool: click to zoom in (⌥ out), drag a band to zoom to it.
    fn zoom_tool(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &WaveArea) {
        let WaveArea { wave, press, alt, .. } = *a;
        let x_to_frame = |x: f32, vs: f64, vl: f64| a.x_to_frame(x, vs, vl);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(if alt { egui::CursorIcon::ZoomOut } else { egui::CursorIcon::ZoomIn });
        }
        if resp.drag_started() {
            self.drag_anchor = press.map(|p| x_to_frame(p.x, self.view_start, self.view_len));
        }
        if resp.dragged() {
            if let (Some(a), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos()) {
                let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                self.zoom_band = Some((a.min(f), a.max(f)));
            }
        }
        if resp.drag_stopped() {
            if let Some((s, e)) = self.zoom_band.take() {
                if e > s + 8 {
                    self.view_start = s as f64;
                    self.view_len = (e - s) as f64;
                    self.clamp_view();
                }
            }
            self.drag_anchor = None;
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len;
                self.zoom(if alt { 2.0 } else { 0.5 }, f);
            }
        }
    }

    /// Pencil tool: draw the waveform by hand when zoomed in close.
    fn pencil_tool(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &WaveArea) {
        let WaveArea { wave, lane_h, n_ch, spp, n, press, shift, .. } = *a;
        let x_to_frame = |x: f32, vs: f64, vl: f64| a.x_to_frame(x, vs, vl);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let value_at = |y: f32, ch: usize| {
            let mid = wave.top() + ch as f32 * lane_h + lane_h / 2.0;
            ((mid - y) / (lane_h / 2.0 * 0.92)).clamp(-1.0, 1.0)
        };
        if (resp.drag_started() || resp.clicked()) && self.pencil.is_none() {
            if spp > PENCIL_MAX_SPP || n == 0 {
                self.status = "Zoom in closer to draw with the pencil".into();
            } else if let Some(p) = press {
                let mut channel = (((p.y - wave.top()).max(0.0) / lane_h) as usize).min(n_ch - 1);
                // Shift-click draws a straight line on from the end of the last stroke.
                let mut last = None;
                if let (true, Some((ch, f, v))) = (shift, self.pencil_last) {
                    channel = ch.min(n_ch - 1);
                    last = Some((f.min(n - 1), v));
                }
                self.push_undo("Pencil");
                self.pencil = Some(PencilStroke { channel, last });
                let f = x_to_frame(p.x, self.view_start, self.view_len).min(n - 1);
                self.pencil_to(f, value_at(p.y, channel));
            }
        }
        if resp.dragged() {
            if let (Some(ch), Some(p)) = (self.pencil.as_ref().map(|s| s.channel), resp.interact_pointer_pos()) {
                let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len).min(n - 1);
                self.pencil_to(f, value_at(p.y, ch));
            }
        }
        if (resp.drag_stopped() || resp.clicked()) && self.pencil.is_some() {
            let stroke = self.pencil.take().unwrap();
            self.pencil_last = stroke.last.map(|(f, v)| (stroke.channel, f, v));
            let a = self.audio.clone().unwrap();
            self.set_audio(a);
            self.dirty = true;
            self.status = "Pencil — ⌘Z to undo".into();
        }
    }

    /// The editor's scroll bar; returns where its thumb is.
    fn wave_scrollbar(&mut self, ui: &mut egui::Ui, scroll: Rect, total: usize) -> Rect {
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let thumb_w = (self.view_len / total.max(1) as f64 * scroll.width() as f64).max(16.0).min(scroll.width() as f64) as f32;
        let track = scroll.width() - thumb_w;
        let max_start = (total as f64 - self.view_len).max(0.0);
        let thumb_x = if max_start > 0.0 { (self.view_start / max_start) as f32 * track } else { 0.0 };
        let thumb = Rect::from_min_size(Pos2::new(scroll.left() + thumb_x, scroll.top()), Vec2::new(thumb_w, scroll.height()));
        if sresp.drag_started() {
            self.scrollbar_drag = true;
        }
        if sresp.drag_stopped() {
            self.scrollbar_drag = false;
        }
        if (sresp.dragged() || sresp.clicked()) && track > 0.0 {
            if let Some(p) = sresp.interact_pointer_pos() {
                let t = ((p.x - scroll.left() - thumb_w / 2.0) / track).clamp(0.0, 1.0) as f64;
                self.view_start = t * max_start;
                self.clamp_view();
            }
        }
        thumb
    }

    /// Paint the editor: ruler, waveform, selection, cues, regions, loop, cursor and scroll bar.
    fn paint_waveform(&self, ui: &egui::Ui, a: &WaveArea, thumb: Rect, hover: Option<Pos2>, playing: bool, play_pos: usize) {
        let WaveArea { full, strip, ruler, wave, scroll, rate, spp, .. } = *a;
        let x_to_frame = |x: f32, vs: f64, vl: f64| a.x_to_frame(x, vs, vl);
        // --- painting: dark waveform on white, selection as a grey band ---
        let audio = self.audio.clone().unwrap();
        let painter = ui.painter_at(full);
        let vs = self.view_start;
        let vl = self.view_len;
        let frame_to_x = |f: f64| wave.left() + ((f - vs) / vl) as f32 * wave.width();
        let black = gray(0);

        painter.rect_filled(wave, 0.0, gray(255));
        painter.rect_filled(ruler, 0.0, panel());
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, rule()));
        if strip.height() > 0.0 {
            painter.rect_filled(strip, 0.0, panel());
            // The loop: a brace in the strip (black when Loop is on), as in the song view.
            if let Some((ls, le)) = self.loop_points {
                let (x0, x1) = (frame_to_x(ls as f64).max(wave.left()), frame_to_x(le as f64).min(wave.right()));
                if x1 > x0 {
                    let col = if self.looping { ink() } else { gray(175) };
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, strip.top() + 3.0), Pos2::new(x1, strip.bottom() - 3.0)), 2.0, col);
                    if self.looping {
                        painter.rect_filled(Rect::from_min_max(Pos2::new(x0, wave.top()), Pos2::new(x1, wave.bottom())), 0.0, Color32::from_black_alpha(8));
                    }
                }
            }
        }
        if let Some((s, e)) = self.sel {
            let (x0, x1) = (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right()));
            if x1 > x0 {
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, ruler.bottom() - 1.0)), 0.0, gray(222));
            }
        }

        // Bars and beats, as in the song view, counting 1.1.1 from the start of the clip: bar
        // lines and numbers, beat and sixteenth ticks as zoom allows.
        let bar = self.bar_frames();
        let beat = self.beat_frames();
        let bar_ruler = (matches!(self.view, View::Editor { .. }) || self.split) && bar > 0.0;
        if bar_ruler {
            let mut g = beat / 4.0;
            while g > 0.0 && g / spp < 8.0 {
                g *= 2.0;
            }
            let mut label_every = 1u64;
            while bar * label_every as f64 / spp < 40.0 && label_every < 4096 {
                label_every *= 2;
            }
            let on = |f: f64, unit: f64| {
                let r = (f / unit).rem_euclid(1.0);
                r < 1e-6 || r > 1.0 - 1e-6
            };
            // Counting from 1.1.1, which may not be the start: bars before it are -1, -2…
            let origin = self.ed_markers().1;
            let mut k = ((vs - origin) / g).floor() as i64;
            for _ in 0..MAX_TICKS {
                let f = origin + k as f64 * g;
                if !(f <= vs + vl) {
                    break;
                }
                let x = frame_to_x(f);
                if f >= -0.5 && x >= wave.left() - 0.5 {
                    let (on_bar, on_beat) = (on(f - origin, bar), on(f - origin, beat));
                    let (h, line) = if on_bar { (9.0, 228) } else if on_beat { (5.0, 240) } else { (3.0, 246) };
                    painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, muted()));
                    painter.line_segment([Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, gray(line)));
                    let bar_no = ((f - origin) / bar).round() as i64;
                    if on_bar && bar_no.rem_euclid(label_every as i64) == 0 {
                        let label = if bar_no >= 0 { bar_no + 1 } else { bar_no };
                        painter.text(Pos2::new(x + 3.0, ruler.top() + 1.0), Align2::LEFT_TOP, label.to_string(), FontId::proportional(11.0), muted());
                    }
                }
                k += 1;
            }
        }

        // Otherwise a time ruler with "nice" tick spacing (~90 px apart) and minor ticks.
        let secs_per_px = if bar_ruler { f64::INFINITY } else { vl / rate / wave.width() as f64 };
        let nice = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = *nice.iter().find(|&&s| s / secs_per_px >= 90.0).unwrap_or(&1200.0);
        let minor = step / 5.0;
        let mut t = (vs / rate / minor).floor() * minor;
        let mut k = (t / minor).round() as i64;
        for _ in 0..if bar_ruler { 0 } else { MAX_TICKS } {
            if !(t <= (vs + vl) / rate) {
                break;
            }
            let x = frame_to_x(t * rate);
            if x >= wave.left() {
                let major = k % 5 == 0;
                let h = if major { 7.0 } else { 3.0 };
                painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, muted()));
                if major {
                    painter.line_segment([Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, gray(242)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + 1.0), Align2::LEFT_TOP, ruler_label(self.ruler, rate as u32, t, step), FontId::proportional(11.0), muted());
                }
            }
            k += 1;
            t = k as f64 * minor;
        }

        // Selection: a grey band behind the active track's waveform.
        let sel_px = self.sel.map(|(s, e)| (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right())));

        // Waveform, split into one strip per channel.
        {
            let peaks = self.peaks.clone().unwrap();
            let lane = wave;
            let tn = audio.frames();
            if let Some((x0, x1)) = sel_px {
                if x1 > x0 {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, lane.top()), Pos2::new(x1, lane.bottom())), 0.0, gray(222));
                }
            }
            let tc = live_color(self.tracks.get(self.active).map_or(0, |t| t.color));
            let base = tc;
            let tch = audio.num_channels().max(1);
            let lane_h = lane.height() / tch as f32;
            for (ci, ch) in audio.channels.iter().enumerate() {
                let top = lane.top() + ci as f32 * lane_h;
                let mid = top + lane_h / 2.0;
                let amp = lane_h / 2.0 * 0.92;
                let in_sel = |f: usize| matches!(self.sel, Some((s, e)) if f >= s && f < e);
                let color_at = |f: usize| if in_sel(f) { base.lerp_to_gamma(black, 0.6) } else { base };
                painter.line_segment([Pos2::new(wave.left(), mid), Pos2::new(wave.right(), mid)], Stroke::new(1.0, gray(212)));
                if ci > 0 {
                    painter.line_segment([Pos2::new(wave.left(), top), Pos2::new(wave.right(), top)], Stroke::new(1.0, gray(232)));
                }
                if spp <= 1.5 {
                    // Zoomed in: connect individual samples.
                    let first = vs.floor().max(0.0) as usize;
                    let last = ((vs + vl).ceil() as usize + 1).min(tn);
                    let mut prev: Option<Pos2> = None;
                    for (i, &s) in ch.iter().enumerate().take(last).skip(first) {
                        let p = Pos2::new(frame_to_x(i as f64), mid - s.clamp(-1.0, 1.0) * amp);
                        if let Some(q) = prev {
                            painter.line_segment([q, p], Stroke::new(1.0, color_at(i)));
                        }
                        if spp < 0.12 {
                            painter.rect_filled(Rect::from_center_size(p, Vec2::splat(3.0)), 0.0, color_at(i));
                        }
                        prev = Some(p);
                    }
                } else {
                    let blocks = &peaks.data[ci];
                    let w = wave.width().ceil() as usize;
                    for px in 0..w {
                        let f0 = (vs + px as f64 * spp).max(0.0) as usize;
                        let f1 = ((vs + (px + 1) as f64 * spp) as usize).min(tn);
                        if f0 >= f1 {
                            continue;
                        }
                        let (lo, hi) = if spp >= peaks.block as f64 * 2.0 {
                            blocks[f0 / peaks.block..(f1 / peaks.block).max(f0 / peaks.block + 1).min(blocks.len())]
                                .iter()
                                .fold((f32::MAX, f32::MIN), |(a, b), &(l, h)| (a.min(l), b.max(h)))
                        } else {
                            ch[f0..f1].iter().fold((f32::MAX, f32::MIN), |(a, b), &s| (a.min(s), b.max(s)))
                        };
                        let x = wave.left() + px as f32 + 0.5;
                        let y0 = mid - hi.clamp(-1.0, 1.0) * amp;
                        let y1 = (mid - lo.clamp(-1.0, 1.0) * amp).max(y0 + 1.0);
                        painter.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], Stroke::new(1.0, color_at(f0)));
                    }
                }
            }
        }

        // Zoom tool rubber band.
        if let Some((s, e)) = self.zoom_band {
            let r = Rect::from_min_max(Pos2::new(frame_to_x(s as f64), wave.top() + 1.0), Pos2::new(frame_to_x(e as f64), wave.bottom() - 1.0));
            for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
                painter.add(egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, gray(80)), 3.0, 3.0));
            }
        }

        // Cue points: a dotted line through the sound and a labelled flag in the ruler.
        for c in &self.cues {
            let x = frame_to_x(c.frame as f64);
            if x < wave.left() - 1.0 || x > wave.right() + 1.0 {
                continue;
            }
            let in_sel = matches!(self.sel, Some((s, e)) if c.frame >= s && c.frame < e);
            let col = if in_sel { black } else { gray(110) };
            painter.add(egui::Shape::dashed_line(&[Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, col), 4.0, 3.0));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x, ruler.top() + 2.0), Pos2::new(x + 7.0, ruler.top() + 6.0), Pos2::new(x, ruler.top() + 10.0)],
                gray(60),
                Stroke::NONE,
            ));
            painter.line_segment([Pos2::new(x, ruler.top() + 2.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, gray(60)));
            let label = painter.layout_no_wrap(c.name.clone(), FontId::proportional(10.0), ink());
            let bg = Rect::from_min_size(Pos2::new(x + 8.0, ruler.top() + 1.0), label.size() + Vec2::new(4.0, 0.0));
            painter.rect_filled(bg, 2.0, gray(255));
            painter.galley(bg.min + Vec2::new(2.0, 0.0), label, ink());
        }

        // Regions: a band across the middle of the ruler with their names.
        let band = Rect::from_min_max(Pos2::new(ruler.left(), ruler.top() + 15.0), Pos2::new(ruler.right(), ruler.top() + 23.0));
        for r in &self.regions {
            let (x0, x1) = (frame_to_x(r.start as f64).max(wave.left()), frame_to_x(r.end as f64).min(wave.right()));
            if x1 <= x0 {
                continue;
            }
            let rr = Rect::from_min_max(Pos2::new(x0, band.top()), Pos2::new(x1, band.bottom()));
            painter.rect_filled(rr, 2.0, gray(150));
            let label = painter.layout_no_wrap(r.name.clone(), FontId::proportional(9.0), gray(255));
            if label.size().x + 6.0 < rr.width() {
                painter.galley(Pos2::new(rr.left() + 3.0, rr.center().y - label.size().y / 2.0), label, gray(255));
            }
        }
        // Loop points: a black bar with end ticks along the bottom of the ruler (in the clip
        // editor they're the brace in the strip instead).
        if let Some((ls, le)) = self.loop_points.filter(|_| strip.height() == 0.0) {
            let (x0, x1) = (frame_to_x(ls as f64), frame_to_x(le as f64));
            let y = ruler.bottom() - 4.0;
            let (c0, c1) = (x0.max(wave.left()), x1.min(wave.right()));
            if c1 > c0 {
                painter.line_segment([Pos2::new(c0, y), Pos2::new(c1, y)], Stroke::new(2.5, black));
            }
            for x in [x0, x1] {
                if x >= wave.left() && x <= wave.right() {
                    painter.line_segment([Pos2::new(x, y - 5.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.5, black));
                }
            }
        }

        // The start flag at 1.1.1 (where it's being dragged to, while dragged).
        if strip.height() > 0.0 {
            let f = match self.ed_ruler {
                Some(EdRuler::Start(f)) => f,
                _ => self.ed_markers().0,
            };
            let x = frame_to_x(f as f64);
            if x >= wave.left() - 1.0 && x <= wave.right() {
                let x = x.max(wave.left() + 1.0);
                painter.add(egui::Shape::convex_polygon(
                    vec![Pos2::new(x, ruler.bottom() - 15.0), Pos2::new(x + 12.0, ruler.bottom() - 10.0), Pos2::new(x, ruler.bottom() - 5.0)],
                    ink(),
                    Stroke::NONE,
                ));
                painter.line_segment([Pos2::new(x, ruler.bottom() - 15.0), Pos2::new(x, wave.bottom())], Stroke::new(1.5, ink()));
            }
        }

        // Insertion point (blinks like a text caret) and playhead.
        let cx = frame_to_x(self.cursor as f64);
        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        if self.sel.is_none() && cx >= wave.left() && cx <= wave.right() {
            if blink || playing {
                painter.line_segment([Pos2::new(cx, wave.top()), Pos2::new(cx, wave.bottom())], Stroke::new(1.0, black));
            }
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(cx - 4.0, ruler.bottom() - 6.0), Pos2::new(cx + 4.0, ruler.bottom() - 6.0), Pos2::new(cx, ruler.bottom())],
                black,
                Stroke::NONE,
            ));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        if playing {
            let px = frame_to_x(play_pos as f64);
            if px >= wave.left() && px <= wave.right() {
                painter.line_segment([Pos2::new(px, ruler.top()), Pos2::new(px, wave.bottom())], Stroke::new(1.5, black));
            }
        }

        // Scroll bar: light track with a grey thumb.
        painter.rect_filled(scroll, 0.0, panel());
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, rule()));
        let thumb_col = if self.scrollbar_drag { gray(120) } else { gray(185) };
        painter.rect_filled(thumb.shrink2(Vec2::new(1.0, 4.0)), 4.0, thumb_col);

        // Hover time readout.
        if let Some(p) = hover {
            let f = x_to_frame(p.x, vs, vl);
            painter.text(Pos2::new(wave.right() - 6.0, ruler.top() + 1.0), Align2::RIGHT_TOP, self.fmt_pos(f), FontId::monospace(11.0), muted());
        }
    }

    /// Draw a pencil line from the previous point to (`f`, `v`) in the stroke's channel.
    fn pencil_to(&mut self, f: usize, v: f32) {
        let Some(stroke) = self.pencil.as_mut() else { return };
        let (lf, lv) = stroke.last.unwrap_or((f, v));
        stroke.last = Some((f, v));
        let channel = stroke.channel;
        let Some(arc) = self.audio.as_mut() else { return };
        // Copies the audio once at the start of a stroke (the undo step keeps the original).
        let ch = &mut Arc::make_mut(arc).channels[channel];
        let ((f0, v0), (f1, v1)) = if lf <= f { ((lf, lv), (f, v)) } else { ((f, v), (lf, lv)) };
        for i in f0..=f1.min(ch.len().saturating_sub(1)) {
            let t = if f1 > f0 { (i - f0) as f32 / (f1 - f0) as f32 } else { 0.0 };
            ch[i] = v0 + (v1 - v0) * t;
        }
    }

}
