//! The song as the audio thread plays it: the tracks' clips, mixed live a block at a time (as
//! Live does) instead of rendered to one buffer first. Edits and tempo changes only swap in a
//! new `Song`, which is cheap, so nothing waits on a render.

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use crate::audio::Audio;
use crate::devices::{self, Device};

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

/// Samples over which a delay line moves to a new delay (a crossfade, so no click).
const COMP_FADE: usize = 1024;

/// Delay compensation's delay line, as Live's: a track's sound held back by however much it
/// needs to line up with the most-delayed track. When that changes (an effect with latency
/// added, removed or switched off), it crossfades from the old delay to the new.
#[derive(Default)]
pub struct Comp {
    buf: Vec<[f32; 2]>,
    w: usize,
    cur: usize,
    from: usize,
    fade: usize,
}

impl Comp {
    /// Delay `buf` (in place) by `delay` frames.
    pub fn process(&mut self, buf: &mut [[f32; 2]], delay: usize) {
        let need = (delay + 1).next_power_of_two().max(8192);
        if self.buf.len() < need {
            // New (or grown, losing what it held): straight to the delay, nothing to fade from.
            self.buf = vec![[0.0; 2]; need];
            (self.w, self.cur, self.fade) = (0, delay, 0);
        }
        if delay != self.cur {
            (self.from, self.cur, self.fade) = (self.cur, delay, COMP_FADE);
        }
        let mask = self.buf.len() - 1;
        for f in buf.iter_mut() {
            self.buf[self.w] = *f;
            let a = self.buf[(self.w + mask + 1 - self.cur) & mask];
            *f = if self.fade > 0 {
                let b = self.buf[(self.w + mask + 1 - self.from) & mask];
                let x = self.fade as f32 / COMP_FADE as f32;
                self.fade -= 1;
                [a[0] * (1.0 - x) + b[0] * x, a[1] * (1.0 - x) + b[1] * x]
            } else {
                a
            };
            self.w = (self.w + 1) & mask;
        }
    }
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
    /// Its effects in order, each with the layer (index in `Song::layers`) it listens to.
    pub devices: Vec<(Device, Option<usize>)>,
    /// The effects' running state (filters, envelopes), carried from block to block.
    pub device_state: std::sync::Mutex<Vec<Option<devices::State>>>,
    /// How much goes to each return track (post-fader, 0..1).
    pub sends: Vec<f32>,
    /// Its delay compensation (carried from song to song, as the effects' state is).
    pub comp: std::sync::Mutex<Comp>,
}

/// Return track `r` counts as track `RETURN + r` (for meters and carrying effects over).
pub const RETURN: usize = 1 << 20;

impl LayerSrc {
    /// How many samples late its effects make it.
    pub fn latency(&self) -> usize {
        self.devices.iter().map(|d| d.0.latency()).sum()
    }

    pub fn new(track: usize, gain: f32, pan: (f32, f32), clips: Vec<ClipSrc>) -> LayerSrc {
        LayerSrc { gain, pan, clips, track, peak: std::sync::atomic::AtomicU32::new(0), devices: Vec::new(), device_state: std::sync::Mutex::new(Vec::new()), sends: Vec::new(), comp: Default::default() }
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
    /// Return tracks: fed by the layers' sends, then their effects, into the mix.
    pub returns: Vec<LayerSrc>,
    /// A beat's length in song frames (for tempo-synced effects).
    pub beat: f64,
    /// Delay compensation (Live's Options ▸ Delay Compensation): on, every track is held back
    /// to line up with the most-delayed one (`comp`, the largest latency of any track's effects,
    /// heard or not, so muting doesn't move things), and the tracks' mix to line up with the
    /// most-delayed return track (`ret_comp`).
    pub pdc: bool,
    pub comp: usize,
    pub ret_comp: usize,
    /// The tracks' mix's delay line, for `ret_comp`.
    pub bus: std::sync::Mutex<Comp>,
}

/// Run `layer`'s effects on `buf`; `dry` holds every layer's sound (for sidechains).
fn run_devices(layers: &[LayerSrc], li: Option<usize>, layer: &LayerSrc, buf: &mut [[f32; 2]], dry: &[Vec<[f32; 2]>], rate: u32, beat: f64, pos: f64) {
    if layer.devices.is_empty() {
        return;
    }
    let mut state = layer.device_state.lock().unwrap();
    state.resize_with(layer.devices.len(), || None);
    for ((d, src), st) in layer.devices.iter().zip(state.iter_mut()) {
        if !d.on() {
            continue;
        }
        let side: Option<Vec<[f32; 2]>> = src.filter(|&s| Some(s) != li && s < dry.len()).map(|s| {
            let g = layers[s].gain;
            dry[s].iter().map(|f| [f[0] * g, f[1] * g]).collect()
        });
        st.get_or_insert_with(|| devices::State::new(d, rate)).process(d, buf, side.as_deref(), beat, pos);
    }
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
        // Each layer's own sound first (its clips, before volume and pan), so a Spacer can duck
        // one by another.
        let n = out.len();
        let mut bufs = vec![vec![[0.0f32; 2]; n]; self.layers.len()];
        for (k, ..) in out.iter().enumerate() {
            let t = from + step * k as f64;
            let ti = t as usize;
            for (li, buf) in bufs.iter_mut().enumerate() {
                for c in &clips[li] {
                    if ti < c.start || ti >= c.end() {
                        continue;
                    }
                    let g = c.gain(ti);
                    buf[k][0] += c.sample(t, 0) * g;
                    buf[k][1] += c.sample(t, 1) * g;
                }
            }
        }
        // Each layer's effects, in order. One that listens to another track (a Spacer's
        // sidechain) hears that track after its volume, before its own effects.
        let dry: Vec<Vec<[f32; 2]>> = if self.layers.iter().any(|l| l.devices.iter().any(|d| d.1.is_some())) { bufs.clone() } else { Vec::new() };
        for (li, layer) in self.layers.iter().enumerate() {
            run_devices(&self.layers, Some(li), layer, &mut bufs[li], &dry, self.rate, self.beat, from);
            // Held back to line up with the most-delayed track (its sends too: they're taken after).
            let delay = if self.pdc { self.comp.saturating_sub(layer.latency()) } else { 0 };
            layer.comp.lock().unwrap().process(&mut bufs[li], delay);
        }
        let mut peaks = vec![0.0f32; self.layers.len()];
        let mut rbufs = vec![vec![[0.0f32; 2]; n]; self.returns.len()];
        for (k, frame) in out.iter_mut().enumerate() {
            *frame = [0.0, 0.0];
            for (li, layer) in self.layers.iter().enumerate() {
                let (pl, pr) = layer.pan;
                let lr = [bufs[li][k][0] * layer.gain * pl, bufs[li][k][1] * layer.gain * pr];
                frame[0] += lr[0];
                frame[1] += lr[1];
                peaks[li] = peaks[li].max(lr[0].abs()).max(lr[1].abs());
                // Sends, after the track's volume and pan (as Live's post-fader sends).
                for (rb, &s) in rbufs.iter_mut().zip(&layer.sends) {
                    if s > 0.0 {
                        rb[k][0] += lr[0] * s;
                        rb[k][1] += lr[1] * s;
                    }
                }
            }
        }
        // The tracks' mix waits for the most-delayed return track.
        self.bus.lock().unwrap().process(out, if self.pdc { self.ret_comp } else { 0 });
        // The return tracks: their effects, then into the mix.
        for (ret, rb) in self.returns.iter().zip(rbufs.iter_mut()) {
            run_devices(&self.returns, None, ret, rb, &[], self.rate, self.beat, from);
            let delay = if self.pdc { self.ret_comp.saturating_sub(ret.latency()) } else { 0 };
            ret.comp.lock().unwrap().process(rb, delay);
            let (pl, pr) = ret.pan;
            let mut peak = 0.0f32;
            for (frame, f) in out.iter_mut().zip(rb.iter()) {
                let lr = [f[0] * ret.gain * pl, f[1] * ret.gain * pr];
                frame[0] += lr[0];
                frame[1] += lr[1];
                peak = peak.max(lr[0].abs()).max(lr[1].abs());
            }
            let old = f32::from_bits(ret.peak.load(std::sync::atomic::Ordering::Relaxed));
            if peak > old {
                ret.peak.store(peak.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
        // The metronome is held back with the music, so it stays in time with it.
        let late = self.latency() as f64 * step;
        for (k, frame) in out.iter_mut().enumerate() {
            let t = from + step * k as f64 - late;
            frame[0] *= self.master;
            frame[1] *= self.master;
            if let Some((beat, bpb)) = self.click.filter(|_| t >= 0.0) {
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

    /// How many frames late the mix is heard, with delay compensation on (else 0: tracks with
    /// latency are simply heard late).
    pub fn latency(&self) -> usize {
        if self.pdc { self.comp + self.ret_comp } else { 0 }
    }

    /// Each heard track's loudest level since the last look, as (track, level), resetting them.
    pub fn take_track_peaks(&self) -> Vec<(usize, f32)> {
        self.layers.iter().chain(&self.returns).map(|l| (l.track, f32::from_bits(l.peak.swap(0, std::sync::atomic::Ordering::Relaxed)))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metronome_clicks_on_beats_higher_on_the_bar() {
        // A beat every 22050 frames (120 BPM at 44.1 kHz), 4 to a bar.
        let song = Song { layers: Vec::new(), end: 200_000, rate: 44100, master: 1.0, click: Some((22050.0, 4)), returns: Vec::new(), beat: 22050.0, pdc: true, comp: 0, ret_comp: 0, bus: Default::default() };
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
        let song = Song { layers: vec![LayerSrc::new(0, 1.0, (1.0, 1.0), vec![clip])], end: 50, rate: 1000, master: 1.0, click: None, returns: Vec::new(), beat: 1000.0, pdc: true, comp: 0, ret_comp: 0, bus: Default::default() };
        let mut out = vec![[0.0; 2]; 4];
        song.render(20.0, 1.0, &mut out);
        assert_eq!(out.iter().map(|f| f[0]).collect::<Vec<_>>(), vec![20.0, 22.0, 24.0, 26.0]);
        song.render(5.0, 1.0, &mut out);
        assert_eq!(out[0], [0.0, 0.0], "silence before the clip");
    }

    #[test]
    fn sends_feed_the_return_tracks() {
        // A steady 0.25 clip sent at full to one return: the return adds it again.
        let audio = Arc::new(Audio { rate: 1000, channels: vec![vec![0.25; 100]] });
        let clip = ClipSrc { audio, speed: 1.0, start: 0, len: 100, trim: 0, looping: None, frames: 100, fades: (0, 0), gain: 1.0 };
        let mut layer = LayerSrc::new(0, 1.0, (1.0, 1.0), vec![clip]);
        layer.sends = vec![1.0];
        let ret = LayerSrc::new(RETURN, 1.0, (1.0, 1.0), Vec::new());
        let song = Song { layers: vec![layer], end: 100, rate: 1000, master: 1.0, click: None, returns: vec![ret], beat: 500.0, pdc: true, comp: 0, ret_comp: 0, bus: Default::default() };
        let mut out = vec![[0.0; 2]; 4];
        song.render(10.0, 1.0, &mut out);
        assert_eq!(out[0], [0.5, 0.5]);
    }

    /// A song of an impulse at frame 100 on two tracks: the first hard left through a Tame that
    /// changes nothing but delays it, the second hard right and plain. With `via_return` the
    /// first is plain and the second also sends to a return track with the Tame on it.
    fn impulse_song(pdc: bool, via_return: bool) -> Song {
        let audio = Arc::new(Audio { rate: 44100, channels: vec![(0..1000).map(|i| if i == 100 { 1.0 } else { 0.0 }).collect()] });
        let clip = || ClipSrc { audio: audio.clone(), speed: 1.0, start: 0, len: 1000, trim: 0, looping: None, frames: 1000, fades: (0, 0), gain: 1.0 };
        let tame = Device::Tame(crate::tame::Settings { depth: 0.0, ..Default::default() });
        let mut a = LayerSrc::new(0, 1.0, (1.0, 0.0), vec![clip()]);
        let mut b = LayerSrc::new(1, 1.0, (0.0, 1.0), vec![clip()]);
        let mut returns = Vec::new();
        if via_return {
            b.sends = vec![1.0];
            let mut r = LayerSrc::new(RETURN, 1.0, (1.0, 1.0), Vec::new());
            r.devices = vec![(tame, None)];
            returns.push(r);
        } else {
            a.devices = vec![(tame, None)];
        }
        let (comp, ret_comp) = (a.latency().max(b.latency()), returns.iter().map(|r| r.latency()).max().unwrap_or(0));
        Song { layers: vec![a, b], end: 20000, rate: 44100, master: 1.0, click: None, returns, beat: 22050.0, pdc, comp, ret_comp, bus: Default::default() }
    }

    /// `song`'s first 8192 frames, played in blocks.
    fn play(song: &Song) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0f32; 2]; 8192];
        for (b, chunk) in out.chunks_mut(512).enumerate() {
            song.render((b * 512) as f64, 1.0, chunk);
        }
        out
    }

    /// Where the loudest frame is on each side.
    fn peaks(out: &[[f32; 2]]) -> [usize; 2] {
        let at = |c: usize| (0..out.len()).max_by(|&i, &j| out[i][c].abs().total_cmp(&out[j][c].abs())).unwrap();
        [at(0), at(1)]
    }

    #[test]
    fn delay_compensation_lines_tracks_up() {
        let late = crate::tame::LATENCY;
        assert_eq!(peaks(&play(&impulse_song(true, false))), [100 + late, 100 + late]);
        // Off, as before: the Tame track is simply late.
        assert_eq!(peaks(&play(&impulse_song(false, false))), [100 + late, 100]);
        assert_eq!(impulse_song(true, false).latency(), late);
        assert_eq!(impulse_song(false, false).latency(), 0);
    }

    #[test]
    fn delay_compensation_waits_for_return_tracks() {
        let late = crate::tame::LATENCY;
        // On: the plain track, the second track and its send through the delayed return all
        // land together (the right side's two paths adding up).
        let on = play(&impulse_song(true, true));
        assert!((on[100 + late][0] - 1.0).abs() < 1e-3 && (on[100 + late][1] - 2.0).abs() < 1e-3, "{:?}", on[100 + late]);
        assert!(on[100][0].abs() < 1e-3 && on[100][1].abs() < 1e-3);
        // Off: the return's echo comes late.
        let off = play(&impulse_song(false, true));
        assert!((off[100][1] - 1.0).abs() < 1e-3 && (off[100 + late][1] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn changing_the_delay_does_not_click() {
        // A steady level stays steady while the delay line moves to a new delay.
        let mut c = Comp::default();
        let mut buf = vec![[0.5f32; 2]; 10000];
        c.process(&mut buf[..5000], 0);
        c.process(&mut buf[5000..], 2048);
        assert!(buf.iter().all(|f| (f[0] - 0.5).abs() < 1e-6));
    }
}
