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
}
