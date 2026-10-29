//! Layers: only one clip is heard at a time. The topmost layer with a clip at a moment wins,
//! like layers in a picture editor.

use super::*;
use crate::engine;

/// Length of the fade where one layer cuts off another mid-sound (5 ms at 44.1 kHz).
const SWITCH_FADE_SECS: f64 = 0.005;

/// One layer ready to flatten: its rendered audio, its volume, and the spans its clips cover.
pub(super) struct Layer {
    pub(super) audio: Audio,
    pub(super) gain: f32,
    pub(super) spans: Vec<(usize, usize)>,
}

impl Layer {
    pub(super) fn of(t: &Track, audio: Audio) -> Layer {
        let spans = merge(t.clips.iter().filter(|c| c.len > 0).map(|c| (c.start, c.end())).collect());
        Layer { audio, gain: t.gain, spans }
    }

    fn covers(&self, t: usize) -> bool {
        self.spans.iter().any(|&(s, e)| s <= t && t < e)
    }

    /// Where the span holding `t` ends (`t` itself if none does).
    fn cover_end(&self, t: usize) -> usize {
        self.spans.iter().find(|&&(s, e)| s <= t && t < e).map_or(t, |&(_, e)| e)
    }

    /// Where the span holding `t - 1` starts (`t` itself if none does).
    fn cover_start(&self, t: usize) -> usize {
        if t == 0 {
            return 0;
        }
        self.spans.iter().find(|&&(s, e)| s < t && t <= e).map_or(t, |&(s, _)| s)
    }
}

/// Which layer is heard over each stretch of time: (start, end, layer) in time order, with
/// gaps left out. Layer 0 is the top one.
pub(super) fn segments(layers: &[Layer]) -> Vec<(usize, usize, usize)> {
    let mut edges: Vec<usize> = layers.iter().flat_map(|l| l.spans.iter().flat_map(|&(s, e)| [s, e])).collect();
    edges.sort();
    edges.dedup();
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for w in edges.windows(2) {
        let (s, e) = (w[0], w[1]);
        let Some(o) = layers.iter().position(|l| l.covers(s)) else { continue };
        match out.last_mut() {
            Some(last) if last.1 == s && last.2 == o => last.1 = e,
            _ => out.push((s, e, o)),
        }
    }
    out
}

/// The heard pieces of the visible layers as one layer of clips: each clip cut down to the
/// stretches where it wins. Where a piece is cut off mid-sound it runs on under its neighbour so
/// the two crossfade instead of clicking: `resume` frames where it comes back in after a layer
/// above ends, `cut` frames where a layer above cuts in (kept short, so that clip's attack stays
/// sharp). Both are 0 for a Live Set, where clips on a track can't overlap.
pub(super) fn flatten_clips(tracks: &[Track], resume: usize, cut: usize) -> Track {
    let visible: Vec<&Track> = tracks.iter().filter(|t| !t.mute).collect();
    let layers: Vec<Layer> = visible.iter().map(|t| Layer::of(t, Audio { rate: 1, channels: Vec::new() })).collect();
    let mut out = Track::new("Arrangement");
    out.color = visible.first().map_or(0, |t| t.color);
    for (s, e, o) in segments(&layers) {
        for c in &visible[o].clips {
            let (mut a, mut b) = (s.max(c.start), e.min(c.end()));
            if b > a {
                a -= resume.min(a - c.start);
                b += cut.min(c.end() - b);
                let mut piece = c.clone();
                piece.trim += a - c.start;
                piece.start = a;
                piece.len = b - a;
                out.clips.push(piece);
            }
        }
    }
    out
}

/// The song as the live engine plays it: the visible layers' clips, read straight from their
/// audio (a re-pitched clip from its original, at the speed the tempo needs), and the plan of
/// which layer is heard when.
pub(super) fn song(tracks: &[Track], rate: u32, end: usize, master: f32) -> engine::Song {
    let visible: Vec<&Track> = tracks.iter().filter(|t| !t.mute).collect();
    let plan: Vec<Layer> = visible.iter().map(|t| Layer::of(t, Audio { rate, channels: Vec::new() })).collect();
    let layers = visible
        .iter()
        .map(|t| engine::LayerSrc {
            gain: t.gain,
            clips: t.clips.iter().zip(crossfades(&t.clips)).filter(|(c, _)| c.len > 0).map(|(c, cross)| clip_src(c, cross, rate)).collect(),
        })
        .collect();
    engine::Song { layers, pieces: pieces(&plan, rate), end, rate, master }
}

fn clip_src(c: &Clip, cross: (usize, usize), rate: u32) -> engine::ClipSrc {
    let (audio, speed, frames) = match c.warp.as_ref().filter(|_| repitch_mode()) {
        // Re-pitched live from the original, so a new tempo needs no re-render.
        Some(w) => (w.source.clone(), w.tempo / w.bpm, (w.source.frames() as f64 * w.bpm / w.tempo).round() as usize),
        None => (c.audio.clone(), 1.0, c.audio.frames()),
    };
    let looping = c.looping.then(|| c.loop_points.map(|(s, e)| (s.min(frames), e.min(frames))).filter(|(s, e)| e > s).unwrap_or((0, frames)));
    let mut src = engine::ClipSrc { audio, speed, start: c.start, len: c.len, trim: c.trim, looping, frames, fades: (0, 0), gain: c.gain };
    let level = |t: usize| (0..2).map(|ch| src.sample_at(t, ch).abs()).fold(0.0f32, f32::max);
    let (first, last) = (level(c.start), level(c.end() - 1));
    src.fades = edge_fades_for(first, last, c.len, rate, cross);
    src
}

/// For each layer, the spans hidden by visible layers above it (merged, in time order).
pub(super) fn covered_above(tracks: &[Track]) -> Vec<Vec<(usize, usize)>> {
    let mut above: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    for t in tracks {
        out.push(merge(above.clone()));
        if !t.mute {
            above.extend(t.clips.iter().filter(|c| c.len > 0).map(|c| (c.start, c.end())));
        }
    }
    out
}

fn merge(mut v: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    v.sort();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (s, e) in v {
        match spans.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => spans.push((s, e)),
        }
    }
    spans
}

/// Frames of the fade where one layer cuts off another, at `rate`.
pub(super) fn switch_fade(rate: u32) -> usize {
    ((rate as f64 * SWITCH_FADE_SECS).round() as usize).max(1)
}

pub(super) use crate::engine::Ramp;

/// Which layer is heard over each stretch, as (layer, from, to, ramp): the plan `flatten` mixes
/// by, and that the live engine plays from. Where a layer cuts in over one that is still
/// sounding, the one cut off fades out over a few ms; where a lower layer is uncovered
/// mid-sound it fades in just before. Clips that start or end on their own keep their edges.
pub(super) fn pieces(layers: &[Layer], rate: u32) -> Vec<(usize, usize, usize, Ramp)> {
    let segs = segments(layers);
    let fade = switch_fade(rate);
    let mut pieces: Vec<(usize, usize, usize, Ramp)> = segs.iter().map(|&(s, e, o)| (o, s, e, Ramp::Full)).collect();
    for k in 1..segs.len() {
        let (ps, s, p) = segs[k - 1];
        let (s2, e, o) = segs[k];
        if s != s2 {
            continue;
        }
        let p_goes_on = layers[p].covers(s);
        let o_was_on = layers[o].covers(s - 1);
        if p_goes_on && o_was_on {
            let h = (fade / 2).min((s - ps) / 2).min((e - s) / 2).max(1);
            pieces[k - 1].2 = s - h;
            pieces[k].1 = s + h;
            pieces.push((p, s - h, s + h, Ramp::Out));
            pieces.push((o, s - h, s + h, Ramp::In));
        } else if p_goes_on {
            let n = fade.min(layers[p].cover_end(s) - s).min(e - s);
            pieces.push((p, s, s + n, Ramp::Out));
        } else if o_was_on {
            let n = fade.min(s - layers[o].cover_start(s)).min(s - ps);
            pieces.push((o, s - n, s, Ramp::In));
        }
    }
    pieces
}

/// Flatten layers into what is heard, by the plan from `pieces` (for exports and groups; playback
/// mixes the same plan live, see `engine`).
pub(super) fn flatten(layers: &[Layer], rate: u32) -> Audio {
    let frames = segments(layers).last().map_or(0, |s| s.1);
    let channels = layers.iter().map(|l| l.audio.num_channels()).max().unwrap_or(1).max(1);
    let mut out = vec![vec![0.0f32; frames]; channels];
    for (li, a, b, ramp) in pieces(layers, rate) {
        let l = &layers[li];
        if b <= a || l.audio.num_channels() == 0 || l.gain == 0.0 {
            continue;
        }
        let n = (b - a) as f32;
        for (ci, o) in out.iter_mut().enumerate() {
            let src = &l.audio.channels[ci.min(l.audio.num_channels() - 1)];
            for t in a..b.min(src.len()) {
                let x = (t - a) as f32 + 0.5;
                let w = match ramp {
                    Ramp::Full => 1.0,
                    Ramp::In => (x / n * std::f32::consts::FRAC_PI_2).sin(),
                    Ramp::Out => (x / n * std::f32::consts::FRAC_PI_2).cos(),
                };
                o[t] += src[t] * l.gain * w;
            }
        }
    }
    Audio { rate, channels: out }
}

impl App {
    pub(super) fn set_active(&mut self, i: usize) {
        if i < self.tracks.len() {
            self.active = i;
        }
    }

    pub(super) fn add_track(&mut self, track: Track, label: &'static str) {
        self.push_undo(label);
        let at = (self.active + 1).min(self.tracks.len());
        self.tracks.insert(at, track);
        self.active = at;
        self.dirty = true;
        self.refresh_playback();
    }

    pub(super) fn new_track(&mut self) {
        let n = (1..).find(|i| !self.tracks.iter().any(|t| t.name == format!("Track {i}"))).unwrap();
        self.add_track(Track::new(format!("Track {n}")), "New Track");
        self.status = "New empty track — paste, record or import clips into it".into();
    }

    pub(super) fn duplicate_track(&mut self) {
        let Some(t) = self.tracks.get(self.active) else { return };
        let mut t = t.clone();
        t.name = format!("{} copy", t.name);
        self.add_track(t, "Duplicate Track");
    }

    pub(super) fn delete_track(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        self.push_undo("Delete Track");
        self.tracks.remove(self.active);
        self.active = self.active.min(self.tracks.len() - 1);
        self.sel_clip = None;
        self.dirty = true;
        self.refresh_playback();
    }

    /// Move the active layer above or below its neighbour, changing which one is heard.
    pub(super) fn move_layer(&mut self, up: bool) {
        let a = self.active;
        let Some(b) = (if up { a.checked_sub(1) } else { Some(a + 1) }).filter(|&b| b < self.tracks.len()) else { return };
        self.push_undo(if up { "Move Layer Up" } else { "Move Layer Down" });
        self.tracks.swap(a, b);
        let swap = |t: usize| if t == a { b } else if t == b { a } else { t };
        self.sel_clip = self.sel_clip.map(|(t, c)| (swap(t), c));
        self.sel_extra = self.sel_extra.iter().map(|&(t, c)| (swap(t), c)).collect();
        self.active = b;
        self.dirty = true;
        self.refresh_playback();
    }

    /// Merge Down needs a layer below, and both layers showing.
    pub(super) fn can_merge_down(&self) -> bool {
        let a = self.active;
        a + 1 < self.tracks.len() && !self.tracks[a].mute && !self.tracks[a + 1].mute
    }

    /// Merge the active layer into the one below: one layer of clips that sounds like the two
    /// did, each clip cut down to what was heard. The lower layer's name and settings stay.
    pub(super) fn merge_down(&mut self) {
        if !self.can_merge_down() {
            return;
        }
        let a = self.active;
        self.push_undo("Merge Down");
        let (top, low) = (self.tracks[a].clone(), self.tracks[a + 1].clone());
        let gains_differ = (top.gain - low.gain).abs() > 0.001;
        let fade = layers::switch_fade(self.rate());
        let flat = layers::flatten_clips(&[top, low.clone()], fade, (fade / 5).max(1));
        let mut merged = low;
        merged.clips = flat.clips;
        self.tracks[a + 1] = merged;
        self.tracks.remove(a);
        self.active = a;
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = if gains_differ {
            "Merged down. The two layers had different volumes; the merged one keeps the lower layer's".into()
        } else {
            "Merged down — ⌘Z to undo".into()
        };
    }

    pub(super) fn mix_down(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        let mix = Arc::new(self.mixdown());
        self.push_undo("Mix Down");
        let mut t = Track::new("Mix");
        t.clips.push(Clip::new("Mix", mix, 0));
        self.tracks = vec![t];
        self.active = 0;
        self.sel_clip = Some((0, 0));
        self.dirty = true;
        self.refresh_playback();
        self.status = "Mixed all tracks into one — ⌘Z to undo".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(v: f32, frames: usize, spans: Vec<(usize, usize)>) -> Layer {
        let mut a = vec![0.0; frames];
        for &(s, e) in &spans {
            a[s..e].iter_mut().for_each(|x| *x = v);
        }
        Layer { audio: Audio { rate: 1000, channels: vec![a] }, gain: 1.0, spans }
    }

    #[test]
    fn top_layer_wins() {
        let layers = [layer(1.0, 100, vec![(20, 40)]), layer(0.5, 100, vec![(0, 100)])];
        assert_eq!(segments(&layers), vec![(0, 20, 1), (20, 40, 0), (40, 100, 1)]);
        let out = flatten(&layers, 1000);
        let c = &out.channels[0];
        assert_eq!(c[10], 0.5);
        assert!(c[19] == 0.5 && c[20] > 1.4, "the top clip starts with its own edge");
        assert_eq!(c[30], 1.0, "only the top layer is heard");
        assert_eq!(c[70], 0.5);
        // The lower layer fades out as the top one cuts in, and back in before it ends.
        assert!(c[22] > 1.0 && c[22] < 1.5);
        assert!(c[38] > 1.0 && c[38] < 1.5);
        assert_eq!(out.frames(), 100);
    }

    #[test]
    fn flattened_clips_are_cut_to_what_is_heard() {
        let audio = Arc::new(Audio { rate: 1000, channels: vec![vec![0.0; 100]] });
        let mut top = Track::new("top");
        top.clips.push(Clip::new("a", audio.clone(), 20));
        top.clips[0].len = 20;
        let mut low = Track::new("low");
        low.clips.push(Clip::new("b", audio, 0));
        let flat = flatten_clips(&[top, low], 0, 0);
        let got: Vec<(usize, usize, usize)> = flat.clips.iter().map(|c| (c.start, c.len, c.trim)).collect();
        assert_eq!(got, vec![(0, 20, 0), (20, 20, 0), (40, 60, 40)]);
    }

    #[test]
    fn gaps_are_silent_and_butted_clips_cut() {
        let layers = [layer(1.0, 100, vec![(10, 30)]), layer(0.5, 100, vec![(30, 60)])];
        let c = flatten(&layers, 1000).channels.remove(0);
        assert_eq!(c[5], 0.0);
        assert_eq!(c[29], 1.0);
        assert_eq!(c[30], 0.5);
    }
}
