//! Chopping: splitting under the pointer, Slice at Grid, Reverse, Mangle and BBCut.

use super::*;

/// Ways the mangler rearranges a run of slices.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Mangle {
    Shuffle,
    ReverseOrder,
    SwapPairs,
    Stutter,
    RandomReverse,
    DropOut,
    /// Breakbeat cutting in the style of Nick Collins' BBCut, in one of several rhythms.
    Cut(CutStyle),
}

/// Rhythms the BBCut-style cutter knows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum CutStyle {
    Jungle,
    House,
    HipHop,
    Dembow,
    Footwork,
    Trap,
    Breakcore,
    Halftime,
}

/// How a cut style behaves. Lengths are in units of `unit` beats.
pub(super) struct CutRules {
    pub(super) unit: f64,
    /// Block sizes with their weights (when there's no fixed pattern).
    pub(super) sizes: &'static [(usize, f64)],
    /// Block sizes repeated through each bar instead (e.g. 3+3+2).
    pub(super) pattern: &'static [usize],
    /// Chance a block plays another part of the loop.
    pub(super) jump: f64,
    /// Chance a block becomes a roll; with `fills`, only a bar's last block can.
    pub(super) roll: f64,
    pub(super) fills: bool,
    /// First repeat lengths to pick from, how each stage of the roll shrinks (1 = steady), shortest repeat.
    pub(super) roll_pieces: &'static [f64],
    pub(super) roll_ratio: f64,
    pub(super) min_piece: f64,
    /// Chance a block plays backwards.
    pub(super) reverse: f64,
}

impl CutStyle {
    fn rules(self) -> CutRules {
        let base = CutRules { unit: 0.5, sizes: &[], pattern: &[], jump: 0.3, roll: 0.2, fills: false, roll_pieces: &[1.0], roll_ratio: 0.5, min_piece: 0.25, reverse: 0.0 };
        match self {
            // Eighths in 3+3+2-ish blocks, jumps and accelerating rolls.
            CutStyle::Jungle => CutRules { sizes: &[(3, 0.6), (2, 0.3), (4, 0.1)], jump: 0.4, roll: 0.3, roll_pieces: &[1.0, 0.5], ..base },
            // Straight quarter-note blocks, few jumps, a roll as a fill at the end of a bar.
            CutStyle::House => CutRules { pattern: &[2], jump: 0.15, roll: 0.5, fills: true, roll_pieces: &[0.5], ..base },
            // Beat and two-beat chops shuffled around, the odd repeated chop.
            CutStyle::HipHop => CutRules { sizes: &[(2, 0.5), (4, 0.4), (1, 0.1)], jump: 0.5, roll: 0.1, roll_pieces: &[1.0], roll_ratio: 1.0, min_piece: 1.0, reverse: 0.05, ..base },
            // Sixteenths in the 3+3+2 tresillo of reggaeton.
            CutStyle::Dembow => CutRules { unit: 0.25, pattern: &[3, 3, 2], jump: 0.2, roll: 0.15, roll_pieces: &[1.0], min_piece: 0.5, ..base },
            // Sixteenths in threes against the four, busy rolls.
            CutStyle::Footwork => CutRules { unit: 0.25, pattern: &[3, 3, 3, 3, 2, 2], jump: 0.35, roll: 0.35, roll_pieces: &[1.0], min_piece: 0.5, ..base },
            // Sixteenth blocks with steady triplet rolls, like trap hats.
            CutStyle::Trap => CutRules { unit: 0.25, sizes: &[(4, 0.5), (2, 0.3), (8, 0.2)], jump: 0.25, roll: 0.45, roll_pieces: &[2.0 / 3.0, 1.0 / 3.0], roll_ratio: 1.0, min_piece: 1.0 / 3.0, ..base },
            // Everything, everywhere: small blocks, lots of jumps, rolls and reversals.
            CutStyle::Breakcore => CutRules { unit: 0.25, sizes: &[(1, 0.3), (2, 0.35), (3, 0.2), (4, 0.15)], jump: 0.7, roll: 0.45, min_piece: 0.25, reverse: 0.15, ..base },
            // Long half-time blocks, sparse fills.
            CutStyle::Halftime => CutRules { sizes: &[(4, 0.6), (2, 0.3), (6, 0.1)], jump: 0.3, roll: 0.4, fills: true, reverse: 0.05, ..base },
        }
    }
}

impl App {
    /// Split the selected clip (or the clip under the playhead on the active track) at the playhead.
    pub(super) fn split_clip(&mut self) {
        self.sel_extra.clear();
        // With the pointer over a clip, cut it there; otherwise at the playhead.
        if let Some((t, c, f)) = self.arr_hover.filter(|&(t, c, f)| self.view == View::Arrangement && self.clip((t, c)).is_some_and(|cl| f > cl.start && f < cl.end())) {
            self.active = t;
            return self.split_clip_at((t, c), f);
        }
        let at = self.arr_cursor;
        let pos = self.sel_clip.filter(|&p| self.clip(p).is_some_and(|c| at > c.start && at < c.end())).or_else(|| {
            let t = self.active;
            self.tracks.get(t)?.clips.iter().position(|c| at > c.start && at < c.end()).map(|c| (t, c))
        });
        let Some((t, c)) = pos else {
            self.status = "Put the playhead inside a clip to split it".into();
            return;
        };
        self.split_clip_at((t, c), at);
    }

    /// Clip ▸ Reverse Clip: play the selected clips backwards. The audio underneath is reversed
    /// too, so trimmed edges can still be dragged back out (a loop or group becomes plain audio).
    pub(super) fn reverse_clips(&mut self) {
        let sel = self.selected_clips();
        if sel.is_empty() {
            return;
        }
        self.push_undo("Reverse Clip");
        for (t, c) in sel {
            if let Some(clip) = self.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
                reverse_clip(clip);
            }
        }
        self.dirty = true;
        self.refresh_playback();
        self.status = "Reversed".into();
    }

    /// Clip ▸ Slice at Grid: cut the selected clips at every grid line inside them (every beat
    /// when the grid is off), so the pieces can be moved around.
    pub(super) fn slice_clip(&mut self) {
        let sel = self.selected_clips();
        if sel.is_empty() {
            return;
        }
        self.push_undo("Slice Clip");
        let mut pieces = 0;
        for p in sel {
            pieces += self.slice_raw(p);
        }
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Sliced into {pieces} pieces");
    }

    /// Slice one clip at the grid without undo or refresh; returns how many pieces it made.
    /// The first piece keeps index `c`; the others go on the end of the track.
    fn slice_raw(&mut self, (t, c): (usize, usize)) -> usize {
        // A chosen grid size slices at that size; with the grid off or automatic, every beat.
        let step = match GRID[self.grid.min(GRID.len() - 1)].1 {
            -2.0 => self.bar_frames(),
            b if b > 0.0 => self.beat_frames() * b,
            _ => self.beat_frames(),
        }
        .max(1.0);
        {
            let Some(clip) = self.clip((t, c)) else { return 0 };
            let (start, end) = (clip.start, clip.end());
            // Cut from the right so the left-hand piece keeps index `c`.
            let first = (start as f64 / step).floor() as i64 + 1;
            let cuts: Vec<usize> = (first..).map(|k| (k as f64 * step).round() as usize).take_while(|&f| f < end).filter(|&f| f > start).collect();
            // No slivers: a clip a few samples longer than its bars doesn't get a tiny last piece.
            let min = (step / 8.0) as usize;
            let cuts: Vec<usize> = cuts.into_iter().filter(|&f| f - start >= min && end - f >= min).collect();
            for &f in cuts.iter().rev() {
                self.split_raw((t, c), f);
            }
            cuts.len() + 1
        }
    }

    /// Mangle ▸ BBCut: re-cut the selection (per track) as a breakbeat cutter does. Each bar is
    /// split into blocks of eighths (mostly 3+3+2), each block plays either its own spot or another
    /// spot of the loop, and some blocks become stutter rolls of shorter and shorter repeats.
    fn bbcut(&mut self, sel: Vec<(usize, usize)>, style: CutStyle) {
        self.last_mangle = Some(Mangle::Cut(style));
        let rules = style.rules();
        let mut rng = self.mangle_seed;
        let mut rand = move || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        };
        let unit = self.beat_frames() * rules.unit;
        let bar_units = ((self.beats_per_bar as f64 / rules.unit).round() as usize).max(2);
        let min_size = rules.pattern.iter().chain(rules.sizes.iter().map(|s| &s.0)).copied().min().unwrap_or(1);
        let mut tracks: Vec<usize> = sel.iter().map(|p| p.0).collect();
        tracks.dedup();
        let mut new_sel = Vec::new();
        for t in tracks {
            let idx: Vec<usize> = sel.iter().filter(|p| p.0 == t).map(|p| p.1).collect();
            let picked: Vec<Clip> = idx.iter().map(|&i| self.tracks[t].clips[i].clone()).collect();
            let lo = picked.iter().map(|c| c.start).min().unwrap_or(0);
            let hi = picked.iter().map(|c| c.end()).max().unwrap_or(0);
            let name = picked.first().map_or("Cut".to_string(), |c| c.name.clone());
            // The selection as one piece of audio (and backwards), which every cut reads from.
            let mut tmp = Track::new("tmp");
            tmp.clips = picked
                .into_iter()
                .map(|mut c| {
                    c.start -= lo;
                    c
                })
                .collect();
            let src = self.render_track(&tmp);
            if src.frames() == 0 {
                continue;
            }
            let total_units = (((hi - lo) as f64 / unit).round() as usize).max(1);
            let rev = Arc::new(Audio { rate: src.rate, channels: src.channels.iter().map(|c| c.iter().rev().copied().collect()).collect() });
            let src = Arc::new(src);
            let (peaks, rev_peaks) = (Arc::new(Peaks::build(&src)), Arc::new(Peaks::build(&rev)));
            let frame = |u: f64| (u * unit).round() as usize;
            // Plan the cuts: (position, length, source position, backwards), in units.
            let mut cuts: Vec<(f64, f64, f64, bool)> = Vec::new();
            let mut bar = 0;
            while bar * bar_units < total_units {
                let bar_end = ((bar + 1) * bar_units).min(total_units);
                let mut pos = bar * bar_units;
                let mut k = 0;
                while pos < bar_end {
                    let left = bar_end - pos;
                    let mut size = if !rules.pattern.is_empty() {
                        rules.pattern[k % rules.pattern.len()]
                    } else {
                        let total: f64 = rules.sizes.iter().map(|s| s.1).sum();
                        let mut r = rand() * total;
                        rules.sizes.iter().find(|s| {
                            r -= s.1;
                            r <= 0.0
                        }).map_or(rules.sizes[0].0, |s| s.0)
                    };
                    // Don't leave a scrap smaller than the smallest block at the end of the bar.
                    if size >= left || left - size < min_size {
                        size = left;
                    }
                    k += 1;
                    let last = pos + size >= bar_end;
                    let from = if rand() >= rules.jump || total_units <= size {
                        pos
                    } else {
                        (rand() * (total_units - size + 1) as f64) as usize
                    };
                    let backwards = rand() < rules.reverse;
                    if (!rules.fills || last) && rand() < rules.roll && size >= 1 {
                        // A roll: repeats of the block's start, shrinking stage by stage.
                        let mut p = pos as f64;
                        let end = (pos + size) as f64;
                        let mut piece = rules.roll_pieces[(rand() * rules.roll_pieces.len() as f64) as usize % rules.roll_pieces.len()];
                        while p < end - 1e-9 {
                            let reps = if rules.roll_ratio >= 1.0 { usize::MAX } else { ((end - p) / piece / 2.0).ceil().max(1.0) as usize };
                            for _ in 0..reps {
                                if p >= end - 1e-9 {
                                    break;
                                }
                                let l = piece.min(end - p);
                                cuts.push((p, l, from as f64, backwards));
                                p += l;
                            }
                            piece = (piece * rules.roll_ratio).max(rules.min_piece);
                        }
                    } else {
                        cuts.push((pos as f64, size as f64, from as f64, backwards));
                    }
                    pos += size;
                }
                bar += 1;
            }
            // Swap the selection for the cuts.
            let mut desc = idx.clone();
            desc.sort_unstable_by(|a, b| b.cmp(a));
            for i in desc {
                self.tracks[t].clips.remove(i);
            }
            let n_src = src.frames();
            for (p, l, from, backwards) in cuts {
                let start = lo + frame(p);
                let f0 = frame(from).min(n_src.saturating_sub(1));
                let n = frame(p + l).saturating_sub(frame(p)).min(n_src - f0).min(hi.saturating_sub(start));
                if n == 0 {
                    continue;
                }
                // Backwards: the same stretch of audio, read from the reversed copy.
                let (audio, pk, trim) = if backwards { (rev.clone(), rev_peaks.clone(), n_src - (f0 + n)) } else { (src.clone(), peaks.clone(), f0) };
                let mut c = Clip::new(name.clone(), audio, start);
                c.peaks = pk;
                c.trim = trim;
                c.len = n;
                self.tracks[t].clips.push(c);
                new_sel.push((t, self.tracks[t].clips.len() - 1));
            }
        }
        self.mangle_seed = self.mangle_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
        self.sel_clip = new_sel.first().copied();
        self.sel_extra = new_sel.iter().skip(1).copied().collect();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("BBCut {style:?}: {} cuts (⌘Y cuts again)", new_sel.len());
    }

    /// The span of time Mangle works on: the time selection (the blue one), or else the
    /// arrangement loop when Loop is on.
    pub(super) fn loop_span(&self) -> Option<Sel> {
        if let Some((range, _)) = self.time_sel.filter(|_| self.view == View::Arrangement) {
            return Some(range);
        }
        self.arr_loop.filter(|&(s, e)| self.looping && e > s)
    }

    /// The layers a span Mangle works on: those of the time selection, or of the selected
    /// clips, or the active one.
    fn loop_tracks(&self) -> Vec<usize> {
        if let Some((_, (t0, t1))) = self.time_sel.filter(|_| self.view == View::Arrangement) {
            return (t0..=t1.min(self.tracks.len().saturating_sub(1))).collect();
        }
        let mut tracks: Vec<usize> = self.selected_clips().iter().map(|p| p.0).collect();
        if tracks.is_empty() {
            tracks.push(self.active);
        }
        tracks.sort_unstable();
        tracks.dedup();
        tracks.retain(|&t| t < self.tracks.len());
        tracks
    }

    /// Cut the clips on the loop tracks at the loop's edges, slice what's inside at the grid,
    /// and return those pieces.
    fn cut_loop_span(&mut self, (ls, le): Sel) -> Vec<(usize, usize)> {
        let mut pieces = Vec::new();
        for t in self.loop_tracks() {
            for at in [ls, le] {
                for c in 0..self.tracks[t].clips.len() {
                    let cl = &self.tracks[t].clips[c];
                    if at > cl.start && at < cl.end() {
                        self.split_raw((t, c), at);
                    }
                }
            }
            let inside = |cl: &Clip| cl.start >= ls && cl.end() <= le;
            let whole: Vec<usize> = (0..self.tracks[t].clips.len()).filter(|&c| inside(&self.tracks[t].clips[c])).collect();
            for c in whole {
                self.slice_raw((t, c));
            }
            pieces.extend((0..self.tracks[t].clips.len()).filter(|&c| inside(&self.tracks[t].clips[c])).map(|c| (t, c)));
        }
        pieces
    }

    /// Clip ▸ Mangle: rearrange the selected clips in the slots they sit in (a single clip is
    /// sliced at the grid first). Each track's clips are mangled among themselves. With Loop on,
    /// it works on the loop span instead: clips are cut at its edges and sliced at the grid, and
    /// the pieces inside are mangled.
    pub(super) fn mangle(&mut self, how: Mangle) {
        let span = self.loop_span();
        let mut sel = self.selected_clips();
        if let Some((ls, le)) = span {
            let touches = self.loop_tracks().iter().any(|&t| self.tracks[t].clips.iter().any(|c| c.start < le && c.end() > ls));
            if !touches {
                self.status = "Nothing inside the selection to mangle".into();
                return;
            }
        } else if sel.is_empty() {
            return;
        }
        self.push_undo("Mangle");
        if let Some(span) = span {
            sel = self.cut_loop_span(span);
        }
        if let Mangle::Cut(style) = how {
            return self.bbcut(sel, style);
        }
        if span.is_none() && sel.len() == 1 {
            let (t, c) = sel[0];
            let before = self.tracks[t].clips.len();
            self.slice_raw((t, c));
            sel = std::iter::once((t, c)).chain((before..self.tracks[t].clips.len()).map(|i| (t, i))).collect();
        }
        self.last_mangle = Some(how);
        let mut rng = self.mangle_seed;
        let mut rand = move || {
            // xorshift64*: plenty for shuffling slices.
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut new_sel = Vec::new();
        let tracks: Vec<usize> = {
            let mut v: Vec<usize> = sel.iter().map(|p| p.0).collect();
            v.dedup();
            v
        };
        for t in tracks {
            let mut idx: Vec<usize> = sel.iter().filter(|p| p.0 == t).map(|p| p.1).collect();
            idx.sort_by_key(|&i| self.tracks[t].clips[i].start);
            let clips: Vec<Clip> = idx.iter().map(|&i| self.tracks[t].clips[i].clone()).collect();
            let n = clips.len();
            // Each slot: where a clip started, and how long until the next one.
            let slots: Vec<(usize, usize)> = (0..n)
                .map(|i| {
                    let s = clips[i].start;
                    let len = clips.get(i + 1).map(|d| d.start.saturating_sub(s)).filter(|&l| l > 0).unwrap_or(clips[i].len);
                    (s, len)
                })
                .collect();
            let mut out: Vec<Option<Clip>> = clips.iter().cloned().map(Some).collect();
            match how {
                Mangle::Shuffle => {
                    let mut order: Vec<usize> = (0..n).collect();
                    for _ in 0..8 {
                        for i in (1..n).rev() {
                            let j = (rand() * (i + 1) as f64) as usize;
                            order.swap(i, j.min(i));
                        }
                        if n < 2 || order.iter().enumerate().any(|(i, &o)| i != o) {
                            break;
                        }
                    }
                    out = order.iter().map(|&o| Some(clips[o].clone())).collect();
                }
                Mangle::ReverseOrder => out.reverse(),
                Mangle::SwapPairs => {
                    for i in (0..n.saturating_sub(1)).step_by(2) {
                        out.swap(i, i + 1);
                    }
                }
                Mangle::Stutter => {
                    // Some slices repeat the one before (a stutter); at least one does.
                    let pick: Vec<bool> = (0..n).map(|i| i > 0 && rand() < 0.35).collect();
                    let forced = if n > 1 && !pick.contains(&true) { 1 + (rand() * (n - 1) as f64) as usize } else { 0 };
                    for i in 1..n {
                        if pick[i] || i == forced {
                            out[i] = out[i - 1].clone();
                        }
                    }
                }
                Mangle::RandomReverse => {
                    let mut pick: Vec<bool> = (0..n).map(|_| rand() < 0.35).collect();
                    if !pick.contains(&true) {
                        pick[(rand() * n as f64) as usize % n] = true;
                    }
                    for (o, p) in out.iter_mut().zip(pick) {
                        if let (Some(c), true) = (o.as_mut(), p) {
                            reverse_clip(c);
                        }
                    }
                }
                Mangle::Cut(_) => {}
                Mangle::DropOut => {
                    let mut drop: Vec<bool> = (0..n).map(|_| rand() < 0.25).collect();
                    if n > 1 && !drop.contains(&true) {
                        drop[(rand() * n as f64) as usize % n] = true;
                    }
                    if !drop.contains(&false) {
                        drop[0] = false;
                    }
                    for (o, d) in out.iter_mut().zip(drop) {
                        if d {
                            *o = None;
                        }
                    }
                }
            }
            // Take the originals out (highest index first), then put the results in the slots.
            let mut desc = idx.clone();
            desc.sort_unstable_by(|a, b| b.cmp(a));
            for i in desc {
                self.tracks[t].clips.remove(i);
            }
            for (clip, (start, len)) in out.into_iter().zip(slots) {
                let Some(mut clip) = clip else { continue };
                clip.start = start;
                clip.len = if clip.looping { len } else { clip.len.min(len) };
                self.tracks[t].clips.push(clip);
                new_sel.push((t, self.tracks[t].clips.len() - 1));
            }
        }
        self.mangle_seed = self.mangle_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
        self.sel_clip = new_sel.first().copied();
        self.sel_extra = new_sel.iter().skip(1).copied().collect();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Mangled: {how:?} (⌘Y mangles again)");
    }

    /// Cut clip `(t, c)` in two at timeline frame `at`, selecting the right-hand piece.
    pub(super) fn split_clip_at(&mut self, (t, c): (usize, usize), at: usize) {
        if !self.clip((t, c)).is_some_and(|cl| at > cl.start && at < cl.end()) {
            return;
        }
        self.sel_extra.clear();
        self.push_undo("Split Clip");
        self.split_raw((t, c), at);
        self.sel_clip = Some((t, self.tracks[t].clips.len() - 1));
        self.dirty = true;
        self.refresh_playback();
    }

    /// Cut a clip in two without undo or refresh; the right piece goes on the end of the track.
    pub(super) fn split_raw(&mut self, (t, c): (usize, usize), at: usize) {
        let left = &mut self.tracks[t].clips[c];
        let cut = at - left.start;
        let mut right = left.clone();
        left.len = cut;
        left.fade_out = 0;
        right.fade_in = 0;
        right.start = at;
        right.trim += cut;
        right.len -= cut;
        self.tracks[t].clips.push(right);
    }
}
