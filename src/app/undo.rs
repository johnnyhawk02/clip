//! Undo and redo: whole-state snapshots, kept within a memory budget.

use super::*;

impl App {
    pub(super) fn push_undo(&mut self, label: &'static str) {
        if self.tracks.is_empty() && self.audio.is_none() {
            return;
        }
        self.undo.push(Snapshot {
            audio: self.audio.clone(),
            tracks: self.tracks.clone(),
            nest: self.nest.clone(),
            active: self.active,
            sel_clip: self.sel_clip,
            cues: self.cues.clone(),
            regions: self.regions.clone(),
            loop_points: self.loop_points,
            sel: self.sel,
            cursor: self.cursor,
            tempo: self.tempo,
            beats_per_bar: self.beats_per_bar,
            arr_loop: self.arr_loop,
            label,
        });
        self.redo.clear();
        // Drop oldest steps beyond the memory budget (always keep at least one).
        let bytes = |s: &Snapshot| s.audio.as_ref().map_or(0, |a| a.bytes());
        let mut total: usize = self.undo.iter().map(bytes).sum();
        while total > UNDO_BUDGET_BYTES && self.undo.len() > 1 {
            total -= bytes(&self.undo.remove(0));
        }
    }

    /// After undo/redo (which stay in the current view): reload the clip being edited from the
    /// restored tracks, or refresh the arrangement.
    pub(super) fn restore_audio(&mut self, a: Option<Arc<Audio>>) {
        match self.view {
            View::Editor { track, clip } => match self.clip((track, clip)).cloned() {
                Some(c) => {
                    let audio = if c.looping || (c.trim == 0 && c.len >= c.audio.frames()) { c.audio.clone() } else { Arc::new(c.visible()) };
                    self.set_audio(audio);
                }
                // The clip being edited no longer exists in the restored state.
                None => {
                    self.view = View::Arrangement;
                    self.refresh_playback();
                }
            },
            View::Arrangement => {
                if let Some(a) = a {
                    self.audio = Some(a);
                }
                self.refresh_playback();
            }
        }
    }

    pub(super) fn undo(&mut self) {
        let Some(snap) = self.undo.pop() else { return };
        let cur = self.audio.clone();
        self.redo.push(Snapshot {
            audio: cur,
            tracks: std::mem::replace(&mut self.tracks, snap.tracks),
            nest: std::mem::replace(&mut self.nest, snap.nest),
            active: std::mem::replace(&mut self.active, snap.active),
            sel_clip: std::mem::replace(&mut self.sel_clip, snap.sel_clip),
            cues: std::mem::replace(&mut self.cues, snap.cues),
            regions: std::mem::replace(&mut self.regions, snap.regions),
            loop_points: std::mem::replace(&mut self.loop_points, snap.loop_points),
            sel: self.sel,
            cursor: self.cursor,
            tempo: std::mem::replace(&mut self.tempo, snap.tempo),
            beats_per_bar: std::mem::replace(&mut self.beats_per_bar, snap.beats_per_bar),
            arr_loop: std::mem::replace(&mut self.arr_loop, snap.arr_loop),
            label: snap.label,
        });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.laid_out_tempo = self.tempo;
        self.restore_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Undid {}", snap.label);
    }

    pub(super) fn redo(&mut self) {
        let Some(snap) = self.redo.pop() else { return };
        let cur = self.audio.clone();
        self.undo.push(Snapshot {
            audio: cur,
            tracks: std::mem::replace(&mut self.tracks, snap.tracks),
            nest: std::mem::replace(&mut self.nest, snap.nest),
            active: std::mem::replace(&mut self.active, snap.active),
            sel_clip: std::mem::replace(&mut self.sel_clip, snap.sel_clip),
            cues: std::mem::replace(&mut self.cues, snap.cues),
            regions: std::mem::replace(&mut self.regions, snap.regions),
            loop_points: std::mem::replace(&mut self.loop_points, snap.loop_points),
            sel: self.sel,
            cursor: self.cursor,
            tempo: std::mem::replace(&mut self.tempo, snap.tempo),
            beats_per_bar: std::mem::replace(&mut self.beats_per_bar, snap.beats_per_bar),
            arr_loop: std::mem::replace(&mut self.arr_loop, snap.arr_loop),
            label: snap.label,
        });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.laid_out_tempo = self.tempo;
        self.restore_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Redid {}", snap.label);
    }
}
