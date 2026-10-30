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
        let mut jobs = Vec::new();
        // (Runs every frame: only touch the tracks when there's something to fit.)
        if moved || off_tempo(&self.tracks, tempo) {
            // A copy, as rendering a group needs the tracks' sample rate.
            let mut tracks = self.tracks.clone();
            changed |= self.refit_tracks(&mut tracks, tempo, r, moved, Some(&mut jobs));
            self.tracks = tracks;
        }
        // The song outside an open group follows too, as does where its Session clips have got to.
        let mut nest = std::mem::take(&mut self.nest);
        for f in nest.iter_mut().filter(|f| moved || off_tempo(&f.tracks, tempo)) {
            changed |= self.refit_tracks(&mut f.tracks, tempo, r, moved, Some(&mut jobs));
            if moved {
                f.outer_at = f.outer_at.map(|(at, when)| (sc(at + (when.elapsed().as_secs_f64() * self.rate() as f64) as usize), std::time::Instant::now()));
            }
        }
        self.nest = nest;
        self.render_groups(jobs);
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
    /// groups. `r` scales unwarped places when the layout has `moved`. This only moves things:
    /// a group plays its old mix re-pitched (a passing warp) until its new one is rendered on
    /// another thread; each top-level group to render goes in `jobs`. Returns whether anything
    /// changed.
    fn refit_tracks(&self, tracks: &mut [Track], tempo: f64, r: f64, moved: bool, mut jobs: Option<&mut Vec<GroupJob>>) -> bool {
        let mut changed = false;
        for t in tracks {
            for (slot, c) in t.clips.iter_mut().map(|c| (false, c)).chain(t.slots.iter_mut().flatten().map(|c| (true, c))) {
                let mut this = false;
                if let Some(g) = c.group.clone() {
                    let mut inner = (*g).clone();
                    if self.refit_tracks(&mut inner, tempo, r, moved, None) {
                        // Until its new mix is ready, the old one plays re-pitched to the tempo.
                        if c.warp.is_none() {
                            let made_at = if moved { tempo * r } else { inner_tempo(&g).unwrap_or(tempo) };
                            c.warp = Some(Warp { bpm: made_at, source: c.audio.clone(), source_peaks: c.peaks.clone(), tempo: made_at, repitch: true });
                        }
                        c.fit_place_to_tempo(tempo);
                        let inner = Arc::new(inner);
                        c.group = Some(inner.clone());
                        if let Some(jobs) = jobs.as_deref_mut() {
                            jobs.push(GroupJob { key: Arc::as_ptr(&inner) as usize, inner, brace: c.loop_points });
                        }
                        this = true;
                    }
                } else {
                    if c.warp.is_none() && moved && !slot {
                        c.start = (c.start as f64 * r).round() as usize;
                        this = true;
                    }
                    this |= c.fit_place_to_tempo(tempo);
                }
                if slot && this {
                    *c = session::slot_clip(c.clone());
                }
                changed |= this;
            }
        }
        changed
    }

    /// Render the groups refitted to a new tempo on another thread (their inner clips
    /// stretched, groups inside them first); `poll_groups` puts the new mixes in.
    fn render_groups(&mut self, jobs: Vec<GroupJob>) {
        if jobs.is_empty() {
            return;
        }
        let (rate, beat, repitch) = (self.rate(), self.beat_frames(), repitch_mode());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for job in jobs {
                let mut inner = (*job.inner).clone();
                render_inner(&mut inner, rate, beat, repitch);
                let mut audio = mix_tracks(&inner, &[], rate, beat, true);
                if let Some((_, e)) = job.brace.filter(|&(_, e)| e > audio.frames()) {
                    audio.channels.iter_mut().for_each(|ch| ch.resize(e, 0.0));
                }
                if tx.send((job.key, Arc::new(inner), Arc::new(audio))).is_err() {
                    return;
                }
            }
        });
        self.group_renders.push(rx);
    }

    /// Put newly rendered group mixes into their clips (wherever they are now, if still the
    /// same content).
    pub(super) fn poll_groups(&mut self) {
        let mut done = Vec::new();
        self.group_renders.retain(|rx| loop {
            match rx.try_recv() {
                Ok(x) => done.push(x),
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        });
        if done.is_empty() {
            return;
        }
        let mut nest = std::mem::take(&mut self.nest);
        for (key, inner, audio) in done {
            let peaks = Arc::new(Peaks::build(&audio));
            for t in self.tracks.iter_mut().chain(nest.iter_mut().flat_map(|f| f.tracks.iter_mut())) {
                for (slot, c) in t.clips.iter_mut().map(|c| (false, c)).chain(t.slots.iter_mut().flatten().map(|c| (true, c))) {
                    if c.group.as_ref().is_some_and(|g| Arc::as_ptr(g) as usize == key) {
                        (c.group, c.audio, c.peaks, c.warp) = (Some(inner.clone()), audio.clone(), peaks.clone(), None);
                        c.trim = c.trim.min(audio.frames().saturating_sub(1));
                        c.len = if c.looping { c.len } else { c.len.min(audio.frames() - c.trim).max(1) };
                        if slot {
                            *c = session::slot_clip(c.clone());
                        }
                    }
                }
            }
        }
        self.nest = nest;
        self.refresh_playback();
    }

    /// Stretch warped clips' audio for the current tempo on another thread, so the app doesn't
    /// stop while it works; `poll_stretch` swaps the results in.
    pub(super) fn stretch_in_background(&mut self) {
        let mut jobs: Vec<(Arc<Audio>, f64)> = Vec::new();
        // (Groups re-pitch until `render_groups` gives them a new mix.)
        for c in self.tracks.iter().flat_map(|t| t.clips.iter().chain(t.slots.iter().flatten())).filter(|c| c.group.is_none()) {
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


/// A group refitted to a new tempo, waiting for its mix: its content (whose address finds it
/// again) and its loop brace.
pub(super) struct GroupJob {
    key: usize,
    inner: Arc<Vec<Track>>,
    brace: Option<Sel>,
}

/// The tempo a group's warped clips were laid out for.
fn inner_tempo(tracks: &[Track]) -> Option<f64> {
    tracks.iter().flat_map(|t| &t.clips).find_map(|c| c.warp.as_ref().map(|w| w.tempo).or_else(|| c.group.as_ref().and_then(|g| inner_tempo(g))))
}

/// Make a group's clips ready to mix (on the render thread): stretch warped clips to their
/// tempo, and render the groups inside it.
fn render_inner(tracks: &mut [Track], rate: u32, beat: f64, repitch: bool) {
    for c in tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if let Some(g) = c.group.clone() {
            if c.warp.take().is_some() {
                let mut inner = (*g).clone();
                render_inner(&mut inner, rate, beat, repitch);
                let mut audio = mix_tracks(&inner, &[], rate, beat, true);
                if let Some((_, e)) = c.loop_points.filter(|&(_, e)| e > audio.frames()) {
                    audio.channels.iter_mut().for_each(|ch| ch.resize(e, 0.0));
                }
                c.peaks = Arc::new(Peaks::build(&audio));
                c.audio = Arc::new(audio);
                c.group = Some(Arc::new(inner));
                c.trim = c.trim.min(c.audio.frames().saturating_sub(1));
                c.len = if c.looping { c.len } else { c.len.min(c.audio.frames() - c.trim).max(1) };
            }
        } else if let (Some((_, factor)), Some(w)) = (c.awaiting_stretch(), c.warp.as_mut()) {
            let (audio, peaks) = stretched(&w.source, factor, repitch);
            w.repitch = repitch;
            (c.audio, c.peaks) = (audio, peaks);
        }
    }
}
