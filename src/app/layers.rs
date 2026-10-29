//! Layers: only one clip is heard at a time. The topmost layer with a clip at a moment wins,
//! like layers in a picture editor.

use super::*;

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

#[derive(Clone, Copy)]
enum Ramp {
    Full,
    In,
    Out,
}

/// Flatten layers into what is heard. Where a layer cuts in over one that is still sounding, the
/// one cut off fades out over a few ms; where a lower layer is uncovered mid-sound it fades in
/// just before. Clips that start or end on their own keep their edges.
pub(super) fn flatten(layers: &[Layer], rate: u32) -> Audio {
    let segs = segments(layers);
    let frames = segs.last().map_or(0, |s| s.1);
    let channels = layers.iter().map(|l| l.audio.num_channels()).max().unwrap_or(1).max(1);
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
    let mut out = vec![vec![0.0f32; frames]; channels];
    for (li, a, b, ramp) in pieces {
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
