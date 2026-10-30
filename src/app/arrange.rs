//! The arrangement view: ruler, track lanes and headers, clip dragging, and the clip and track
//! right-click menus.

use super::*;

/// Height of the master channel strip under the tracks.
const MASTER_H: f32 = 40.0;

/// Where the parts of the arrangement view sit this frame.
#[derive(Clone, Copy)]
pub(super) struct ArrArea {
    pub(super) full: Rect,
    pub(super) ruler: Rect,
    pub(super) lanes: Rect,
    pub(super) scroll: Rect,
    pub(super) lane_h: f32,
}

impl ArrArea {
    fn lane(&self, i: usize) -> Rect {
        Rect::from_min_size(Pos2::new(self.lanes.left(), self.lanes.top() + i as f32 * self.lane_h), Vec2::new(self.lanes.width(), self.lane_h))
    }

    pub(super) fn x_to_f(&self, x: f32, vs: f64, vl: f64) -> f64 {
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
fn draw_clip(p: &egui::Painter, r: Rect, c: &Clip, color: Color32, selected: bool, muted: bool, handles: bool, clip_to: Rect) {
    let p = p.with_clip_rect(clip_to);
    // As in Live: the name strip in the clip's colour, a lighter body, a near-black waveform and
    // black text in either theme; a clip that's off goes grey.
    const TEXT: Color32 = Color32::from_gray(10);
    let wave_col = if muted { Color32::from_gray(70) } else { Color32::from_rgba_unmultiplied(23, 23, 23, 225) };
    let fill = if muted { Color32::from_gray(140) } else { color.lerp_to_gamma(Color32::WHITE, 0.3) };
    p.rect_filled(r, 3.0, fill);
    let title = Rect::from_min_max(r.min, Pos2::new(r.right(), (r.top() + 16.0).min(r.bottom())));
    p.rect_filled(title, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, if muted { Color32::from_gray(105) } else { color });
    let mut name_x = r.left().max(clip_to.left()) + 5.0;
    if c.group.is_some() && r.right() - name_x > 30.0 {
        // Group clips get a small stacked-boxes mark before the name.
        let m = Rect::from_min_size(Pos2::new(name_x, title.center().y - 4.0), Vec2::new(8.0, 6.0));
        p.rect_stroke(m.translate(Vec2::new(2.0, 2.0)), 1.0, Stroke::new(1.0, TEXT), egui::StrokeKind::Inside);
        p.rect_filled(m, 1.0, TEXT);
        name_x += 15.0;
    }
    // The name stays on one line, cut short with "…" when the clip is narrow (and leaves room
    // for the loop mark).
    // Clips that don't follow the tempo (warp off) say so in the strip.
    let no_warp = c.warp.is_none() && c.group.is_none();
    let badge_w = if no_warp { 50.0 } else { 0.0 };
    let max_w = r.right().min(clip_to.right()) - name_x - badge_w - if c.looping { 20.0 } else { 4.0 };
    if max_w > 12.0 {
        let mut job = egui::text::LayoutJob::simple_singleline(c.name.clone(), FontId::proportional(11.0), TEXT);
        job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = p.ctx().fonts_mut(|f| f.layout_job(job));
        p.with_clip_rect(title.intersect(clip_to)).galley(Pos2::new(name_x, title.center().y - galley.size().y / 2.0), galley, TEXT);
    }
    // Waveform of the visible part, one min/max line per pixel (all channels together).
    let body = Rect::from_min_max(Pos2::new(r.left(), title.bottom() + 2.0), Pos2::new(r.right(), r.bottom() - 2.0));
    if body.height() > 4.0 && c.len > 0 {
        let mid = body.center().y;
        let amp = body.height() / 2.0 * 0.9;
        let x0 = body.left().max(clip_to.left());
        let x1 = body.right().min(clip_to.right());
        let fpp = c.len as f64 / body.width().max(1.0) as f64;
        let (audio, peaks, speed, frames) = c.picture();
        let mut x = x0;
        while x < x1 {
            let i0 = ((x - body.left()) as f64 * fpp) as usize;
            let i1 = (((x + 1.0 - body.left()) as f64 * fpp) as usize).max(i0 + 1);
            let f0 = c.source_in(i0, frames);
            let f1 = (c.source_in(i1 - 1, frames).max(f0) + 1).min(frames);
            // In the frames of what's drawn from.
            let f0 = ((f0 as f64 * speed) as usize).min(audio.frames());
            let f1 = ((f1 as f64 * speed).ceil() as usize).min(audio.frames());
            if f0 >= f1 {
                break;
            }
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            if fpp * speed >= peaks.block as f64 * 2.0 {
                for ch in &peaks.data {
                    for &(l, h) in &ch[f0 / peaks.block..(f1 / peaks.block).max(f0 / peaks.block + 1).min(ch.len())] {
                        lo = lo.min(l);
                        hi = hi.max(h);
                    }
                }
            } else {
                for ch in &audio.channels {
                    for &v in &ch[f0..f1] {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            // Drawn as heard: smaller where the clip fades in or out.
            let g = fade_gain(c, i0);
            let (lo, hi) = (lo * g, hi * g);
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
                p.add(egui::Shape::dashed_line(&[Pos2::new(x, title.bottom()), Pos2::new(x, r.bottom())], Stroke::new(1.0, Color32::from_black_alpha(110)), 2.0, 3.0));
            }
            i += period;
        }
        // Loop mark in the title strip, at the right.
        if r.width() > 40.0 {
            let ctr = Pos2::new(r.right().min(clip_to.right()) - 10.0, title.center().y);
            p.circle_stroke(ctr, 3.5, Stroke::new(1.2, TEXT));
            p.add(egui::Shape::convex_polygon(vec![ctr + Vec2::new(1.5, -5.5), ctr + Vec2::new(4.5, -3.5), ctr + Vec2::new(1.0, -1.5)], TEXT, Stroke::NONE));
        }
    }
    if no_warp && r.width() > 70.0 {
        let right = r.right().min(clip_to.right()) - if c.looping { 20.0 } else { 4.0 };
        let badge = Rect::from_min_max(Pos2::new(right - 46.0, title.top() + 2.0), Pos2::new(right, title.bottom() - 2.0));
        if badge.left() > r.left().max(clip_to.left()) + 20.0 {
            p.rect_filled(badge, 3.0, Color32::from_white_alpha(150));
            p.text(badge.center(), Align2::CENTER_CENTER, "no warp", FontId::proportional(9.5), TEXT);
        }
    }
    // Fades: a line from silence at the edge up to full level, as in Live.
    let (fi, fo) = fade_xs(r, c);
    let body_top = (r.top() + 18.0).min(r.bottom());
    if c.fade_in > 0 {
        p.line_segment([Pos2::new(r.left(), r.bottom() - 2.0), Pos2::new(fi, body_top)], Stroke::new(1.0, TEXT.gamma_multiply(0.7)));
    }
    if c.fade_out > 0 {
        p.line_segment([Pos2::new(fo, body_top), Pos2::new(r.right(), r.bottom() - 2.0)], Stroke::new(1.0, TEXT.gamma_multiply(0.7)));
    }
    // The handles that set them, on the selected or hovered clip.
    if handles && r.width() > 24.0 && r.height() > 30.0 {
        let (hi, ho) = fade_handle_xs(r, c);
        for x in [hi, ho] {
            let h = Rect::from_center_size(Pos2::new(x, body_top + 5.0), Vec2::splat(7.0));
            p.rect_filled(h, 1.0, Color32::WHITE);
            p.rect_stroke(h, 1.0, Stroke::new(1.0, TEXT), egui::StrokeKind::Inside);
        }
    }
    let stroke = if selected { Stroke::new(2.0, ink()) } else { Stroke::new(1.0, Color32::from_black_alpha(90)) };
    p.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Inside);
}

/// A clip's fade level `i` frames in (equal power, as heard), from its fade handles.
fn fade_gain(c: &Clip, i: usize) -> f32 {
    let mut g = 1.0;
    if i < c.fade_in {
        g *= (i as f32 / c.fade_in as f32 * std::f32::consts::FRAC_PI_2).sin();
    }
    let k = c.len.saturating_sub(1 + i);
    if k < c.fade_out {
        g *= ((k + 1) as f32 / c.fade_out as f32 * std::f32::consts::FRAC_PI_2).sin();
    }
    g
}

/// Where a clip's fades end (x), in its rect `r`.
fn fade_xs(r: Rect, c: &Clip) -> (f32, f32) {
    let px = |f: usize| f as f32 / c.len.max(1) as f32 * r.width();
    (r.left() + px(c.fade_in), r.right() - px(c.fade_out))
}

/// Where its fade handles sit: at the fades' ends, kept inside the clip's border.
fn fade_handle_xs(r: Rect, c: &Clip) -> (f32, f32) {
    let (fi, fo) = fade_xs(r, c);
    let (lo, hi) = (r.left() + 5.0, (r.right() - 5.0).max(r.left() + 5.0));
    (fi.clamp(lo, hi), fo.clamp(lo, hi))
}

impl App {
    /// Give the selected clips palette colour `i`, as Live's clip colour picker.
    pub(super) fn set_clip_color(&mut self, i: usize) {
        let mut ps: Vec<(usize, usize)> = self.sel_clip.into_iter().collect();
        ps.extend(self.sel_extra.iter().copied());
        self.push_undo("Clip Colour");
        for p in ps {
            if let Some(c) = self.clip_mut(p) {
                c.color = Some(i);
            }
        }
        self.dirty = true;
    }

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
        let current = self.sel_clip.and_then(|p| Some(self.clip(p)?.color.unwrap_or(self.tracks.get(p.0)?.color)));
        if let Some(i) = color_picker(ui, current.unwrap_or(usize::MAX)) {
            ui.close();
            self.set_clip_color(i);
        }
        ui.separator();
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
        if ui.button("Copy to Session").clicked() {
            ui.close();
            self.copy_to_session();
        }
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
        let (mute, has_clips, color) = (track.mute, !track.clips.is_empty(), track.color);
        if let Some(i) = color_picker(ui, color) {
            ui.close();
            self.push_undo("Track Colour");
            self.tracks[t].color = i;
            self.dirty = true;
        }
        ui.separator();
        let mut chosen = None;
        let mut collapse = false;
        let item = |ui: &mut egui::Ui, label: &str, enabled: bool| {
            let clicked = ui.add_enabled(enabled, Button::new(label)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        if item(ui, "New Track", true) {
            chosen = Some(Action::NewTrack);
        }
        if item(ui, "Duplicate Track", true) {
            chosen = Some(Action::DuplicateTrack);
        }
        if item(ui, "Delete Track", self.tracks.len() > 1) {
            chosen = Some(Action::DeleteTrack);
        }
        ui.separator();
        if item(ui, "Move Track Up", t > 0) {
            chosen = Some(Action::LayerUp);
        }
        if item(ui, "Move Track Down", t + 1 < self.tracks.len()) {
            chosen = Some(Action::LayerDown);
        }
        if item(ui, if mute { "Unmute" } else { "Mute" }, true) {
            chosen = Some(Action::MuteTrack);
        }
        ui.menu_button("Add Effect", |ui| self.add_effect_menu(ui, t));
        if item(ui, "Show Effects", true) {
            self.show_track_devices(t);
        }
        ui.separator();
        if item(ui, "Collapse Track to One Clip", has_clips) {
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

    /// Per-track controls beside each lane, as in Live: name, activator, Solo, volume, colour and pan.
    fn track_headers(&mut self, ui: &mut egui::Ui, header: Rect, n_tracks: usize, lane: &dyn Fn(usize) -> Rect) {
        // The whole column, including the corners beside the ruler and scroll bar.
        let column = Rect::from_min_max(Pos2::new(header.left(), header.top() - 22.0 - LOOP_H - LOC_H), Pos2::new(header.right(), header.bottom() + 15.0));
        ui.painter().rect_filled(column, 0.0, panel());
        ui.painter().line_segment([header.right_top(), header.right_bottom()], Stroke::new(1.0, rule()));
        let mut activate = None;
        let mut changed = false;
        for ti in 0..n_tracks {
            let l = lane(ti);
            let r = Rect::from_min_max(Pos2::new(header.left(), l.top()), Pos2::new(header.right() - 1.0, l.bottom()));
            let is_active = ti == self.active;
            if is_active {
                ui.painter().rect_filled(r, 0.0, gray(255));
                ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ink());
            }
            if ti > 0 {
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, gray(190)));
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
                        // Live's track activator (its number, lit while the track plays) and Solo.
                        let (act, act_r) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
                        let on = !t.mute;
                        ui.painter().rect(act, 3.0, if on { ink() } else { Color32::TRANSPARENT }, Stroke::new(1.0, ink()), egui::StrokeKind::Inside);
                        ui.painter().text(act.center(), Align2::CENTER_CENTER, (ti + 1).to_string(), FontId::proportional(11.0), if on { gray(255) } else { ink() });
                        if act_r.on_hover_text(if on { "Track on (click to mute)" } else { "Track muted (click to turn on)" }).clicked() {
                            t.mute = !t.mute;
                            changed = true;
                        }
                        if ui.add(Button::new("S").selected(t.solo).min_size(Vec2::new(22.0, 20.0))).on_hover_text("Solo").clicked() {
                            t.solo = !t.solo;
                            changed = true;
                        }
                        ui.spacing_mut().slider_width = (inner.width() - 60.0).max(30.0);
                        let vol = ui.add(egui::Slider::new(&mut t.gain, 0.0..=2.0).show_value(false)).on_hover_text(format!("Volume {:.0}%", t.gain * 100.0));
                        if vol.drag_stopped() || (vol.changed() && !vol.dragged()) {
                            changed = true;
                        }
                    });
                }
                if inner.height() > 70.0 {
                    ui.horizontal(|ui| {
                        let (sw, swr) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
                        ui.painter().rect_filled(sw.shrink(3.0), 3.0, live_color(t.color));
                        let swr = swr.on_hover_text("Track colour");
                        if let Some(i) = egui::Popup::menu(&swr).show(|ui| color_picker(ui, t.color)).and_then(|r| r.inner) {
                            t.color = i;
                            changed = true;
                        }
                        ui.label(egui::RichText::new("Pan").size(11.0).color(muted()));
                        ui.spacing_mut().slider_width = (inner.width() - 64.0).max(30.0);
                        let tip = match t.pan {
                            p if p.abs() < 0.01 => "Pan centre".to_string(),
                            p if p < 0.0 => format!("Pan {:.0}% left", -p * 100.0),
                            p => format!("Pan {:.0}% right", p * 100.0),
                        };
                        let pan = ui.add(egui::Slider::new(&mut t.pan, -1.0..=1.0).show_value(false)).on_hover_text(tip);
                        if pan.double_clicked() {
                            t.pan = 0.0;
                            changed = true;
                        }
                        if pan.drag_stopped() || (pan.changed() && !pan.dragged()) {
                            changed = true;
                        }
                    });
                }
            });
        }
        if let Some(ti) = activate {
            // Clicking a track's header shows its effects below, as in Live.
            self.show_track_devices(ti);
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
        if !self.arr_view_len.is_finite() || !self.arr_view_start.is_finite() {
            (self.arr_view_start, self.arr_view_len) = (0.0, n);
        }
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

    /// A beat in frames at the tempo the clips are laid out for. While a new tempo is still being
    /// dragged in, the bars stay where the clips are; they all move together once it's set.
    pub(super) fn beat_frames(&self) -> f64 {
        self.rate() as f64 * 60.0 / self.laid_out_tempo.max(1.0)
    }

    pub(super) fn bar_frames(&self) -> f64 {
        self.beat_frames() * self.beats_per_bar as f64
    }

    /// A span widened out to whole bars (at least one): loops never stop part-way through a bar.
    pub(super) fn whole_bars(&self, (s, e): Sel) -> Sel {
        let bar = self.bar_frames();
        let bs = (s as f64 / bar + 1e-6).floor();
        let be = (e as f64 / bar - 1e-6).ceil().max(bs + 1.0);
        ((bs * bar) as usize, (be * bar) as usize)
    }

    /// The grid spacing in frames at the current zoom (`spp` = frames per pixel), if the grid is on.
    pub(super) fn grid_frames(&self, spp: f64) -> Option<f64> {
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
    pub(super) fn snap_to_grid(&self, f: f64, spp: f64) -> f64 {
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
        ui.painter().rect(r, 4.0, ink().gamma_multiply(0.12), Stroke::new(1.5, ink()), egui::StrokeKind::Inside);
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
            // With the detail view under the song, the bar is the song's.
            match if self.split { View::Arrangement } else { self.view } {
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
                            ui.label(egui::RichText::new("▸").color(muted()));
                        }
                        if i == depth {
                            ui.label(egui::RichText::new(n).strong().color(ink()));
                        } else if ui.link(egui::RichText::new(n).color(muted())).clicked() {
                            go_up = Some(depth - i);
                        }
                    }
                    ui.add_space(16.0);
                    ui.separator();
                    // Tempo and beats per bar are undoable: one step per drag or typed value.
                    let (mut tempo, mut bpb) = (self.tempo, self.beats_per_bar);
                    // Drag up or down: 1 BPM per step, 0.20 with Shift. Click to type a value.
                    // A fixed width, so the box doesn't change size as the digits change.
                    let t = ui.add_sized([104.0, ui.spacing().interact_size.y], egui::DragValue::new(&mut tempo).range(20.0..=400.0).speed(0.0).fixed_decimals(2).suffix(" BPM"))
                        .on_hover_text("Song tempo — drag up or down (⇧ for fine steps), or click to type");
                    if t.hovered() || t.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if t.drag_started() {
                        self.tempo_drag = Some((self.tempo, 0.0));
                    }
                    if let (true, Some((start, acc))) = (t.dragged(), self.tempo_drag.as_mut()) {
                        *acc -= t.drag_delta().y;
                        let fine = ui.input(|i| i.modifiers.shift);
                        let steps = (*acc / 4.0).round() as f64;
                        tempo = (*start + steps * if fine { 0.2 } else { 1.0 }).clamp(20.0, 400.0);
                        // Each step taken becomes the new base, so pressing or letting go of
                        // Shift mid-drag never makes the value jump.
                        if steps != 0.0 {
                            (*start, *acc) = (tempo, *acc - steps as f32 * 4.0);
                        }
                        tempo = (tempo * 100.0).round() / 100.0;
                    }
                    if t.drag_stopped() {
                        self.tempo_drag = None;
                    }
                    let b = ui.add(egui::DragValue::new(&mut bpb).range(1..=16).suffix(" / 4")).on_hover_text("Beats per bar");
                    // Number boxes drag up and down, as in Live.
                    if b.hovered() || b.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if tempo != self.tempo || bpb != self.beats_per_bar {
                        if !self.song_edit {
                            self.push_undo(if tempo != self.tempo { "Change Tempo" } else { "Change Time Signature" });
                            self.song_edit = true;
                        }
                        (self.tempo, self.beats_per_bar) = (tempo, bpb);
                        self.dirty = true;
                    }
                    if self.song_edit && ![&t, &b].iter().any(|r| r.dragged() || r.has_focus()) {
                        self.song_edit = false;
                    }
                    ui.label(egui::RichText::new("Grid").color(muted()));
                    egui::ComboBox::from_id_salt("grid").width(80.0).selected_text(GRID[self.grid].0).show_ui(ui, |ui| {
                        for (i, (label, _)) in GRID.iter().enumerate() {
                            ui.selectable_value(&mut self.grid, i, *label);
                        }
                    });
                    if ui.add(Button::new("Metronome").selected(self.metronome)).on_hover_text("Click on every beat while playing").clicked() {
                        self.metronome = !self.metronome;
                        self.refresh_playback();
                    }
                    if self.show_session {
                        // Launch quantization, as Live's.
                        let qs: [(&str, f64); 7] = [("None", 0.0), ("1/4", 0.25), ("1/2", 0.5), ("1 Bar", 1.0), ("2 Bars", 2.0), ("4 Bars", 4.0), ("8 Bars", 8.0)];
                        let cur = qs.iter().find(|q| (q.1 - self.launch_q).abs() < 1e-9).map_or("1 Bar", |q| q.0);
                        egui::ComboBox::from_id_salt("launchq").width(76.0).selected_text(cur).show_ui(ui, |ui| {
                            for (label, v) in qs {
                                ui.selectable_value(&mut self.launch_q, v, label);
                            }
                        }).response.on_hover_text("When launched clips start: the next bar, beat or none");
                    }
                    if self.show_session && ui.add(Button::new("● Record to Song").selected(self.session_rec)).on_hover_text("Write the clips you launch into the song view (when playback stops)").clicked() {
                        self.session_rec = !self.session_rec;
                    }
                    if self.session_owns_any() && ui.button("Back to Arrangement").on_hover_text("Tracks play their song-view clips again").clicked() {
                        self.back_to_song_view();
                    }
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
                        ui.label(egui::RichText::new(self.variation.to_string()).color(ink()));
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
                    let back = if self.show_session { "◀ Session" } else { "◀ Arrangement" };
                    if ui.button(back).on_hover_text("Back (⌘Return)").clicked() {
                        self.back_to_arrangement();
                        return;
                    }
                    let (tname, cname) = self
                        .tracks
                        .get(track)
                        .map(|t| (t.name.clone(), String::new()))
                        .unwrap_or_default();
                    ui.label(egui::RichText::new("Editing").color(muted()));
                    if let Some(c) = self.clip_mut((track, clip)) {
                        ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).on_hover_text("Clip name");
                    } else {
                        ui.label(cname);
                    }
                    ui.label(egui::RichText::new(format!("on {tname}")).color(muted()));
                }
            }
        });
    }

    /// The arrangement: track headers on the left, clips on track lanes along a timeline.
    pub(super) fn arrangement(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let header_w = 150.0;
        let ruler_h = 22.0 + LOOP_H + LOC_H;
        let ruler = Rect::from_min_max(Pos2::new(full.left() + header_w, full.top()), Pos2::new(full.right(), full.top() + ruler_h));
        let scroll = Rect::from_min_max(Pos2::new(full.left() + header_w, full.bottom() - 15.0), full.max);
        // The master channel sits under the tracks, above the scroll bar.
        let master = Rect::from_min_max(Pos2::new(full.left(), scroll.top() - MASTER_H), Pos2::new(full.right(), scroll.top()));
        let lanes = Rect::from_min_max(Pos2::new(full.left() + header_w, ruler.bottom()), Pos2::new(full.right(), master.top()));
        let n_tracks = self.tracks.len().max(1);
        // Layers are up to 120 points tall, or share all the height (H, as Live's Optimize Height).
        let lane_h = if self.fit_height { lanes.height() / n_tracks as f32 } else { (lanes.height() / n_tracks as f32).min(120.0) };
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
                // The hand only over the name strip, which is what moves a clip; the body selects
                // time, so it keeps the plain pointer (⌥ shows the copy cursor anywhere).
                let alt = ui.input(|i| i.modifiers.alt);
                let on_strip = match (resp.hover_pos(), hover_hit) {
                    (Some(q), Some((t, _, _))) => q.y <= lane(t).top() + 19.0,
                    _ => false,
                };
                if alt {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
                } else if on_strip {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
            _ => {}
        }

        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        let split_tool = self.tool == Tool::Split;
        // The ruler has two parts, as in Live. The strip along the top is for the loop: drag
        // there to draw one, drag the brace to move it, or an end of it to resize. Loops are
        // always whole bars. Below it, the bar numbers zoom: drag down to zoom in, up to zoom
        // out, and sideways to scroll.
        let ruler_drag = press.is_some_and(|q| q.y < ruler.bottom());
        let in_loop_strip = |q: Pos2| q.y < ruler.top() + LOOP_H;
        let brace_at = |q: Pos2| -> Option<LoopGrab> {
            let (ls, le) = self.arr_loop?;
            let (x0, x1) = (a.f_to_x(ls as f64, vs, vl), a.f_to_x(le as f64, vs, vl));
            if !in_loop_strip(q) || q.x < x0 - 4.0 || q.x > x1 + 4.0 {
                return None;
            }
            let edge = 6.0f32.min((x1 - x0) / 3.0);
            Some(if q.x < x0 + edge { LoopGrab::Start } else if q.x > x1 - edge { LoopGrab::End } else { LoopGrab::Move })
        };
        if self.clip_drag.is_none() {
            let hover = ui.input(|i| i.pointer.hover_pos()).filter(|q| ruler.contains(*q));
            if let Some(g) = hover.and_then(|q| brace_at(q)).or(self.arr_loop_grab.map(|g| g.2)) {
                ui.ctx().set_cursor_icon(if g == LoopGrab::Move { egui::CursorIcon::Grab } else { egui::CursorIcon::ResizeHorizontal });
            } else if self.ruler_zoom.is_none() && hover.is_some_and(|q| q.y < ruler.top() + LOOP_H + LOC_H && self.locator_at(q.x, &|f| a.f_to_x(f, vs, vl)).is_some()) {
                // A locator: click to jump there.
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if self.ruler_zoom.is_some() || hover.is_some_and(|q| !in_loop_strip(q)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
            }
        }
        let bar = self.bar_frames();
        let to_bar = |f: f64| ((f / bar).round().max(0.0) * bar) as usize;
        let bar_n = bar.round() as usize;
        let in_loc_row = |q: Pos2| q.y >= ruler.top() + LOOP_H && q.y < ruler.top() + LOOP_H + LOC_H;
        if resp.drag_started() && ruler_drag {
            let q = press.unwrap();
            let f = x_to_f(q.x, vs, vl);
            let loc = in_loc_row(q).then(|| self.locator_at(q.x, &|f| a.f_to_x(f, vs, vl))).flatten();
            if let Some(i) = loc {
                // Drag a locator along the ruler.
                self.push_undo("Move Locator");
                self.loc_drag = Some(i);
                self.selected_locator = Some(i);
            } else if !in_loop_strip(q) {
                self.ruler_zoom = Some((f, q.y, vl));
            } else if let (Some(g), Some(range)) = (brace_at(q), self.arr_loop) {
                self.arr_loop_grab = Some((f, range, g));
            } else {
                self.arr_loop_drag = Some(to_bar(f));
            }
        }
        if let (Some(i), true, Some(q)) = (self.loc_drag, resp.dragged(), resp.interact_pointer_pos()) {
            let f = x_to_f(q.x, vs, vl);
            let f = if ui.input(|i| i.modifiers.command) { f } else { self.snap_to_grid(f, spp) };
            if let Some(l) = self.locators.get_mut(i) {
                l.0 = f.max(0.0) as usize;
                self.dirty = true;
            }
        }
        if resp.drag_stopped() {
            if let Some(i) = self.loc_drag.take() {
                // Back in time order, keeping it selected.
                let moved = self.locators.get(i).cloned();
                self.locators.sort_by_key(|l| l.0);
                self.selected_locator = moved.and_then(|m| self.locators.iter().position(|l| *l == m));
            }
        }
        if let (Some((anchor, y0, len0)), true, Some(q)) = (self.ruler_zoom, resp.dragged(), resp.interact_pointer_pos()) {
            // The bar grabbed stays under the pointer while the zoom changes around it.
            self.arr_view_len = len0 * ((y0 - q.y) as f64 / 80.0).exp();
            self.arr_clamp();
            self.arr_view_start = anchor - ((q.x - lanes.left()) / lanes.width()) as f64 * self.arr_view_len;
            self.arr_clamp();
        }
        if let (Some((at, (ls, le), g)), true, Some(q)) = (self.arr_loop_grab, resp.dragged(), resp.interact_pointer_pos()) {
            let d = x_to_f(q.x, vs, vl) - at;
            let moved = |f: usize| to_bar(f as f64 + d);
            let range = match g {
                LoopGrab::Move => {
                    let s = moved(ls);
                    (s, s + (le - ls))
                }
                LoopGrab::Start => (moved(ls).min(le.saturating_sub(bar_n)), le),
                LoopGrab::End => (ls, moved(le).max(ls + bar_n)),
            };
            self.arr_loop = Some(range);
        }
        if let (Some(a), true, Some(q)) = (self.arr_loop_drag, resp.dragged(), resp.interact_pointer_pos()) {
            let f = x_to_f(q.x, vs, vl);
            let b = to_bar(f);
            // At least one bar, reaching the way the pointer went.
            let b = if b != a { b } else if f < a as f64 { a.saturating_sub(bar_n) } else { a + bar_n };
            if b != a {
                self.arr_loop = Some((a.min(b), a.max(b)));
                self.looping = true;
            }
        }
        if resp.drag_stopped() {
            self.ruler_zoom = None;
            if self.arr_loop_grab.take().is_some() || self.arr_loop_drag.take().is_some() {
                self.dirty = true;
                if self.player.is_playing() {
                    self.toggle_play();
                    self.toggle_play();
                }
            }
        }
        // Fade handles at a clip's top corners: drag to set its fades.
        let fade_at = |tracks: &[Track], q: Pos2| -> Option<(usize, usize, bool)> {
            let (t, c, _) = hit(tracks, q, vs, vl)?;
            let clip = &tracks[t].clips[c];
            let l = lane(t);
            let r = Rect::from_min_max(Pos2::new(a.f_to_x(clip.start as f64, vs, vl), l.top() + 3.0), Pos2::new(a.f_to_x(clip.end() as f64, vs, vl), l.bottom() - 3.0));
            let y = (r.top() + 18.0).min(r.bottom()) + 5.0;
            let (fi, fo) = fade_handle_xs(r, clip);
            if (q.y - y).abs() > 6.0 || r.width() <= 24.0 {
                return None;
            }
            if (q.x - fi).abs() <= 6.0 {
                Some((t, c, true))
            } else if (q.x - fo).abs() <= 6.0 {
                Some((t, c, false))
            } else {
                None
            }
        };
        if self.fade_drag.is_some() || resp.hover_pos().and_then(|q| fade_at(&self.tracks, q)).is_some() {
            // The plain pointer over a fade handle, as in Live (it wins over the edge's arrows).
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }
        if resp.drag_started() && !split_tool && !ruler_drag {
            if let Some(f) = press.and_then(|q| fade_at(&self.tracks, q)) {
                self.push_undo("Fade");
                self.fade_drag = Some(f);
            }
        }
        if let (Some((t, c, is_in)), true, Some(q)) = (self.fade_drag, resp.dragged(), resp.interact_pointer_pos()) {
            if let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
                let f = x_to_f(q.x, vs, vl).round() as i64;
                if is_in {
                    clip.fade_in = (f - clip.start as i64).clamp(0, (clip.len - clip.fade_out) as i64) as usize;
                } else {
                    clip.fade_out = (clip.end() as i64 - f).clamp(0, (clip.len - clip.fade_in) as i64) as usize;
                }
                self.dirty = true;
                self.refresh_playback();
            }
        }
        if resp.drag_stopped() {
            self.fade_drag = None;
        }
        if resp.drag_started() && !split_tool && !ruler_drag && self.fade_drag.is_none() {
            if let Some(q) = press {
                // As in Live, a clip moves by its name strip; dragging across its waveform selects
                // time instead (⌥ still drags a copy from anywhere on it).
                let on_body = |t: usize| q.y > lane(t).top() + 19.0 && !ui.input(|i| i.modifiers.alt);
                let slide = ui.input(|i| i.modifiers.shift && i.modifiers.alt);
                if let (true, Some((t, c, DragMode::Move))) = (slide, hit(&self.tracks, q, vs, vl)) {
                    // ⇧⌥-drag (as Live): slide the audio inside the clip, which stays put.
                    self.push_undo("Slide Clip Contents");
                    self.slide_drag = Some((t, c, self.tracks[t].clips[c].trim, x_to_f(q.x, vs, vl)));
                    self.sel_clip = Some((t, c));
                    self.sel_extra.clear();
                } else if let Some((t, _, DragMode::Move)) = hit(&self.tracks, q, vs, vl).filter(|h| on_body(h.0)) {
                    self.set_active(t);
                    self.start_time_sel(q, &a, vs, vl, spp, ui.input(|i| i.modifiers.command));
                } else if let Some((t, mut c, mode)) = hit(&self.tracks, q, vs, vl) {
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
                    // Empty space (or a clip's waveform, above): drag out a time selection.
                    self.start_time_sel(q, &a, vs, vl, spp, ui.input(|i| i.modifiers.command));
                }
            }
        }
        if let (Some((f0, t0)), true, Some(q)) = (self.time_drag, resp.dragged(), resp.interact_pointer_pos()) {
            let f = x_to_f(q.x, vs, vl);
            let f = if ui.input(|i| i.modifiers.command) { f } else { self.snap_to_grid(f, spp) } as usize;
            let t = (((q.y - lanes.top()) / lane_h).max(0.0) as usize).min(self.tracks.len().saturating_sub(1));
            self.time_sel = (f != f0).then_some(((f0.min(f), f0.max(f)), (t0.min(t), t0.max(t))));
        }
        if resp.drag_stopped() {
            self.time_drag = None;
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
        if let (Some((t, c, trim0, at)), true, Some(q)) = (self.slide_drag, resp.dragged(), resp.interact_pointer_pos()) {
            let d = x_to_f(q.x, vs, vl) - at;
            let d = if ui.input(|i| i.modifiers.command) { d } else { self.snap_to_grid(d.abs(), spp).copysign(d) };
            if let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
                let (_, _, _, frames) = clip.picture();
                let trim = trim0 as f64 - d;
                clip.trim = if clip.looping {
                    // A looping clip wraps round its loop.
                    let (ls, le) = clip.loop_points.map(|(s, e)| (s.min(frames), e.min(frames))).filter(|(s, e)| e > s).unwrap_or((0, frames));
                    let n = (le - ls).max(1) as f64;
                    if trim < ls as f64 { (ls as f64 + (trim - ls as f64).rem_euclid(n)) as usize } else { trim as usize }
                } else {
                    trim.clamp(0.0, frames.saturating_sub(clip.len) as f64) as usize
                };
                self.dirty = true;
                self.refresh_playback();
            }
        }
        if resp.drag_stopped() {
            self.slide_drag = None;
        }
        if let (true, Some(d)) = (resp.drag_stopped(), self.clip_drag.as_ref()) {
            // Dropped over other clips on its layer: it cuts them away there. An edge dragged
            // over a neighbour instead overlaps it, and the two crossfade across the overlap.
            let (p, moved) = ((d.track, d.clip), d.mode == DragMode::Move);
            self.clip_drag = None;
            if moved && self.clip(p).is_some() {
                self.place_over(p);
            }
            self.dirty = true;
            self.refresh_playback();
        }
        self.arr_right_click(ui, &resp, &a);
        if split_tool {
            // Handled above.
        } else if resp.double_clicked() {
            // Double-click the bar numbers: zoom to the selection, or out to the whole song.
            if resp.interact_pointer_pos().is_some_and(|q| q.y < lanes.top() && q.y > ruler.top() + LOOP_H + LOC_H) {
                let ctx = ui.ctx().clone();
                let a = if self.time_sel.is_some() || self.sel_clip.is_some() { Action::ZoomSel } else { Action::ZoomFit };
                self.run(a, &ctx);
                return;
            }
            if let Some((t, c, _)) = resp.interact_pointer_pos().and_then(|q| hit(&self.tracks, q, vs, vl)) {
                // Double-click: the clip opens in full in the audio editor.
                self.edit_clip((t, c));
                self.full_edit = true;
                return;
            }
        } else if resp.clicked() {
            if resp.interact_pointer_pos().is_some_and(|q| q.y > ruler.bottom()) {
                self.time_sel = None;
            }
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
                    None if q.y < lanes.top() && self.locator_at(q.x, &|f| a.f_to_x(f, vs, vl)).is_some() => {
                        // A click on a locator jumps there and selects it (Delete, ⌘R).
                        let i = self.locator_at(q.x, &|f| a.f_to_x(f, vs, vl)).unwrap();
                        self.go_to_frame(self.locators[i].0);
                        self.selected_locator = Some(i);
                    }
                    None => {
                        // Clicking empty space moves the playhead (and picks that track).
                        self.selected_locator = None;
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
            if self.follow && self.clip_drag.is_none() && (p > self.arr_view_start + self.arr_view_len || p < self.arr_view_start) {
                self.arr_view_start = p - self.arr_view_len * 0.05;
                self.arr_clamp();
            }
        }

        let thumb_x = if max_start > 0.0 { (self.arr_view_start / max_start) as f32 * track_w } else { 0.0 };
        let thumb = Rect::from_min_size(Pos2::new(scroll.left() + thumb_x, scroll.top()), Vec2::new(thumb_w, scroll.height()));
        self.paint_arrangement(ui, &a, thumb, resp.hover_pos(), playing, play_pos);
        self.pool_drop(ui, &resp);
        // The time selection: a light wash over the chosen stretch of the chosen layers, with
        // marks in the ruler at its ends (as in Live).
        if let Some(((s0, e0), (t0, t1))) = self.time_sel {
            let (x0, x1) = (a.f_to_x(s0 as f64, vs, vl).max(lanes.left()), a.f_to_x(e0 as f64, vs, vl).min(lanes.right()));
            if x1 > x0 {
                let p = ui.painter_at(Rect::from_min_max(ruler.min, lanes.max));
                for t in t0..=t1.min(self.tracks.len().saturating_sub(1)) {
                    let l = lane(t);
                    p.rect_filled(Rect::from_min_max(Pos2::new(x0, l.top()), Pos2::new(x1, l.bottom())), 0.0, Color32::from_rgba_unmultiplied(90, 200, 225, 70));
                }
                for (x, dir) in [(a.f_to_x(s0 as f64, vs, vl), 1.0), (a.f_to_x(e0 as f64, vs, vl), -1.0)] {
                    if x >= lanes.left() && x <= lanes.right() {
                        p.add(egui::Shape::convex_polygon(
                            vec![Pos2::new(x, ruler.bottom() - 7.0), Pos2::new(x + 6.0 * dir, ruler.bottom() - 7.0), Pos2::new(x, ruler.bottom())],
                            Color32::from_rgb(40, 150, 180),
                            Stroke::NONE,
                        ));
                    }
                }
            }
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
        p.rect_filled(r, 0.0, panel());
        // The corner beside the scroll bar, below the header column.
        p.rect_filled(Rect::from_min_size(Pos2::new(r.left(), r.bottom()), Vec2::new(header_w, 15.0)), 0.0, panel());
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, gray(190)));
        p.line_segment([Pos2::new(r.left() + header_w - 1.0, r.top()), Pos2::new(r.left() + header_w - 1.0, r.bottom())], Stroke::new(1.0, rule()));
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
            ui.label(egui::RichText::new("Master").strong().color(ink()));
            let (b, resp) = ui.allocate_exact_size(Vec2::new(26.0, 22.0), Sense::click());
            let on = self.crossfeed;
            let hover = resp.hovered();
            ui.painter().rect(b, 4.0, if on { ink() } else if hover { gray(232) } else { gray(255) }, Stroke::new(1.0, if on { ink() } else { rule() }), egui::StrokeKind::Inside);
            paint_icon(ui.painter(), b, Action::Crossfeed, if on { gray(255) } else { ink() }, false);
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
                ui.painter().line_segment([Pos2::new(x, l.top() + 2.0), Pos2::new(x, l.bottom() - 2.0)], Stroke::new(1.5, ink()));
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
            self.loc_menu = None;
            // In the ruler: the locator menu.
            if let Some(q) = resp.interact_pointer_pos().filter(|q| q.y < lanes.top()) {
                let f = a.x_to_f(q.x, vs, vl);
                let f = self.snap_to_grid(f, vl / lanes.width() as f64) as usize;
                self.loc_menu = Some((self.locator_at(q.x, &|f| a.f_to_x(f, vs, vl)), f));
            } else if let Some(q) = resp.interact_pointer_pos() {
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
            if let Some(m) = self.loc_menu {
                return self.locator_menu(ui, m);
            }
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
        painter.rect_filled(lanes, 0.0, gray(255));
        painter.rect_filled(ruler, 0.0, panel());
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, rule()));
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
            for _ in 0..MAX_TICKS {
                let f = k as f64 * g;
                if !(f <= vs + vl) {
                    break;
                }
                let x = f_to_x(f, vs, vl);
                if x >= lanes.left() {
                    let on_bar = (f / bar).fract().abs() < 1e-6 || (f / bar).fract() > 1.0 - 1e-6;
                    let on_beat = (f / beat).fract().abs() < 1e-6 || (f / beat).fract() > 1.0 - 1e-6;
                    let (h, line) = if on_bar { (9.0, 228) } else if on_beat { (5.0, 240) } else { (3.0, 246) };
                    painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, muted()));
                    painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, gray(line)));
                    let bar_no = (f / bar).round() as u64;
                    if on_bar && bar_no % label_every == 0 {
                        painter.text(Pos2::new(x + 3.0, ruler.top() + LOOP_H + LOC_H + 1.0), Align2::LEFT_TOP, (bar_no + 1).to_string(), FontId::proportional(11.0), muted());
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
        for _ in 0..MAX_TICKS {
            let t = k as f64 * minor;
            if t > (vs + vl) / rate || !secs_per_px.is_finite() {
                break;
            }
            let x = f_to_x(t * rate, vs, vl);
            if x >= lanes.left() {
                let major = k % 5 == 0;
                painter.line_segment([Pos2::new(x, ruler.bottom() - if major { 7.0 } else { 3.0 }), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, muted()));
                if major {
                    painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, gray(242)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + LOOP_H + LOC_H + 1.0), Align2::LEFT_TOP, ruler_label(self.ruler, rate as u32, t, step), FontId::proportional(11.0), muted());
                }
            }
            k += 1;
        }
        // Arrangement loop: a brace along the top of the ruler (black when Loop is on).
        if let Some((ls, le)) = self.arr_loop {
            let (x0, x1) = (f_to_x(ls as f64, vs, vl).max(lanes.left()), f_to_x(le as f64, vs, vl).min(lanes.right()));
            if x1 > x0 {
                let col = if self.looping { ink() } else { gray(175) };
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, ruler.top() + 3.0), Pos2::new(x1, ruler.top() + LOOP_H - 3.0)), 2.0, col);
                if self.looping {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, lanes.top()), Pos2::new(x1, lanes.bottom())), 0.0, Color32::from_black_alpha(8));
                }
            }
        }
        // Lanes and clips.
        for (ti, t) in self.tracks.iter().enumerate() {
            let l = lane(ti);
            if ti == self.active {
                painter.rect_filled(l, 0.0, gray(250));
            }
            painter.line_segment([l.left_bottom(), l.right_bottom()], Stroke::new(1.0, gray(225)));
            let muted = t.mute;
            let fades = crossfades(&t.clips);
            for (ci, c) in t.clips.iter().enumerate() {
                let (x0, x1) = (f_to_x(c.start as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
                if x1 < lanes.left() || x0 > lanes.right() {
                    continue;
                }
                let r = Rect::from_min_max(Pos2::new(x0, l.top() + 3.0), Pos2::new(x1.max(x0 + 2.0), l.bottom() - 3.0));
                let selected = self.sel_clip == Some((ti, ci)) || self.sel_extra.contains(&(ti, ci));
                let hovered = self.arr_hover.is_some_and(|h| (h.0, h.1) == (ti, ci));
                draw_clip(&painter, r, c, live_color(c.color.unwrap_or(t.color)), selected, muted || !c.active, selected || hovered || self.fade_drag.is_some_and(|d| (d.0, d.1) == (ti, ci)), lanes);
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
                p.rect_filled(r, 0.0, wash(140));
                let stroke = Stroke::new(1.5, ink());
                p.line_segment([r.left_top(), r.right_bottom()], stroke);
                p.line_segment([r.left_bottom(), r.right_top()], stroke);
                p.rect_stroke(r, 0.0, Stroke::new(1.0, ink()), egui::StrokeKind::Inside);
            }
        }
        if self.tracks.is_empty() {
            painter.text(lanes.center(), Align2::CENTER_CENTER, "Drop a sound here, or choose File ▸ Open", FontId::proportional(16.0), gray(120));
        }
        // Phrase lines: a fine darker line every few bars (Preferences), counting from bar 1, drawn over the clips.
        if self.phrase_bars > 0 {
            let every = self.bar_frames() * self.phrase_bars as f64;
            if every / spp > 6.0 {
                let mut k = (vs / every).ceil() as i64;
                for _ in 0..MAX_TICKS {
                    if !(k as f64 * every <= vs + vl) {
                        break;
                    }
                    let x = f_to_x(k as f64 * every, vs, vl);
                    if x >= lanes.left() {
                        painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, Color32::from_black_alpha(70)));
                    }
                    k += 1;
                }
            }
        }
        // Locators: a flag in the ruler with its name, and a faint line down through the song.
        for (li, (f, name)) in self.locators.iter().enumerate() {
            let x = f_to_x(*f as f64, vs, vl);
            let flag = if self.selected_locator == Some(li) { ink() } else { Color32::from_rgb(40, 150, 180) };
            if x < lanes.left() - 1.0 || x > lanes.right() {
                continue;
            }
            let top = ruler.top() + LOOP_H + 1.0;
            painter.add(egui::Shape::convex_polygon(vec![Pos2::new(x, top), Pos2::new(x + 8.0, top + 5.0), Pos2::new(x, top + 10.0)], flag, Stroke::NONE));
            let label = painter.layout_no_wrap(name.clone(), FontId::proportional(10.5), ink());
            let bg = Rect::from_min_size(Pos2::new(x + 9.0, top), label.size() + Vec2::new(4.0, 0.0));
            painter.rect_filled(bg, 2.0, panel());
            painter.galley(Pos2::new(x + 11.0, top), label, ink());
            for y in (lanes.top() as i32..lanes.bottom() as i32).step_by(6) {
                painter.line_segment([Pos2::new(x, y as f32), Pos2::new(x, y as f32 + 3.0)], Stroke::new(1.0, Color32::from_rgb(40, 150, 180).gamma_multiply(0.6)));
            }
        }
        // Playhead (or the playback position).
        let ph = if playing { play_pos } else { self.arr_cursor };
        let x = f_to_x(ph as f64, vs, vl);
        if x >= lanes.left() && x <= lanes.right() {
            painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.5, ink()));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x - 5.0, ruler.bottom() - 7.0), Pos2::new(x + 5.0, ruler.bottom() - 7.0), Pos2::new(x, ruler.bottom())],
                ink(),
                Stroke::NONE,
            ));
        }
        // Scroll bar.
        painter.rect_filled(scroll, 0.0, panel());
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, rule()));
        painter.rect_filled(thumb.shrink2(Vec2::new(1.0, 4.0)), 4.0, gray(185));
        if let Some(q) = hover {
            let f = x_to_f(q.x, vs, vl) as usize;
            let text = if self.grid_frames(spp).is_some() { format!("{}  {}", self.fmt_bars(f), self.fmt_pos(f)) } else { self.fmt_pos(f) };
            painter.text(Pos2::new(lanes.right() - 6.0, ruler.top() + LOOP_H + LOC_H + 1.0), Align2::RIGHT_TOP, text, FontId::monospace(11.0), muted());
        }
    }
}

/// Height of the loop strip along the top of the arrangement ruler.
pub(super) const LOOP_H: f32 = 13.0;

/// Height of the locator row, between the loop strip and the bar numbers.
pub(super) const LOC_H: f32 = 12.0;

/// Most ticks or lines a ruler draws in one frame. Normally a few hundred at most; the cap means a
/// bad view value (not a number, say) can never keep a drawing loop going forever.
pub(super) const MAX_TICKS: usize = 5000;

/// Which part of the arrangement's loop brace is being dragged.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum LoopGrab {
    Move,
    Start,
    End,
}
