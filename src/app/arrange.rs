//! The arrangement view: ruler, track lanes and headers, clip dragging, and the clip and track
//! right-click menus.

use super::*;

/// Height of the master channel strip under the tracks.
const MASTER_H: f32 = 40.0;

/// Where the parts of the arrangement view sit this frame.
#[derive(Clone, Copy)]
struct ArrArea {
    full: Rect,
    ruler: Rect,
    lanes: Rect,
    scroll: Rect,
    lane_h: f32,
}

impl ArrArea {
    fn lane(&self, i: usize) -> Rect {
        Rect::from_min_size(Pos2::new(self.lanes.left(), self.lanes.top() + i as f32 * self.lane_h), Vec2::new(self.lanes.width(), self.lane_h))
    }

    fn x_to_f(&self, x: f32, vs: f64, vl: f64) -> f64 {
        (vs + ((x - self.lanes.left()) / self.lanes.width()) as f64 * vl).max(0.0)
    }

    fn f_to_x(&self, f: f64, vs: f64, vl: f64) -> f32 {
        self.lanes.left() + ((f - vs) / vl) as f32 * self.lanes.width()
    }

    /// Which clip is at a point, and whether the point is on one of its edges.
    fn hit(&self, tracks: &[Track], q: Pos2, vs: f64, vl: f64) -> Option<(usize, usize, DragMode)> {
        let (lanes, lane_h) = (self.lanes, self.lane_h);
        let f_to_x = |f: f64, vs: f64, vl: f64| self.f_to_x(f, vs, vl);
        if q.y < lanes.top() {
            return None;
        }
        let t = ((q.y - lanes.top()) / lane_h) as usize;
        let clips = &tracks.get(t)?.clips;
        // Later clips are drawn on top, so test them first.
        clips.iter().enumerate().rev().find_map(|(i, c)| {
            let (x0, x1) = (f_to_x(c.start as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
            if q.x < x0 - 1.0 || q.x > x1 + 1.0 {
                return None;
            }
            let edge = 6.0f32.min((x1 - x0) / 3.0);
            let mode = if q.x - x0 < edge {
                DragMode::TrimStart
            } else if x1 - q.x < edge {
                DragMode::TrimEnd
            } else {
                DragMode::Move
            };
            Some((t, i, mode))
        })
    }
}

/// A clip block in the arrangement: a name strip on top and its waveform below.
fn draw_clip(p: &egui::Painter, r: Rect, c: &Clip, color: Color32, selected: bool, muted: bool, clip_to: Rect) {
    let p = p.with_clip_rect(clip_to);
    let wave_col = if muted { Color32::from_gray(190) } else { color };
    let fill = if muted { Color32::from_gray(248) } else { color.lerp_to_gamma(Color32::WHITE, 0.88) };
    p.rect_filled(r, 4.0, fill);
    let title = Rect::from_min_max(r.min, Pos2::new(r.right(), (r.top() + 16.0).min(r.bottom())));
    p.rect_filled(title, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, if muted { Color32::from_gray(225) } else { color.lerp_to_gamma(Color32::WHITE, 0.55) });
    let mut name_x = r.left().max(clip_to.left()) + 5.0;
    if c.group.is_some() && r.right() - name_x > 30.0 {
        // Group clips get a small stacked-boxes mark before the name.
        let m = Rect::from_min_size(Pos2::new(name_x, title.center().y - 4.0), Vec2::new(8.0, 6.0));
        p.rect_stroke(m.translate(Vec2::new(2.0, 2.0)), 1.0, Stroke::new(1.0, INK), egui::StrokeKind::Inside);
        p.rect_filled(m, 1.0, INK);
        name_x += 15.0;
    }
    // The name stays on one line, cut short with "…" when the clip is narrow (and leaves room
    // for the loop mark).
    let max_w = r.right().min(clip_to.right()) - name_x - if c.looping { 20.0 } else { 4.0 };
    if max_w > 12.0 {
        let mut job = egui::text::LayoutJob::simple_singleline(c.name.clone(), FontId::proportional(11.0), INK);
        job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = p.ctx().fonts_mut(|f| f.layout_job(job));
        p.with_clip_rect(title.intersect(clip_to)).galley(Pos2::new(name_x, title.center().y - galley.size().y / 2.0), galley, INK);
    }
    // Waveform of the visible part, one min/max line per pixel (all channels together).
    let body = Rect::from_min_max(Pos2::new(r.left(), title.bottom() + 2.0), Pos2::new(r.right(), r.bottom() - 2.0));
    if body.height() > 4.0 && c.len > 0 {
        let mid = body.center().y;
        let amp = body.height() / 2.0 * 0.9;
        let x0 = body.left().max(clip_to.left());
        let x1 = body.right().min(clip_to.right());
        let fpp = c.len as f64 / body.width().max(1.0) as f64;
        let mut x = x0;
        while x < x1 {
            let i0 = ((x - body.left()) as f64 * fpp) as usize;
            let i1 = (((x + 1.0 - body.left()) as f64 * fpp) as usize).max(i0 + 1);
            let f0 = c.source(i0);
            let f1 = c.source(i1 - 1).max(f0) + 1;
            let f1 = f1.min(c.audio.frames());
            if f0 >= f1 {
                break;
            }
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            if fpp >= c.peaks.block as f64 * 2.0 {
                for ch in &c.peaks.data {
                    for &(l, h) in &ch[f0 / c.peaks.block..(f1 / c.peaks.block).max(f0 / c.peaks.block + 1).min(ch.len())] {
                        lo = lo.min(l);
                        hi = hi.max(h);
                    }
                }
            } else {
                for ch in &c.audio.channels {
                    for &v in &ch[f0..f1] {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            let y0 = mid - hi.clamp(-1.0, 1.0) * amp;
            let y1 = (mid - lo.clamp(-1.0, 1.0) * amp).max(y0 + 1.0);
            p.line_segment([Pos2::new(x + 0.5, y0), Pos2::new(x + 0.5, y1)], Stroke::new(1.0, wave_col));
            x += 1.0;
        }
    }
    if c.looping {
        // Dotted lines where the loop starts over.
        let (ls, le) = c.loop_range();
        let period = (le - ls).max(1);
        let mut i = le.saturating_sub(c.trim);
        let fpp = c.len as f64 / r.width().max(1.0) as f64;
        while i < c.len && (period as f64 / fpp) > 3.0 {
            let x = r.left() + (i as f64 / fpp) as f32;
            if x > r.left() + 1.0 {
                p.add(egui::Shape::dashed_line(&[Pos2::new(x, title.bottom()), Pos2::new(x, r.bottom())], Stroke::new(1.0, Color32::from_gray(120)), 2.0, 3.0));
            }
            i += period;
        }
        // Loop mark in the title strip, at the right.
        if r.width() > 40.0 {
            let ctr = Pos2::new(r.right().min(clip_to.right()) - 10.0, title.center().y);
            p.circle_stroke(ctr, 3.5, Stroke::new(1.2, INK));
            p.add(egui::Shape::convex_polygon(vec![ctr + Vec2::new(1.5, -5.5), ctr + Vec2::new(4.5, -3.5), ctr + Vec2::new(1.0, -1.5)], INK, Stroke::NONE));
        }
    }
    let stroke = if selected { Stroke::new(2.0, INK) } else { Stroke::new(1.0, color.lerp_to_gamma(Color32::WHITE, 0.3)) };
    p.rect_stroke(r, 4.0, stroke, egui::StrokeKind::Inside);
}

impl App {
    /// Right-click menu for a clip.
    fn clip_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let is_group = self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.group.is_some());
        let looping = self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.looping);
        let warped = self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.warp.is_some());
        let pick = |ui: &mut egui::Ui, label: &str, a: Action| {
            if ui.button(label).clicked() {
                ui.close();
                Some(a)
            } else {
                None
            }
        };
        let mut chosen = None;
        chosen = chosen.or(pick(ui, if is_group { "Open Group" } else { "Edit Clip" }, Action::EditClip));
        ui.separator();
        chosen = chosen.or(pick(ui, "Cut", Action::Cut));
        chosen = chosen.or(pick(ui, "Copy", Action::Copy));
        chosen = chosen.or(pick(ui, "Duplicate", Action::DuplicateClip));
        chosen = chosen.or(pick(ui, "Delete", Action::Delete));
        ui.separator();
        chosen = chosen.or(pick(ui, "Split Here", Action::SplitClip));
        chosen = chosen.or(pick(ui, "Slice at Grid", Action::SliceClip));
        chosen = chosen.or(pick(ui, "Reverse", Action::ReverseClip));
        chosen = chosen.or(pick(ui, "Add to Pool", Action::AddToPool));
        ui.menu_button("Mangle", |ui| {
            for (label, a) in [
                ("Shuffle", Action::MangleShuffle),
                ("Reverse Order", Action::MangleReverse),
                ("Swap Pairs", Action::MangleSwap),
                ("Stutter", Action::MangleStutter),
                ("Random Reverse", Action::MangleRandomReverse),
                ("Drop Out", Action::MangleDropOut),
            ] {
                chosen = chosen.or(pick(ui, label, a));
            }
            ui.menu_button("BBCut", |ui| {
                for (label, a) in [
                    ("Jungle", Action::MangleBBCut),
                    ("House", Action::MangleCutHouse),
                    ("Hip-Hop", Action::MangleCutHipHop),
                    ("Dembow", Action::MangleCutDembow),
                    ("Footwork", Action::MangleCutFootwork),
                    ("Trap", Action::MangleCutTrap),
                    ("Breakcore", Action::MangleCutBreakcore),
                    ("Halftime", Action::MangleCutHalftime),
                ] {
                    chosen = chosen.or(pick(ui, label, a));
                }
            });
        });
        chosen = chosen.or(pick(ui, if looping { "Stop Looping" } else { "Loop Clip" }, Action::LoopClip));
        chosen = chosen.or(pick(ui, if warped { "Original Speed" } else { "Warp to Tempo" }, Action::WarpClip));
        chosen = chosen.or(pick(ui, "Group", Action::GroupClips));
        if is_group {
            chosen = chosen.or(pick(ui, "Ungroup", Action::Ungroup));
        }
        chosen = chosen.or(pick(ui, "Collapse to Audio", Action::CollapseClip));
        ui.separator();
        chosen = chosen.or(pick(ui, "Set Tempo from Clip", Action::TempoFromClip));
        if let Some(a) = chosen {
            self.run(a, ctx);
        }
    }

    /// Right-click menu for a track.
    fn track_menu(&mut self, ui: &mut egui::Ui, t: usize, ctx: &egui::Context) {
        self.set_active(t);
        let Some(track) = self.tracks.get(t) else { return };
        let (mute, has_clips) = (track.mute, !track.clips.is_empty());
        let mut chosen = None;
        let mut collapse = false;
        let item = |ui: &mut egui::Ui, label: &str, enabled: bool| {
            let clicked = ui.add_enabled(enabled, Button::new(label)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        if item(ui, "New Layer", true) {
            chosen = Some(Action::NewTrack);
        }
        if item(ui, "Duplicate Layer", true) {
            chosen = Some(Action::DuplicateTrack);
        }
        if item(ui, "Delete Layer", self.tracks.len() > 1) {
            chosen = Some(Action::DeleteTrack);
        }
        ui.separator();
        if item(ui, "Move Layer Up", t > 0) {
            chosen = Some(Action::LayerUp);
        }
        if item(ui, "Move Layer Down", t + 1 < self.tracks.len()) {
            chosen = Some(Action::LayerDown);
        }
        if item(ui, "Merge Down", self.can_merge_down()) {
            chosen = Some(Action::MergeDown);
        }
        if item(ui, if mute { "Show Layer" } else { "Hide Layer" }, true) {
            chosen = Some(Action::MuteTrack);
        }
        if item(ui, "Next Colour", true) {
            chosen = Some(Action::TrackColor);
        }
        ui.separator();
        if item(ui, "Collapse Layer to One Clip", has_clips) {
            collapse = true;
        }
        if item(ui, "Import File as Clip…", true) {
            chosen = Some(Action::ImportClip);
        }
        if let Some(a) = chosen {
            self.run(a, ctx);
        }
        if collapse {
            self.collapse_track(t);
        }
    }

    /// Per-layer controls beside each lane: name, show/hide, volume and colour.
    fn track_headers(&mut self, ui: &mut egui::Ui, header: Rect, n_tracks: usize, lane: &dyn Fn(usize) -> Rect) {
        // The whole column, including the corners beside the ruler and scroll bar.
        let column = Rect::from_min_max(Pos2::new(header.left(), header.top() - RULER_H), Pos2::new(header.right(), header.bottom() + 15.0));
        ui.painter().rect_filled(column, 0.0, PANEL);
        ui.painter().line_segment([header.right_top(), header.right_bottom()], Stroke::new(1.0, LINE));
        let mut activate = None;
        let mut changed = false;
        for ti in 0..n_tracks {
            let l = lane(ti);
            let r = Rect::from_min_max(Pos2::new(header.left(), l.top()), Pos2::new(header.right() - 1.0, l.bottom()));
            let is_active = ti == self.active;
            if is_active {
                ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, INK);
            }
            if ti > 0 {
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, Color32::from_gray(190)));
            }
            let bg = ui.interact(r, ui.scope_id().with(("track_bg", ti)), Sense::click());
            if bg.clicked() {
                activate = Some(ti);
            }
            let ctx = ui.ctx().clone();
            bg.context_menu(|ui| self.track_menu(ui, ti, &ctx));
            let inner = r.shrink2(Vec2::new(10.0, 6.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                let t = &mut self.tracks[ti];
                let name = ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(inner.width()).font(egui::TextStyle::Body));
                if name.gained_focus() {
                    activate = Some(ti);
                }
                if inner.height() > 44.0 {
                    ui.horizontal(|ui| {
                        let (eye, eye_r) = ui.allocate_exact_size(Vec2::new(24.0, 20.0), Sense::click());
                        paint_eye(ui.painter(), eye, !t.mute, eye_r.hovered());
                        if eye_r.on_hover_text(if t.mute { "Show layer" } else { "Hide layer" }).clicked() {
                            t.mute = !t.mute;
                            changed = true;
                        }
                        let (sw, swr) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), Sense::click());
                        ui.painter().rect_filled(sw.shrink(3.0), 3.0, TRACK_COLORS[t.color % TRACK_COLORS.len()]);
                        if swr.on_hover_text("Layer colour (click to change)").clicked() {
                            t.color = (t.color + 1) % TRACK_COLORS.len();
                        }
                        ui.spacing_mut().slider_width = (inner.width() - 60.0).max(30.0);
                        let vol = ui.add(egui::Slider::new(&mut t.gain, 0.0..=2.0).show_value(false)).on_hover_text(format!("Volume {:.0}%", t.gain * 100.0));
                        if vol.drag_stopped() || (vol.changed() && !vol.dragged()) {
                            changed = true;
                        }
                    });
                }
            });
        }
        if let Some(ti) = activate {
            self.set_active(ti);
        }
        if changed {
            self.dirty = true;
            self.refresh_playback();
        }
    }

    // ---------- arrangement view ----------

    pub(super) fn arr_fit(&mut self) {
        self.arr_view_start = 0.0;
        self.arr_view_len = (self.arrangement_frames().max(self.rate() as usize * 4) as f64 * 1.1).max(32.0);
    }

    pub(super) fn arr_clamp(&mut self) {
        let n = (self.arrangement_frames() as f64 * 1.5).max(self.rate() as f64 * 30.0);
        self.arr_view_len = self.arr_view_len.clamp(32.0, n);
        self.arr_view_start = self.arr_view_start.clamp(0.0, (n - self.arr_view_len).max(0.0));
    }

    pub(super) fn arr_zoom(&mut self, factor: f64, anchor: f64) {
        let new_len = (self.arr_view_len * factor).max(32.0);
        let t = (anchor - self.arr_view_start) / self.arr_view_len;
        self.arr_view_start = anchor - t * new_len;
        self.arr_view_len = new_len;
        self.arr_clamp();
    }

    pub(super) fn beat_frames(&self) -> f64 {
        self.rate() as f64 * 60.0 / self.tempo.max(1.0)
    }

    pub(super) fn bar_frames(&self) -> f64 {
        self.beat_frames() * self.beats_per_bar as f64
    }

    /// The grid spacing in frames at the current zoom (`spp` = frames per pixel), if the grid is on.
    fn grid_frames(&self, spp: f64) -> Option<f64> {
        let beat = self.beat_frames();
        match GRID[self.grid.min(GRID.len() - 1)].1 {
            0.0 => None,
            -2.0 => Some(self.bar_frames()),
            -1.0 => {
                // Automatic: the finest division that still leaves about 14 pixels between lines.
                let options = [self.bar_frames() * 4.0, self.bar_frames() * 2.0, self.bar_frames(), beat * 2.0, beat, beat / 2.0, beat / 4.0, beat / 8.0];
                options.iter().rev().copied().find(|g| g / spp >= 14.0).or(Some(options[0]))
            }
            b => Some(beat * b),
        }
    }

    /// Snap a timeline position to the grid (when snapping is on).
    fn snap_to_grid(&self, f: f64, spp: f64) -> f64 {
        match (self.snap, self.grid_frames(spp)) {
            (true, Some(g)) => ((f / g).round() * g).max(0.0),
            _ => f,
        }
    }

    /// Position as bar.beat.sixteenth (1-based), for the arrangement ruler and readouts.
    pub(super) fn fmt_bars(&self, f: usize) -> String {
        let beats = f as f64 / self.beat_frames();
        let bar = (beats / self.beats_per_bar as f64).floor() as u64 + 1;
        let beat = (beats % self.beats_per_bar as f64).floor() as u64 + 1;
        let sixteenth = ((beats.fract()) * 4.0).floor() as u64 + 1;
        format!("{bar}.{beat}.{sixteenth}")
    }

    /// Where a file dropped at window point `q` would go: (track, or None for a new track; frame).
    /// A loop being dragged in from the pool: show where it will land, and place it on release.
    fn pool_drop(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let Some(q) = ui.input(|i| i.pointer.latest_pos()) else { return };
        if let Some(p) = resp.dnd_release_payload::<PoolDrag>() {
            return self.drop_from_pool(p.0, q);
        }
        let (Some(p), Some((track, frame)), Some((lanes, lane_h))) = (resp.dnd_hover_payload::<PoolDrag>(), self.drop_target_at(q), self.arr_lanes) else { return };
        let Some(len) = self.pool.get(p.0).map(|l| l.clip.len) else { return };
        let x = |f: usize| lanes.left() + ((f as f64 - self.arr_view_start) / self.arr_view_len) as f32 * lanes.width();
        let t = track.unwrap_or(self.tracks.len());
        let top = lanes.top() + t as f32 * lane_h;
        let r = Rect::from_min_max(Pos2::new(x(frame), top + 2.0), Pos2::new(x(frame + len), (top + lane_h - 2.0).min(lanes.bottom()))).intersect(lanes);
        ui.painter().rect(r, 4.0, INK.gamma_multiply(0.12), Stroke::new(1.5, INK), egui::StrokeKind::Inside);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
    }

    pub(super) fn drop_target_at(&self, q: Pos2) -> Option<(Option<usize>, usize)> {
        let (lanes, lane_h) = self.arr_lanes?;
        if !lanes.contains(q) {
            return None;
        }
        let raw = (self.arr_view_start + ((q.x - lanes.left()) / lanes.width()) as f64 * self.arr_view_len).max(0.0);
        let frame = self.snap_to_grid(raw, self.arr_view_len / lanes.width() as f64) as usize;
        let t = ((q.y - lanes.top()) / lane_h) as usize;
        Some((if t < self.tracks.len() { Some(t) } else { None }, frame))
    }

    /// Strip above the main view: where you are, and the way back from the editor.
    pub(super) fn view_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            match self.view {
                View::Arrangement => {
                    // Path: Song ▸ Group ▸ … ; click a level to go back up to it.
                    let mut names: Vec<String> = vec!["Song".into()];
                    for f in &self.nest {
                        names.push(f.tracks.get(f.track).and_then(|t| t.clips.get(f.clip)).map_or("Group".into(), |c| c.name.clone()));
                    }
                    let depth = names.len() - 1;
                    let mut go_up = None;
                    for (i, n) in names.iter().enumerate() {
                        if i > 0 {
                            ui.label(egui::RichText::new("▸").color(MUTED));
                        }
                        if i == depth {
                            ui.label(egui::RichText::new(n).strong().color(INK));
                        } else if ui.link(egui::RichText::new(n).color(MUTED)).clicked() {
                            go_up = Some(depth - i);
                        }
                    }
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add(egui::DragValue::new(&mut self.tempo).range(20.0..=400.0).speed(0.1).fixed_decimals(2).suffix(" BPM"))
                        .on_hover_text("Song tempo — drag or type");
                    ui.add(egui::DragValue::new(&mut self.beats_per_bar).range(1..=16).suffix(" / 4")).on_hover_text("Beats per bar");
                    ui.label(egui::RichText::new("Grid").color(MUTED));
                    egui::ComboBox::from_id_salt("grid").width(80.0).selected_text(GRID[self.grid].0).show_ui(ui, |ui| {
                        for (i, (label, _)) in GRID.iter().enumerate() {
                            ui.selectable_value(&mut self.grid, i, *label);
                        }
                    });
                    ui.checkbox(&mut self.snap, "Snap").on_hover_text("Snap moves, trims and the playhead to the grid (hold ⌘ to bypass)");
                    ui.separator();
                    let can_arrange = self.nest.is_empty() && self.tracks.iter().any(|t| !t.clips.is_empty());
                    if ui.add_enabled(can_arrange, Button::new("Auto Arrange")).on_hover_text("Build a 4-minute track from the loops in the song").clicked() {
                        self.auto_arrange();
                    }
                    if !self.pool.is_empty() && self.nest.is_empty() {
                        if ui.add_enabled(self.variation > 1, Button::new("◀")).on_hover_text("Previous variation").clicked() {
                            self.step_variation(false);
                        }
                        ui.label(egui::RichText::new(self.variation.to_string()).color(INK));
                        if ui.button("▶").on_hover_text("Next variation").clicked() {
                            self.step_variation(true);
                        }
                        let before = self.section_bars;
                        egui::ComboBox::from_id_salt("sections").width(80.0).selected_text(format!("{} bars", self.section_bars)).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.section_bars, 8, "8 bars");
                            ui.selectable_value(&mut self.section_bars, 16, "16 bars");
                        });
                        if self.section_bars != before {
                            self.build_arrangement();
                        }
                    }
                    if let Some(levels) = go_up {
                        for _ in 0..levels {
                            self.leave_group();
                        }
                    }
                }
                View::Editor { track, clip } => {
                    if ui.button("◀ Arrangement").on_hover_text("Back to the arrangement (⌘Return)").clicked() {
                        self.back_to_arrangement();
                        return;
                    }
                    let (tname, cname) = self
                        .tracks
                        .get(track)
                        .map(|t| (t.name.clone(), t.clips.get(clip).map(|c| c.name.clone()).unwrap_or_default()))
                        .unwrap_or_default();
                    ui.label(egui::RichText::new("Editing").color(MUTED));
                    if let Some(c) = self.tracks.get_mut(track).and_then(|t| t.clips.get_mut(clip)) {
                        ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).on_hover_text("Clip name");
                    } else {
                        ui.label(cname);
                    }
                    ui.label(egui::RichText::new(format!("on {tname}")).color(MUTED));
                }
            }
        });
    }

    /// The arrangement: track headers on the left, clips on track lanes along a timeline.
    pub(super) fn arrangement(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let header_w = 150.0;
        let ruler_h = 22.0;
        let ruler = Rect::from_min_max(Pos2::new(full.left() + header_w, full.top()), Pos2::new(full.right(), full.top() + ruler_h));
        let scroll = Rect::from_min_max(Pos2::new(full.left() + header_w, full.bottom() - 15.0), full.max);
        // The master channel sits under the tracks, above the scroll bar.
        let master = Rect::from_min_max(Pos2::new(full.left(), scroll.top() - MASTER_H), Pos2::new(full.right(), scroll.top()));
        let lanes = Rect::from_min_max(Pos2::new(full.left() + header_w, ruler.bottom()), Pos2::new(full.right(), master.top()));
        let n_tracks = self.tracks.len().max(1);
        let lane_h = (lanes.height() / n_tracks as f32).min(120.0);
        self.arr_lanes = Some((lanes, lane_h));
        let rate = self.rate() as f64;
        let spp = self.arr_view_len / lanes.width() as f64;

        let resp = ui.allocate_rect(Rect::from_min_max(ruler.min, lanes.max), Sense::click_and_drag());
        let a = ArrArea { full, ruler, lanes, scroll, lane_h };
        let lane = |i: usize| a.lane(i);
        let x_to_f = |x: f32, vs: f64, vl: f64| a.x_to_f(x, vs, vl);
        let hit = |tracks: &[Track], q: Pos2, vs: f64, vl: f64| a.hit(tracks, q, vs, vl);

        // Zoom and scroll, as in the editor.
        if resp.hovered() {
            let (zoom, delta, hover, alt) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos(), i.modifiers.alt));
            let anchor = hover.map_or(self.arr_view_start + self.arr_view_len / 2.0, |p| x_to_f(p.x, self.arr_view_start, self.arr_view_len));
            if zoom != 1.0 {
                self.arr_zoom(1.0 / zoom as f64, anchor);
            } else if alt && delta.y != 0.0 {
                self.arr_zoom((-delta.y as f64 / 150.0).exp(), anchor);
            } else {
                let d = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
                if d != 0.0 {
                    self.arr_view_start -= d as f64 / lanes.width() as f64 * self.arr_view_len;
                    self.arr_clamp();
                }
            }
        }

        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        let hover_hit = resp.hover_pos().and_then(|q| hit(&self.tracks, q, vs, vl));
        self.arr_hover = match (resp.hover_pos(), hover_hit) {
            (Some(q), Some((t, c, _))) => {
                let f = x_to_f(q.x, vs, vl);
                let f = if ui.input(|i| i.modifiers.command) { f } else { self.snap_to_grid(f, vl / lanes.width() as f64) };
                Some((t, c, f as usize))
            }
            _ => None,
        };
        if self.tool == Tool::Split {
            self.arr_split_tool(ui, &resp, &a, hover_hit);
        }
        match (self.clip_drag.as_ref().map(|d| d.mode), hover_hit.filter(|_| self.tool != Tool::Split).map(|h| h.2)) {
            (Some(DragMode::Move), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            (Some(_), _) | (None, Some(DragMode::TrimStart | DragMode::TrimEnd)) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            (None, Some(DragMode::Move)) => {
                let alt = ui.input(|i| i.modifiers.alt);
                ui.ctx().set_cursor_icon(if alt { egui::CursorIcon::Copy } else { egui::CursorIcon::Grab });
            }
            _ => {}
        }

        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        let split_tool = self.tool == Tool::Split;
        // Dragging in the ruler draws the arrangement's loop range (and turns Loop on).
        let ruler_drag = press.is_some_and(|q| q.y < ruler.bottom());
        if resp.drag_started() && ruler_drag {
            let f = x_to_f(press.unwrap().x, vs, vl);
            self.arr_loop_drag = Some(self.snap_to_grid(f, spp) as usize);
        }
        if let (Some(a), true, Some(q)) = (self.arr_loop_drag, resp.dragged(), resp.interact_pointer_pos()) {
            let b = self.snap_to_grid(x_to_f(q.x, vs, vl), spp) as usize;
            if b != a {
                self.arr_loop = Some((a.min(b), a.max(b)));
                self.looping = true;
            }
        }
        if resp.drag_stopped() && self.arr_loop_drag.take().is_some() {
            self.dirty = true;
            if self.player.is_playing() {
                self.toggle_play();
                self.toggle_play();
            }
        }
        if resp.drag_started() && !split_tool && !ruler_drag {
            if let Some(q) = press {
                if let Some((t, mut c, mode)) = hit(&self.tracks, q, vs, vl) {
                    // ⌥-drag leaves the original in place and drags a copy (as in Ableton).
                    let copy = mode == DragMode::Move && ui.input(|i| i.modifiers.alt);
                    self.push_undo(match (mode, copy) {
                        (_, true) => "Duplicate Clip",
                        (DragMode::Move, _) => "Move Clip",
                        _ => "Trim Clip",
                    });
                    if copy {
                        let dup = self.tracks[t].clips[c].clone();
                        self.tracks[t].clips.push(dup);
                        c = self.tracks[t].clips.len() - 1;
                    }
                    let orig = self.tracks[t].clips[c].clone();
                    let grab = x_to_f(q.x, vs, vl) as i64 - orig.start as i64;
                    self.clip_drag = Some(ClipDrag { track: t, clip: c, mode, grab, orig, new_track: None });
                    self.sel_extra.clear();
                    self.sel_clip = Some((t, c));
                    self.active = t;
                } else if q.y > lanes.top() {
                    // Empty space: drag a box to select the clips it touches.
                    let keep = if ui.input(|i| i.modifiers.shift) { self.selected_clips() } else { Vec::new() };
                    self.marquee = Some((q, keep));
                }
            }
        }
        if let (Some((from, keep)), true, Some(q)) = (self.marquee.clone(), resp.dragged(), resp.interact_pointer_pos()) {
            let r = Rect::from_two_pos(from, q);
            let mut picked = keep;
            for (ti, t) in self.tracks.iter().enumerate() {
                let l = lane(ti);
                if l.bottom() < r.top() || l.top() > r.bottom() {
                    continue;
                }
                for (ci, c) in t.clips.iter().enumerate() {
                    let (x0, x1) = (a.f_to_x(c.start as f64, vs, vl), a.f_to_x(c.end() as f64, vs, vl));
                    if x1 >= r.left() && x0 <= r.right() && !picked.contains(&(ti, ci)) {
                        picked.push((ti, ci));
                    }
                }
            }
            self.sel_clip = picked.first().copied();
            self.sel_extra = picked.iter().skip(1).copied().collect();
            if let Some((t, _)) = self.sel_clip {
                self.active = t;
            }
        }
        if resp.drag_stopped() {
            self.marquee = None;
        }
        if let (true, Some(q)) = (resp.dragged(), resp.interact_pointer_pos()) {
            let drag_grid = if self.snap { self.grid_frames(spp) } else { None };
            let drag_rate = self.rate() as usize;
            if let Some(d) = self.clip_drag.as_mut() {
                let f = x_to_f(q.x, vs, vl);
                // Snap to other clips' edges and the playhead within 8 pixels (⌘ turns it off).
                let snap_px = 8.0 * spp;
                let no_snap = ui.input(|i| i.modifiers.command);
                let (dt, dc) = (d.track, d.clip);
                let edges: Vec<f64> = std::iter::once(self.arr_cursor as f64)
                    .chain(self.tracks.iter().enumerate().flat_map(|(ti, t)| {
                        t.clips
                            .iter()
                            .enumerate()
                            .filter(move |(ci, _)| !(ti == dt && *ci == dc))
                            .flat_map(|(_, c)| [c.start as f64, c.end() as f64])
                    }))
                    .chain(std::iter::once(0.0))
                    .collect();
                let grid = drag_grid;
                let snap = |x: f64| -> f64 {
                    if no_snap {
                        return x;
                    }
                    let edge = edges.iter().copied().filter(|e| (e - x).abs() < snap_px).min_by(|a, b| (a - x).abs().total_cmp(&(b - x).abs()));
                    match (edge, grid) {
                        (Some(e), _) => e,
                        (None, Some(g)) => ((x / g).round() * g).max(0.0),
                        _ => x,
                    }
                };
                let o = &d.orig;
                match d.mode {
                    DragMode::Move => {
                        let raw = (f - d.grab as f64).max(0.0);
                        // Snap whichever edge (start or end) is closer to something.
                        let s1 = snap(raw);
                        let s2 = snap(raw + o.len as f64) - o.len as f64;
                        // With the grid on, the clip's start lands on the grid; otherwise use
                        // whichever edge snapped closer.
                        let start = if grid.is_some() && !no_snap { s1 } else if (s1 - raw).abs() <= (s2 - raw).abs() && s1 != raw { s1 } else if s2 != raw { s2 } else { raw };
                        let row = ((q.y - lanes.top()) / lane_h).max(0.0) as usize;
                        // Below the last track: give the clip a new track of its own (as in Ableton).
                        let target = match d.new_track {
                            Some(nt) if row >= nt => nt,
                            None if row >= self.tracks.len() && q.y < lanes.bottom() => {
                                let n = (1..).find(|i| !self.tracks.iter().any(|t| t.name == format!("Track {i}"))).unwrap();
                                self.tracks.push(Track::new(format!("Track {n}")));
                                d.new_track = Some(self.tracks.len() - 1);
                                self.tracks.len() - 1
                            }
                            _ => row.min(self.tracks.len() - 1),
                        };
                        let mut clip = self.tracks[d.track].clips.remove(d.clip);
                        clip.start = start.max(0.0) as usize;
                        self.tracks[target].clips.push(clip);
                        // Dragged back up out of the track it made: take that track away again.
                        if let Some(nt) = d.new_track.filter(|&nt| nt != target && self.tracks[nt].clips.is_empty() && nt == self.tracks.len() - 1) {
                            self.tracks.remove(nt);
                            d.new_track = None;
                        }
                        d.track = target;
                        d.clip = self.tracks[target].clips.len() - 1;
                        self.sel_clip = Some((d.track, d.clip));
                        self.active = target;
                    }
                    DragMode::TrimStart => {
                        // The left edge can move back as far as the audio goes, and up to near the end.
                        let lo = o.start as f64 - o.trim as f64;
                        let hi = (o.end() - 32) as f64;
                        let start = snap(f).clamp(lo.max(0.0), hi);
                        let c = &mut self.tracks[d.track].clips[d.clip];
                        let delta = start as i64 - o.start as i64;
                        c.start = start as usize;
                        c.trim = (o.trim as i64 + delta) as usize;
                        c.len = (o.len as i64 - delta) as usize;
                    }
                    DragMode::TrimEnd => {
                        let max_end = if o.looping {
                            (o.start + drag_rate * 3600) as f64
                        } else {
                            (o.start + o.audio.frames() - o.trim) as f64
                        };
                        let end = snap(f).clamp((o.start + 32) as f64, max_end);
                        self.tracks[d.track].clips[d.clip].len = end as usize - o.start;
                    }
                }
            }
        }
        if resp.drag_stopped() && self.clip_drag.take().is_some() {
            self.dirty = true;
            self.refresh_playback();
        }
        self.arr_right_click(ui, &resp, &a);
        if split_tool {
            // Handled above.
        } else if resp.double_clicked() {
            if let Some((t, c, _)) = resp.interact_pointer_pos().and_then(|q| hit(&self.tracks, q, vs, vl)) {
                self.edit_clip((t, c));
                return;
            }
        } else if resp.clicked() {
            if let Some(q) = resp.interact_pointer_pos() {
                match hit(&self.tracks, q, vs, vl) {
                    Some((t, c, _)) => {
                        if ui.input(|i| i.modifiers.shift) {
                            // Shift-click adds or removes clips from the selection.
                            if let Some(prev) = self.sel_clip.filter(|&p| p != (t, c)) {
                                if !self.sel_extra.contains(&prev) {
                                    self.sel_extra.push(prev);
                                }
                            }
                            if let Some(i) = self.sel_extra.iter().position(|&p| p == (t, c)) {
                                self.sel_extra.remove(i);
                            }
                        } else {
                            self.sel_extra.clear();
                        }
                        self.sel_clip = Some((t, c));
                        self.active = t;
                    }
                    None => {
                        // Clicking empty space moves the playhead (and picks that track).
                        self.sel_clip = None;
                        self.sel_extra.clear();
                        let f = x_to_f(q.x, vs, vl);
                        let f = if ui.input(|i| i.modifiers.command) { f } else { self.snap_to_grid(f, spp) };
                        self.arr_cursor = f as usize;
                        self.play_origin = self.arr_cursor;
                        if q.y > lanes.top() {
                            self.set_active((((q.y - lanes.top()) / lane_h) as usize).min(n_tracks - 1));
                        }
                        if self.player.is_playing() {
                            let end = self.arrangement_frames();
                            self.player.play(self.arr_cursor, end, false);
                        }
                    }
                }
            }
        }

        // Scroll bar.
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let total = (self.arrangement_frames() as f64 * 1.5).max(rate * 30.0);
        let thumb_w = ((self.arr_view_len / total) * scroll.width() as f64).clamp(16.0, scroll.width() as f64) as f32;
        let track_w = scroll.width() - thumb_w;
        let max_start = (total - self.arr_view_len).max(0.0);
        if (sresp.dragged() || sresp.clicked()) && track_w > 0.0 {
            if let Some(p) = sresp.interact_pointer_pos() {
                self.arr_view_start = (((p.x - scroll.left() - thumb_w / 2.0) / track_w).clamp(0.0, 1.0) as f64) * max_start;
                self.arr_clamp();
            }
        }

        // Follow the playhead.
        let playing = self.player.is_playing();
        let play_pos = self.player.position();
        if playing {
            ui.ctx().request_repaint();
            let p = play_pos as f64;
            if self.clip_drag.is_none() && (p > self.arr_view_start + self.arr_view_len || p < self.arr_view_start) {
                self.arr_view_start = p - self.arr_view_len * 0.05;
                self.arr_clamp();
            }
        }

        let thumb_x = if max_start > 0.0 { (self.arr_view_start / max_start) as f32 * track_w } else { 0.0 };
        let thumb = Rect::from_min_size(Pos2::new(scroll.left() + thumb_x, scroll.top()), Vec2::new(thumb_w, scroll.height()));
        self.paint_arrangement(ui, &a, thumb, resp.hover_pos(), playing, play_pos);
        self.pool_drop(ui, &resp);
        if let (Some((from, _)), Some(q)) = (&self.marquee, resp.interact_pointer_pos()) {
            let r = Rect::from_two_pos(*from, q).intersect(lanes);
            ui.painter().rect(r, 0.0, INK.gamma_multiply(0.06), Stroke::new(1.0, INK.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        }
        if !self.tracks.is_empty() {
            let header = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.left() + header_w, lanes.bottom()));
            self.track_headers(ui, header, self.tracks.len(), &lane);
            self.master_strip(ui, master, header_w);
        }
    }

    /// The master channel: the whole mix's volume and level, and the headphone crossfeed switch.
    fn master_strip(&mut self, ui: &mut egui::Ui, r: Rect, header_w: f32) {
        let p = ui.painter();
        p.rect_filled(r, 0.0, PANEL);
        // The corner beside the scroll bar, below the header column.
        p.rect_filled(Rect::from_min_size(Pos2::new(r.left(), r.bottom()), Vec2::new(header_w, 15.0)), 0.0, PANEL);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, Color32::from_gray(190)));
        p.line_segment([Pos2::new(r.left() + header_w - 1.0, r.top()), Pos2::new(r.left() + header_w - 1.0, r.bottom())], Stroke::new(1.0, LINE));
        // Level meters along the lane side.
        let m = Rect::from_min_max(Pos2::new(r.left() + header_w + 12.0, r.top() + 11.0), Pos2::new((r.left() + header_w + 372.0).min(r.right() - 12.0), r.bottom() - 11.0));
        let n = self.meter.len().max(1);
        let h = (m.height() - 2.0 * (n - 1) as f32) / n as f32;
        for c in 0..n {
            let row = Rect::from_min_size(Pos2::new(m.left(), m.top() + c as f32 * (h + 2.0)), Vec2::new(m.width(), h));
            draw_meter(p, row, self.meter.get(c).copied().unwrap_or(0.0));
        }
        let inner = Rect::from_min_max(Pos2::new(r.left() + 10.0, r.top() + 1.0), Pos2::new(r.left() + header_w - 10.0, r.bottom()));
        let mut toggle = false;
        let mut changed = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
            ui.label(egui::RichText::new("Master").strong().color(INK));
            let (b, resp) = ui.allocate_exact_size(Vec2::new(26.0, 22.0), Sense::click());
            let on = self.crossfeed;
            let hover = resp.hovered();
            ui.painter().rect(b, 4.0, if on { INK } else if hover { Color32::from_gray(232) } else { Color32::WHITE }, Stroke::new(1.0, if on { INK } else { LINE }), egui::StrokeKind::Inside);
            paint_icon(ui.painter(), b, Action::Crossfeed, if on { Color32::WHITE } else { INK }, false);
            if resp.on_hover_text(if on { "Headphone crossfeed is on (click to turn off)" } else { "Headphone crossfeed: hear the mix as if from speakers" }).clicked() {
                toggle = true;
            }
            ui.spacing_mut().slider_width = ui.available_width().max(20.0);
            let vol = ui.add(egui::Slider::new(&mut self.master_gain, 0.0..=2.0).show_value(false)).on_hover_text(format!("Master volume {:.0}%", self.master_gain * 100.0));
            if vol.double_clicked() {
                self.master_gain = 1.0;
                changed = true;
            }
            if vol.drag_stopped() || (vol.changed() && !vol.dragged()) {
                changed = true;
            }
        });
        if toggle {
            let ctx = ui.ctx().clone();
            self.run(Action::Crossfeed, &ctx);
        }
        if changed {
            self.dirty = true;
            self.refresh_playback();
        }
    }

    /// Split tool: a line shows where the cut will go; a click cuts the clip there.
    fn arr_split_tool(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &ArrArea, hover_hit: Option<(usize, usize, DragMode)>) {
        let ArrArea { lanes, .. } = *a;
        let lane = |i: usize| a.lane(i);
        let x_to_f = |x: f32, vs: f64, vl: f64| a.x_to_f(x, vs, vl);
        let f_to_x = |f: f64, vs: f64, vl: f64| a.f_to_x(f, vs, vl);
        let hit = |tracks: &[Track], q: Pos2, vs: f64, vl: f64| a.hit(tracks, q, vs, vl);
        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        let spp = vl / lanes.width() as f64;
        let cmd = ui.input(|i| i.modifiers.command);
        let at = |q: Pos2| {
            let f = x_to_f(q.x, vs, vl);
            (if cmd { f } else { self.snap_to_grid(f, spp) }) as usize
        };
        if let (Some(q), Some((t, c, _))) = (resp.hover_pos(), hover_hit) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            let f = at(q);
            let clip = &self.tracks[t].clips[c];
            if f > clip.start && f < clip.end() {
                let x = f_to_x(f as f64, vs, vl);
                let l = lane(t);
                ui.painter().line_segment([Pos2::new(x, l.top() + 2.0), Pos2::new(x, l.bottom() - 2.0)], Stroke::new(1.5, INK));
            }
        }
        if resp.clicked() {
            if let Some(q) = resp.interact_pointer_pos() {
                if let Some((t, c, _)) = hit(&self.tracks, q, vs, vl) {
                    let f = at(q);
                    self.split_clip_at((t, c), f);
                    self.active = t;
                } else {
                    let f = at(q);
                    self.arr_cursor = f;
                    self.sel_clip = None;
                }
            }
        }
    }

    /// Right-click: select what's under the pointer, then show the clip or track menu.
    fn arr_right_click(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &ArrArea) {
        let ArrArea { lanes, lane_h, .. } = *a;
        let hit = |tracks: &[Track], q: Pos2, vs: f64, vl: f64| a.hit(tracks, q, vs, vl);
        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        if resp.secondary_clicked() {
            if let Some(q) = resp.interact_pointer_pos() {
                match hit(&self.tracks, q, vs, vl) {
                    Some((t, c, _)) => {
                        if !self.selected_clips().contains(&(t, c)) {
                            self.sel_clip = Some((t, c));
                            self.sel_extra.clear();
                        }
                        self.active = t;
                        self.ctx_menu_track = None;
                    }
                    None => {
                        self.sel_clip = None;
                        let t = (((q.y - lanes.top()) / lane_h).max(0.0) as usize).min(self.tracks.len().saturating_sub(1));
                        self.ctx_menu_track = Some(t);
                    }
                }
            }
        }
        let ctx = ui.ctx().clone();
        resp.context_menu(|ui| {
            match self.ctx_menu_track {
                Some(t) => self.track_menu(ui, t, &ctx),
                None if self.sel_clip.is_some() => self.clip_menu(ui, &ctx),
                None => {}
            }
        });
    }

    /// Paint the arrangement: ruler, loop brace, lanes and clips, playhead and scroll bar.
    fn paint_arrangement(&self, ui: &egui::Ui, a: &ArrArea, thumb: Rect, hover: Option<Pos2>, playing: bool, play_pos: usize) {
        let ArrArea { full, ruler, lanes, scroll, .. } = *a;
        let lane = |i: usize| a.lane(i);
        let x_to_f = |x: f32, vs: f64, vl: f64| a.x_to_f(x, vs, vl);
        let f_to_x = |f: f64, vs: f64, vl: f64| a.f_to_x(f, vs, vl);
        let rate = self.rate() as f64;
        let spp = self.arr_view_len / lanes.width() as f64;
        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        let painter = ui.painter_at(full);
        painter.rect_filled(lanes, 0.0, Color32::WHITE);
        painter.rect_filled(ruler, 0.0, PANEL);
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, LINE));
        // Ruler: bars and beats when the grid is on, otherwise time.
        if let Some(g) = self.grid_frames(spp) {
            let bar = self.bar_frames();
            let beat = self.beat_frames();
            // Label every bar, or every 2/4/8… bars when zoomed out.
            let mut label_every = 1u64;
            while bar * label_every as f64 / spp < 40.0 && label_every < 4096 {
                label_every *= 2;
            }
            let mut k = (vs / g).floor() as i64;
            loop {
                let f = k as f64 * g;
                if f > vs + vl {
                    break;
                }
                let x = f_to_x(f, vs, vl);
                if x >= lanes.left() {
                    let on_bar = (f / bar).fract().abs() < 1e-6 || (f / bar).fract() > 1.0 - 1e-6;
                    let on_beat = (f / beat).fract().abs() < 1e-6 || (f / beat).fract() > 1.0 - 1e-6;
                    let (h, line) = if on_bar { (9.0, 228) } else if on_beat { (5.0, 240) } else { (3.0, 246) };
                    painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, MUTED));
                    painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, Color32::from_gray(line)));
                    let bar_no = (f / bar).round() as u64;
                    if on_bar && bar_no % label_every == 0 {
                        painter.text(Pos2::new(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, (bar_no + 1).to_string(), FontId::proportional(11.0), MUTED);
                    }
                }
                k += 1;
            }
        }
        let secs_per_px = if self.grid_frames(spp).is_some() { f64::INFINITY } else { vl / rate / lanes.width() as f64 };
        let nice = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = *nice.iter().find(|&&s| s / secs_per_px >= 90.0).unwrap_or(&1200.0);
        let minor = step / 5.0;
        let mut k = (vs / rate / minor).floor() as i64;
        loop {
            let t = k as f64 * minor;
            if t > (vs + vl) / rate || !secs_per_px.is_finite() {
                break;
            }
            let x = f_to_x(t * rate, vs, vl);
            if x >= lanes.left() {
                let major = k % 5 == 0;
                painter.line_segment([Pos2::new(x, ruler.bottom() - if major { 7.0 } else { 3.0 }), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, MUTED));
                if major {
                    painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, Color32::from_gray(242)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, ruler_label(self.ruler, rate as u32, t, step), FontId::proportional(11.0), MUTED);
                }
            }
            k += 1;
        }
        // Arrangement loop: a brace along the top of the ruler (black when Loop is on).
        if let Some((ls, le)) = self.arr_loop {
            let (x0, x1) = (f_to_x(ls as f64, vs, vl).max(lanes.left()), f_to_x(le as f64, vs, vl).min(lanes.right()));
            if x1 > x0 {
                let col = if self.looping { INK } else { Color32::from_gray(175) };
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, ruler.top() + 1.0), Pos2::new(x1, ruler.top() + 5.0)), 1.0, col);
                if self.looping {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, lanes.top()), Pos2::new(x1, lanes.bottom())), 0.0, Color32::from_black_alpha(8));
                }
            }
        }
        // Lanes and clips.
        let hidden = layers::covered_above(&self.tracks);
        for (ti, t) in self.tracks.iter().enumerate() {
            let l = lane(ti);
            if ti == self.active {
                painter.rect_filled(l, 0.0, Color32::from_gray(250));
            }
            painter.line_segment([l.left_bottom(), l.right_bottom()], Stroke::new(1.0, Color32::from_gray(225)));
            let muted = t.mute;
            let color = TRACK_COLORS[t.color % TRACK_COLORS.len()];
            let fades = crossfades(&t.clips);
            for (ci, c) in t.clips.iter().enumerate() {
                let (x0, x1) = (f_to_x(c.start as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
                if x1 < lanes.left() || x0 > lanes.right() {
                    continue;
                }
                let r = Rect::from_min_max(Pos2::new(x0, l.top() + 3.0), Pos2::new(x1.max(x0 + 2.0), l.bottom() - 3.0));
                let selected = self.sel_clip == Some((ti, ci)) || self.sel_extra.contains(&(ti, ci));
                draw_clip(&painter, r, c, color, selected, muted, lanes);
                // Parts a layer above plays over are washed out: they are not heard.
                for &(s, e) in &hidden[ti] {
                    let (s, e) = (s.max(c.start), e.min(c.end()));
                    if e > s {
                        let (hx0, hx1) = (f_to_x(s as f64, vs, vl), f_to_x(e as f64, vs, vl));
                        let h = Rect::from_min_max(Pos2::new(hx0, r.top()), Pos2::new(hx1, r.bottom())).intersect(lanes);
                        painter.rect_filled(h, 0.0, Color32::from_white_alpha(185));
                    }
                }
            }
            // Crossfades: an X across each overlap, drawn over both clips.
            for (c, &(_, fout)) in t.clips.iter().zip(&fades) {
                if fout == 0 {
                    continue;
                }
                let (x0, x1) = (f_to_x((c.end() - fout) as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
                if x1 < lanes.left() || x0 > lanes.right() {
                    continue;
                }
                let r = Rect::from_min_max(Pos2::new(x0, (l.top() + 19.0).min(l.bottom() - 3.0)), Pos2::new(x1, l.bottom() - 3.0));
                let p = painter.with_clip_rect(lanes);
                p.rect_filled(r, 0.0, Color32::from_white_alpha(140));
                let stroke = Stroke::new(1.5, INK);
                p.line_segment([r.left_top(), r.right_bottom()], stroke);
                p.line_segment([r.left_bottom(), r.right_top()], stroke);
                p.rect_stroke(r, 0.0, Stroke::new(1.0, INK), egui::StrokeKind::Inside);
            }
        }
        if self.tracks.is_empty() {
            painter.text(lanes.center(), Align2::CENTER_CENTER, "Drop a sound here, or choose File ▸ Open", FontId::proportional(16.0), Color32::from_gray(120));
        }
        // Playhead (or the playback position).
        let ph = if playing { play_pos } else { self.arr_cursor };
        let x = f_to_x(ph as f64, vs, vl);
        if x >= lanes.left() && x <= lanes.right() {
            painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.5, INK));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x - 5.0, ruler.bottom() - 7.0), Pos2::new(x + 5.0, ruler.bottom() - 7.0), Pos2::new(x, ruler.bottom())],
                INK,
                Stroke::NONE,
            ));
        }
        // Scroll bar.
        painter.rect_filled(scroll, 0.0, PANEL);
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, LINE));
        painter.rect_filled(thumb.shrink2(Vec2::new(1.0, 4.0)), 4.0, Color32::from_gray(185));
        if let Some(q) = hover {
            let f = x_to_f(q.x, vs, vl) as usize;
            let text = if self.grid_frames(spp).is_some() { format!("{}  {}", self.fmt_bars(f), self.fmt_pos(f)) } else { self.fmt_pos(f) };
            painter.text(Pos2::new(lanes.right() - 6.0, ruler.top() + 2.0), Align2::RIGHT_TOP, text, FontId::monospace(11.0), MUTED);
        }
    }
}
