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
        // (Runs every frame: only touch the tracks when there's something to fit.)
        if moved || off_tempo(&self.tracks, tempo) {
            // A copy, as rendering a group needs the tracks' sample rate.
            let mut tracks = self.tracks.clone();
            changed |= self.refit_tracks(&mut tracks, tempo, r, moved, false);
            self.tracks = tracks;
        }
        // The song outside an open group follows too, as does where its Session clips have got to.
        let mut nest = std::mem::take(&mut self.nest);
        for f in nest.iter_mut().filter(|f| moved || off_tempo(&f.tracks, tempo)) {
            changed |= self.refit_tracks(&mut f.tracks, tempo, r, moved, false);
            if moved {
                f.outer_at = f.outer_at.map(|(at, when)| (sc(at + (when.elapsed().as_secs_f64() * self.rate() as f64) as usize), std::time::Instant::now()));
            }
        }
        self.nest = nest;
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

    /// Fit `tracks` to `tempo`: their song clips and Session slots, and the tracks inside their
    /// groups (re-rendered, so a group plays in time as it would opened). `r` scales unwarped
    /// places when the layout has `moved`. Clips inside groups are stretched straight away
    /// (`nested`), since the group's mix is made from them. Returns whether anything changed.
    fn refit_tracks(&self, tracks: &mut [Track], tempo: f64, r: f64, moved: bool, nested: bool) -> bool {
        let mut changed = false;
        for t in tracks {
            for (slot, c) in t.clips.iter_mut().map(|c| (false, c)).chain(t.slots.iter_mut().flatten().map(|c| (true, c))) {
                let was = changed;
                changed = false;
                if c.warp.is_none() && moved && !slot {
                    c.start = (c.start as f64 * r).round() as usize;
                    changed = true;
                }
                if let Some(g) = c.group.clone() {
                    let mut inner = (*g).clone();
                    if self.refit_tracks(&mut inner, tempo, r, moved, true) {
                        // Its trim and loop stay on the same beats of the new mix.
                        let showed_all = c.trim == 0 && c.len >= c.audio.frames();
                        let f = content_end(&inner) as f64 / content_end(&g).max(1) as f64;
                        let sc = |x: usize| (x as f64 * f).round() as usize;
                        (c.trim, c.len) = (sc(c.trim), sc(c.len).max(1));
                        c.loop_points = c.loop_points.map(|(a, b)| (sc(a), sc(b)));
                        let audio = Arc::new(self.group_mix(&inner, c.loop_points));
                        if showed_all {
                            (c.trim, c.len) = (0, audio.frames().max(1));
                        }
                        c.peaks = Arc::new(Peaks::build(&audio));
                        c.audio = audio;
                        c.group = Some(Arc::new(inner));
                        changed = true;
                    }
                } else if c.fit_place_to_tempo(tempo) {
                    changed = true;
                    if nested {
                        if let (Some((_, factor)), Some(w)) = (c.awaiting_stretch(), c.warp.as_mut()) {
                            let (audio, peaks) = stretched(&w.source, factor, repitch_mode());
                            w.repitch = repitch_mode();
                            (c.audio, c.peaks) = (audio, peaks);
                        }
                    }
                }
                if slot && changed {
                    *c = session::slot_clip(c.clone());
                }
                changed |= was;
            }
        }
        changed
    }

    /// Stretch warped clips' audio for the current tempo on another thread, so the app doesn't
    /// stop while it works; `poll_stretch` swaps the results in.
    pub(super) fn stretch_in_background(&mut self) {
        let mut jobs: Vec<(Arc<Audio>, f64)> = Vec::new();
        for c in self.tracks.iter().flat_map(|t| t.clips.iter().chain(t.slots.iter().flatten())) {
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
                    for c in self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut().chain(t.slots.iter_mut().flatten())) {
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

/// Whether any warped clip in `tracks` (in a slot, or inside a group) is laid out for another tempo.
fn off_tempo(tracks: &[Track], tempo: f64) -> bool {
    tempo > 0.0
        && tracks.iter().flat_map(|t| t.clips.iter().chain(t.slots.iter().flatten())).any(|c| {
            c.warp.as_ref().is_some_and(|w| (w.tempo - tempo).abs() >= 1e-6) || c.group.as_ref().is_some_and(|g| off_tempo(g, tempo))
        })
}

/// Where the last clip in `tracks` ends.
fn content_end(tracks: &[Track]) -> usize {
    tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.end())).max().unwrap_or(0)
}
