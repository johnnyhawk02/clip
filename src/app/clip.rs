//! Clips and tracks: the Clip and Track types, warping a loop to the song tempo, and the clip
//! operations (editing, looping, groups, collapsing).

use super::*;

/// A piece of audio placed on a track's timeline. Only `len` frames from `trim` onward are
/// heard, so its edges can be trimmed without losing the audio underneath (as in Ableton).
#[derive(Clone)]
pub(super) struct Clip {
    pub(super) name: String,
    pub(super) audio: Arc<Audio>,
    pub(super) peaks: Arc<Peaks>,
    /// Timeline position of the clip's first frame.
    pub(super) start: usize,
    pub(super) trim: usize,
    pub(super) len: usize,
    pub(super) cues: Vec<Cue>,
    pub(super) regions: Vec<Region>,
    pub(super) loop_points: Option<Sel>,
    /// A group clip holds its own tracks of clips; `audio` is then their rendered mix. Copies
    /// share this until one is opened and changed, so they stay independent.
    pub(super) group: Option<Arc<Vec<Track>>>,
    /// Repeat the clip's loop section (its loop points, or the whole audio) for as long as the
    /// clip is stretched out, independently of other clips.
    pub(super) looping: bool,
    /// A warped clip plays its source audio stretched to the song tempo (keeping its pitch),
    /// as in Ableton; `audio` is then the stretched version.
    /// Transposition in semitones from the Clip View (keeps the length), and the audio from
    /// before it, to go back to. Baked in when the project is saved.
    pub(super) transpose: i32,
    pub(super) untransposed: Option<Arc<Audio>>,
    /// Off (0 key, as Live): kept in place but silent, so the layer below shows through.
    pub(super) active: bool,
    /// Fades drawn with the clip's corner handles (frames), as in Live.
    pub(super) fade_in: usize,
    pub(super) fade_out: usize,
    /// The clip's own volume (linear), set in the Clip View.
    pub(super) gain: f32,
    /// Where bar 1.1.1 is in the audio (frames): the clip editor's ruler counts from here, with
    /// -1, -2… before it.
    pub(super) origin: usize,
    pub(super) warp: Option<Warp>,
    /// Its own colour from Live's palette; none follows the track's.
    pub(super) color: Option<usize>,
}

#[derive(Clone)]
pub(super) struct Warp {
    /// The clip's own tempo.
    pub(super) bpm: f64,
    /// The audio at its original speed.
    pub(super) source: Arc<Audio>,
    /// `source`'s peaks: the clip is drawn from these, scaled to the tempo, so its picture never
    /// waits for the audio to be re-stretched.
    pub(super) source_peaks: Arc<Peaks>,
    /// The song tempo `audio` is currently stretched to.
    pub(super) tempo: f64,
    /// Whether `audio` was made by re-pitching (speed and pitch together) rather than stretching.
    pub(super) repitch: bool,
}

/// How warped clips follow the song tempo: re-pitched like a record (speed and pitch change
/// together, as in Live's Re-Pitch), or stretched keeping their pitch. Set in Preferences.
pub(super) static REPITCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(super) fn repitch_mode() -> bool {
    REPITCH.load(std::sync::atomic::Ordering::Relaxed)
}

/// A tempo and bar count written in a file name, as in "Loop 118bpm 16bar" or "Kick - 124 BPM".
pub(super) fn tempo_in_name(name: &str) -> (Option<f64>, Option<f64>) {
    let lower = name.to_lowercase();
    let b = lower.as_bytes();
    let is_sep = |c: u8| c == b' ' || c == b'_' || c == b'-';
    // The number just before position `i` (skipping separators), or just after `j`.
    let before = |i: usize| {
        let mut e = i;
        while e > 0 && is_sep(b[e - 1]) {
            e -= 1;
        }
        let mut st = e;
        while st > 0 && (b[st - 1].is_ascii_digit() || b[st - 1] == b'.') {
            st -= 1;
        }
        lower[st..e].trim_start_matches('.').parse::<f64>().ok()
    };
    let after = |j: usize| {
        let mut st = j;
        while st < b.len() && is_sep(b[st]) {
            st += 1;
        }
        let mut e = st;
        while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'.') {
            e += 1;
        }
        lower[st..e].trim_end_matches('.').parse::<f64>().ok()
    };
    let mut bpm = None;
    let mut bars = None;
    for (i, _) in lower.match_indices("bpm") {
        bpm = bpm.or(before(i).or_else(|| after(i + 3)).filter(|v| (40.0..=300.0).contains(v)));
    }
    for (i, _) in lower.match_indices("bar") {
        // "bar" or "bars" as a word ending (not "barrel").
        let mut j = i + 3;
        if b.get(j) == Some(&b's') {
            j += 1;
        }
        if b.get(j).is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        bars = bars.or(before(i).filter(|v| *v >= 1.0 && *v <= 256.0 && v.fract() == 0.0));
    }
    (bpm, bars)
}

/// The tempo of a clip from its name: the bar count gives the exact tempo when it agrees with
/// the BPM written there (or when only bars are given).
pub(super) fn clip_tempo_from_name(name: &str, frames: usize, rate: u32, beats_per_bar: u32) -> Option<f64> {
    let (bpm, bars) = tempo_in_name(name);
    let secs = frames as f64 / rate.max(1) as f64;
    let from_bars = bars.map(|n| n * beats_per_bar as f64 * 60.0 / secs).filter(|v| secs > 0.0 && (40.0..=300.0).contains(v));
    match (bpm, from_bars) {
        // The written BPM when the length agrees with it, else the tempo the length gives.
        (Some(b), Some(f)) if (f - b).abs() < 0.02 => Some(b),
        (Some(b), Some(f)) if (f / b - 1.0).abs() < 0.03 => Some(f),
        (Some(b), _) => Some(b),
        (None, f) => f,
    }
}

/// Play a clip backwards. The audio underneath is reversed too, so trimmed edges can still be
/// dragged back out; a loop or group becomes plain audio.
pub(super) fn reverse_clip(clip: &mut Clip) {
    let rev = |a: &Audio| Audio { rate: a.rate, channels: a.channels.iter().map(|c| c.iter().rev().copied().collect()).collect() };
    if clip.looping || clip.group.is_some() {
        *clip = Clip::new(clip.name.clone(), Arc::new(rev(&clip.visible())), clip.start);
        return;
    }
    let n = clip.audio.frames();
    let end = (clip.trim + clip.len).min(n);
    clip.audio = Arc::new(rev(&clip.audio));
    clip.peaks = Arc::new(Peaks::build(&clip.audio));
    if let Some(w) = clip.warp.as_mut() {
        w.source = Arc::new(rev(&w.source));
    }
    clip.trim = n - end;
    let m = |f: usize| n - f.min(n);
    clip.loop_points = clip.loop_points.map(|(s, e)| (m(e), m(s)));
    for q in &mut clip.cues {
        q.frame = m(q.frame);
    }
    for g in &mut clip.regions {
        (g.start, g.end) = (m(g.end), m(g.start));
    }
}

impl Clip {
    pub(super) fn new(name: impl Into<String>, audio: Arc<Audio>, start: usize) -> Clip {
        let peaks = Arc::new(Peaks::build(&audio));
        let len = audio.frames();
        Clip { name: name.into(), audio, peaks, start, trim: 0, len, cues: Vec::new(), regions: Vec::new(), loop_points: None, group: None, looping: false, warp: None, origin: 0, gain: 1.0, fade_in: 0, fade_out: 0, active: true, transpose: 0, untransposed: None, color: None }
    }

    pub(super) fn end(&self) -> usize {
        self.start + self.len
    }

    /// Stretch a warped clip to `tempo`, keeping its place and length in beats.
    pub(super) fn fit_to_tempo(&mut self, tempo: f64) -> bool {
        if !self.fit_place_to_tempo(tempo) {
            return false;
        }
        let repitch = repitch_mode();
        let w = self.warp.as_mut().unwrap();
        w.repitch = repitch;
        let (audio, peaks) = stretched(&w.source, w.bpm / tempo, repitch);
        (self.audio, self.peaks) = (audio, peaks);
        true
    }

    /// The cheap half of `fit_to_tempo`: move and size the clip for `tempo` straight away, leaving
    /// its audio to be stretched (see `stretched`). False if it was already fitted.
    pub(super) fn fit_place_to_tempo(&mut self, tempo: f64) -> bool {
        let Some(w) = self.warp.as_mut() else { return false };
        if (w.tempo - tempo).abs() < 1e-6 || tempo <= 0.0 {
            return false;
        }
        let r = w.tempo / tempo;
        w.tempo = tempo;
        let sc = |f: usize| (f as f64 * r).round() as usize;
        // The length the audio will have once stretched.
        let n = (w.source.frames() as f64 * w.bpm / tempo).round() as usize;
        self.start = sc(self.start);
        self.trim = sc(self.trim).min(n.saturating_sub(1));
        self.len = sc(self.len).max(1);
        if !self.looping {
            self.len = self.len.min(n - self.trim);
        }
        self.loop_points = self.loop_points.map(|(a, b)| (sc(a).min(n), sc(b).min(n)));
        for q in &mut self.cues {
            q.frame = sc(q.frame).min(n);
        }
        for g in &mut self.regions {
            (g.start, g.end) = (sc(g.start).min(n), sc(g.end).min(n));
        }
        true
    }

    /// Whether this clip still shows audio made for an earlier tempo or the other warp mode.
    pub(super) fn awaiting_stretch(&self) -> Option<(usize, f64)> {
        let w = self.warp.as_ref()?;
        let n = (w.source.frames() as f64 * w.bpm / w.tempo).round() as usize;
        let stale = self.audio.frames().abs_diff(n) > 2 || (w.repitch != repitch_mode() && (w.bpm - w.tempo).abs() > 1e-6);
        stale.then(|| (Arc::as_ptr(&w.source) as usize, w.bpm / w.tempo))
    }

    /// Start warping at clip tempo `bpm`: from now on the clip follows the song tempo.
    pub(super) fn warp_from(&mut self, bpm: f64, song_tempo: f64) {
        self.warp = Some(Warp { bpm, source: self.audio.clone(), source_peaks: self.peaks.clone(), tempo: bpm, repitch: repitch_mode() });
        self.fit_to_tempo(song_tempo);
    }

    /// Back to the audio's own speed.
    pub(super) fn unwarp(&mut self) {
        let Some(bpm) = self.warp.as_ref().map(|w| w.bpm) else { return };
        self.fit_to_tempo(bpm);
        self.warp = None;
    }

    /// The part of the audio the clip plays.
    pub(super) fn visible(&self) -> Audio {
        if !self.looping {
            let end = (self.trim + self.len).min(self.audio.frames());
            return self.audio.slice(self.trim.min(end), end);
        }
        Audio {
            rate: self.audio.rate,
            channels: self.audio.channels.iter().map(|c| (0..self.len).map(|i| c[self.source(i)]).collect()).collect(),
        }
    }

    /// The section a looping clip repeats: its loop points if it has them, else all of it.
    pub(super) fn loop_range(&self) -> (usize, usize) {
        let n = self.audio.frames();
        self.loop_points.map(|(s, e)| (s.min(n), e.min(n))).filter(|(s, e)| e > s).unwrap_or((0, n))
    }

    /// What to draw the clip from: its audio and peaks, how many of their frames go by per clip
    /// frame, and how long the audio is in clip frames. A warped clip draws from its original,
    /// scaled to the tempo, so the picture is right even before its audio is re-stretched.
    pub(super) fn picture(&self) -> (&Arc<Audio>, &Arc<Peaks>, f64, usize) {
        match &self.warp {
            Some(w) => (&w.source, &w.source_peaks, w.tempo / w.bpm, (w.source.frames() as f64 * w.bpm / w.tempo).round() as usize),
            None => (&self.audio, &self.peaks, 1.0, self.audio.frames()),
        }
    }

    /// `source` for audio `frames` long (the length a warped clip's audio will have).
    pub(super) fn source_in(&self, i: usize, frames: usize) -> usize {
        let p = self.trim + i;
        if self.looping {
            let (ls, le) = self.loop_points.map(|(s, e)| (s.min(frames), e.min(frames))).filter(|(s, e)| e > s).unwrap_or((0, frames));
            if p >= le && le > ls {
                return ls + (p - le) % (le - ls);
            }
        }
        p.min(frames.max(1) - 1)
    }

    /// The audio frame heard `i` frames into the clip.
    pub(super) fn source(&self, i: usize) -> usize {
        let p = self.trim + i;
        let n = self.audio.frames().max(1);
        if self.looping {
            let (ls, le) = self.loop_range();
            if p >= le && le > ls {
                return ls + (p - le) % (le - ls);
            }
        }
        p.min(n - 1)
    }
}

/// Where clips on one track overlap, the earlier one fades out across the overlap while the
/// later one fades in (equal power). Returns each clip's (fade in, fade out) in frames. A clip
/// lying wholly inside another just plays on top of it.
pub(super) fn crossfades(clips: &[Clip]) -> Vec<(usize, usize)> {
    clips
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (mut fin, mut fout) = (0, 0);
            for (j, o) in clips.iter().enumerate() {
                if i == j {
                    continue;
                }
                if o.start < c.start && o.end() > c.start && o.end() < c.end() {
                    fin = fin.max(o.end() - c.start);
                }
                if o.start > c.start && o.start < c.end() && o.end() > c.end() {
                    fout = fout.max(c.end() - o.start);
                }
            }
            if fin + fout > c.len {
                let k = c.len as f64 / (fin + fout) as f64;
                (fin, fout) = ((fin as f64 * k) as usize, (fout as f64 * k) as usize);
            }
            (fin, fout)
        })
        .collect()
}

/// A warped clip's source made `factor` times as long, re-pitched or stretched (and its peaks
/// for drawing).
pub(super) fn stretched(source: &Arc<Audio>, factor: f64, repitch: bool) -> (Arc<Audio>, Arc<Peaks>) {
    let audio = if (factor - 1.0).abs() < 1e-6 {
        source.clone()
    } else if repitch {
        Arc::new(source.varispeed(factor))
    } else {
        Arc::new(effects::time_stretch(source, factor))
    };
    let peaks = Arc::new(Peaks::build(&audio));
    (audio, peaks)
}

/// Longest edge fade, as in Live's clip-edge fades.
const EDGE_FADE_SECS: f64 = 0.004;

/// Fades for a clip's edges where nothing crossfades them: none where the audio starts or ends
/// at (or near) a zero crossing, so a transient stays untouched, growing to 4 ms where the edge
/// cuts through a loud part of the wave and would click.
pub(super) fn edge_fades(a: &Audio, fades: (usize, usize)) -> (usize, usize) {
    let n = a.frames();
    if n == 0 {
        return fades;
    }
    let level = |i: usize| a.channels.iter().map(|c| c[i].abs()).fold(0.0f32, f32::max);
    edge_fades_for(level(0), level(n - 1), n, a.rate, fades)
}

/// A clip's fades: its own (from the corner handles) where longer than the automatic ones
/// (crossfades and edge fades), shared out so they never overlap.
pub(super) fn with_clip_fades(c: &Clip, (fin, fout): (usize, usize)) -> (usize, usize) {
    let (fin, fout) = (fin.max(c.fade_in), fout.max(c.fade_out));
    if fin + fout > c.len {
        let k = c.len as f64 / (fin + fout) as f64;
        return ((fin as f64 * k) as usize, (fout as f64 * k) as usize);
    }
    (fin, fout)
}

/// `edge_fades` from the levels at a clip's first and last frames, its length and rate.
pub(super) fn edge_fades_for(first: f32, last: f32, n: usize, rate: u32, (fin, fout): (usize, usize)) -> (usize, usize) {
    let full = (rate as f64 * EDGE_FADE_SECS).round();
    let len = |level: f32| (full * (level as f64 / 0.05).min(1.0)).round() as usize;
    let fin = if fin == 0 { len(first) } else { fin };
    let fout = if fout == 0 { len(last) } else { fout };
    // A very short clip shares its length between the two.
    if fin + fout > n {
        return (n / 2, n - n / 2);
    }
    (fin, fout)
}

/// Apply a crossfade's fade in and fade out to rendered clip audio.
pub(super) fn apply_crossfade(a: &mut Audio, (fin, fout): (usize, usize)) {
    let n = a.frames();
    for ch in &mut a.channels {
        for k in 0..fin.min(n) {
            ch[k] *= (k as f32 / fin as f32 * std::f32::consts::FRAC_PI_2).sin();
        }
        for k in 0..fout.min(n) {
            ch[n - 1 - k] *= ((k + 1) as f32 / fout as f32 * std::f32::consts::FRAC_PI_2).sin();
        }
    }
}

/// One track of the arrangement: its clips and mixer settings. All clips share a sample rate.
#[derive(Clone)]
pub(super) struct Track {
    pub(super) name: String,
    pub(super) clips: Vec<Clip>,
    /// The track's Session view slots, one per scene (a slot may be empty).
    pub(super) slots: Vec<Option<Clip>>,
    /// Linear volume (1 = unchanged).
    pub(super) gain: f32,
    /// Pan, -1 (left) to 1 (right).
    pub(super) pan: f32,
    /// Its colour in Live's palette (`LIVE_PALETTE`).
    pub(super) color: usize,
    pub(super) mute: bool,
    pub(super) solo: bool,
}

impl Track {
    pub(super) fn new(name: impl Into<String>) -> Track {
        Track { name: name.into(), clips: Vec::new(), slots: Vec::new(), gain: 1.0, pan: 0.0, color: next_color(), mute: false, solo: false }
    }
}

impl App {
    /// The clip at (track, index): one in the song view, or a Session slot's (index `SLOT + scene`).
    pub(super) fn clip(&self, (t, c): (usize, usize)) -> Option<&Clip> {
        let t = self.tracks.get(t)?;
        if c >= SLOT { t.slots.get(c - SLOT)?.as_ref() } else { t.clips.get(c) }
    }

    pub(super) fn clip_mut(&mut self, (t, c): (usize, usize)) -> Option<&mut Clip> {
        let t = self.tracks.get_mut(t)?;
        if c >= SLOT { t.slots.get_mut(c - SLOT)?.as_mut() } else { t.clips.get_mut(c) }
    }

    /// Open a clip in the waveform editor.
    pub(super) fn edit_clip(&mut self, (t, c): (usize, usize)) {
        let Some(clip) = self.clip((t, c)).cloned() else { return };
        if clip.group.is_some() {
            return self.enter_group((t, c));
        }
        self.player.stop();
        // The detail view may already show this clip; then just give it the focus.
        if self.detail_key != self.detail_key_of((t, c)) {
            self.load_detail((t, c));
        }
        self.view = View::Editor { track: t, clip: c };
        self.active = t;
        self.sel_clip = Some((t, c));
        self.refresh_playback();
        self.status = format!("Editing {} — ⌘Return goes back", clip.name);
    }

    /// What identifies the clip shown in the detail view: its place and what it plays. When it
    /// changes, the detail view is loaded again.
    pub(super) fn detail_key_of(&self, p: (usize, usize)) -> Option<DetailKey> {
        let c = self.clip(p)?;
        Some(((p.0, p.1), Arc::as_ptr(&c.audio) as usize, c.trim, c.len, c.looping))
    }

    /// Load clip `p` into the detail view (the editor below the song) without moving the focus.
    pub(super) fn load_detail(&mut self, p: (usize, usize)) {
        let Some(clip) = self.clip(p).cloned() else { return };
        // Editing works on what the clip shows: bake its trim in first.
        let visible = if clip.looping || clip.warp.is_some() || (clip.trim == 0 && clip.len >= clip.audio.frames()) {
            clip.audio.clone()
        } else {
            Arc::new(clip.visible())
        };
        self.cues = clip.cues;
        self.regions = clip.regions;
        self.loop_points = clip.loop_points;
        self.sel = None;
        self.cursor = 0;
        self.view_start = 0.0;
        self.view_len = visible.frames().max(1) as f64;
        self.detail_key = self.detail_key_of(p);
        self.set_audio(visible);
        self.zoom_fit();
    }

    /// Store the editor's cues, regions and loop with the clip being edited.
    pub(super) fn sync_clip_markers(&mut self) {
        if let View::Editor { track, clip } = self.view {
            let (cues, regions, loop_points) = (self.cues.clone(), self.regions.clone(), self.loop_points);
            if let Some(c) = self.clip_mut((track, clip)) {
                (c.cues, c.regions, c.loop_points) = (cues, regions, loop_points);
            }
        }
    }

    pub(super) fn back_to_arrangement(&mut self) {
        if self.view == View::Arrangement {
            return self.leave_group();
        }
        self.player.stop();
        self.sync_clip_markers();
        self.view = View::Arrangement;
        self.full_edit = false;
        self.sel = None;
        self.refresh_playback();
        self.status = "Arrangement — double-click a clip to edit it".into();
    }

    /// Add a clip to the active track at the playhead, select it, and return its position.
    pub(super) fn add_clip(&mut self, name: &str, audio: Arc<Audio>, label: &'static str) -> (usize, usize) {
        self.push_undo(label);
        if self.tracks.is_empty() {
            self.tracks.push(Track::new("Track 1"));
            self.active = 0;
        }
        let rate = self.rate();
        let audio = if audio.rate != rate { Arc::new(audio.resampled(rate)) } else { audio };
        let t = self.active.min(self.tracks.len() - 1);
        self.tracks[t].clips.push(Clip::new(name, audio, self.arr_cursor));
        let c = self.tracks[t].clips.len() - 1;
        self.sel_clip = Some((t, c));
        self.dirty = true;
        self.refresh_playback();
        (t, c)
    }

    pub(super) fn new_clip(&mut self) {
        let rate = self.rate();
        let ch = self.tracks.iter().flat_map(|t| t.clips.first()).map(|c| c.audio.num_channels()).next().unwrap_or(2);
        let pos = self.add_clip("New clip", Arc::new(effects::silence(rate, ch, 2.0)), "New Clip");
        let pos = self.place_over(pos);
        self.edit_clip(pos);
    }

    pub(super) fn delete_clip(&mut self) {
        let mut picked: Vec<(usize, usize)> = self.selected_clips().into_iter().filter(|&p| self.clip(p).is_some()).collect();
        self.sel_extra.clear();
        if picked.is_empty() {
            return;
        }
        self.push_undo(if picked.len() > 1 { "Delete Clips" } else { "Delete Clip" });
        // Highest index first, so the others keep their places.
        picked.sort_unstable_by(|a, b| b.cmp(a));
        for (t, c) in picked {
            self.tracks[t].clips.remove(c);
        }
        self.sel_clip = None;
        self.dirty = true;
        self.refresh_playback();
    }

    /// Clip ▸ Loop Clip: turn looping on or off for the selected clips. Turning it on also
    /// doubles a clip's length so the repeat is heard straight away.
    pub(super) fn toggle_clip_loop(&mut self) {
        let sel = self.selected_clips();
        if sel.is_empty() {
            return;
        }
        let on = !self.clip(sel[0]).is_some_and(|c| c.looping);
        self.push_undo(if on { "Loop Clip" } else { "Unloop Clip" });
        for (t, c) in sel {
            let clip = &mut self.tracks[t].clips[c];
            clip.looping = on;
            if on {
                let (ls, le) = clip.loop_range();
                if clip.len <= le.saturating_sub(clip.trim) {
                    clip.len = le.saturating_sub(clip.trim) + (le - ls);
                }
            } else {
                clip.len = clip.len.min(clip.audio.frames().saturating_sub(clip.trim)).max(1);
            }
        }
        self.dirty = true;
        self.refresh_playback();
        self.status = if on { "Clip loops — drag its right edge to repeat it more".into() } else { "Clip no longer loops".into() };
    }

    /// Collapse: render the selected clip (a group, a loop, a trimmed clip) down to one plain
    /// audio clip in the same place.
    pub(super) fn collapse_clip(&mut self) {
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some()) else { return };
        self.push_undo("Collapse Clip");
        let old = self.tracks[t].clips[c].clone();
        let mut clip = Clip::new(old.name.clone(), Arc::new(old.visible()), old.start);
        // Markers inside the visible part come along (loops and groups have none to keep).
        if !old.looping {
            let place = |f: usize| (f >= old.trim && f < old.trim + old.len).then(|| f - old.trim);
            clip.cues = old.cues.iter().filter_map(|q| place(q.frame).map(|frame| Cue { frame, name: q.name.clone() })).collect();
        }
        // A warped clip collapses to a warped clip, so it still follows the tempo.
        if old.warp.is_some() {
            clip.warp = Some(Warp { bpm: self.tempo, source: clip.audio.clone(), source_peaks: clip.peaks.clone(), tempo: self.tempo, repitch: repitch_mode() });
        }
        self.tracks[t].clips[c] = clip;
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Collapsed {} to plain audio", old.name);
    }

    /// Collapse Track: render all of a track's clips into one audio clip.
    pub(super) fn collapse_track(&mut self, t: usize) {
        let Some(track) = self.tracks.get(t).cloned() else { return };
        let Some(start) = track.clips.iter().map(|c| c.start).min() else { return };
        self.push_undo("Collapse Track");
        let rendered = self.render_track(&track);
        let audio = rendered.slice(start.min(rendered.frames()), rendered.frames());
        let name = track.name.clone();
        let mut clip = Clip::new(name.clone(), Arc::new(audio), start);
        // Warped clips collapse to a warped clip, so the layer still follows the tempo.
        if track.clips.iter().any(|c| c.warp.is_some()) {
            clip.warp = Some(Warp { bpm: self.tempo, source: clip.audio.clone(), source_peaks: clip.peaks.clone(), tempo: self.tempo, repitch: repitch_mode() });
        }
        self.tracks[t].clips = vec![clip];
        self.sel_clip = Some((t, 0));
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Collapsed track {name} into one clip");
    }

    /// Open a group clip as its own arrangement.
    fn enter_group(&mut self, (t, c): (usize, usize)) {
        let Some(clip) = self.clip((t, c)).cloned() else { return };
        let Some(inner) = clip.group.clone() else { return };
        // Playing on into the group: the same moment, now counted from the group's start.
        let playing = self.player.is_playing().then(|| self.player.position());
        self.nest.push(NestFrame {
            tracks: std::mem::replace(&mut self.tracks, (*inner).clone()),
            track: t,
            clip: c,
            active: self.active,
            view_start: self.arr_view_start,
            view_len: self.arr_view_len,
            from_session: self.show_session,
        });
        // A group opens as its own arrangement, whichever view it was launched from.
        self.show_session = false;
        self.full_edit = false;
        self.active = 0;
        self.sel_clip = None;
        self.sel_extra.clear();
        self.arr_cursor = 0;
        self.arr_fit();
        self.refresh_playback();
        if let Some(pos) = playing.filter(|_| c < SLOT) {
            let inside = (pos.saturating_sub(clip.start) + clip.trim).min(self.arrangement_frames());
            self.go_to_frame(inside);
        }
        self.status = format!("Inside {} — ⌘Return or the path above goes back up", clip.name);
    }

    /// Close the open group, folding its arrangement back into its clip.
    pub(super) fn leave_group(&mut self) {
        let Some(f) = self.nest.pop() else { return };
        let playing = self.player.is_playing().then(|| self.player.position());
        let inner = std::mem::replace(&mut self.tracks, f.tracks);
        if let Some(mut c) = self.clip((f.track, f.clip)).cloned() {
            self.set_group_content(&mut c, inner);
            if f.clip >= SLOT {
                c = session::slot_clip(c);
            }
            if let Some(slot) = self.clip_mut((f.track, f.clip)) {
                *slot = c;
            }
        }
        self.active = f.active;
        self.show_session = f.from_session;
        self.status = "Closed the group".into();
        if f.clip >= SLOT {
            self.sel_slot = Some((f.track, f.clip - SLOT));
            self.sel_clip = None;
        } else {
            self.sel_clip = Some((f.track, f.clip));
        }
        self.sel_extra.clear();
        self.arr_view_start = f.view_start;
        self.arr_view_len = f.view_len;
        self.refresh_playback();
        // Playing on back out in the song, at the same moment.
        if let (Some(pos), Some(c)) = (playing.filter(|_| f.clip < SLOT), self.clip((f.track, f.clip))) {
            let out = c.start + pos.saturating_sub(c.trim);
            self.go_to_frame(out);
        }
    }

    /// Group the selected clips into one clip that holds them, in the same places (⌘G).
    pub(super) fn group_clips(&mut self) {
        let sel = self.selected_clips();
        if sel.is_empty() {
            return;
        }
        self.push_undo("Group Clips");
        let start = sel.iter().map(|&p| self.clip(p).unwrap().start).min().unwrap();
        let first_track = sel[0].0;
        // One inner track per source track, keeping their mixer settings.
        let mut inner: Vec<Track> = Vec::new();
        let mut track_of: Vec<(usize, usize)> = Vec::new();
        for &(t, c) in &sel {
            let i = match track_of.iter().find(|(src, _)| *src == t) {
                Some(&(_, i)) => i,
                None => {
                    let mut nt = self.tracks[t].clone();
                    nt.clips.clear();
                    inner.push(nt);
                    track_of.push((t, inner.len() - 1));
                    inner.len() - 1
                }
            };
            let mut clip = self.tracks[t].clips[c].clone();
            clip.start -= start;
            inner[i].clips.push(clip);
        }
        // Remove the originals (highest index first so the others stay valid).
        let mut gone = sel.clone();
        gone.sort_by(|a, b| b.cmp(a));
        for (t, c) in gone {
            self.tracks[t].clips.remove(c);
        }
        let n = (1..).find(|i| !self.tracks.iter().flat_map(|t| &t.clips).any(|c| c.name == format!("Group {i}"))).unwrap();
        let mut g = Clip::new(format!("Group {n}"), Arc::new(Audio { rate: self.rate(), channels: vec![Vec::new()] }), start);
        self.set_group_content(&mut g, inner);
        g.trim = 0;
        g.len = g.audio.frames();
        self.tracks[first_track].clips.push(g);
        self.sel_clip = Some((first_track, self.tracks[first_track].clips.len() - 1));
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Grouped {} clip{} into Group {n} — double-click it to open it", sel.len(), if sel.len() == 1 { "" } else { "s" });
    }

    /// Put a group's clips back onto the tracks where it sits (⇧⌘G).
    pub(super) fn ungroup_clip(&mut self) {
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some_and(|c| c.group.is_some())) else { return };
        self.push_undo("Ungroup");
        let g = self.tracks[t].clips.remove(c);
        let inner = g.group.clone().unwrap();
        for (i, it) in inner.iter().enumerate() {
            let dest = t + i;
            if dest >= self.tracks.len() {
                let mut nt = it.clone();
                nt.clips.clear();
                self.tracks.push(nt);
            }
            for clip in &it.clips {
                // Only what the group showed (its trim) comes back out.
                if clip.end() <= g.trim || clip.start >= g.trim + g.len {
                    continue;
                }
                let mut clip = clip.clone();
                if clip.start < g.trim {
                    let cut = g.trim - clip.start;
                    clip.trim += cut;
                    clip.len -= cut;
                    clip.start = g.trim;
                }
                clip.len = clip.len.min(g.trim + g.len - clip.start);
                clip.start = g.start + clip.start - g.trim;
                self.tracks[dest].clips.push(clip);
            }
        }
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Ungrouped {}", g.name);
    }

    /// Put clip `(t, ci)` down over what's beneath it on its layer, as Live does: the other clips
    /// there are cut away where it sits, so clips on a layer never overlap (stacking is what
    /// layers are for). Returns where the clip ends up, and selects it.
    pub(super) fn place_over(&mut self, (t, ci): (usize, usize)) -> (usize, usize) {
        let c = self.tracks[t].clips.remove(ci);
        self.clear_span(t, (c.start, c.end()));
        self.tracks[t].clips.push(c);
        let p = (t, self.tracks[t].clips.len() - 1);
        self.sel_clip = Some(p);
        self.sel_extra.clear();
        p
    }

    /// Clip View ▸ Pitch: transpose clip `p` to `semis` semitones, keeping its length and place.
    pub(super) fn set_transpose(&mut self, (t, ci): (usize, usize), semis: i32) {
        let Some(c) = self.clip_mut((t, ci)) else { return };
        if c.transpose == semis || c.group.is_some() {
            return;
        }
        let base = c.untransposed.get_or_insert_with(|| c.warp.as_ref().map_or(c.audio.clone(), |w| w.source.clone())).clone();
        let new = if semis == 0 { base } else { Arc::new(effects::pitch_shift(&base, semis as f32)) };
        c.transpose = semis;
        match c.warp.as_mut() {
            Some(w) => {
                w.source = new;
                w.source_peaks = Arc::new(Peaks::build(&w.source));
                let (audio, peaks) = stretched(&w.source, w.bpm / w.tempo, repitch_mode());
                (c.audio, c.peaks) = (audio, peaks);
            }
            None => {
                c.peaks = Arc::new(Peaks::build(&new));
                c.audio = new;
            }
        }
        self.dirty = true;
        self.refresh_playback();
    }

    /// Duplicate the selected clip, placing the copy right after it (as in Ableton).
    pub(super) fn duplicate_clip(&mut self) {
        self.sel_extra.clear();
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some()) else { return };
        self.push_undo("Duplicate Clip");
        let mut copy = self.tracks[t].clips[c].clone();
        copy.start = copy.end();
        self.tracks[t].clips.push(copy);
        self.place_over((t, self.tracks[t].clips.len() - 1));
        self.dirty = true;
        self.refresh_playback();
    }

    /// Warp a newly imported clip if its name gives its tempo (and warping on import is on).
    pub(super) fn auto_warp(&mut self, (t, c): (usize, usize)) {
        if !self.auto_warp {
            return;
        }
        let (tempo, bpb) = (self.tempo, self.beats_per_bar);
        let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) else { return };
        if clip.group.is_some() {
            return;
        }
        if let Some(bpm) = clip_tempo_from_name(&clip.name, clip.audio.frames(), clip.audio.rate, bpb) {
            clip.warp_from(bpm, tempo);
            self.status = format!("{} is {:.2} BPM; warped to {:.2} BPM", clip.name, bpm, tempo);
            self.refresh_playback();
        }
    }

    /// Clip ▸ Warp to Tempo: switch warping on or off for the selected clips. The clip's tempo
    /// comes from its name, or else is guessed as the whole-bar loop closest to the song tempo.
    pub(super) fn toggle_warp(&mut self) {
        let sel = self.selected_clips();
        let Some(first) = sel.first().and_then(|&p| self.clip(p)) else { return };
        let on = first.warp.is_none();
        self.push_undo(if on { "Warp Clip" } else { "Unwarp Clip" });
        let (tempo, bpb) = (self.tempo, self.beats_per_bar);
        let mut done = Vec::new();
        for (t, c) in sel {
            let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) else { continue };
            if clip.group.is_some() {
                continue;
            }
            if !on {
                clip.unwarp();
                done.push(clip.name.clone());
                continue;
            }
            if clip.warp.is_some() {
                continue;
            }
            let (frames, rate) = (clip.audio.frames(), clip.audio.rate);
            let secs = frames as f64 / rate.max(1) as f64;
            let guess = || {
                [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0]
                    .iter()
                    .map(|bars| 60.0 * bpb as f64 * bars / secs)
                    .filter(|bpm| (40.0..=300.0).contains(bpm))
                    .min_by(|a, b| (a / tempo).ln().abs().total_cmp(&(b / tempo).ln().abs()))
            };
            if let Some(bpm) = clip_tempo_from_name(&clip.name, frames, rate, bpb).or_else(guess) {
                clip.warp_from(bpm, tempo);
                done.push(format!("{} ({bpm:.2} BPM)", clip.name));
            }
        }
        self.dirty = true;
        self.refresh_playback();
        self.status = if on { format!("Warped to {:.2} BPM: {}", tempo, done.join(", ")) } else { format!("Back to original speed: {}", done.join(", ")) };
    }




    /// Clip ▸ Set Tempo from Clip: treat the selected clip as a loop of a whole number of bars
    /// and pick the tempo (nearest 120 BPM, between 70 and 180) that fits it exactly.
    pub(super) fn tempo_from_clip(&mut self) {
        let Some(c) = self.sel_clip.and_then(|p| self.clip(p)) else { return };
        // A warped clip knows its tempo.
        if let Some(bpm) = c.warp.as_ref().map(|w| w.bpm) {
            self.push_undo("Set Tempo");
            self.tempo = (bpm * 100.0).round() / 100.0;
            self.dirty = true;
            self.status = format!("Tempo set to {:.2} BPM", self.tempo);
            return;
        }
        let secs = c.len as f64 / self.rate() as f64;
        if secs <= 0.0 {
            return;
        }
        let bpb = self.beats_per_bar as f64;
        let best = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0]
            .iter()
            .map(|bars| (bars, 60.0 * bpb * bars / secs))
            .filter(|(_, bpm)| (70.0..=180.0).contains(bpm))
            .min_by(|a, b| (a.1 - 120.0).abs().total_cmp(&(b.1 - 120.0).abs()));
        match best {
            Some((bars, bpm)) => {
                self.push_undo("Set Tempo");
                self.tempo = (bpm * 100.0).round() / 100.0;
                self.dirty = true;
                self.status = format!("Tempo set to {:.2} BPM ({} bar{} loop)", self.tempo, bars, if *bars == 1.0 { "" } else { "s" });
            }
            None => self.status = "That clip's length doesn't fit a tempo between 70 and 180 BPM".into(),
        }
    }
}
