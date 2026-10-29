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

/// What a row of the Loop Pool carries while it is dragged onto the tracks.
#[derive(Clone, Copy)]
pub(super) struct PoolDrag(pub(super) usize);

/// What makes two clips the same loop: the group they show, or the file they play.
fn pool_key(c: &Clip) -> usize {
    match &c.group {
        Some(g) => Arc::as_ptr(g) as *const () as usize,
        None => Arc::as_ptr(c.warp.as_ref().map_or(&c.audio, |w| &w.source)) as *const () as usize,
    }
}

impl App {
    /// The distinct loops and grouped clips in the song (copies and slices of one count once),
    /// at full length.
    fn collect_pool(&self) -> Vec<PoolLoop> {
        let mut seen: Vec<usize> = Vec::new();
        let mut pool = Vec::new();
        for c in self.tracks.iter().flat_map(|t| &t.clips) {
            let key = pool_key(c);
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
        self.show_pool = true;
        self.build_arrangement();
    }

    /// Clip ▸ Swap Section (R): replace each selected clip with a random section of the same
    /// length from the pool, in the same place. Without a pool yet, the loops in the song are it.
    pub(super) fn swap_sections(&mut self) {
        let mut picked: Vec<(usize, usize)> = self.sel_clip.into_iter().chain(self.sel_extra.iter().copied()).collect();
        picked.retain(|&(t, c)| self.tracks.get(t).is_some_and(|tr| tr.clips.get(c).is_some()));
        if picked.is_empty() {
            return;
        }
        if self.pool.is_empty() {
            self.pool = self.collect_pool();
            self.variation = 1;
            self.show_pool = true;
        }
        if self.pool.is_empty() {
            return;
        }
        self.push_undo("Swap Section");
        let mut rng = self.mangle_seed;
        let mut rand = move || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        };
        let bar = self.bar_frames();
        for (t, c) in picked {
            let old = self.tracks[t].clips[c].clone();
            // Try a few times for something that differs from what is there now.
            let mut clip = old.clone();
            for _ in 0..8 {
                let mut cand = self.pool[(rand() * self.pool.len() as f64) as usize % self.pool.len()].clip.clone();
                let frames = cand.audio.frames();
                // Sections start on whole multiples of the clip's length (in bars where it is bar-sized).
                let step = if old.len as f64 >= bar * 0.95 { ((old.len as f64 / bar).round().max(1.0) * bar) as usize } else { old.len };
                let sections = if step == 0 { 0 } else { (frames + step / 20) / step };
                (cand.start, cand.len) = (old.start, old.len);
                if sections == 0 {
                    cand.looping = true;
                } else {
                    cand.trim = ((rand() * sections as f64) as usize % sections) * step;
                    cand.len = cand.len.min(frames - cand.trim.min(frames));
                }
                let same = Arc::ptr_eq(&cand.audio, &old.audio) && cand.trim == old.trim;
                clip = cand;
                if !same {
                    break;
                }
            }
            self.tracks[t].clips[c] = clip;
        }
        self.mangle_seed = self.mangle_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
        self.dirty = true;
        self.refresh_playback();
        self.status = "Swapped for a random section from the pool (R again for another, ⌘Z to undo)".into();
    }

    /// Add the song's loops that aren't in the pool yet.
    pub(super) fn gather_pool(&mut self) {
        let have: Vec<usize> = self.pool.iter().map(|p| pool_key(&p.clip)).collect();
        let new: Vec<PoolLoop> = self.collect_pool().into_iter().filter(|p| !have.contains(&pool_key(&p.clip))).collect();
        self.status = format!("Added {} loop{} to the pool", new.len(), if new.len() == 1 { "" } else { "s" });
        self.pool.extend(new);
    }

    /// Clip ▸ Add to Pool: the selected clips (grouped ones too) as they are, trimmed or not.
    pub(super) fn add_to_pool(&mut self) {
        let mut added = 0;
        for p in self.selected_clips() {
            let Some(c) = self.clip(p) else { continue };
            let mut clip = c.clone();
            clip.start = 0;
            let same = |q: &PoolLoop| pool_key(&q.clip) == pool_key(&clip) && q.clip.trim == clip.trim && q.clip.len == clip.len;
            if !self.pool.iter().any(same) {
                self.pool.push(PoolLoop { clip });
                added += 1;
            }
        }
        self.show_pool = true;
        self.status = match added {
            0 => "Already in the pool".into(),
            1 => "Added to the pool".into(),
            n => format!("Added {n} clips to the pool"),
        };
    }

    /// The Loop Pool side panel: each loop can be dragged onto a track (or below the last one
    /// for a new track), landing on the grid where it's dropped.
    pub(super) fn pool_section(&mut self, ui: &mut egui::Ui) {
        if self.view != View::Arrangement {
            ui.label(egui::RichText::new("Go back to the arrangement to drag loops from the pool.").color(muted()));
            return;
        }
        if self.pool.is_empty() {
            ui.label(egui::RichText::new("Empty. Add the loops in your song (or any clip, grouped ones too, with Clip ▸ Add to Pool), then drag them onto any track.").color(muted()));
        }
        let bar = self.bar_frames();
        let mut remove = None;
        for (i, p) in self.pool.iter().enumerate() {
            ui.horizontal(|ui| {
                let bars = p.clip.len as f64 / bar;
                let text = format!("{}  ·  {} bar{}", p.clip.name, (bars * 10.0).round() / 10.0, if (bars - 1.0).abs() < 0.05 { "" } else { "s" });
                let row = ui.dnd_drag_source(egui::Id::unique(("pool", i)), PoolDrag(i), |ui| {
                    egui::Frame::new().fill(gray(255)).stroke(Stroke::new(1.0, rule())).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.set_width(230.0);
                        ui.add(egui::Label::new(egui::RichText::new(text).color(ink())).truncate().selectable(false));
                    });
                });
                row.response.on_hover_text("Drag onto a track");
                if ui.small_button("×").on_hover_text("Take this loop out of the pool").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.pool.remove(i);
        }
        ui.add_space(8.0);
        if ui.add_enabled(self.tracks.iter().any(|t| !t.clips.is_empty()), Button::new("Add Loops from Song")).clicked() {
            self.gather_pool();
        }
    }

    /// A pool loop dropped on the tracks: place it at `q` (snapped), on a new track below the last.
    pub(super) fn drop_from_pool(&mut self, i: usize, q: Pos2) {
        let (Some(p), Some((track, frame))) = (self.pool.get(i), self.drop_target_at(q)) else { return };
        let mut clip = p.clip.clone();
        clip.start = frame;
        self.push_undo("Add Loop from Pool");
        let t = track.unwrap_or_else(|| {
            let mut t = Track::new(clip.name.clone());
            t.color = auto_color(self.tracks.len());
            self.tracks.push(t);
            self.tracks.len() - 1
        });
        self.status = format!("Added {}", clip.name);
        self.tracks[t].clips.push(clip);
        self.place_over((t, self.tracks[t].clips.len() - 1));
        self.active = t;
        self.dirty = true;
        self.refresh_playback();
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
                    t.color = auto_color(i);
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
