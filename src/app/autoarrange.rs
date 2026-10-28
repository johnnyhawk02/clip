//! Auto Arrange: lay out a whole track from a pool of loops. Each loop is fitted to the song
//! tempo and cut into 8 or 16 bar sections, and the sections are placed on one track per loop
//! following an intro, build, breakdown and outro shape. Every variation number gives the same
//! layout again, so ◀ ▶ can step back to one you liked.

use super::*;

/// How long an arrangement runs.
const SONG_SECS: f64 = 240.0;

/// A loop in the Auto Arrange pool, warped to the song tempo and reset to its full length.
#[derive(Clone)]
pub(super) struct PoolLoop {
    pub(super) clip: Clip,
}

impl App {
    /// The distinct loops in the song (copies and slices of one file count once), at full length.
    fn collect_pool(&self) -> Vec<PoolLoop> {
        let mut seen: Vec<*const Audio> = Vec::new();
        let mut pool = Vec::new();
        for c in self.tracks.iter().flat_map(|t| &t.clips) {
            if c.group.is_some() {
                continue;
            }
            let key = Arc::as_ptr(c.warp.as_ref().map_or(&c.audio, |w| &w.source));
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let mut clip = c.clone();
            (clip.start, clip.trim, clip.len, clip.looping) = (0, 0, clip.audio.frames(), false);
            clip.loop_points = None;
            if clip.warp.is_none() {
                if let Some(bpm) = clip_tempo_from_name(&clip.name, clip.audio.frames(), clip.audio.rate, self.beats_per_bar) {
                    clip.warp_from(bpm, self.tempo);
                }
            }
            pool.push(PoolLoop { clip });
        }
        pool
    }

    /// Clip ▸ Auto Arrange: take the loops in the song as the pool and lay out variation 1.
    pub(super) fn auto_arrange(&mut self) {
        let pool = self.collect_pool();
        if pool.is_empty() {
            self.status = "Auto Arrange needs some loops: drop a few in first".into();
            return;
        }
        self.pool = pool;
        self.variation = 1;
        self.build_arrangement();
    }

    /// ◀ ▶: the previous or next variation of the current pool.
    pub(super) fn step_variation(&mut self, forward: bool) {
        if self.pool.is_empty() {
            return self.auto_arrange();
        }
        self.variation = if forward { self.variation + 1 } else { self.variation.saturating_sub(1).max(1) };
        self.build_arrangement();
    }

    /// Replace the song with variation `self.variation` of the pool.
    pub(super) fn build_arrangement(&mut self) {
        self.push_undo("Auto Arrange");
        let mut rng = (self.variation as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut rand = move || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        };
        let bar = self.bar_frames();
        let sec_bars = self.section_bars as f64;
        let song_bars = SONG_SECS / 60.0 * self.tempo / self.beats_per_bar as f64;
        let phrases = ((song_bars / sec_bars).round() as usize).max(4);
        let n = self.pool.len();
        // How busy each phrase is (0–1): an intro, a build, a breakdown a little past halfway,
        // a second peak and an outro.
        let brk = (phrases as f64 * 0.55).round() as usize;
        let energy: Vec<f64> = (0..phrases)
            .map(|i| {
                let base = match i {
                    0 => 0.2,
                    1 => 0.45,
                    _ if i + 1 == phrases => 0.25,
                    _ if i + 2 == phrases => 0.55,
                    _ if i == brk => 0.3,
                    _ if i == brk + 1 => 0.65,
                    _ => 0.8,
                };
                (base + (rand() - 0.5) * 0.3).clamp(0.05, 1.0)
            })
            .collect();
        let mut tracks: Vec<Track> = self
            .pool
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // Keep the mixer settings of a track that already plays this loop.
                let mut t = self.tracks.iter().find(|t| t.name == p.clip.name).cloned().unwrap_or_else(|| Track::new(p.clip.name.clone()));
                t.clips.clear();
                if !self.tracks.iter().any(|o| o.name == p.clip.name) {
                    t.color = i % TRACK_COLORS.len();
                }
                t
            })
            .collect();
        let mut active: Vec<bool> = vec![false; n];
        let mut last_section: Vec<Option<usize>> = vec![None; n];
        for (pi, &e) in energy.iter().enumerate() {
            let want = ((e * n as f64).round() as usize).clamp(1, n);
            // Change a layer now and then even when the count stays the same.
            if rand() < 0.35 {
                let on: Vec<usize> = (0..n).filter(|&i| active[i]).collect();
                if let Some(&i) = on.get((rand() * on.len() as f64) as usize) {
                    active[i] = false;
                }
            }
            while active.iter().filter(|&&a| a).count() > want {
                let on: Vec<usize> = (0..n).filter(|&i| active[i]).collect();
                active[on[(rand() * on.len() as f64) as usize % on.len()]] = false;
            }
            while active.iter().filter(|&&a| a).count() < want {
                let off: Vec<usize> = (0..n).filter(|&i| !active[i]).collect();
                active[off[(rand() * off.len() as f64) as usize % off.len()]] = true;
            }
            let start = (pi as f64 * sec_bars * bar).round() as usize;
            let end = ((pi + 1) as f64 * sec_bars * bar).round() as usize;
            for li in (0..n).filter(|&i| active[i]) {
                let mut clip = self.pool[li].clip.clone();
                let frames = clip.audio.frames();
                let sections = ((frames as f64 / bar + 0.05) / sec_bars).floor() as usize;
                clip.start = start;
                clip.len = end - start;
                if sections == 0 {
                    // Shorter than a section: loop it to fill.
                    clip.looping = true;
                } else {
                    let s = match last_section[li] {
                        Some(s) if rand() < 0.5 => s,
                        _ => (rand() * sections as f64) as usize % sections,
                    };
                    last_section[li] = Some(s);
                    clip.trim = (s as f64 * sec_bars * bar).round() as usize;
                    clip.len = clip.len.min(frames - clip.trim.min(frames));
                }
                tracks[li].clips.push(clip);
            }
        }
        self.tracks = tracks;
        self.active = 0;
        self.sel_clip = None;
        self.sel_extra.clear();
        self.arr_loop = None;
        self.looping = false;
        self.dirty = true;
        self.arr_fit();
        self.refresh_playback();
        let mins = phrases as f64 * sec_bars * bar / self.rate() as f64 / 60.0;
        self.status = format!("Variation {}: {} loops, {} sections of {} bars, {:.1} min (◀ ▶ for others)", self.variation, n, phrases, self.section_bars, mins);
    }
}
