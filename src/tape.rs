//! Tape: a tape echo built on Chowdhury's physical model of an analog tape machine ("Real-time
//! physical modelling for analog tape machines", DAFx-19). Each pass round the loop is recorded
//! onto tape (Jiles-Atherton hysteresis solved by RK4, eqs. 18-25), wobbles with wow and flutter
//! (a modulated delay line, §4.4) and is played back through the play head's losses (spacing,
//! thickness and gap, eq. 13, as an FIR filter for the tape speed). So the repeats get warmer,
//! darker and more saturated as they go round, as on a real tape echo.

use serde::{Deserialize, Serialize};

/// Tape settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Delay time in beats (synced to the song tempo).
    pub beats: f32,
    /// 0..1.05: how much comes round again (past 1 it runs away into the tape's saturation).
    pub feedback: f32,
    /// 0..1: how hard the record head drives the tape.
    pub drive: f32,
    /// 0..1: record bias. Low leaves the tape's "deadzone" (quiet sounds crunch, §4.3); high
    /// is clean until it saturates. It makes the magnetisation more reversible (raises c),
    /// standing in for the bias tone.
    #[serde(default = "default_bias")]
    pub bias: f32,
    /// 0..1: wow and flutter depth.
    pub wow: f32,
    /// Tape speed, inches per second: slower is darker.
    pub speed: f32,
    /// Play head to tape spacing, microns: more is darker.
    pub spacing: f32,
    /// 0..1: wet against dry (all wet on a return track).
    pub mix: f32,
    pub on: bool,
    /// An LFO on the delay time: its shape (`LFO_SHAPES`), rate (Hz, times 0.01 with `lfo_slow`
    /// for very slow drifts), depth (ms) and slew (0..1: how slowly it follows its shape).
    #[serde(default)]
    pub lfo_shape: u8,
    #[serde(default = "default_lfo_rate")]
    pub lfo_rate: f32,
    #[serde(default)]
    pub lfo_slow: bool,
    #[serde(default)]
    pub lfo_depth: f32,
    #[serde(default)]
    pub lfo_slew: f32,
}

/// The LFO's shapes, by `lfo_shape`.
pub const LFO_SHAPES: [&str; 7] = ["Sine", "Triangle", "Square", "Saw", "Random", "Smooth Random", "Drunk"];

fn default_lfo_rate() -> f32 {
    0.5
}

fn default_bias() -> f32 {
    0.9
}

impl Default for Settings {
    fn default() -> Self {
        Settings { beats: 0.75, feedback: 0.5, drive: 0.5, bias: default_bias(), wow: 0.3, speed: 15.0, spacing: 5.0, mix: 1.0, on: true, lfo_shape: 0, lfo_rate: default_lfo_rate(), lfo_slow: false, lfo_depth: 0.0, lfo_slew: 0.0 }
    }
}

/// The tape's Jiles-Atherton constants, scaled from the paper's ferric oxide values (fig. 6:
/// Ms 3.5e5, a 22e3, k 27e3 A/m, c 0.17, α 1.6e-3) so a full-scale signal at middle drive just
/// starts to saturate.
const A: f64 = 0.25;
const K: f64 = A * 27.0 / 22.0;
const MS: f64 = A * 3.5e5 / 22e3;
const C: f64 = 0.17;
const ALPHA: f64 = 1.6e-3;
/// The hysteresis runs at this many times the sample rate (the paper uses 16 with its bias tone;
/// without one, 2 keeps RK4 stable and the aliasing low).
const OVER: usize = 2;
/// Play head constants (m): gap from §3.3.1, and tape thickness. The paper's 35 µm loses a lot
/// of top that a real machine's playback EQ gives back (it leaves the circuits out), so a
/// thinner tape stands in for that EQ.
const GAP: f64 = 5e-6;
const THICK: f64 = 10e-6;
const FIR_LEN: usize = 33;

/// The reversible fraction for `bias`: none is the paper's ferric oxide (c = 0.17), full is
/// almost all reversible (no deadzone, little hysteresis). Narrowing the loop instead (k) makes
/// the equation stiff, and RK4 blows up on high frequencies.
fn c_for(bias: f32) -> f64 {
    C + (0.9 - C) * bias.clamp(0.0, 1.0) as f64
}

/// Langevin function and its derivative, with the small-argument forms of eqs. 23-24.
fn langevin(x: f64) -> (f64, f64) {
    if x.abs() < 1e-4 {
        return (x / 3.0, 1.0 / 3.0);
    }
    let coth = 1.0 / x.tanh();
    (coth - 1.0 / x, 1.0 / (x * x) - coth * coth + 1.0)
}

/// One side's tape magnetisation (eq. 18), stepped by RK4 (eq. 22).
#[derive(Clone, Copy, Default)]
struct Hysteresis {
    m: f64,
    h: f64,
    hd: f64,
}

impl Hysteresis {
    /// dM/dt for magnetisation `m`, field `h` and its rate of change `hd` (eq. 18), with `c`
    /// the reversible fraction.
    fn dmdt(m: f64, h: f64, hd: f64, c: f64) -> f64 {
        let (l, dl) = langevin((h + ALPHA * m) / A);
        let man = MS * l;
        let ds = if hd >= 0.0 { 1.0 } else { -1.0 };
        let dm = if ds * (man - m) > 0.0 { 1.0 } else { 0.0 };
        let mut den = (1.0 - c) * ds * K - ALPHA * (man - m);
        if den.abs() < 1e-9 {
            den = 1e-9f64.copysign(den);
        }
        let irr = (1.0 - c) * dm * (man - m) / den * hd;
        let rev = c * MS / A * hd * dl;
        (irr + rev) / (1.0 - c * ALPHA * MS / A * dl)
    }

    /// Record field `h` for one step of `t` seconds; returns the magnetisation.
    fn step(&mut self, h: f64, t: f64, c: f64) -> f64 {
        // Trapezoidal derivative of the field (eq. 21), very slightly damped so it can't ring at Nyquist.
        let hd = 2.0 * (h - self.h) / t - 0.98 * self.hd;
        let (hm, hdm) = (0.5 * (h + self.h), 0.5 * (hd + self.hd));
        let k1 = t * Self::dmdt(self.m, self.h, self.hd, c);
        let k2 = t * Self::dmdt(self.m + k1 / 2.0, hm, hdm, c);
        let k3 = t * Self::dmdt(self.m + k2 / 2.0, hm, hdm, c);
        let k4 = t * Self::dmdt(self.m + k3, h, hd, c);
        let m = self.m + k1 / 6.0 + k2 / 3.0 + k3 / 3.0 + k4 / 6.0;
        (self.m, self.h, self.hd) = if m.is_finite() { (m.clamp(-MS, MS), h, hd) } else { (0.0, 0.0, 0.0) };
        self.m
    }
}

/// The play head's loss filter for tape speed `ips` and spacing `spacing_um` (eq. 13), as a
/// linear-phase FIR from its inverse DFT, Hann windowed.
fn loss_fir(rate: f32, ips: f32, spacing_um: f32) -> [f32; FIR_LEN] {
    let v = ips as f64 * 0.0254;
    let d = spacing_um as f64 * 1e-6;
    let n = 256;
    let gain = |f: f64| -> f64 {
        let k = 2.0 * std::f64::consts::PI * f / v;
        if k == 0.0 {
            return 1.0;
        }
        let space = (-k * d).exp();
        let thick = (1.0 - (-k * THICK).exp()) / (k * THICK);
        let x = k * GAP / 2.0;
        space * thick * (x.sin() / x)
    };
    let mags: Vec<f64> = (0..=n / 2).map(|b| gain(b as f64 * rate as f64 / n as f64)).collect();
    let mut h = [0.0f32; FIR_LEN];
    let mid = (FIR_LEN / 2) as f64;
    for (i, out) in h.iter_mut().enumerate() {
        let t = i as f64 - mid;
        let mut s = mags[0];
        for (b, &m) in mags.iter().enumerate().skip(1) {
            let w = if b == n / 2 { 1.0 } else { 2.0 };
            s += w * m * (2.0 * std::f64::consts::PI * b as f64 * t / n as f64).cos();
        }
        let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 1.0) / (FIR_LEN as f64 + 1.0)).cos();
        *out = (s / n as f64 * hann) as f32;
    }
    // Unity at DC.
    let sum: f32 = h.iter().sum();
    h.iter_mut().for_each(|x| *x /= sum.max(1e-6));
    h
}

/// The record head and tape on their own, both sides: a signal recorded onto modelled tape
/// and read straight back (no echo, no play-head losses). Riser uses it as its saturation.
pub struct Saturator {
    rate: f32,
    /// The tape's reversible fraction (c) in use, set by the bias.
    pub(crate) c: f64,
    tape: [Hysteresis; 2],
    /// The last input, for the oversampling's linear interpolation.
    prev_in: [f32; 2],
}

impl Saturator {
    pub fn new(rate: u32) -> Saturator {
        Saturator { rate: rate as f32, c: c_for(default_bias()), tape: [Hysteresis::default(); 2], prev_in: [0.0; 2] }
    }

    pub fn set_bias(&mut self, bias: f32) {
        self.c = c_for(bias);
    }

    /// Record `x` (side `c`, at the sample rate) onto the tape: oversampled hysteresis, back down.
    pub fn record(&mut self, c: usize, x: f32, drive: f32) -> f32 {
        // Full scale reaches 0.6 a (barely bending) with no drive, 4 a (well into saturation) at full:
        // the Langevin curve is flat past that, which is a hard clip, not tape.
        let g = A as f32 * (0.6 + 3.4 * drive * drive);
        let t = 1.0 / (self.rate as f64 * OVER as f64);
        let mut acc = 0.0;
        for j in 1..=OVER {
            let xi = self.prev_in[c] + (x - self.prev_in[c]) * j as f32 / OVER as f32;
            acc += self.tape[c].step((xi * g) as f64, t, self.c);
        }
        self.prev_in[c] = x;
        // Back to signal level: divide by the tape's gain for a normal level (MS / 3A times
        // about c + 0.095 (1 − c), measured: below the loop's width only the reversible part
        // moves) and the drive. So a normal level comes back as it went in at any bias, and
        // underbiased tape loses its quiet sounds rather than running away with its loud ones.
        let norm = self.c + 0.095 * (1.0 - self.c);
        (acc / OVER as f64 / (MS / (3.0 * A) * norm) / g as f64) as f32
    }
}

pub struct Tape {
    rate: f32,
    pub(crate) sat: Saturator,
    buf: Vec<[f32; 2]>,
    i: usize,
    fir: [f32; FIR_LEN],
    fir_for: (f32, f32),
    hist: [[f32; FIR_LEN]; 2],
    hi: usize,
    /// High-pass in the loop (so the low end can't build up), per side.
    hp: [f32; 2],
    hp_x: [f32; 2],
    wobble: Wobble,
    seed: u32,
    lfo: Lfo,
}

/// Wow and flutter: a slow and a fast sine and a slow random drift, wandering a tape's delay
/// time. Chorus's Tape mode uses it too.
#[derive(Default)]
pub(crate) struct Wobble {
    wow_ph: f32,
    flut_ph: f32,
    drift: f32,
    drift_to: f32,
}

impl Wobble {
    /// Advance one sample (`sr` a second, with the tape's speed factor `speed_k`) and give the
    /// delay-time swing in seconds at full wow. On `new_drift` the drift heads somewhere new,
    /// drawn from `seed`.
    pub(crate) fn next(&mut self, new_drift: bool, seed: &mut u32, speed_k: f32, sr: f32) -> f32 {
        let tau = std::f32::consts::TAU;
        self.wow_ph = (self.wow_ph + 0.55 * speed_k / sr) % 1.0;
        self.flut_ph = (self.flut_ph + 7.3 * speed_k / sr) % 1.0;
        if new_drift {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.drift_to = (*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        }
        self.drift += (self.drift_to - self.drift) * 2.0 / sr;
        0.0016 * (tau * self.wow_ph).sin() + 0.00012 * (tau * self.flut_ph).sin() + 0.0008 * self.drift
    }
}

/// The delay-time LFO's running state. Auto Filter's cutoff LFO is one too.
#[derive(Default)]
pub(crate) struct Lfo {
    phase: f64,
    /// Random shapes: the value held, the one before (to glide from) and the drunk walk's.
    held: f32,
    from: f32,
    walk: f32,
    /// Its output after the slew.
    out: f32,
}

impl Lfo {
    /// Advance one sample at `rate` Hz (`sr` samples a second) and give the shape's value, -1..1,
    /// after a slew of `slew` (0..1, up to about 2 s to follow).
    pub(crate) fn next(&mut self, shape: u8, rate: f64, slew: f32, sr: f32, rand: &mut impl FnMut() -> f32) -> f32 {
        self.phase += rate / sr as f64;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            // A new cycle: new random values.
            self.from = self.held;
            self.held = rand();
            let step = rand() * 0.4;
            self.walk = if (self.walk + step).abs() > 1.0 { self.walk - step } else { self.walk + step };
        }
        let p = self.phase as f32;
        let tau = std::f32::consts::TAU;
        let v = match shape {
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => if p < 0.5 { 1.0 } else { -1.0 },
            3 => 2.0 * p - 1.0,
            4 => self.held,
            5 => self.from + (self.held - self.from) * (0.5 - 0.5 * (std::f32::consts::PI * p).cos()),
            // Drunk: a random walk, gliding to each new step.
            6 => self.walk,
            _ => (tau * p).sin(),
        };
        // (Never quite instant: a jump in delay time is a click.)
        let secs = (2.0 * slew.clamp(0.0, 1.0).powi(2)).max(0.002);
        // (A drunk walk glides a little even with no slew, or each step would click.)
        let secs = if shape == 6 { secs.max(0.5 / rate.max(1e-3) as f32 * 0.2) } else { secs };
        self.out += (v - self.out) * (1.0 - (-1.0 / (secs * sr)).exp());
        self.out
    }
}

impl Tape {
    pub fn new(rate: u32) -> Tape {
        let s = Settings::default();
        let r = rate as f32;
        Tape {
            rate: r,
            sat: Saturator::new(rate),
            buf: vec![[0.0; 2]; rate as usize * 5],
            i: 0,
            fir: loss_fir(r, s.speed, s.spacing),
            fir_for: (s.speed, s.spacing),
            hist: [[0.0; FIR_LEN]; 2],
            hi: 0,
            hp: [0.0; 2],
            hp_x: [0.0; 2],
            wobble: Wobble::default(),
            seed: 0x9e37_79b9,
            lfo: Lfo::default(),
        }
    }

    /// Record `x` (one side, at the sample rate) onto the tape.
    fn record(&mut self, c: usize, x: f32, drive: f32) -> f32 {
        self.sat.record(c, x, drive)
    }

    /// `beat` is a beat's length in frames.
    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]], beat: f64) {
        if (s.speed, s.spacing) != self.fir_for {
            self.fir = loss_fir(self.rate, s.speed, s.spacing);
            self.fir_for = (s.speed, s.spacing);
        }
        self.sat.set_bias(s.bias);
        let n = self.buf.len();
        // The FIR's own delay comes off the echo time.
        let base = (s.beats.max(0.0625) as f64 * beat) as f32 - (FIR_LEN / 2) as f32;
        let fb = s.feedback.clamp(0.0, 1.05);
        let mix = s.mix.clamp(0.0, 1.0);
        let wow = s.wow.clamp(0.0, 1.0);
        let a_hp = (-2.0 * std::f32::consts::PI * 60.0 / self.rate).exp();
        // Slower tape wobbles slower.
        let speed_k = (s.speed / 7.5).clamp(0.3, 4.0).sqrt();
        for f in main.iter_mut() {
            // Wow (slow) and flutter (fast) as a wandering delay time (§4.4), in samples.
            let wobble = wow * self.rate * self.wobble.next(self.i % 2048 == 0, &mut self.seed, speed_k, self.rate);
            // The delay-time LFO.
            let lfo_rate = s.lfo_rate.clamp(0.01, 20.0) as f64 * if s.lfo_slow { 0.01 } else { 1.0 };
            let seed = &mut self.seed;
            let mut rand = || {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            };
            let lfo = self.lfo.next(s.lfo_shape, lfo_rate, s.lfo_slew, self.rate, &mut rand);
            let swing = lfo * s.lfo_depth.clamp(0.0, 200.0) * 0.001 * self.rate;
            let d = (base + wobble + swing).clamp(2.0, (n - 4) as f32);
            // Play head: read the tape (cubic interpolation), then its losses.
            let pos = self.i as f32 + n as f32 - d;
            let k = pos.floor() as usize;
            let fr = pos - pos.floor();
            let mut echo = [0.0f32; 2];
            for c in 0..2 {
                let p = |o: usize| self.buf[(k + n + o - 1) % n][c];
                let (y0, y1, y2, y3) = (p(0), p(1), p(2), p(3));
                let raw = y1 + 0.5 * fr * (y2 - y0 + fr * (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3 + fr * (3.0 * (y1 - y2) + y3 - y0)));
                self.hist[c][self.hi] = raw;
                let mut y = 0.0;
                for (t, h) in self.fir.iter().enumerate() {
                    y += h * self.hist[c][(self.hi + FIR_LEN - t) % FIR_LEN];
                }
                // Keep the low end from building up round the loop.
                let hp = a_hp * (self.hp[c] + y - self.hp_x[c]);
                self.hp_x[c] = y;
                self.hp[c] = hp;
                echo[c] = hp;
            }
            self.hi = (self.hi + 1) % FIR_LEN;
            // Record head: what comes in plus the echo coming round, onto the tape.
            for c in 0..2 {
                let rec = self.record(c, f[c] + echo[c] * fb, s.drive);
                self.buf[self.i][c] = rec;
            }
            self.i = (self.i + 1) % n;
            *f = [f[0] * (1.0 - mix) + echo[0] * mix, f[1] * (1.0 - mix) + echo[1] * mix];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level of a sine through one pass of the tape, at peak amplitude `amp`.
    fn pass_gain(amp: f32, drive: f32) -> f32 {
        pass_gain_k(amp, drive, c_for(default_bias()))
    }

    fn pass_gain_k(amp: f32, drive: f32, c: f64) -> f32 {
        let mut t = Tape::new(44100);
        t.sat.c = c;
        let n = 44100;
        let x: Vec<f32> = (0..n).map(|i| amp * (std::f32::consts::TAU * 220.0 * i as f32 / 44100.0).sin()).collect();
        let mut y = 0.0f32;
        for (i, &v) in x.iter().enumerate() {
            let r = t.record(0, v, drive);
            if i > n / 2 {
                y = y.max(r.abs());
            }
        }
        y / amp
    }

    #[test]
    fn tape_levels() {
        for drive in [0.0, 0.5, 1.0] {
            for amp in [0.01, 0.05, 0.3, 1.0] {
                eprintln!("drive {drive} amp {amp}: gain {:.3}", pass_gain(amp, drive));
            }
        }
        let g = pass_gain(0.1, 0.5);
        assert!(g.is_finite() && g > 0.3 && g < 3.0, "quiet gain {g}");
        // Loud signals saturate: less gain than quiet ones.
        assert!(pass_gain(1.0, 1.0) < pass_gain(0.05, 1.0));
    }

    #[test]
    fn loss_filter_darker_when_slower() {
        let resp = |h: &[f32; FIR_LEN], f: f32| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, v) in h.iter().enumerate() {
                let w = std::f32::consts::TAU * f / 44100.0 * i as f32;
                re += v * w.cos();
                im -= v * w.sin();
            }
            (re * re + im * im).sqrt()
        };
        let slow = loss_fir(44100.0, 3.75, 20.0);
        let fast = loss_fir(44100.0, 15.0, 20.0);
        eprintln!("8 kHz: 3.75 ips {:.3}, 15 ips {:.3}", resp(&slow, 8000.0), resp(&fast, 8000.0));
        assert!(resp(&slow, 8000.0) < resp(&fast, 8000.0));
        assert!((resp(&fast, 50.0) - 1.0).abs() < 0.05);
    }

    #[test]
    fn echo_stays_bounded() {
        let mut t = Tape::new(44100);
        let s = Settings { feedback: 1.05, drive: 1.0, ..Settings::default() };
        let mut buf = vec![[0.0f32; 2]; 44100 * 4];
        buf[0] = [1.0, 1.0];
        t.process(&s, &mut buf, 22050.0);
        let peak = buf.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        eprintln!("runaway peak {peak}");
        assert!(peak.is_finite() && peak < 8.0);
    }
}

#[cfg(test)]
/// Clicks came from the hysteresis blowing up (and resetting) with a narrow loop: these check a
/// loud sine comes through smooth at every bias and drive.
mod click_probe {
    use super::*;

    /// Largest jump in the second difference relative to the signal's own (a click detector).
    fn clickiness(y: &[f32]) -> f32 {
        let d2: Vec<f32> = y.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
        let rms = (d2.iter().map(|v| v * v).sum::<f32>() / d2.len() as f32).sqrt();
        d2.iter().fold(0.0f32, |m, v| m.max(v.abs())) / rms.max(1e-9)
    }

    #[test]
    fn probe() {
        let sr = 44100.0;
        let x: Vec<f32> = (0..44100).map(|i| 0.4 * (std::f32::consts::TAU * 220.0 * i as f32 / sr).sin()).collect();
        eprintln!("input clickiness {:.1}", clickiness(&x));
        for bias in [0.0, 0.5, 0.9, 1.0] {
            let mut t = Tape::new(44100);
            t.sat.c = c_for(bias as f32);
            let y: Vec<f32> = x.iter().map(|&v| t.record(0, v, 0.5)).collect();
            eprintln!("record bias {bias}: clickiness {:.1}", clickiness(&y[1000..]));
        }
        for (amp, drive) in [(1.0f32, 0.5f32), (1.0, 1.0), (0.2, 1.0)] {
            let mut t = Tape::new(44100);
            t.sat.c = c_for(0.9);
            let xx: Vec<f32> = x.iter().map(|v| v / 0.4 * amp).collect();
            let y: Vec<f32> = xx.iter().map(|&v| t.record(0, v, drive)).collect();
            let worst = y.windows(2).skip(1000).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            eprintln!("record amp {amp} drive {drive}: clickiness {:.1}, max step {worst:.3}", clickiness(&y[1000..]));
            assert!(clickiness(&y[1000..]) < 5.0 && y.iter().all(|v| v.is_finite()));
            if amp == 1.0 && drive == 1.0 {
                let i0 = 2000;
                eprintln!("{:?}", y[i0..i0 + 120].iter().step_by(4).map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
            }
        }
        for wow in [0.0, 0.3] {
            let mut t = Tape::new(44100);
            let s = Settings { feedback: 0.0, wow, ..Settings::default() };
            let mut buf: Vec<[f32; 2]> = x.iter().map(|&v| [v, v]).collect();
            t.process(&s, &mut buf, 22050.0);
            let y: Vec<f32> = buf.iter().map(|f| f[0]).collect();
            eprintln!("process wow {wow}: clickiness {:.1}", clickiness(&y[20000..]));
        }
    }
}

#[cfg(test)]
mod bias_sweep {
    use super::*;

    /// The echo must come through at every bias, on a drum-like signal (noise bursts).
    #[test]
    fn echo_at_every_bias() {
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let x: Vec<f32> = (0..44100 * 3).map(|i| if i % 11025 < 2000 { 0.9 * noise() * (1.0 - (i % 11025) as f32 / 2000.0) } else { 0.0 }).collect();
        for b in 0..=10 {
            for drive in [0.0, 0.5, 1.0] {
                let s = Settings { bias: b as f32 / 10.0, drive, ..Settings::default() };
                let mut t = Tape::new(44100);
                let mut buf: Vec<[f32; 2]> = x.iter().map(|&v| [v, v]).collect();
                t.process(&s, &mut buf, 22050.0);
                let tail = &buf[44100..];
                let rms = (tail.iter().map(|f| f[0] * f[0]).sum::<f32>() / tail.len() as f32).sqrt();
                eprintln!("bias {:.1} drive {drive}: echo rms {rms:.3}", s.bias);
                assert!(rms.is_finite() && rms > 0.02, "bias {} drive {drive}: echo gone ({rms})", s.bias);
            }
        }
    }
}

#[cfg(test)]
mod lfo_tests {
    use super::*;

    #[test]
    fn every_shape_stays_in_range_and_slows_down() {
        let mut seed = 3u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for shape in 0..LFO_SHAPES.len() as u8 {
            let mut l = Lfo::default();
            let v: Vec<f32> = (0..44100).map(|_| l.next(shape, 4.0, 0.0, 44100.0, &mut rand)).collect();
            let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
            // Biggest step between samples: no jumps (the 2 ms floor on the slew).
            let step = v.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            eprintln!("{}: {lo:.2}..{hi:.2}, biggest step {step:.4}", LFO_SHAPES[shape as usize]);
            assert!(lo >= -1.001 && hi <= 1.001 && step < 0.05);
        }
        // A cycle at 1 Hz × 0.01 takes 100 s: after 1 s a sine has barely moved.
        let mut l = Lfo::default();
        let mut v = 0.0;
        for _ in 0..44100 {
            v = l.next(0, 0.01, 0.0, 44100.0, &mut rand);
        }
        assert!(v.abs() < 0.07, "{v}");
    }

    #[test]
    fn echo_with_lfo_is_finite() {
        let mut t = Tape::new(44100);
        let s = Settings { lfo_shape: 5, lfo_rate: 2.0, lfo_depth: 30.0, lfo_slew: 0.3, ..Settings::default() };
        let mut buf: Vec<[f32; 2]> = (0..44100 * 2).map(|i| { let v = if i % 5000 < 50 { 0.8 } else { 0.0 }; [v, v] }).collect();
        t.process(&s, &mut buf, 22050.0);
        assert!(buf.iter().all(|f| f[0].is_finite() && f[0].abs() < 4.0));
    }
}
