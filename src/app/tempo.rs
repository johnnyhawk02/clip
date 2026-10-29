//! Following the song tempo: clips stay on their bars, warped clips re-pitch or stretch (the
//! stretching on a background thread).

use super::*;

impl App {
    /// After the song tempo changes, keep everything on its bar: warped clips re-stretch (moving
    /// with it), unwarped clips move to the same bar at their own speed, and the loop, playhead
    /// and time selection follow.
    pub(super) fn refit_warped(&mut self) {
        let tempo = self.tempo;
        let mut changed = false;
        let r = self.laid_out_tempo / tempo;
        let moved = (r - 1.0).abs() > 1e-9 && tempo > 0.0;
        let sc = |f: usize| (f as f64 * r).round() as usize;
        for t in &mut self.tracks {
            for c in &mut t.clips {
                if c.warp.is_none() && moved {
                    c.start = sc(c.start);
                    changed = true;
                }
                changed |= c.fit_place_to_tempo(tempo);
            }
        }
        if moved {
            self.laid_out_tempo = tempo;
            self.player.rescale(r);
            self.live_speed = 1.0;
            self.arr_loop = self.arr_loop.map(|(a, b)| (sc(a), sc(b)));
            self.time_sel = self.time_sel.map(|((a, b), l)| ((sc(a), sc(b)), l));
            self.arr_cursor = sc(self.arr_cursor);
            for play in &mut self.session_play {
                for seg in &mut play.segments {
                    (seg.1, seg.2) = (sc(seg.1), seg.2.map(sc));
                }
            }
            // Same bars in view: the zoom doesn't change, only the time each bar takes.
            self.arr_view_start *= r;
            self.arr_view_len *= r;
            self.arr_clamp();
        }
        if changed {
            self.dirty = true;
            // Playback picks up the new layout straight away (re-pitched clips need nothing
            // more); the stretch below is for drawing, exports and Keep pitch.
            self.refresh_playback();
            self.stretch_in_background();
        }
    }

    /// Stretch warped clips' audio for the current tempo on another thread, so the app doesn't
    /// stop while it works; `poll_stretch` swaps the results in.
    pub(super) fn stretch_in_background(&mut self) {
        let mut jobs: Vec<(Arc<Audio>, f64)> = Vec::new();
        for c in self.tracks.iter().flat_map(|t| &t.clips) {
            if let (Some((key, factor)), Some(w)) = (c.awaiting_stretch(), c.warp.as_ref()) {
                if !jobs.iter().any(|(s, f)| Arc::as_ptr(s) as usize == key && (f - factor).abs() < 1e-9) {
                    jobs.push((w.source.clone(), factor));
                }
            }
        }
        if jobs.is_empty() {
            self.refresh_playback();
            return;
        }
        let (tx, rx) = channel();
        let repitch = repitch_mode();
        std::thread::spawn(move || {
            for (source, factor) in jobs {
                let (audio, peaks) = stretched(&source, factor, repitch);
                if tx.send((Arc::as_ptr(&source) as usize, factor, audio, peaks)).is_err() {
                    return;
                }
            }
        });
        self.stretching = Some(rx);
    }

    /// Put stretched audio from `stretch_in_background` into the clips still waiting for it
    /// (results for a tempo that has since changed again are dropped).
    pub(super) fn poll_stretch(&mut self) {
        let Some(rx) = &self.stretching else { return };
        let mut got = false;
        loop {
            match rx.try_recv() {
                Ok((key, factor, audio, peaks)) => {
                    for c in self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
                        if c.awaiting_stretch().is_some_and(|(k, f)| k == key && (f - factor).abs() < 1e-9) {
                            (c.audio, c.peaks) = (audio.clone(), peaks.clone());
                            if let Some(w) = c.warp.as_mut() {
                                w.repitch = repitch_mode();
                            }
                            got = true;
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.stretching = None;
                    break;
                }
            }
        }
        if got {
            self.refresh_playback();
        }
    }
}
