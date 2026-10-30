//! Chorus: bucket-brigade (BBD) choruses, after Holters and Parker, "A Combined Model for a
//! Bucket Brigade Device and its Input and Output Filters" (DAFx 2018), plus a tape chorus and a
//! clean digital one after Dattorro, "Effect Design Part 2" (JAES 1997).
//!
//! The BBD: the analog anti-aliasing filter in front of it and the reconstruction filter after it
//! are each split into parallel first-order complex sections (partial fractions of a Butterworth
//! low-pass), run at the audio rate. The BBD clock runs on its own, at any rate and modulated
//! freely: at each input tick the input filter's state is carried from the last audio sample to
//! the tick instant (exactly, for an input held over the sample) and its sum is stored in the
//! bucket chain; at each output tick (half a clock later) the step the chain's output takes is
//! fed into the output filter, each section given its exact step response up to the next audio
//! sample. So the clock never has to line up with the audio samples, and sweeping it (which is
//! how a BBD chorus modulates) is smooth, as in the circuit.
//!
//! The modes are presets of that engine: a Juno-60/106's chorus (two BBDs on one triangle LFO,
//! the second inverted), Roland's Dimension D (two BBDs in antiphase on a trapezoid LFO, each
//! cross-mixed into both sides so the pitch movement cancels in mono), a string machine's
//! ensemble (three BBDs, LFOs 120° apart, each a slow and a fast sine), a tape chorus (a short
//! delay wandered by the Tape device's wow and flutter, through its hysteresis) and Dattorro's
//! white chorus (an all-pass interpolated delay read, with his feedback from a fixed tap).

use serde::{Deserialize, Serialize};

use crate::tape;

/// The modes, in the order of `Settings::mode`.
pub const MODES: [&str; 10] = ["Juno I", "Juno II", "Juno I+II", "Dimension 1", "Dimension 2", "Dimension 3", "Dimension 4", "Ensemble", "Tape", "Clean"];

const TAPE: u8 = 8;
const CLEAN: u8 = 9;

/// Chorus settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Index into `MODES`.
    pub mode: u8,
    /// LFO rate, Hz (choosing a mode sets its own).
    pub rate: f32,
    /// Depth against the mode's own: 1 is the original.
    pub amount: f32,
    /// 0..1: dry against wet. At 0.5 both are heard in full, as the hardware mixes them.
    pub mix: f32,
    /// 0..1: the BBDs' hiss (Tape: the tape's).
    pub noise: f32,
    /// 0..1, Tape mode: drive into the tape.
    pub drive: f32,
    pub on: bool,
    /// Folded down to its title in the device view (Live: double-click the title bar).
    #[serde(default)]
    pub folded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { mode: 0, rate: preset(0).rate, amount: 1.0, mix: 0.5, noise: 0.0, drive: 0.0, on: true, folded: false }
    }
}

impl Settings {
    /// Switch to mode `m`, at its own rate.
    pub fn set_mode(&mut self, m: u8) {
        self.mode = m.min(CLEAN);
        self.rate = preset(self.mode).rate;
    }
}

/// A mode's original settings.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    /// LFO rate, Hz (Ensemble: the slow one; the fast one is ten times it).
    pub rate: f32,
    /// Delay in the middle of the sweep, and how far it goes each way, ms.
    pub centre_ms: f32,
    pub depth_ms: f32,
    /// The wet signal's level (Dimension 4's boost).
    pub wet: f32,
}

/// Mode `m`'s original settings. Juno: the rates commonly given for the Juno-60's chorus
/// (I 0.513 Hz, II 0.863 Hz, I+II 9.75 Hz), its delay sweeping 1.66-5.35 ms in I and II and
/// 3.3-3.7 ms in I+II. Dimension: 0.25 Hz for 1 and 2 and 0.5 Hz for 3 and 4, depth low in 1,
/// less in 3 and 4 to make up for the faster rate, and 4 louder (as measured by users; Roland
/// gave no figures).
pub fn preset(m: u8) -> Preset {
    let p = |rate, centre_ms, depth_ms| Preset { rate, centre_ms, depth_ms, wet: 1.0 };
    match m {
        0 => p(0.513, 3.505, 1.845),
        1 => p(0.863, 3.505, 1.845),
        2 => p(9.75, 3.5, 0.2),
        3 => p(0.25, 3.2, 0.35),
        4 => p(0.25, 3.2, 0.7),
        5 => p(0.5, 3.2, 0.5),
        6 => Preset { wet: 1.4, ..p(0.5, 3.2, 0.5) },
        7 => p(0.6, 6.0, 1.5),
        // Tape: the wow's rate; depth is the Tape device's wow at full (about ±1.6 ms).
        TAPE => p(0.55, 6.0, 1.0),
        _ => p(0.8, 12.0, 3.0),
    }
}

/// The BBD circuit for a mode: buckets in the chain, and the filters' cutoff, Hz.
fn circuit(m: u8) -> (usize, f64) {
    match m {
        // MN3009 (256 stages), filters around 9 kHz.
        0..=2 => (256, 9000.0),
        3..=6 => (256, 10000.0),
        // Longer chain (MN3007-style 1024 stages would be darker still): 512 buckets.
        _ => (512, 8000.0),
    }
}

/// Which circuit (so the BBDs are rebuilt only when it changes).
fn circuit_id(m: u8) -> u8 {
    match m {
        0..=2 => 0,
        3..=6 => 1,
        _ => 2,
    }
}

/// The fast LFO's rate and depth against the slow one's, in Ensemble.
const FAST_RATE: f64 = 10.0;
const FAST_DEPTH: f64 = 1.0 / 6.0;
/// The shortest delay a BBD is swept to, s (its clock tops out there).
const MIN_DELAY: f64 = 0.0008;
/// The Butterworth filters' order.
const ORDER: usize = 5;
/// Full-scale BBD hiss.
const HISS: f32 = 0.008;
/// Dimension: how much of each BBD crosses into the other side.
const CROSS: f64 = 0.4;
/// Dattorro's white chorus: blend, and feedback from the fixed tap (its negative).
const BLEND: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    fn new(re: f64, im: f64) -> C {
        C { re, im }
    }
    fn mul(self, o: C) -> C {
        C::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
    fn div(self, o: C) -> C {
        let d = o.re * o.re + o.im * o.im;
        C::new((self.re * o.re + self.im * o.im) / d, (self.im * o.re - self.re * o.im) / d)
    }
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
    fn scale(self, k: f64) -> C {
        C::new(self.re * k, self.im * k)
    }
    fn exp(self) -> C {
        let m = self.re.exp();
        C::new(m * self.im.cos(), m * self.im.sin())
    }
}

/// An `ORDER`th-order Butterworth low-pass at `fc` Hz as parallel first-order sections:
/// H(s) = Σ r_k / (s − p_k), as (poles, residues).
fn butterworth(fc: f64) -> ([C; ORDER], [C; ORDER]) {
    let wc = std::f64::consts::TAU * fc;
    let n = ORDER as f64;
    let mut p = [C::default(); ORDER];
    for (k, pk) in p.iter_mut().enumerate() {
        let th = std::f64::consts::PI * (2.0 * k as f64 + n + 1.0) / (2.0 * n);
        *pk = C::new(wc * th.cos(), wc * th.sin());
    }
    let mut r = [C::default(); ORDER];
    for k in 0..ORDER {
        let mut den = C::new(1.0, 0.0);
        for j in 0..ORDER {
            if j != k {
                den = den.mul(p[k].sub(p[j]));
            }
        }
        r[k] = C::new(wc.powi(ORDER as i32), 0.0).div(den);
    }
    (p, r)
}

/// One bucket-brigade delay with its input and output filters.
struct Bbd {
    ts: f64,
    /// Input filter: poles, residue over pole, pole per audio sample, state.
    p_in: [C; ORDER],
    rp_in: [C; ORDER],
    g_in: [C; ORDER],
    x: [C; ORDER],
    /// Output filter likewise, and its gain at DC (on the held output).
    p_out: [C; ORDER],
    rp_out: [C; ORDER],
    g_out: [C; ORDER],
    y: [C; ORDER],
    h0: f64,
    /// The buckets (one per two stages: a sample moves a bucket each clock).
    buf: Vec<f64>,
    wi: usize,
    /// Audio samples to the next clock tick, and whether it's an input tick.
    next: f64,
    input_tick: bool,
    /// What the chain's output holds.
    held: f64,
    seed: u32,
}

impl Bbd {
    fn new(fs: f64, stages: usize, fc: f64, seed: u32) -> Bbd {
        let ts = 1.0 / fs;
        let (p, r) = butterworth(fc);
        let rp: [C; ORDER] = std::array::from_fn(|k| r[k].div(p[k]));
        let g: [C; ORDER] = std::array::from_fn(|k| p[k].scale(ts).exp());
        let h0 = -rp.iter().fold(C::default(), |a, &v| a.add(v)).re;
        Bbd {
            ts,
            p_in: p,
            rp_in: rp,
            g_in: g,
            x: [C::default(); ORDER],
            p_out: p,
            rp_out: rp,
            g_out: g,
            y: [C::default(); ORDER],
            h0,
            buf: vec![0.0; stages / 2],
            wi: 0,
            next: 0.0,
            input_tick: true,
            held: 0.0,
            seed,
        }
    }

    /// One audio sample in, one out, with the chain's delay at `d` seconds (which sets the
    /// clock: the chain's buckets in `d`) and hiss `hiss` added in the chain.
    fn step(&mut self, u: f64, d: f64, hiss: f64) -> f64 {
        let half = d.max(MIN_DELAY) / self.ts / (2.0 * self.buf.len() as f64);
        let before = self.x;
        // Input filter, over the sample, with the input held (so exact for a held input).
        for k in 0..ORDER {
            self.x[k] = self.g_in[k].mul(self.x[k]).add(self.rp_in[k].mul(self.g_in[k].sub(C::new(1.0, 0.0))).scale(u));
        }
        for k in 0..ORDER {
            self.y[k] = self.g_out[k].mul(self.y[k]);
        }
        while self.next <= 1.0 {
            let tau = self.next;
            if self.input_tick {
                // The input filter's output at the tick, into the chain.
                let mut v = 0.0;
                for k in 0..ORDER {
                    let e = self.p_in[k].scale(tau * self.ts).exp();
                    v += e.mul(before[k]).add(self.rp_in[k].mul(e.sub(C::new(1.0, 0.0))).scale(u)).re;
                }
                if hiss > 0.0 {
                    self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    v += hiss * ((self.seed >> 8) as f64 / (1u32 << 24) as f64 * 2.0 - 1.0);
                }
                self.buf[self.wi] = v;
                self.wi = (self.wi + 1) % self.buf.len();
            } else {
                // The oldest bucket reaches the output: a step, through the output filter to
                // the end of this sample.
                let v = self.buf[self.wi];
                let delta = v - self.held;
                self.held = v;
                for k in 0..ORDER {
                    let e = self.p_out[k].scale((1.0 - tau) * self.ts).exp();
                    self.y[k] = self.y[k].add(self.rp_out[k].mul(e).scale(delta));
                }
            }
            self.input_tick = !self.input_tick;
            self.next += half;
        }
        self.next -= 1.0;
        self.y.iter().fold(0.0, |a, v| a + v.re) + self.h0 * self.held
    }
}

pub struct Chorus {
    fs: f32,
    /// LFO phase, 0..1.
    ph: f64,
    bbd: Vec<Bbd>,
    built: u8,
    /// Amount, smoothed (a jump in an interpolated read clicks).
    amount: f32,
    /// Tape and Clean: a short delay line per side, its write index and the all-pass's states.
    line: Vec<[f32; 2]>,
    li: usize,
    ap: [f32; 2],
    sat: tape::Saturator,
    wobble: tape::Wobble,
    seed: u32,
    n: usize,
}

/// A triangle, -1..1, for phase 0..1.
fn tri(p: f64) -> f64 {
    let p = p.rem_euclid(1.0);
    1.0 - 4.0 * (p - 0.5).abs()
}

impl Chorus {
    pub fn new(rate: u32) -> Chorus {
        Chorus {
            fs: rate as f32,
            ph: 0.0,
            bbd: Vec::new(),
            built: u8::MAX,
            amount: 1.0,
            line: vec![[0.0; 2]; (rate as f32 * 0.05) as usize + 8],
            li: 0,
            ap: [0.0; 2],
            sat: tape::Saturator::new(rate),
            wobble: tape::Wobble::default(),
            seed: 0x2545_f491,
            n: 0,
        }
    }

    /// The BBD voices' delays (s) for mode `m` at LFO phase `ph`, depth scaled by `amount`.
    fn delays(m: u8, ph: f64, amount: f64) -> [f64; 3] {
        let p = preset(m);
        let (c, d) = (p.centre_ms as f64 * 1e-3, p.depth_ms as f64 * 1e-3 * amount);
        match m {
            // Juno: one triangle, the second voice's inverted.
            0..=2 => [c + d * tri(ph), c - d * tri(ph), c],
            // Dimension: a trapezoid (a clipped triangle), in antiphase.
            3..=6 => {
                let t = (1.5 * tri(ph)).clamp(-1.0, 1.0);
                [c + d * t, c - d * t, c]
            }
            // Ensemble: three voices 120° apart, each a slow and a fast sine.
            _ => std::array::from_fn(|v| {
                let o = v as f64 / 3.0;
                let tau = std::f64::consts::TAU;
                c + d * ((tau * (ph + o)).sin() + FAST_DEPTH * (tau * (FAST_RATE * ph + o)).sin())
            }),
        }
    }

    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]]) {
        let m = s.mode.min(CLEAN);
        let p = preset(m);
        let fs = self.fs as f64;
        if m < TAPE && circuit_id(m) != self.built {
            let (stages, fc) = circuit(m);
            self.bbd = (0..3).map(|v| Bbd::new(fs, stages, fc, 0x9e37_79b9 ^ (v as u32 * 7919))).collect();
            self.built = circuit_id(m);
        }
        let step = s.rate.clamp(0.01, 20.0) as f64 / fs;
        let mix = s.mix.clamp(0.0, 1.0);
        let (gd, gw) = ((2.0 * (1.0 - mix)).min(1.0), (2.0 * mix).min(1.0) * p.wet);
        let target = s.amount.clamp(0.0, 2.0);
        let smooth = 1.0 - (-1.0 / (0.02 * self.fs)).exp();
        let hiss = (s.noise.clamp(0.0, 1.0) * HISS) as f64;
        let n = self.line.len();
        for f in main.iter_mut() {
            self.ph = (self.ph + step) % 1.0;
            self.amount += (target - self.amount) * smooth;
            let (l, r) = (f[0] as f64, f[1] as f64);
            let wet: [f64; 2] = match m {
                0..=2 => {
                    let d = Self::delays(m, self.ph, self.amount as f64);
                    [self.bbd[0].step(l, d[0], hiss), self.bbd[1].step(r, d[1], hiss)]
                }
                3..=6 => {
                    let d = Self::delays(m, self.ph, self.amount as f64);
                    let (a, b) = (self.bbd[0].step(l, d[0], hiss), self.bbd[1].step(r, d[1], hiss));
                    [(a + CROSS * b) / (1.0 + CROSS), (b + CROSS * a) / (1.0 + CROSS)]
                }
                7 => {
                    let d = Self::delays(m, self.ph, self.amount as f64);
                    let u = 0.5 * (l + r);
                    let v: [f64; 3] = std::array::from_fn(|k| self.bbd[k].step(u, d[k], hiss));
                    // Voices left, centre and right.
                    [0.7 * v[0] + 0.5 * v[1], 0.7 * v[2] + 0.5 * v[1]]
                }
                TAPE => self.tape(s, p, l as f32, r as f32, n),
                _ => self.clean(p, l as f32, r as f32, n),
            };
            self.li = (self.li + 1) % n;
            self.n = self.n.wrapping_add(1);
            *f = [(l * gd as f64 + wet[0] * gw as f64) as f32, (r * gd as f64 + wet[1] * gw as f64) as f32];
        }
    }

    /// Read the line `d` samples back on side `c`, cubic interpolated.
    fn read(&self, c: usize, d: f32, n: usize) -> f32 {
        let pos = self.li as f32 + n as f32 - d;
        let k = pos.floor() as usize;
        let fr = pos - pos.floor();
        let p = |o: usize| self.line[(k + n + o - 1) % n][c];
        let (y0, y1, y2, y3) = (p(0), p(1), p(2), p(3));
        y1 + 0.5 * fr * (y2 - y0 + fr * (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3 + fr * (3.0 * (y1 - y2) + y3 - y0)))
    }

    /// Tape mode: through the tape (with drive), then a short delay wandered by wow and flutter,
    /// the right side a touch longer.
    fn tape(&mut self, s: &Settings, p: Preset, l: f32, r: f32, n: usize) -> [f64; 2] {
        let speed_k = s.rate.clamp(0.01, 20.0) / p.rate;
        let wob = self.wobble.next(self.n % 2048 == 0, &mut self.seed, speed_k, self.fs) * self.amount * p.depth_ms;
        let drive = s.drive.clamp(0.0, 1.0);
        let input = [l, r];
        for c in 0..2 {
            self.line[self.li][c] = if drive > 0.0 { self.sat.record(c, input[c], drive) } else { input[c] };
        }
        let mut out = [0.0; 2];
        for (c, o) in out.iter_mut().enumerate() {
            let d = ((p.centre_ms + c as f32) * 1e-3 + wob) * self.fs;
            let mut v = self.read(c, d.clamp(2.0, (n - 4) as f32), n);
            if s.noise > 0.0 {
                self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                v += s.noise.min(1.0) * HISS * 0.5 * ((self.seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0);
            }
            *o = v as f64;
        }
        out
    }

    /// Clean: Dattorro's white chorus per side, its delay read through a first-order all-pass
    /// (fraction kept in 0.5..1.5 so its pole stays clear of −1), fed back from a fixed tap so
    /// the static response is flat. The sides' LFOs are a quarter cycle apart.
    fn clean(&mut self, p: Preset, l: f32, r: f32, n: usize) -> [f64; 2] {
        let centre = (p.centre_ms * 1e-3 * self.fs).round();
        let input = [l, r];
        let mut out = [0.0; 2];
        for c in 0..2 {
            let fixed = self.line[(self.li + n - centre as usize) % n][c];
            let xh = input[c] - BLEND * fixed;
            self.line[self.li][c] = xh;
            let lfo = (std::f64::consts::TAU * (self.ph + 0.25 * c as f64)).sin() as f32;
            let d = (centre + lfo * p.depth_ms * 1e-3 * self.fs * self.amount).clamp(2.0, (n - 4) as f32);
            let i = (d - 0.5).floor();
            let frac = d - i;
            let a = (1.0 - frac) / (1.0 + frac);
            let (x0, x1) = (self.line[(self.li + n - i as usize) % n][c], self.line[(self.li + 2 * n - i as usize - 1) % n][c]);
            let tap = a * x0 + x1 - a * self.ap[c];
            self.ap[c] = tap;
            out[c] = (BLEND * xh + tap) as f64;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize) -> Vec<[f32; 2]> {
        let mut s = 1u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let a = (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                [a, (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5]
            })
            .collect()
    }

    fn sine(hz: f32, n: usize, fs: f32) -> Vec<[f32; 2]> {
        (0..n).map(|i| { let v = 0.5 * (std::f32::consts::TAU * hz * i as f32 / fs).sin(); [v, v] }).collect()
    }

    #[test]
    fn butterworth_is_unity_at_dc() {
        let (p, r) = butterworth(9000.0);
        let dc = p.iter().zip(&r).fold(C::default(), |a, (p, r)| a.sub(r.div(*p)));
        assert!((dc.re - 1.0).abs() < 1e-9 && dc.im.abs() < 1e-6, "{dc:?}");
    }

    #[test]
    fn bbd_delays_by_its_clock() {
        // An impulse comes out after the delay the clock sets (plus the filters' group delay).
        for fs in [44100.0, 96000.0] {
            for d in [0.0017, 0.0035, 0.0053] {
                let mut b = Bbd::new(fs, 256, 9000.0, 1);
                let out: Vec<f64> = (0..(fs * 0.02) as usize).map(|i| b.step(if i == 0 { 1.0 } else { 0.0 }, d, 0.0)).collect();
                let peak = out.iter().enumerate().fold((0, 0.0), |a, (i, &v)| if v.abs() > a.1 { (i, v.abs()) } else { a }).0;
                let secs = peak as f64 / fs;
                eprintln!("fs {fs} delay {:.2} ms -> peak at {:.2} ms", d * 1e3, secs * 1e3);
                assert!(secs > d - 0.0002 && secs < d + 0.0003, "{secs} vs {d}");
            }
        }
    }

    #[test]
    fn bbd_passes_dc_and_lows_at_unity() {
        let mut b = Bbd::new(48000.0, 256, 9000.0, 1);
        let mut last = 0.0;
        for i in 0..48000 {
            last = b.step(0.5 * (std::f64::consts::TAU * 100.0 * i as f64 / 48000.0).sin() + 0.25, 0.003 + 0.001 * (i as f64 * 0.0001).sin(), 0.0);
        }
        assert!(last.is_finite());
        let mut b = Bbd::new(48000.0, 256, 9000.0, 1);
        let y: Vec<f64> = (0..9600).map(|_| b.step(0.5, 0.004, 0.0)).collect();
        assert!((y[9599] - 0.5).abs() < 1e-6, "{}", y[9599]);
    }

    #[test]
    fn every_mode_stays_finite_and_bounded() {
        for fs in [44100u32, 48000, 96000] {
            for m in 0..MODES.len() as u8 {
                for amount in [0.0, 1.0, 2.0] {
                    let mut c = Chorus::new(fs);
                    let mut s = Settings { amount, mix: 1.0, noise: 1.0, drive: 1.0, ..Settings::default() };
                    s.set_mode(m);
                    let mut buf = noise(fs as usize / 2);
                    c.process(&s, &mut buf);
                    // And with the rate at its top.
                    s.rate = 20.0;
                    c.process(&s, &mut buf);
                    let peak = buf.iter().fold(0.0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
                    assert!(buf.iter().all(|f| f[0].is_finite() && f[1].is_finite()), "mode {m} NaN");
                    // (Clean's feedback loop can ring up to 1 / (1 − 0.707) when the modulated tap
                    // runs far from the fixed one.)
                    assert!(peak < 4.0 / (1.0 - BLEND), "{} at {fs}: peak {peak}", MODES[m as usize]);
                }
            }
        }
    }

    #[test]
    fn every_mode_keeps_its_level() {
        // At its own settings, all wet, each mode comes out about as loud as it went in.
        let rms = |b: &[[f32; 2]]| (b.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>() / (2 * b.len()) as f32).sqrt();
        for m in 0..MODES.len() as u8 {
            let mut c = Chorus::new(48000);
            let mut s = Settings { mix: 1.0, ..Settings::default() };
            s.set_mode(m);
            let x = noise(96000);
            let mut y = x.clone();
            c.process(&s, &mut y);
            let g = rms(&y[9600..]) / rms(&x[9600..]);
            eprintln!("{}: level {g:.2}", MODES[m as usize]);
            assert!(g > 0.3 && g < 2.0, "{}: {g}", MODES[m as usize]);
        }
    }

    #[test]
    fn dry_only_is_untouched() {
        let mut c = Chorus::new(44100);
        let s = Settings { mix: 0.0, ..Settings::default() };
        let x = noise(4410);
        let mut y = x.clone();
        c.process(&s, &mut y);
        assert_eq!(x, y);
    }

    #[test]
    fn delays_stay_in_range_and_voices_oppose() {
        for m in 0..TAPE {
            let p = preset(m);
            let (lo, hi) = ((p.centre_ms - p.depth_ms * 1.2) * 1e-3, (p.centre_ms + p.depth_ms * 1.2) * 1e-3);
            for i in 0..1000 {
                let d = Chorus::delays(m, i as f64 / 1000.0, 1.0);
                for v in d {
                    assert!(v as f32 >= lo - 1e-6 && v as f32 <= hi + 1e-6, "{} {v}", MODES[m as usize]);
                }
                if m < 7 {
                    // Juno and Dimension: the two voices in antiphase.
                    assert!((d[0] + d[1] - 2.0 * p.centre_ms as f64 * 1e-3).abs() < 1e-9);
                }
            }
        }
        // Juno I sweeps 1.66 to 5.35 ms.
        let d: Vec<f64> = (0..1000).map(|i| Chorus::delays(0, i as f64 / 1000.0, 1.0)[0]).collect();
        let (mn, mx) = d.iter().fold((1.0f64, 0.0f64), |a, &v| (a.0.min(v), a.1.max(v)));
        assert!((mn - 0.00166).abs() < 2e-5 && (mx - 0.00535).abs() < 2e-5, "{mn} {mx}");
    }

    /// Spread of the periods between rising zero crossings: how much the pitch wanders.
    fn wander(y: &[f32]) -> f32 {
        let xs: Vec<f32> = y.windows(2).enumerate().filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0).map(|(i, w)| i as f32 + -w[0] / (w[1] - w[0])).collect();
        let per: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        let mean = per.iter().sum::<f32>() / per.len() as f32;
        (per.iter().map(|p| (p - mean).powi(2)).sum::<f32>() / per.len() as f32).sqrt()
    }

    #[test]
    fn pitch_movement_cancels_in_mono() {
        // Juno and Dimension: each side's wet signal wobbles in pitch, but in antiphase, so
        // their sum (mono) hardly does.
        for m in [0u8, 1, 4, 6] {
            let mut c = Chorus::new(44100);
            let mut s = Settings { mix: 1.0, ..Settings::default() };
            s.set_mode(m);
            let mut buf = sine(100.0, 44100 * 4, 44100.0);
            c.process(&s, &mut buf);
            let tail = &buf[4410..];
            let side: Vec<f32> = tail.iter().map(|f| f[0]).collect();
            let mono: Vec<f32> = tail.iter().map(|f| f[0] + f[1]).collect();
            let (ws, wm) = (wander(&side), wander(&mono));
            eprintln!("{}: side wander {ws:.4}, mono {wm:.4}", MODES[m as usize]);
            assert!(wm < ws * 0.5, "{}", MODES[m as usize]);
        }
    }

    #[test]
    fn clean_is_flat_when_still() {
        // At no depth Dattorro's white chorus is an all-pass: a sine keeps its level.
        let mut c = Chorus::new(44100);
        let mut s = Settings { amount: 0.0, mix: 1.0, ..Settings::default() };
        s.set_mode(CLEAN);
        c.amount = 0.0;
        for hz in [200.0, 1234.0, 5000.0] {
            let mut buf = sine(hz, 44100, 44100.0);
            c.process(&s, &mut buf);
            let peak = buf[22050..].iter().fold(0.0f32, |a, f| a.max(f[0].abs()));
            assert!((peak - 0.5).abs() < 0.03, "{hz} Hz: {peak}");
        }
    }
}
