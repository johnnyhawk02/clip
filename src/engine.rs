//! The song as the audio thread plays it: the tracks' clips, mixed live a block at a time (as
//! Live does) instead of rendered to one buffer first. Edits and tempo changes only swap in a
//! new `Song`, which is cheap, so nothing waits on a render.

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use crate::audio::Audio;

/// One clip, ready to read from the audio thread. Positions are song frames at the tempo the
/// song is laid out for.
pub struct ClipSrc {
    /// What to read: the original audio of a re-pitched clip (read at `speed`), or the clip's
    /// own audio otherwise.
    pub audio: Arc<Audio>,
    /// Frames of `audio` per song frame (1 unless re-pitched).
    pub speed: f64,
    pub start: usize,
    pub len: usize,
    pub trim: usize,
    /// The clip's loop, in song frames, when it loops.
    pub looping: Option<(usize, usize)>,
    /// Frames the clip runs to before it holds its last sample (non-looping).
    pub frames: usize,
    /// Fade in and out (crossfades with its neighbours, and edge fades), in song frames.
    pub fades: (usize, usize),
    /// The clip's own volume.
    pub gain: f32,
}

impl ClipSrc {
    fn end(&self) -> usize {
        self.start + self.len
    }

    /// The clip's frame heard `i` song frames in (as `Clip::source`).
    fn source(&self, i: usize) -> usize {
        let p = self.trim + i;
        if let Some((ls, le)) = self.looping {
            if p >= le && le > ls {
                return ls + (p - le) % (le - ls);
            }
        }
        p.min(self.frames.saturating_sub(1))
    }

    /// Its sample on channel `c` at song frame `t` (before fades).
    pub fn sample_at(&self, t: usize, c: usize) -> f32 {
        self.sample(t as f64, c)
    }

    fn sample(&self, t: f64, c: usize) -> f32 {
        let i = t - self.start as f64;
        let k = self.source(i as usize) as f64 + i.fract();
        let a = &self.audio;
        let Some(ch) = a.channels.get(c.min(a.num_channels().saturating_sub(1))) else { return 0.0 };
        interpolate(ch, k * self.speed)
    }

    /// Its fade gain at song frame `t` (equal power, as `apply_crossfade`).
    fn gain(&self, t: usize) -> f32 {
        let i = t - self.start;
        let (fin, fout) = self.fades;
        let mut g = self.gain;
        if i < fin {
            g *= (i as f32 / fin as f32 * FRAC_PI_2).sin();
        }
        let k = self.len - 1 - i;
        if k < fout {
            g *= ((k + 1) as f32 / fout as f32 * FRAC_PI_2).sin();
        }
        g
    }
}

/// Taps either side of the interpolation point.
const HALF: usize = 8;
/// Kernel table entries per frame.
const RES: usize = 512;

/// A Kaiser-windowed sinc kernel, `HALF` taps either side, tabulated at `RES` steps a frame.
fn kernel() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let beta = 7.0f64;
        let i0 = |x: f64| {
            let (mut sum, mut term) = (1.0, 1.0);
            for k in 1..40 {
                term *= (x / 2.0).powi(2) / (k * k) as f64;
                sum += term;
            }
            sum
        };
        (0..=HALF * RES + 1)
            .map(|k| {
                let t = k as f64 / RES as f64;
                let sinc = if t == 0.0 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
                let r = t / HALF as f64;
                let w = if r < 1.0 { i0(beta * (1.0 - r * r).sqrt()) / i0(beta) } else { 0.0 };
                (sinc * w) as f32
            })
            .collect()
    })
}

/// Band-limited (windowed sinc) interpolation of `ch` at fractional frame `x`.
fn interpolate(ch: &[f32], x: f64) -> f32 {
    let n = ch.len();
    if n == 0 || x < 0.0 {
        return 0.0;
    }
    let i = x as usize;
    if i >= n {
        return ch[n - 1];
    }
    let f = x - i as f64;
    if f == 0.0 {
        return ch[i];
    }
    let table = kernel();
    let tap = |d: f64| {
        let p = d.abs() * RES as f64;
        let k = p as usize;
        if k >= HALF * RES {
            return 0.0;
        }
        let w = (p - k as f64) as f32;
        table[k] * (1.0 - w) + table[k + 1] * w
    };
    let mut acc = 0.0f32;
    for j in (i as isize - HALF as isize + 1).max(0)..=(i as isize + HALF as isize).min(n as isize - 1) {
        acc += ch[j as usize] * tap(x - j as f64);
    }
    acc
}

/// The metronome's sound at song frame `t`: a short blip on each beat, higher on the bar's
/// first beat (as Live's).
fn click(t: f64, beat: f64, bpb: u32, rate: u32) -> f32 {
    if beat <= 0.0 {
        return 0.0;
    }
    let k = (t / beat).floor();
    let off = (t - k * beat) / rate as f64;
    if off > 0.03 {
        return 0.0;
    }
    let hz = if (k as i64).rem_euclid(bpb.max(1) as i64) == 0 { 1600.0 } else { 1000.0 };
    ((2.0 * std::f64::consts::PI * hz * off).sin() * (-off / 0.006).exp() * 0.35) as f32
}

/// One layer: its volume and its clips in time order.
pub struct LayerSrc {
    pub gain: f32,
    /// Left and right gains from the track's pan.
    pub pan: (f32, f32),
    pub clips: Vec<ClipSrc>,
    /// Which track it is.
    pub track: usize,
    /// Loudest it's been since the meters last looked (f32 bits).
    pub peak: std::sync::atomic::AtomicU32,
}

impl LayerSrc {
    pub fn new(track: usize, gain: f32, pan: (f32, f32), clips: Vec<ClipSrc>) -> LayerSrc {
        LayerSrc { gain, pan, clips, track, peak: std::sync::atomic::AtomicU32::new(0) }
    }
}

/// The whole song for the audio thread.
pub struct Song {
    /// The heard tracks, mixed.
    pub layers: Vec<LayerSrc>,
    pub end: usize,
    pub rate: u32,
    pub master: f32,
    /// The metronome, when on: a beat's length in song frames and the beats in a bar.
    pub click: Option<(f64, u32)>,
}

impl Song {
    /// Mix song frames `from`, `from + step`, … into `out` (stereo pairs).
    pub fn render(&self, from: f64, step: f64, out: &mut [[f32; 2]]) {
        let to = from + step * out.len() as f64 + 1.0;
        // Only the clips this block touches.
        let clips: Vec<Vec<&ClipSrc>> = self
            .layers
            .iter()
            .map(|l| l.clips.iter().filter(|c| (c.start as f64) < to && (c.end() as f64) > from).collect())
            .collect();
        let mut peaks = vec![0.0f32; self.layers.len()];
        for (k, frame) in out.iter_mut().enumerate() {
            let t = from + step * k as f64;
            let ti = t as usize;
            *frame = [0.0, 0.0];
            for (li, layer) in self.layers.iter().enumerate() {
                let w = layer.gain;
                let (pl, pr) = layer.pan;
                let mut lr = [0.0f32; 2];
                for c in &clips[li] {
                    if ti < c.start || ti >= c.end() {
                        continue;
                    }
                    let g = w * c.gain(ti);
                    lr[0] += c.sample(t, 0) * g * pl;
                    lr[1] += c.sample(t, 1) * g * pr;
                }
                frame[0] += lr[0];
                frame[1] += lr[1];
                peaks[li] = peaks[li].max(lr[0].abs()).max(lr[1].abs());
            }
            frame[0] *= self.master;
            frame[1] *= self.master;
            if let Some((beat, bpb)) = self.click {
                let c = click(t, beat, bpb, self.rate);
                frame[0] += c;
                frame[1] += c;
            }
        }
        // Each track's meter, as heard (before the master volume).
        for (layer, p) in self.layers.iter().zip(peaks) {
            let old = f32::from_bits(layer.peak.load(std::sync::atomic::Ordering::Relaxed));
            if p > old {
                layer.peak.store(p.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Each heard track's loudest level since the last look, as (track, level), resetting them.
    pub fn take_track_peaks(&self) -> Vec<(usize, f32)> {
        self.layers.iter().map(|l| (l.track, f32::from_bits(l.peak.swap(0, std::sync::atomic::Ordering::Relaxed)))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metronome_clicks_on_beats_higher_on_the_bar() {
        // A beat every 22050 frames (120 BPM at 44.1 kHz), 4 to a bar.
        let song = Song { layers: Vec::new(), end: 200_000, rate: 44100, master: 1.0, click: Some((22050.0, 4)) };
        let mut out = vec![[0.0; 2]; 400];
        song.render(22050.0, 1.0, &mut out);
        assert!(out.iter().any(|f| f[0].abs() > 0.05), "a click on beat 2");
        song.render(30000.0, 1.0, &mut out);
        assert!(out.iter().all(|f| f[0] == 0.0), "silence between beats");
    }

    #[test]
    fn repitched_clip_reads_its_source_faster() {
        // A ramp 0, 1, 2… read at speed 2: song frame t hears source frame 2t.
        let audio = Arc::new(Audio { rate: 1000, channels: vec![(0..100).map(|i| i as f32).collect()] });
        let clip = ClipSrc { audio, speed: 2.0, start: 10, len: 40, trim: 0, looping: None, frames: 50, fades: (0, 0), gain: 1.0 };
        let song = Song { layers: vec![LayerSrc::new(0, 1.0, (1.0, 1.0), vec![clip])], end: 50, rate: 1000, master: 1.0, click: None };
        let mut out = vec![[0.0; 2]; 4];
        song.render(20.0, 1.0, &mut out);
        assert_eq!(out.iter().map(|f| f[0]).collect::<Vec<_>>(), vec![20.0, 22.0, 24.0, 26.0]);
        song.render(5.0, 1.0, &mut out);
        assert_eq!(out[0], [0.0, 0.0], "silence before the clip");
    }
}
