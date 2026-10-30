//! Tame, a dynamic resonance suppressor: it finds the frequencies that ring out above the rest of
//! the spectrum, moment by moment, and turns down just those.
//!
//! Short-time Fourier analysis (a 2048-point FFT, Hann windows on the way in and out, hops of a
//! quarter frame, overlap-add); both channels share one complex FFT (left real, right imaginary),
//! and since the gains are real and the same at ±f, one inverse FFT brings both back. Per frame:
//! each bin's power (left and right averaged, so the cuts are linked and the image holds) is
//! smoothed in time, taken to dB, and smoothed across frequency (two passes of a box a fixed
//! number of octaves wide, set by Detail) into a spectral envelope. A bin's excess over that
//! envelope goes through a soft-knee gain computer (Giannoulis, Massberg and Reiss, "Digital
//! Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 2012; ratio ∞, so above the
//! knee all the excess goes) to a cut in dB, held to 12 dB, scaled by Depth and faded outside
//! Low–High. The threshold is a little lower around 3.5 kHz, where the ear is most sensitive.
//! Soft takes the threshold relative to the envelope, so it is the same at any level; Hard also
//! holds each cut to how far the bin is over a fixed level (Threshold), so only loud resonances
//! go. Each cut is then spread to its neighbours (the deepest nearby, averaged), so it is a
//! smooth dip as wide as the resonance, and smoothed in time per bin (Attack and Release from
//! Speed). Delta plays only what is taken away.
//!
//! The constants were set by ear and against measurements of a commercial resonance suppressor
//! (its input and output only), on pink noise with resonant peaks, a sweeping resonance, tone
//! bursts and a loop: cuts about a third of an octave wide, about 6 dB on a pure tone at the
//! default Depth, and next to nothing on broadband noise.
//!
//! Latency is one frame (2048 samples); Clip has no delay compensation, so the dry sound is
//! delayed to match for Dry/Wet and Delta. Printed offline (`print`), the time smoothing runs
//! forwards and then backwards over the whole sound, so cuts open ahead of a resonance and the
//! result lines up with the input.

use serde::{Deserialize, Serialize};

/// Frame length (FFT size), hop and latency, in samples.
pub const N: usize = 2048;
const HOP: usize = N / 4;
pub const LATENCY: usize = N;
const BINS: usize = N / 2 + 1;
/// The deepest cut, dB.
pub const MAX_CUT: f32 = 12.0;
/// Soft: a bin starts to be cut this far over the envelope (dB), across a knee this wide.
const SOFT_T: f32 = 8.0;
const KNEE: f32 = 6.0;
/// Extra sensitivity in the presence region, dB.
const PRESENCE: f32 = 3.0;
/// The detector's shortest time constant, ms (shorter lets noise flicker into the cuts).
const DET_MS: f32 = 30.0;
/// The cuts are smoothed across this fraction of the envelope's width.
const CUT_SPREAD: f32 = 0.25;
/// Hann² overlapped at a quarter frame sums to 1.5.
const OLA: f32 = 1.0 / 1.5;
/// Points in the cut curve shown on the device, 20 Hz to 20 kHz.
pub const CURVE: usize = 128;

/// Tame's settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// 0..1: how much of each resonance goes.
    pub depth: f32,
    /// 0..1: broad (gentle, wide cuts) to fine (only narrow peaks).
    pub detail: f32,
    /// 0..1: slow to fast cuts.
    pub speed: f32,
    /// Only frequencies between these (Hz) are cut.
    pub low: f32,
    pub high: f32,
    /// Hard: only resonances over `threshold` are cut.
    pub hard: bool,
    /// Hard's level, dB (full-scale sine = 0).
    pub threshold: f32,
    /// Play only what's taken away.
    pub delta: bool,
    /// 0..1: dry against wet.
    pub mix: f32,
    pub on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { depth: 0.5, detail: 0.5, speed: 0.5, low: 60.0, high: 18000.0, hard: false, threshold: -30.0, delta: false, mix: 1.0, on: true }
    }
}

impl Settings {
    /// The envelope's width, octaves: 2 at Detail 0 down to 1/4 at 1.
    fn width(&self) -> f32 {
        2.0 * (1.0f32 / 8.0).powf(self.detail.clamp(0.0, 1.0))
    }

    /// Attack and release, ms: 30/400 when slow to 1/20 when fast.
    fn times(&self) -> (f32, f32) {
        let s = self.speed.clamp(0.0, 1.0);
        (30.0 * (1.0f32 / 30.0).powf(s), 400.0 * (20.0f32 / 400.0).powf(s))
    }
}

/// A radix-2 complex FFT with its twiddles and bit reversal worked out once.
struct Fft {
    tw: Vec<(f32, f32)>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let tw = (0..n / 2).map(|k| {
            let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
            (a.cos() as f32, a.sin() as f32)
        }).collect();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        Fft { tw, rev }
    }

    fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        for i in 0..n {
            let j = self.rev[i];
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.tw[k * step];
                    let (a, b) = (start + k, start + k + len / 2);
                    let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                    re[b] = re[a] - xr;
                    im[b] = im[a] - xi;
                    re[a] += xr;
                    im[a] += xi;
                }
            }
            len <<= 1;
        }
    }

    /// The inverse, scaled by 1/n (the forward transform with real and imaginary swapped).
    fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        self.forward(im, re);
        let s = 1.0 / re.len() as f32;
        re.iter_mut().chain(im.iter_mut()).for_each(|v| *v *= s);
    }
}

/// `v` averaged over a box `oct` octaves wide (at least `min` bins each side) around each bin;
/// `sum` is scratch, one longer than `v`.
fn box_smooth(v: &mut [f32], sum: &mut [f32], oct: f32, min: usize) {
    let half = 2f32.powf(oct / 2.0);
    let n = v.len();
    sum[0] = 0.0;
    for k in 0..n {
        sum[k + 1] = sum[k] + v[k];
    }
    for k in 1..n {
        let lo = ((k as f32 / half) as usize).min(k.saturating_sub(min)).max(1);
        let hi = ((k as f32 * half).ceil() as usize).max(k + min).min(n - 1);
        v[k] = (sum[hi + 1] - sum[lo]) / (hi + 1 - lo) as f32;
    }
    v[0] = v[1];
}

/// Each bin of `v` as the largest within `oct` octaves around it, into `out`.
fn dilate(v: &[f32], out: &mut [f32], oct: f32) {
    let half = 2f32.powf(oct / 2.0);
    for k in 0..v.len() {
        let lo = ((k as f32 / half) as usize).min(k.saturating_sub(1));
        let hi = ((k as f32 * half).ceil() as usize).max(k + 1).min(v.len() - 1);
        out[k] = v[lo..=hi].iter().fold(0.0f32, |m, &c| m.max(c));
    }
}

/// One-pole coefficient for time constant `ms` at one step per hop.
fn coef(ms: f32, rate: f32) -> f32 {
    (-(HOP as f32) / (rate * ms.max(0.1) * 0.001)).exp()
}

/// The analysis shared by playback and printing: one frame in, cuts and the processed frame out.
struct Core {
    rate: f32,
    fft: Fft,
    win: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Each bin's power, smoothed in time (the detector).
    det: Vec<f32>,
    /// Its level, dB, and the envelope across frequency.
    db: Vec<f32>,
    env: Vec<f32>,
    sum: Vec<f32>,
    /// This frame's cuts before time smoothing, dB.
    raw: Vec<f32>,
}

impl Core {
    fn new(rate: u32) -> Core {
        let win = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        Core {
            rate: rate.max(1) as f32,
            fft: Fft::new(N),
            win,
            re: vec![0.0; N],
            im: vec![0.0; N],
            det: vec![0.0; BINS],
            db: vec![-120.0; BINS],
            env: vec![-120.0; BINS],
            sum: vec![0.0; BINS + 1],
            raw: vec![0.0; BINS],
        }
    }

    /// Window frame `l`, `r` (N samples each) into the spectrum.
    fn spectrum(&mut self, l: &[f32], r: &[f32]) {
        for i in 0..N {
            self.re[i] = l[i] * self.win[i];
            self.im[i] = r[i] * self.win[i];
        }
        self.fft.forward(&mut self.re, &mut self.im);
    }

    /// The cut each bin wants this frame, into `raw`.
    fn cuts(&mut self, p: &Settings) {
        let (att, _) = p.times();
        // The detector follows a little slower than the cuts, so noise doesn't flicker into them.
        let a = coef(att.max(DET_MS), self.rate);
        let norm = (4.0 / N as f32).powi(2);
        for k in 0..BINS {
            let nk = (N - k) % N;
            let (re, im) = (&self.re, &self.im);
            // Left and right out of the shared transform.
            let (lr, li) = (0.5 * (re[k] + re[nk]), 0.5 * (im[k] - im[nk]));
            let (rr, ri) = (0.5 * (im[k] + im[nk]), -0.5 * (re[k] - re[nk]));
            let pw = 0.5 * (lr * lr + li * li + rr * rr + ri * ri) * norm;
            self.det[k] = pw + a * (self.det[k] - pw);
            self.db[k] = 10.0 * (self.det[k] + 1e-12).log10();
        }
        // The envelope: the level averaged over `width` octaves (at least 5 bins), twice.
        self.env.copy_from_slice(&self.db);
        for _ in 0..2 {
            box_smooth(&mut self.env, &mut self.sum, p.width(), 2);
        }
        let depth = p.depth.clamp(0.0, 1.0);
        let (lo, hi) = (p.low.max(1.0), p.high.max(p.low.max(1.0)));
        for k in 0..BINS {
            let f = k as f32 * self.rate / N as f32;
            // Full inside Low–High, fading out over half an octave beyond.
            let mut w: f32 = 1.0;
            if f < lo {
                w = if f <= 0.0 { 0.0 } else { (1.0 - 2.0 * (lo / f).log2()).max(0.0) };
            }
            if f > hi {
                w = w.min((1.0 - 2.0 * (f / hi).log2()).max(0.0));
            }
            // More sensitive where the ear is (up to PRESENCE dB less threshold around 3.5 kHz,
            // over a couple of octaves either side), as harshness lives there.
            let o = if f > 0.0 { (f / 3500.0).log2() / 1.5 } else { 9.0 };
            let t = SOFT_T - PRESENCE * (-o * o).exp();
            let mut c = crate::dynamics::excess(self.db[k] - self.env[k], t, KNEE);
            if p.hard {
                c = c.min(crate::dynamics::excess(self.db[k], p.threshold, KNEE));
            }
            self.raw[k] = c.min(MAX_CUT) * depth * w;
        }
        // Spread across frequency (each bin takes the deepest cut within CUT_SPREAD of the
        // envelope's width, then that is averaged over the same), so a cut is a smooth dip as
        // wide as the resonance's skirts rather than single bins jumping about.
        let spread = p.width() * CUT_SPREAD;
        dilate(&self.raw, &mut self.env, spread);
        self.raw.copy_from_slice(&self.env);
        box_smooth(&mut self.raw, &mut self.sum, spread, 1);
    }

    /// Turn the spectrum down by `cut` (dB per bin) and back into a windowed frame: left in
    /// `re`, right in `im`.
    fn synth(&mut self, cut: &[f32]) {
        for k in 0..BINS {
            let g = 10f32.powf(-cut[k] / 20.0);
            self.re[k] *= g;
            self.im[k] *= g;
            if k > 0 && k < N / 2 {
                self.re[N - k] *= g;
                self.im[N - k] *= g;
            }
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        for i in 0..N {
            let w = self.win[i] * OLA;
            self.re[i] *= w;
            self.im[i] *= w;
        }
    }

    /// The cut curve and level at `CURVE` points from 20 Hz to 20 kHz, for the display.
    fn view(&self, cut: &[f32], curve: &mut [f32; CURVE], level: &mut [f32; CURVE]) {
        let bin = |j: usize| ((20.0 * 1000f32.powf(j as f32 / (CURVE - 1) as f32)) * N as f32 / self.rate).round().clamp(1.0, (BINS - 1) as f32) as usize;
        for j in 0..CURVE {
            let (a, b) = (bin(j), bin(j + 1).max(bin(j) + 1).min(BINS));
            curve[j] = cut[a..b].iter().fold(0.0f32, |m, &c| m.max(c));
            level[j] = self.db[a];
        }
    }
}

/// Tame running on a track: a frame's worth of input and output buffered, the cuts carried over.
pub struct Tame {
    core: Core,
    /// The last N input samples per channel; the newest HOP being filled.
    inp: [Vec<f32>; 2],
    /// Overlap-add accumulator, lined up with `inp`.
    acc: [Vec<f32>; 2],
    /// The finished output being played out over this hop.
    out: [Vec<f32>; 2],
    fill: usize,
    /// The dry sound delayed by the latency.
    dry: Vec<[f32; 2]>,
    dpos: usize,
    /// The current cut per bin, dB.
    cut: Vec<f32>,
    /// For the display: the cut and level at `CURVE` points.
    pub curve: [f32; CURVE],
    pub level: [f32; CURVE],
}

impl Tame {
    pub fn new(rate: u32) -> Tame {
        Tame {
            core: Core::new(rate),
            inp: [vec![0.0; N], vec![0.0; N]],
            acc: [vec![0.0; N], vec![0.0; N]],
            out: [vec![0.0; HOP], vec![0.0; HOP]],
            fill: 0,
            dry: vec![[0.0; 2]; LATENCY],
            dpos: 0,
            cut: vec![0.0; BINS],
            curve: [0.0; CURVE],
            level: [-120.0; CURVE],
        }
    }

    /// Process `main` (stereo frames, in place); the output is `LATENCY` frames late.
    pub fn process(&mut self, p: &Settings, main: &mut [[f32; 2]]) {
        let mix = p.mix.clamp(0.0, 1.0);
        for frame in main.iter_mut() {
            for c in 0..2 {
                self.inp[c][N - HOP + self.fill] = frame[c];
            }
            let wet = [self.out[0][self.fill], self.out[1][self.fill]];
            let dry = std::mem::replace(&mut self.dry[self.dpos], *frame);
            self.dpos = (self.dpos + 1) % LATENCY;
            self.fill += 1;
            if self.fill == HOP {
                self.fill = 0;
                self.frame(p);
            }
            *frame = blend(dry, wet, mix, p.delta);
        }
    }

    fn frame(&mut self, p: &Settings) {
        let core = &mut self.core;
        core.spectrum(&self.inp[0], &self.inp[1]);
        core.cuts(p);
        let (att, rel) = p.times();
        let (a, r) = (coef(att, core.rate), coef(rel, core.rate));
        for (c, &raw) in self.cut.iter_mut().zip(&core.raw) {
            let k = if raw > *c { a } else { r };
            *c = raw + k * (*c - raw);
        }
        core.synth(&self.cut);
        for (c, part) in [&core.re, &core.im].into_iter().enumerate() {
            let acc = &mut self.acc[c];
            acc.copy_within(HOP.., 0);
            acc[N - HOP..].fill(0.0);
            for (a, v) in acc.iter_mut().zip(part.iter()) {
                *a += v;
            }
            self.out[c].copy_from_slice(&acc[..HOP]);
            self.inp[c].copy_within(HOP.., 0);
        }
        core.view(&self.cut, &mut self.curve, &mut self.level);
    }
}

/// The output from the dry sound (lined up) and the processed one.
fn blend(dry: [f32; 2], wet: [f32; 2], mix: f32, delta: bool) -> [f32; 2] {
    let out = [dry[0] + mix * (wet[0] - dry[0]), dry[1] + mix * (wet[1] - dry[1])];
    if delta { [dry[0] - out[0], dry[1] - out[1]] } else { out }
}

/// Tame printed into `input` offline, lined up with it (no latency). The cuts are smoothed in
/// time forwards and then backwards, so they open ahead of each resonance (look-ahead) and the
/// smoothing adds no lag.
pub fn print(p: &Settings, rate: u32, input: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = input.len();
    // A frame of silence either side, so every sample is covered by four frames.
    let len = N + n + N;
    let at = |c: usize, i: usize| if i >= N && i < N + n { input[i - N][c] } else { 0.0 };
    let frames = (len - N) / HOP + 1;
    let mut core = Core::new(rate);
    let (mut l, mut r) = (vec![0.0; N], vec![0.0; N]);
    let fill = |l: &mut [f32], r: &mut [f32], s: usize| {
        for i in 0..N {
            l[i] = at(0, s + i);
            r[i] = at(1, s + i);
        }
    };
    // Pass one: every frame's cuts, kept in twentieths of a dB (to keep long sounds small).
    let mut cuts = vec![0u8; frames * BINS];
    for m in 0..frames {
        fill(&mut l, &mut r, m * HOP);
        core.spectrum(&l, &r);
        core.cuts(p);
        for (q, &c) in cuts[m * BINS..(m + 1) * BINS].iter_mut().zip(&core.raw) {
            *q = (c * 20.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    // Forwards, then backwards: attack and release with no lag.
    let (att, rel) = p.times();
    let (a, rk) = (coef(att, core.rate), coef(rel, core.rate));
    let mut state = vec![0.0f32; BINS];
    let smooth = |m: usize, cuts: &mut [u8], state: &mut [f32]| {
        for (q, s) in cuts[m * BINS..(m + 1) * BINS].iter_mut().zip(state.iter_mut()) {
            let raw = *q as f32 / 20.0;
            let k = if raw > *s { a } else { rk };
            *s = raw + k * (*s - raw);
            *q = (*s * 20.0).round() as u8;
        }
    };
    for m in 0..frames {
        smooth(m, &mut cuts, &mut state);
    }
    state.fill(0.0);
    for m in (0..frames).rev() {
        smooth(m, &mut cuts, &mut state);
    }
    // Pass two: apply them and overlap-add.
    let mut y = vec![[0.0f32; 2]; len];
    let mut cut = vec![0.0f32; BINS];
    for m in 0..frames {
        let s = m * HOP;
        fill(&mut l, &mut r, s);
        core.spectrum(&l, &r);
        for (c, &q) in cut.iter_mut().zip(&cuts[m * BINS..(m + 1) * BINS]) {
            *c = q as f32 / 20.0;
        }
        core.synth(&cut);
        for i in 0..N {
            y[s + i][0] += core.re[i];
            y[s + i][1] += core.im[i];
        }
    }
    let mix = p.mix.clamp(0.0, 1.0);
    input.iter().zip(&y[N..N + n]).map(|(&d, &w)| blend(d, w, mix, p.delta)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44100;

    /// White noise at about -30 dB plus a 1 kHz sine ringing well above it.
    fn noisy_peak(n: usize, sine: f32) -> Vec<[f32; 2]> {
        let mut seed = 12345u32;
        (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.05;
                let s = sine * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin();
                [noise + s, noise + s]
            })
            .collect()
    }

    /// Power at bins inside and outside 1 kHz ± 100 Hz, over `x` from `from` on, in dB.
    fn split_power(x: &[[f32; 2]], from: usize) -> (f32, f32) {
        let mut core = Core::new(RATE);
        let (mut inside, mut outside) = (0.0f64, 0.0f64);
        let mut s = from;
        while s + N <= x.len() {
            let l: Vec<f32> = x[s..s + N].iter().map(|f| f[0]).collect();
            core.spectrum(&l, &l);
            for k in 2..BINS {
                let f = k as f32 * RATE as f32 / N as f32;
                let p = (core.re[k] * core.re[k] + core.im[k] * core.im[k]) as f64;
                if (f - 1000.0).abs() < 100.0 { inside += p } else { outside += p }
            }
            s += N;
        }
        (10.0 * inside.log10() as f32, 10.0 * outside.log10() as f32)
    }

    #[test]
    fn cuts_the_resonance_and_leaves_the_noise() {
        let n = RATE as usize * 3;
        let x = noisy_peak(n, 0.5);
        let p = Settings { depth: 1.0, ..Settings::default() };
        let mut y = x.clone();
        Tame::new(RATE).process(&p, &mut y);
        // Compare the dry and wet once the cuts have settled, lined up.
        let skip = RATE as usize;
        let (pi, po) = split_power(&x[skip - LATENCY..n - LATENCY], 0);
        let (wi, wo) = split_power(&y[skip..], 0);
        assert!(pi - wi > 6.0, "peak cut by {} dB", pi - wi);
        assert!((po - wo).abs() < 1.0, "noise changed by {} dB", po - wo);
        // Printed offline too, without the latency.
        let z = print(&p, RATE, &x);
        let (zi, zo) = split_power(&z[skip..], 0);
        let (qi, qo) = split_power(&x[skip..], 0);
        assert!(qi - zi > 6.0, "printed: peak cut by {} dB", qi - zi);
        assert!((qo - zo).abs() < 1.0, "printed: noise changed by {} dB", qo - zo);
    }

    #[test]
    fn silence_stays_silent() {
        let mut y = vec![[0.0f32; 2]; 20000];
        Tame::new(RATE).process(&Settings { depth: 1.0, ..Settings::default() }, &mut y);
        assert!(y.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
        let z = print(&Settings { depth: 1.0, hard: true, ..Settings::default() }, RATE, &vec![[0.0; 2]; 20000]);
        assert!(z.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
    }

    #[test]
    fn depth_zero_is_the_input_delayed() {
        let x = noisy_peak(30000, 0.3);
        let mut y = x.clone();
        // Wet only, so it is the STFT's own reconstruction being checked.
        let p = Settings { depth: 0.0, ..Settings::default() };
        Tame::new(RATE).process(&p, &mut y);
        let worst = (LATENCY..x.len()).map(|i| (y[i][0] - x[i - LATENCY][0]).abs().max((y[i][1] - x[i - LATENCY][1]).abs())).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "off by {worst}");
        let z = print(&p, RATE, &x);
        let worst = x.iter().zip(&z).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "printed: off by {worst}");
    }

    #[test]
    fn delta_is_what_was_taken() {
        let x = noisy_peak(30000, 0.5);
        let p = Settings { depth: 1.0, ..Settings::default() };
        let (mut wet, mut delta) = (x.clone(), x.clone());
        Tame::new(RATE).process(&p, &mut wet);
        Tame::new(RATE).process(&Settings { delta: true, ..p }, &mut delta);
        for i in LATENCY..x.len() {
            assert!((wet[i][0] + delta[i][0] - x[i - LATENCY][0]).abs() < 1e-5);
        }
    }

    #[test]
    fn hard_leaves_quiet_resonances() {
        // A resonance well under Hard's threshold is left; Soft takes it.
        let x: Vec<[f32; 2]> = noisy_peak(RATE as usize * 2, 0.5).iter().map(|f| [f[0] * 0.01, f[1] * 0.01]).collect();
        let skip = RATE as usize;
        let (pi, _) = split_power(&x[skip..], 0);
        let soft = print(&Settings { depth: 1.0, ..Settings::default() }, RATE, &x);
        let hard = print(&Settings { depth: 1.0, hard: true, threshold: -30.0, ..Settings::default() }, RATE, &x);
        assert!(pi - split_power(&soft[skip..], 0).0 > 6.0);
        assert!((pi - split_power(&hard[skip..], 0).0).abs() < 0.5);
    }
}

