//! Time selection in the song view, as in Live: a stretch of time across a run of layers that
//! Delete, Split, Mangle and the clip processes work on.

use super::*;
use super::arrange::ArrArea;

impl App {
    /// Begin dragging out a time selection from window point `q` (snapped to the grid unless ⌘).
    pub(super) fn start_time_sel(&mut self, q: Pos2, a: &ArrArea, vs: f64, vl: f64, spp: f64, free: bool) {
        let f = a.x_to_f(q.x, vs, vl);
        let f = if free { f } else { self.snap_to_grid(f, spp) } as usize;
        let t = (((q.y - a.lanes.top()) / a.lane_h).max(0.0) as usize).min(self.tracks.len().saturating_sub(1));
        self.time_drag = Some((f, t));
        self.time_sel = None;
        self.sel_clip = None;
        self.sel_extra.clear();
    }

    /// Cut the clips on the selected layers at both ends of the time selection.
    pub(super) fn cut_at_time_sel(&mut self) -> Option<(Sel, (usize, usize))> {
        let ((s, e), (t0, t1)) = self.time_sel?;
        for t in t0..=t1.min(self.tracks.len().saturating_sub(1)) {
            for at in [s, e] {
                if let Some(c) = self.tracks[t].clips.iter().position(|c| at > c.start && at < c.end()) {
                    self.split_raw((t, c), at);
                }
            }
        }
        Some(((s, e), (t0, t1)))
    }

    /// Run clip process `a` on just the stretch inside the time selection: the clips there are
    /// cut at its ends and selected, `a` runs on them, and the selection stays (one undo step).
    pub(super) fn run_on_time_sel(&mut self, a: Action, ctx: &egui::Context) {
        let before = self.undo.len();
        self.push_undo("Process Selection");
        let Some(((s, e), (t0, t1))) = self.cut_at_time_sel() else { return };
        let inside: Vec<(usize, usize)> = (t0..=t1.min(self.tracks.len().saturating_sub(1)))
            .flat_map(|t| self.tracks[t].clips.iter().enumerate().filter(|(_, c)| c.start >= s && c.end() <= e).map(move |(i, _)| (t, i)).collect::<Vec<_>>())
            .collect();
        self.sel_clip = inside.first().copied();
        self.sel_extra = inside.iter().skip(1).copied().collect();
        let keep = self.time_sel.take();
        self.run(a, ctx);
        self.time_sel = keep;
        // Fold the process's own undo step into ours, so ⌘Z undoes the cuts too.
        if self.undo.len() == before + 2 {
            let top = self.undo.pop().unwrap();
            if let Some(first) = self.undo.last_mut() {
                first.label = top.label;
            }
        }
        self.refresh_playback();
    }

    /// ⌘E with a time selection: split the clips it covers at its ends.
    pub(super) fn split_time_sel(&mut self) {
        self.push_undo("Split");
        self.cut_at_time_sel();
        self.dirty = true;
        self.refresh_playback();
        self.status = "Split at the selection's ends — ⌘Z to undo".into();
    }

    /// ⌫ with a time selection: remove that stretch from the selected layers, leaving a gap.
    pub(super) fn delete_time_sel(&mut self) {
        self.push_undo("Delete Time");
        let Some(((s, e), (t0, t1))) = self.cut_at_time_sel() else { return };
        for t in t0..=t1.min(self.tracks.len().saturating_sub(1)) {
            self.tracks[t].clips.retain(|c| c.start < s || c.end() > e);
        }
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = "Deleted the selected time — ⌘Z to undo".into();
    }

    /// Cut layer `t`'s clips at frame `at`.
    fn cut_layer_at(&mut self, t: usize, at: usize) {
        if let Some(c) = self.tracks[t].clips.iter().position(|c| at > c.start && at < c.end()) {
            self.split_raw((t, c), at);
        }
    }

    /// Copies of what layer `t` plays in `[s, e)`, each cut down to the part inside.
    fn copy_span(&self, t: usize, (s, e): Sel) -> Vec<Clip> {
        self.tracks[t]
            .clips
            .iter()
            .filter(|c| c.start < e && c.end() > s)
            .map(|c| {
                let (a, b) = (s.max(c.start), e.min(c.end()));
                let mut piece = c.clone();
                piece.trim += a - c.start;
                (piece.start, piece.len) = (a, b - a);
                piece
            })
            .collect()
    }

    /// Clear `[s, e)` on layer `t`, leaving a gap.
    pub(super) fn clear_span(&mut self, t: usize, (s, e): Sel) {
        self.cut_layer_at(t, s);
        self.cut_layer_at(t, e);
        self.tracks[t].clips.retain(|c| c.start < s || c.end() > e);
    }

    /// Move everything on every layer from `at` on by `d` frames (later if positive), cutting
    /// clips that straddle `at`.
    fn shift_from(&mut self, at: usize, d: i64) {
        for t in 0..self.tracks.len() {
            self.cut_layer_at(t, at);
            for c in self.tracks[t].clips.iter_mut().filter(|c| c.start >= at) {
                c.start = (c.start as i64 + d).max(0) as usize;
            }
        }
        if let Some((ls, le)) = self.arr_loop.filter(|&(ls, _)| ls >= at) {
            self.arr_loop = Some(((ls as i64 + d).max(0) as usize, (le as i64 + d).max(0) as usize));
        }
    }

    /// The layers of the time selection, clamped to the layers there are.
    fn sel_layers(&self, (t0, t1): (usize, usize)) -> std::ops::RangeInclusive<usize> {
        t0..=t1.min(self.tracks.len().saturating_sub(1))
    }

    /// ⌘D with a time selection (Live's Duplicate): a copy of the selected stretch goes right
    /// after it on the same layers, over whatever was there, and the selection moves onto it.
    pub(super) fn duplicate_time_sel(&mut self) {
        let Some(((s, e), layers)) = self.time_sel else { return };
        self.push_undo("Duplicate");
        let len = e - s;
        for t in self.sel_layers(layers) {
            let pieces = self.copy_span(t, (s, e));
            self.clear_span(t, (e, e + len));
            self.tracks[t].clips.extend(pieces.into_iter().map(|mut c| {
                c.start += len;
                c
            }));
        }
        self.time_sel = Some(((e, e + len), layers));
        self.finish_time_edit("Duplicated the selection — ⌘D again for another");
    }

    /// Edit ▸ Duplicate Time (⇧⌘D): as ⌘D, but the copy is inserted, pushing everything after
    /// the selection later on every layer.
    pub(super) fn duplicate_time(&mut self) {
        let Some(((s, e), layers)) = self.time_sel else { return };
        self.push_undo("Duplicate Time");
        let len = e - s;
        let copies: Vec<(usize, Vec<Clip>)> = self.sel_layers(layers).map(|t| (t, self.copy_span(t, (s, e)))).collect();
        self.shift_from(e, len as i64);
        for (t, pieces) in copies {
            self.tracks[t].clips.extend(pieces.into_iter().map(|mut c| {
                c.start += len;
                c
            }));
        }
        self.time_sel = Some(((e, e + len), layers));
        self.finish_time_edit("Duplicated the selected time, pushing the rest later");
    }

    /// Edit ▸ Delete Time (⇧⌘⌫): take the selected stretch out of the whole song and close
    /// the gap.
    pub(super) fn delete_time(&mut self) {
        let Some(((s, e), _)) = self.time_sel else { return };
        self.push_undo("Delete Time");
        for t in 0..self.tracks.len() {
            self.clear_span(t, (s, e));
        }
        self.shift_from(e, -((e - s) as i64));
        self.time_sel = None;
        self.finish_time_edit("Deleted the time and closed the gap — ⌘Z to undo");
    }

    /// Edit ▸ Insert Time (⌘I): open up an empty stretch the length of the selection at its
    /// start, on every layer.
    pub(super) fn insert_time(&mut self) {
        let Some(((s, e), _)) = self.time_sel else { return };
        self.push_undo("Insert Time");
        self.shift_from(s, (e - s) as i64);
        self.finish_time_edit("Inserted empty time — ⌘Z to undo");
    }

    fn finish_time_edit(&mut self, status: &str) {
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = status.into();
    }

    /// Clip ▸ Quantize (⌘U): move the selected clips' starts (or those starting inside the time
    /// selection) to the nearest grid line, or beat when the grid is off.
    pub(super) fn quantize(&mut self) {
        let mut targets = self.selected_clips();
        if let Some(((s, e), layers)) = self.time_sel {
            targets.extend(self.sel_layers(layers).flat_map(|t| self.tracks[t].clips.iter().enumerate().filter(|(_, c)| c.start >= s && c.start < e).map(move |(i, _)| (t, i)).collect::<Vec<_>>()));
        }
        if targets.is_empty() {
            return;
        }
        let spp = self.arr_lanes.map_or(1.0, |(l, _)| self.arr_view_len / l.width().max(1.0) as f64);
        let g = self.grid_frames(spp).unwrap_or_else(|| self.beat_frames());
        self.push_undo("Quantize");
        for (t, c) in targets {
            if let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
                clip.start = ((clip.start as f64 / g).round() * g) as usize;
            }
        }
        self.dirty = true;
        self.refresh_playback();
        self.status = "Quantized to the grid — ⌘Z to undo".into();
    }

    /// Edit ▸ Copy Time (⇧⌘C): copy the selected stretch of every layer.
    pub(super) fn copy_time(&mut self) {
        let Some(((s, e), _)) = self.time_sel else { return };
        let layers = (0..self.tracks.len())
            .map(|t| {
                self.copy_span(t, (s, e))
                    .into_iter()
                    .map(|mut c| {
                        c.start -= s;
                        c
                    })
                    .collect()
            })
            .collect();
        self.time_clip = Some((e - s, layers));
        self.status = "Copied the selected time — ⇧⌘V pastes it at the insert marker".into();
    }

    /// Edit ▸ Cut Time (⇧⌘X): copy the selected time, then take it out and close the gap.
    pub(super) fn cut_time(&mut self) {
        self.copy_time();
        self.delete_time();
        if let Some(u) = self.undo.last_mut() {
            u.label = "Cut Time";
        }
    }

    /// Edit ▸ Paste Time (⇧⌘V): insert the copied time at the selection's start (or the insert
    /// marker), pushing everything after it later, and select it.
    pub(super) fn paste_time(&mut self) {
        let Some((len, layers)) = self.time_clip.clone() else { return };
        let at = self.time_sel.map_or(self.arr_cursor, |((s, _), _)| s);
        self.push_undo("Paste Time");
        self.shift_from(at, len as i64);
        for (t, pieces) in layers.into_iter().enumerate().take(self.tracks.len()) {
            self.tracks[t].clips.extend(pieces.into_iter().map(|mut c| {
                c.start += at;
                c
            }));
        }
        self.time_sel = Some(((at, at + len), (0, self.tracks.len().saturating_sub(1))));
        self.finish_time_edit("Pasted the time, pushing the rest later");
    }

    /// One step of the song's grid at the current zoom (a beat when the grid is off).
    fn grid_step(&self) -> usize {
        let spp = self.arr_lanes.map_or(1.0, |(l, _)| self.arr_view_len / l.width().max(1.0) as f64);
        self.grid_frames(spp).unwrap_or_else(|| self.beat_frames()).round().max(1.0) as usize
    }

    /// ← →: nudge the selected clips by the grid, or move the insert marker to the next grid
    /// line (as in Live).
    pub(super) fn arrow_nudge(&mut self, sign: i64) {
        let g = self.grid_step() as i64;
        let clips = self.selected_clips();
        if !clips.is_empty() && self.time_sel.is_none() {
            self.push_undo("Nudge");
            for (t, c) in clips {
                if let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
                    clip.start = (clip.start as i64 + sign * g).max(0) as usize;
                }
            }
            self.dirty = true;
            self.refresh_playback();
            return;
        }
        let here = self.arr_cursor as i64;
        let next = if sign > 0 { (here / g + 1) * g } else { ((here - 1).max(0) / g) * g };
        self.go_to_frame(next.max(0) as usize);
        self.time_sel = None;
    }

    /// ⇧← ⇧→: grow or shrink the time selection by the grid (from the insert marker if none).
    pub(super) fn extend_time_sel(&mut self, sign: i64) {
        let g = self.grid_step();
        let ((s, e), layers) = self.time_sel.unwrap_or(((self.arr_cursor, self.arr_cursor), (self.active, self.active)));
        let e = if sign > 0 { e + g } else { e.saturating_sub(g).max(s + g) };
        self.time_sel = Some(((s, e), layers));
        self.sel_clip = None;
        self.sel_extra.clear();
    }

    /// ↑ ↓: with Loop on and nothing selected, move the loop by its own length; otherwise move
    /// to the layer above or below (as in Live).
    pub(super) fn arrow_up_down(&mut self, sign: i64) {
        if let (true, Some((ls, le)), None, None) = (self.looping, self.arr_loop, self.sel_clip, self.time_sel) {
            let n = (le - ls) as i64;
            let s = (ls as i64 + sign * n).max(0) as usize;
            self.arr_loop = Some((s, s + n as usize));
            self.dirty = true;
            return;
        }
        let t = (self.active as i64 + sign).clamp(0, self.tracks.len().saturating_sub(1) as i64) as usize;
        self.set_active(t);
    }

    /// ⌘↑ ⌘↓: double or halve the loop's length (never less than a bar).
    pub(super) fn resize_loop(&mut self, longer: bool) {
        let Some((ls, le)) = self.arr_loop else { return };
        let n = le - ls;
        let bar = self.bar_frames().round() as usize;
        let n = if longer { n * 2 } else { (n / 2).max(bar) };
        self.arr_loop = Some(self.whole_bars((ls, ls + n)));
        self.dirty = true;
    }
}
