//! Auto Filter, as Live calls it: a resonant filter with drive, its cutoff moved by an envelope
//! follower and an LFO. Its circuits are chosen from `CIRCUITS`; so far there is one, MS2, after
//! the Korg MS-20's filter.
//!
//! MS2 is a Sallen-Key filter in the Korg 35 form, 12 dB an octave. In low-pass, the sound goes
//! through two one-pole low-passes, and the output is fed back, `k` times over and through a
//! one-pole high-pass, into the second's input. So with each stage 1/(1+s):
//!   H(s) = 1 / (s² + (2 − k)s + 1),
//! a Q of 1/(2 − k): self-oscillation at k = 2. High-pass is the mirror (high-passes for the two
//! stages, a low-pass in the feedback), s² over the same. Each one-pole is a zero-delay (TPT)
//! integrator after Zavalishin, "The Art of VA Filter Design" (ch. 3; the Sallen-Key, ch. 5),
//! and the loop is solved for each sample without a unit delay in it.
//!
//! The MS-20's character is in its clipping (Stinchcombe, "A study of the Korg MS10 & MS20
//! filters", 2006): diodes limit the feedback, and the stage they feed saturates too, so the
//! resonance and the sound fight over the same headroom. Here the feedback goes through a diode
//! pair (an asinh: soft, unbounded, as a diode clipper's log) and the second stage's input
//! through a bounded soft clipper, x/√(1+x²). The loop equation for that input u,
//!   u = T(a + D(k(p·u + q))),
//! is solved by Newton's method from the last sample's u, usually in two steps. (p ≤ ¼ and both
//! curves' slopes are at most 1, so with k < 4 its derivative is at least 1 − k/4: it always has
//! one root and Newton finds it fast, however fast the cutoff moves.) Past k = 2 it oscillates, and the clippers settle
//! how loud: driven, it screams and tears rather than ringing as a clean sine.
//!
//! All of it runs 4x oversampled (`oversample::Over4`), so its latency is `oversample::LATENCY`
//! samples; the dry sound is delayed to match. Cutoff moves in octaves, by three things:
//! - the envelope follower (`dynamics::Smoother` on the input's peak, in dB over a 48 dB range),
//!   up to ±`ENV_OCT`;
//! - an envelope (ADSR), up to ±`ENV_OCT`, triggered on a grid of the song's beats or by hits in
//!   the input (a fast follower jumping 6 dB over a slow one; the gate is held while it stays
//!   over);
//! - the LFO (Tape's, with its random shapes and slew), up to ±`LFO_OCT`: in Hz, or synced, its
//!   phase then taken from the song position so it stays on the beat however playback starts or
//!   loops. Phase offsets the right side's. Drift lets it wander out of time and back: a phase
//!   offset that holds at zero for a stretch (locked), glides out to a random offset (up to half a
//!   cycle), then glides back, every three stretches of two bars (four seconds in Hz). It is a
//!   function of the song position, so it repeats the same each time round; the synced envelope
//!   drifts with it.

use serde::{Deserialize, Serialize};

use crate::dynamics::{self, Smoother};
use crate::oversample::{self, Over4};
use crate::tape;

/// The circuits, by `circuit`.
pub const CIRCUITS: [&str; 1] = ["MS2"];
/// The filter types, by `kind`.
pub const KINDS: [&str; 2] = ["Lowpass", "Highpass"];

/// Auto Filter's settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub circuit: u8,
    /// Lowpass or Highpass (`KINDS`).
    pub kind: u8,
    /// Cutoff, Hz.
    pub freq: f32,
    /// 0..1: past 90% it oscillates on its own.
    pub res: f32,
    /// Gain into the filter, dB.
    pub drive: f32,
    /// −1..1: how far (±`ENV_OCT` octaves) the envelope follower moves the cutoff.
    pub env_amount: f32,
    /// The follower's attack and release, ms.
    pub env_attack: f32,
    pub env_release: f32,
    /// 0..1: how far (±`LFO_OCT` octaves) the LFO moves the cutoff.
    pub lfo_amount: f32,
    /// LFO rate (Hz), shape (`tape::LFO_SHAPES`) and slew (0..1).
    pub lfo_rate: f32,
    pub lfo_shape: u8,
    pub lfo_slew: f32,
    /// LFO synced to the song: one cycle every `lfo_beats` beats (from `SYNC_RATES`).
    #[serde(default)]
    pub lfo_sync: bool,
    #[serde(default = "one")]
    pub lfo_beats: f32,
    /// 0..1: how far it drifts out of time and back.
    #[serde(default)]
    pub lfo_drift: f32,
    /// The right side's LFO phase ahead of the left's, degrees.
    #[serde(default)]
    pub lfo_phase: f32,
    /// The envelope: −1..1 of ±`ENV_OCT` octaves, its trigger (`TRIGGERS`), its grid in beats
    /// (`SYNC_RATES`), attack, decay and release (ms) and sustain (0..1).
    #[serde(default)]
    pub gen_amount: f32,
    #[serde(default)]
    pub gen_trigger: u8,
    #[serde(default = "one")]
    pub gen_beats: f32,
    #[serde(default = "gen_attack")]
    pub gen_attack: f32,
    #[serde(default = "gen_decay")]
    pub gen_decay: f32,
    #[serde(default)]
    pub gen_sustain: f32,
    #[serde(default = "gen_release")]
    pub gen_release: f32,
    /// Output gain, dB.
    pub output: f32,
    /// 0..1: dry against wet.
    pub mix: f32,
    pub on: bool,
}

fn one() -> f32 {
    1.0
}

fn gen_attack() -> f32 {
    2.0
}

fn gen_decay() -> f32 {
    150.0
}

fn gen_release() -> f32 {
    100.0
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            circuit: 0,
            kind: 0,
            freq: 1000.0,
            res: 0.3,
            drive: 0.0,
            env_amount: 0.0,
            env_attack: 6.0,
            env_release: 200.0,
            lfo_amount: 0.0,
            lfo_rate: 1.0,
            lfo_shape: 0,
            lfo_slew: 0.0,
            lfo_sync: false,
            lfo_beats: 1.0,
            lfo_drift: 0.0,
            lfo_phase: 0.0,
            gen_amount: 0.0,
            gen_trigger: 0,
            gen_beats: 1.0,
            gen_attack: gen_attack(),
            gen_decay: gen_decay(),
            gen_sustain: 0.0,
            gen_release: gen_release(),
            output: 0.0,
            mix: 1.0,
            on: true,
        }
    }
}

/// Synced lengths, as Live names them, in beats (a bar taken as four).
pub const SYNC_RATES: [(&str, f32); 16] = [
    ("1/64", 0.0625),
    ("1/32", 0.125),
    ("1/16T", 0.25 * 2.0 / 3.0),
    ("1/16", 0.25),
    ("1/16D", 0.375),
    ("1/8T", 0.5 * 2.0 / 3.0),
    ("1/8", 0.5),
    ("1/8D", 0.75),
    ("1/4T", 2.0 / 3.0),
    ("1/4", 1.0),
    ("1/4D", 1.5),
    ("1/2", 2.0),
    ("1 Bar", 4.0),
    ("2 Bars", 8.0),
    ("4 Bars", 16.0),
    ("8 Bars", 32.0),
];

/// What starts the envelope, by `gen_trigger`.
pub const TRIGGERS: [&str; 3] = ["Off", "Sync", "Input"];

/// Drift: beats (synced) or seconds (Hz) in each stretch of holding, gliding out and gliding back.
const DRIFT_BEATS: f64 = 8.0;
const DRIFT_SECS: f64 = 4.0;

/// A repeatable random number in −1..1 for `n`.
fn hash(n: i64) -> f64 {
    let mut z = (n as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Drift's phase offset (cycles) at `t` stretches in, for Drift `amount`: locked at zero through
/// one stretch, gliding out to a random offset through the next, back to zero through the third.
pub fn drift(t: f64, amount: f32) -> f64 {
    if amount <= 0.0 {
        return 0.0;
    }
    let n = t.floor() as i64;
    let f = t - t.floor();
    let glide = 0.5 - 0.5 * (std::f64::consts::PI * f).cos();
    let out = |k: i64| 0.5 * amount.clamp(0.0, 1.0) as f64 * hash(k.div_euclid(3));
    match n.rem_euclid(3) {
        0 => 0.0,
        1 => out(n) * glide,
        _ => out(n) * (1.0 - glide),
    }
}

/// Octaves of cutoff swing at full envelope amount and full LFO amount.
pub const ENV_OCT: f32 = 5.0;
pub const LFO_OCT: f32 = 4.0;
/// Feedback at full Resonance, and the Resonance where it reaches 2 and oscillation starts.
/// Just past 2 the oscillation is nearly a sine; well past it the clippers bite and it screams.
const K_MAX: f32 = 3.5;
const RES_OSC: f32 = 0.9;
/// Signal into the circuit at no drive (a full-scale peak just bends the clipper), and the diode
/// pair's knee.
const IN_SCALE: f64 = 0.5;
const DIODE: f64 = 0.5;
/// Newton steps at most (two nearly always do; more only at the very top of Resonance, swept fast).
const NEWTON_MAX: usize = 4;
/// Parameter smoothing, ms.
const SMOOTH_MS: f32 = 5.0;

impl Settings {
    /// The loop gain k for Resonance.
    pub fn k(&self) -> f32 {
        let r = self.res.clamp(0.0, 1.0);
        if r < RES_OSC { 2.0 * (r / RES_OSC).powf(1.3) } else { 2.0 + (K_MAX - 2.0) * (r - RES_OSC) / (1.0 - RES_OSC) }
    }

    pub fn highpass(&self) -> bool {
        self.kind == 1
    }
}

/// The envelope generator's stages.
#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    Attack,
    Decay,
    #[default]
    Release,
}

/// The envelope: an ADSR with analogue-style curves (an attack heading past full, decay and
/// release falling exponentially), and its triggers.
#[derive(Default)]
struct Gen {
    v: f32,
    stage: Stage,
    gate: bool,
    /// The grid step it's in (Sync), and the fast and slow followers (Input).
    step: Option<i64>,
    fast: Smoother,
    slow: Smoother,
}

impl Gen {
    fn trigger(&mut self) {
        self.stage = Stage::Attack;
        self.gate = true;
    }

    fn release(&mut self) {
        self.gate = false;
        self.stage = Stage::Release;
    }

    /// One sample: `k` the attack, decay and release coefficients (per-sample fractions).
    fn next(&mut self, sustain: f32, k: [f32; 3]) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.v += (1.3 - self.v) * k[0];
                if self.v >= 1.0 {
                    self.v = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.v += (sustain - self.v) * k[1],
            Stage::Release => self.v -= self.v * k[2],
        }
        self.v
    }
}

/// A one-pole's per-sample step for time constant `ms`.
fn step_coef(ms: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

/// The linear response (dB) at `f` Hz with the cutoff at `fc` (for the display).
pub fn response(s: &Settings, fc: f32, f: f32) -> f32 {
    let x = f / fc;
    // (Self-oscillating, the peak is drawn as at k just under 2.)
    let d = (2.0 - s.k()).max(0.03);
    let mag = 1.0 / ((1.0 - x * x).powi(2) + (d * x).powi(2)).sqrt();
    dynamics::to_db(if s.highpass() { mag * x * x } else { mag })
}

/// The bounded soft clipper and its slope.
fn clip(w: f64) -> (f64, f64) {
    let r = 1.0 / (1.0 + w * w).sqrt();
    (w * r, r * r * r)
}

/// The diode pair and its slope.
fn diode(v: f64) -> (f64, f64) {
    let x = v / DIODE;
    (DIODE * x.asinh(), 1.0 / (1.0 + x * x).sqrt())
}

/// One side of the MS2 circuit: its three one-poles' states and the last solve.
#[derive(Clone, Default)]
struct Ms2 {
    a: f64,
    b: f64,
    c: f64,
    u: f64,
}

impl Ms2 {
    /// One sample at the oversampled rate: `gg` = g/(1+g) for the cutoff, `k` the feedback.
    fn step(&mut self, x: f64, gg: f64, k: f64, hp: bool) -> f64 {
        // A one-pole with state s, as y = m·x + n (its low-pass or high-pass output).
        let lin = |s: f64, low: bool| if low { (gg, (1.0 - gg) * s) } else { (1.0 - gg, -(1.0 - gg) * s) };
        let adv = |s: &mut f64, x: f64| *s += 2.0 * gg * (x - *s);
        // The first stage needs nothing from the loop.
        let (ma, na) = lin(self.a, !hp);
        let ya = ma * x + na;
        adv(&mut self.a, x);
        // u → second stage → feedback stage: the feedback is p·u + q.
        let (mb, nb) = lin(self.b, !hp);
        let (mc, nc) = lin(self.c, hp);
        let (p, q) = (mc * mb, mc * nb + nc);
        let mut u = self.u;
        for _ in 0..NEWTON_MAX {
            let fb = k * (p * u + q);
            let (d, dd) = diode(fb);
            let (t, dt) = clip(ya + d);
            let step = (u - t) / (1.0 - dt * dd * k * p);
            u -= step;
            if step.abs() < 1e-8 {
                break;
            }
        }
        self.u = u;
        let yb = mb * u + nb;
        adv(&mut self.b, u);
        adv(&mut self.c, yb);
        yb
    }
}

/// Smoothed parameters.
#[derive(Clone, Copy)]
struct Params {
    oct: f32,
    k: f32,
    gin: f32,
    out: f32,
    mix: f32,
}

impl Params {
    fn target(s: &Settings) -> Params {
        let gin = dynamics::from_db(s.drive.clamp(0.0, 24.0));
        let k = s.k();
        Params {
            oct: s.freq.clamp(20.0, 20000.0).log2(),
            k,
            gin,
            // Drive's level is half taken back (the clipping does the rest), and the resonance's
            // a little, so it oscillates at about full scale rather than over it.
            out: dynamics::from_db(s.output.clamp(-36.0, 24.0)) / gin.sqrt() / (1.0 + 0.5 * (k - 1.0).max(0.0)),
            mix: s.mix.clamp(0.0, 1.0),
        }
    }
}

pub struct AutoFilter {
    rate: f32,
    sides: [Ms2; 2],
    over: [Over4; 2],
    env: Smoother,
    lfo: [tape::Lfo; 2],
    /// Cycles run (in Hz) and seconds run (for Drift in Hz).
    free: f64,
    secs: f64,
    eg: Gen,
    seed: u32,
    p: Option<Params>,
    dry: Vec<[f32; 2]>,
    dpos: usize,
    /// The cutoff now, Hz, after the modulation (for the display).
    pub cutoff: f32,
}

impl AutoFilter {
    pub fn new(rate: u32) -> AutoFilter {
        AutoFilter {
            rate: rate as f32,
            sides: Default::default(),
            over: Default::default(),
            env: Smoother::default(),
            lfo: Default::default(),
            free: 0.0,
            secs: 0.0,
            eg: Gen::default(),
            seed: 0x2545_f491,
            p: None,
            dry: vec![[0.0; 2]; oversample::LATENCY],
            dpos: 0,
            cutoff: 1000.0,
        }
    }

    /// Process `main` (stereo frames, in place), which starts at song frame `pos` with a beat
    /// `beat` frames long; the output is `oversample::LATENCY` frames late.
    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]], beat: f64, pos: f64) {
        let sr = self.rate;
        let target = Params::target(s);
        let mut p = self.p.unwrap_or(target);
        let a = 1.0 - (-1.0 / (SMOOTH_MS * 0.001 * sr)).exp();
        let (env_a, env_r) = (dynamics::coef(s.env_attack.clamp(0.1, 1000.0), sr), dynamics::coef(s.env_release.clamp(1.0, 5000.0), sr));
        let env_oct = s.env_amount.clamp(-1.0, 1.0) * ENV_OCT;
        let lfo_oct = s.lfo_amount.clamp(0.0, 1.0) * LFO_OCT;
        let beat = beat.max(1.0);
        let lfo_beats = s.lfo_beats.clamp(0.0625, 32.0) as f64;
        let lfo_rate = if s.lfo_sync { sr as f64 / (lfo_beats * beat) } else { s.lfo_rate.clamp(0.01, 40.0) as f64 };
        let phase_r = (s.lfo_phase.rem_euclid(360.0) / 360.0) as f64;
        let gen_oct = s.gen_amount.clamp(-1.0, 1.0) * ENV_OCT;
        let gen_beats = s.gen_beats.clamp(0.0625, 32.0) as f64;
        // (The attack heads for 1.3, so reaches full in about 1.47 time constants.)
        let gen_k = [step_coef(s.gen_attack.clamp(0.1, 5000.0) / 1.47, sr), step_coef(s.gen_decay.clamp(1.0, 10000.0), sr), step_coef(s.gen_release.clamp(1.0, 10000.0), sr)];
        let gen_sus = s.gen_sustain.clamp(0.0, 1.0);
        let (fast_a, fast_r, slow_a, slow_r) = (dynamics::coef(0.5, sr), dynamics::coef(30.0, sr), dynamics::coef(40.0, sr), dynamics::coef(300.0, sr));
        let hp = s.highpass();
        let top = (0.48 * sr).min(22000.0).log2();
        let os = 4.0 * sr as f64;
        for (i, f) in main.iter_mut().enumerate() {
            p.oct += (target.oct - p.oct) * a;
            p.k += (target.k - p.k) * a;
            p.gin += (target.gin - p.gin) * a;
            p.out += (target.out - p.out) * a;
            p.mix += (target.mix - p.mix) * a;
            // The envelope follower: the input's peak, both sides, as 0..1 over 48 dB.
            let level = self.env.next(f[0].abs().max(f[1].abs()), env_a, env_r);
            let e = ((dynamics::to_db(level) + 48.0) / 48.0).clamp(0.0, 1.0);
            let seed = &mut self.seed;
            let mut rand = || {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            };
            // A whisper of noise, as a circuit has, so it can start oscillating from silence.
            let hiss = 1e-7 * rand() as f64;
            // Where in the song (beats), and Drift's offset (cycles).
            let beats = (pos + i as f64) / beat;
            self.free += lfo_rate / sr as f64;
            self.secs += 1.0 / sr as f64;
            let dr = drift(if s.lfo_sync { beats / DRIFT_BEATS } else { self.secs / DRIFT_SECS }, s.lfo_drift);
            // The envelope's triggers.
            match s.gen_trigger {
                1 => {
                    let at = beats / gen_beats + dr;
                    let step = at.floor() as i64;
                    if self.eg.step != Some(step) {
                        self.eg.step = Some(step);
                        self.eg.trigger();
                    } else if self.eg.gate && at - at.floor() >= 0.5 {
                        self.eg.release();
                    }
                }
                2 => {
                    let peak = f[0].abs().max(f[1].abs());
                    let fast = self.eg.fast.next(peak, fast_a, fast_r);
                    let slow = self.eg.slow.next(peak, slow_a, slow_r);
                    if !self.eg.gate && fast > 2.0 * slow && fast > 0.003 {
                        self.eg.trigger();
                    } else if self.eg.gate && fast < slow {
                        self.eg.release();
                    }
                }
                _ => {
                    if self.eg.gate {
                        self.eg.release();
                    }
                }
            }
            if s.gen_trigger != 1 {
                self.eg.step = None;
            }
            let eg = self.eg.next(gen_sus, gen_k);
            let mut octs = [0.0f32; 2];
            for (c, o) in octs.iter_mut().enumerate() {
                let lfo = if lfo_oct > 0.0 {
                    let base = if s.lfo_sync { beats / lfo_beats } else { self.free };
                    let ph = base + dr + if c == 1 { phase_r } else { 0.0 };
                    // Synced, the random shapes are a function of the cycle, so they repeat on the beat.
                    let cycle = ph.floor() as i64;
                    let mut calls = 0i64;
                    let mut draw = || {
                        calls += 1;
                        if s.lfo_sync { hash(cycle * 4 + calls + 7919 * c as i64) as f32 } else { rand() }
                    };
                    self.lfo[c].at(ph, s.lfo_shape, lfo_rate, s.lfo_slew, sr, &mut draw)
                } else {
                    0.0
                };
                *o = (p.oct + env_oct * e + gen_oct * eg + lfo_oct * lfo).clamp(20f32.log2(), top);
            }
            self.cutoff = octs[0].exp2();
            let gg = octs.map(|o| {
                let g = (std::f64::consts::PI * o.exp2() as f64 / os).tan();
                g / (1.0 + g)
            });
            let (k, gin) = (p.k as f64, p.gin as f64 * IN_SCALE);
            let back = p.gin as f64 / gin;
            let dry = std::mem::replace(&mut self.dry[self.dpos], *f);
            self.dpos = (self.dpos + 1) % self.dry.len();
            for c in 0..2 {
                let side = &mut self.sides[c];
                let x = f[c] as f64 * gin + hiss;
                let y = match s.circuit {
                    // (Only MS2 so far.)
                    _ => self.over[c].run(x, |v| side.step(v, gg[c], k, hp)),
                };
                let wet = (y * back) as f32 * p.out;
                f[c] = dry[c] + (wet - dry[c]) * p.mix;
            }
        }
        self.p = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48000;
    /// 120 BPM.
    const BEAT: f64 = 24000.0;

    fn sine(hz: f32, amp: f32, secs: f32) -> Vec<[f32; 2]> {
        (0..(secs * SR as f32) as usize).map(|i| {
            let v = amp * (std::f32::consts::TAU * hz * i as f32 / SR as f32).sin();
            [v, v]
        }).collect()
    }

    fn rms(b: &[[f32; 2]]) -> f32 {
        (b.iter().map(|f| f[0] * f[0]).sum::<f32>() / b.len().max(1) as f32).sqrt()
    }

    /// Gain (dB) for a quiet sine at `hz`, once settled.
    fn gain(s: &Settings, hz: f32) -> f32 {
        let mut a = AutoFilter::new(SR);
        let x = sine(hz, 0.1, 0.5);
        let mut y = x.clone();
        a.process(s, &mut y, BEAT, 0.0);
        dynamics::to_db(rms(&y[SR as usize / 4..]) / rms(&x[SR as usize / 4..]))
    }

    #[test]
    fn lowpass_and_highpass_responses() {
        let lp = Settings { freq: 1000.0, res: 0.0, ..Default::default() };
        let hp = Settings { kind: 1, ..lp };
        let (lp_lo, lp_hi) = (gain(&lp, 100.0), gain(&lp, 10000.0));
        let (hp_lo, hp_hi) = (gain(&hp, 100.0), gain(&hp, 10000.0));
        let (lp_at, hp_at) = (gain(&lp, 1000.0), gain(&hp, 1000.0));
        eprintln!("LP: 100 Hz {lp_lo:.1} dB, 1 kHz {lp_at:.1}, 10 kHz {lp_hi:.1}; HP: {hp_lo:.1}, {hp_at:.1}, {hp_hi:.1}");
        assert!(lp_lo.abs() < 0.5 && hp_hi.abs() < 0.5);
        // 12 dB an octave: a decade past is about 40 dB down (k = 0 is two plain one-poles).
        assert!(lp_hi < -35.0 && hp_lo < -35.0, "{lp_hi} {hp_lo}");
        // At the cutoff, Q = ½: −6 dB.
        assert!((lp_at + 6.0).abs() < 1.0 && (hp_at + 6.0).abs() < 1.0, "{lp_at} {hp_at}");
        // Resonance raises a peak at the cutoff.
        let peak = gain(&Settings { res: 0.8, ..lp }, 1000.0);
        eprintln!("res 80%: {peak:.1} dB at the cutoff");
        assert!(peak > 5.0, "{peak}");
    }

    #[test]
    fn self_oscillates_at_full_resonance() {
        for (kind, fc) in [(0, 440.0), (1, 440.0), (0, 2000.0)] {
            let s = Settings { kind, freq: fc, res: 1.0, ..Default::default() };
            let mut a = AutoFilter::new(SR);
            // Silence (only its own hiss) for two seconds.
            let mut y = vec![[0.0f32; 2]; 2 * SR as usize];
            a.process(&s, &mut y, BEAT, 0.0);
            let tail = &y[SR as usize..];
            let level = rms(tail);
            let crossings = tail.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
            let hz = crossings as f32 / (tail.len() as f32 / SR as f32);
            let harm = |h: f32| {
                let w = std::f32::consts::TAU * hz * h / SR as f32;
                let (re, im) = tail.iter().enumerate().fold((0.0, 0.0), |(r, m), (i, v)| (r + v[0] * (i as f32 * w).cos(), m + v[0] * (i as f32 * w).sin()));
                2.0 * (re * re + im * im).sqrt() / tail.len() as f32
            };
            let (h1, h3, h5) = (harm(1.0), harm(3.0), harm(5.0));
            eprintln!("{} {fc} Hz: oscillates at {hz:.0} Hz, rms {level:.3}, 3rd {:.1} dB, 5th {:.1} dB, peak {:.2}", KINDS[kind as usize], dynamics::to_db(h3 / h1), dynamics::to_db(h5 / h1), tail.iter().map(|f| f[0].abs()).fold(0.0, f32::max));
            assert!(level > 0.2 && level < 1.0, "{level}");
            assert!((hz / fc - 1.0).abs() < 0.15, "{hz}");
            // Not a clean sine: the clippers put odd harmonics on it.
            if fc < 1000.0 {
                assert!(h3 / h1 > dynamics::from_db(-30.0), "{}", dynamics::to_db(h3 / h1));
            }
        }
    }

    #[test]
    fn stable_swept_fast_and_driven() {
        for kind in [0, 1] {
            let s = Settings { kind, freq: 800.0, res: 1.0, drive: 24.0, lfo_amount: 1.0, lfo_rate: 30.0, lfo_shape: 2, env_amount: 1.0, env_attack: 0.1, env_release: 5.0, output: 0.0, lfo_drift: 1.0, lfo_phase: 90.0, gen_amount: -1.0, gen_trigger: 2, gen_attack: 0.1, gen_decay: 5.0, ..Default::default() };
            let mut a = AutoFilter::new(SR);
            let mut seed = 1u32;
            let mut y: Vec<[f32; 2]> = (0..3 * SR as usize).map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                // Loud noise in bursts, with silences between.
                let v = if (i / 4000) % 2 == 0 { 4.0 * n } else { 0.0 };
                [v, -v]
            }).collect();
            for chunk in y.chunks_mut(97) {
                a.process(&s, chunk, BEAT, 0.0);
            }
            let worst = y.iter().map(|f| f[0].abs().max(f[1].abs())).fold(0.0f32, f32::max);
            eprintln!("{}: peak {worst:.2}", KINDS[kind as usize]);
            assert!(y.iter().all(|f| f.iter().all(|v| v.is_finite() && (*v == 0.0 || v.is_normal()))));
            assert!(worst < 20.0, "{worst}");
            for side in &a.sides {
                assert!([side.a, side.b, side.c, side.u].iter().all(|v| v.is_finite() && v.abs() < 100.0));
            }
        }
    }

    #[test]
    fn newton_converges_on_a_fast_sweep() {
        // Sweep the cutoff 20 Hz → 20 kHz in 50 ms at full resonance with a loud input, and check
        // each solve against the loop equation.
        let mut m = Ms2::default();
        let os = 4.0 * SR as f64;
        let n = (0.05 * os) as usize;
        let mut worst: f64 = 0.0;
        for i in 0..n {
            let fc = 20.0 * 1000f64.powf(i as f64 / n as f64);
            let g = (std::f64::consts::PI * fc / os).tan();
            let gg = g / (1.0 + g);
            let x = 3.0 * (i as f64 * 0.01).sin();
            let k = K_MAX as f64;
            // The equation, from the states before the step.
            let (ya, mb, nb, mc, nc) = (gg * x + (1.0 - gg) * m.a, gg, (1.0 - gg) * m.b, 1.0 - gg, -(1.0 - gg) * m.c);
            m.step(x, gg, k, false);
            let u = m.u;
            let resid = u - clip(ya + diode(k * (mc * (mb * u + nb) + nc)).0).0;
            worst = worst.max(resid.abs());
        }
        eprintln!("worst loop residual {worst:.2e}");
        assert!(worst < 1e-4, "{worst}");
    }

    #[test]
    fn dry_lines_up_with_wet() {
        // Wide open, no resonance, half wet: a click comes out as one click, LATENCY late.
        let s = Settings { freq: 20000.0, res: 0.0, mix: 0.5, ..Default::default() };
        let mut a = AutoFilter::new(SR);
        let mut y = vec![[0.0f32; 2]; 400];
        y[10] = [0.3, 0.3];
        a.process(&s, &mut y, BEAT, 0.0);
        let peak = y.iter().enumerate().max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs())).unwrap().0;
        assert_eq!(peak, 10 + oversample::LATENCY);
        assert!(y[peak][0] > 0.15, "{}", y[peak][0]);
    }

    /// The cutoff at each sample of `n`, run in blocks of `block` from song frame `from`.
    fn cutoffs(s: &Settings, from: f64, n: usize, block: usize) -> Vec<f32> {
        let mut a = AutoFilter::new(SR);
        let mut out = Vec::new();
        let mut at = from;
        let mut y = vec![[0.0f32; 2]; 1];
        while out.len() < n {
            for _ in 0..block.min(n - out.len()) {
                y[0] = [0.0; 2];
                a.process(s, &mut y, BEAT, at);
                at += 1.0;
                out.push(a.cutoff);
            }
        }
        out
    }

    #[test]
    fn synced_lfo_follows_the_song() {
        // A 1/4 saw, two octaves each way, around 1 kHz: a quarter of the way through each beat it
        // is an octave down (500 Hz), half way it is back at 1 kHz.
        let s = Settings { lfo_amount: 0.5, lfo_sync: true, lfo_beats: 1.0, lfo_shape: 3, ..Default::default() };
        let run = cutoffs(&s, 0.0, 5 * BEAT as usize, 512);
        let at = |beats: f64| run[(beats * BEAT) as usize];
        eprintln!("cutoff a quarter into beats 2, 3, 4: {:.0} {:.0} {:.0}; mid-beat {:.0}", at(2.25), at(3.25), at(4.25), at(3.5));
        for b in [2.25, 3.25, 4.25] {
            assert!((at(b).log2() - (1000f32.log2() - 1.0)).abs() < 0.05, "{}", at(b));
        }
        assert!((at(3.5).log2() - 1000f32.log2()).abs() < 0.1, "{}", at(3.5));
        // Started from somewhere else in the song (as after a loop jump), it is in the same place.
        let late = cutoffs(&s, 3.25 * BEAT, (0.5 * BEAT) as usize, 64);
        let i = late.len() - 1;
        let same = run[(3.25 * BEAT) as usize + i];
        assert!((late[i] / same - 1.0).abs() < 0.01, "{} {}", late[i], same);
    }

    #[test]
    fn drift_wanders_and_locks_again() {
        let mut most: f64 = 0.0;
        let mut jump: f64 = 0.0;
        let mut last = 0.0;
        for i in 0..30000 {
            let t = i as f64 / 1000.0;
            let d = drift(t, 1.0);
            // Locked through every third stretch, and never more than half a cycle out.
            if (t.floor() as i64).rem_euclid(3) == 0 {
                assert_eq!(d, 0.0);
            }
            assert!(d.abs() <= 0.5);
            most = most.max(d.abs());
            jump = jump.max((d - last).abs());
            last = d;
        }
        eprintln!("drift: most {most:.2} cycles, biggest step {jump:.4}");
        assert!(most > 0.2, "{most}");
        // It glides: no jumps (a step of 1/1000 of a stretch moves it very little).
        assert!(jump < 0.002, "{jump}");
        assert_eq!(drift(5.5, 0.0), 0.0);
        // The same each time round.
        assert_eq!(drift(7.3, 0.6), drift(7.3, 0.6));
        // Synced with Drift, the LFO is still on the beat in the locked stretches (beats 0-8).
        let s = Settings { lfo_amount: 0.5, lfo_sync: true, lfo_beats: 1.0, lfo_shape: 3, lfo_drift: 1.0, ..Default::default() };
        let run = cutoffs(&s, 0.0, 20 * BEAT as usize, 512);
        let off = |b: f64| (run[(b * BEAT) as usize].log2() - (1000f32.log2() - 1.0)).abs();
        assert!(off(5.25) < 0.05, "{}", off(5.25));
        // ... and somewhere in the next stretch, out of it.
        let out = (9..16).map(|b| off(b as f64 + 0.25)).fold(0.0f32, f32::max);
        eprintln!("drifting: up to {out:.2} octaves off the beat");
        assert!(out > 0.3, "{out}");
    }

    #[test]
    fn envelope_triggers_on_the_grid_and_on_hits() {
        // Sync: an 1/8 grid, up four octaves with a short decay: high just after each eighth, low before.
        let s = Settings { gen_amount: 0.8, gen_trigger: 1, gen_beats: 0.5, gen_attack: 1.0, gen_decay: 40.0, ..Default::default() };
        let run = cutoffs(&s, 0.0, 2 * BEAT as usize, 256);
        let at = |b: f64| run[(b * BEAT) as usize];
        for b in [0.5, 1.0, 1.5] {
            eprintln!("eighth at beat {b}: {:.0} Hz just after, {:.0} Hz just before", at(b + 0.02), at(b - 0.01));
            assert!(at(b + 0.02) > 8.0 * at(b - 0.01), "{b}");
        }
        // Input: each hit in the sound opens it.
        let s = Settings { gen_amount: 0.8, gen_trigger: 2, gen_attack: 1.0, gen_decay: 30.0, ..Default::default() };
        let mut a = AutoFilter::new(SR);
        let mut hits = 0;
        let mut was_high = false;
        for i in 0..SR as usize {
            let t = i % 12000;
            let v = if t < 2000 { 0.5 * (i as f32 * 0.05).sin() * (-(t as f32) / 400.0).exp() } else { 0.0 };
            let mut y = [[v, v]];
            a.process(&s, &mut y, BEAT, i as f64);
            let high = a.cutoff > 4000.0;
            if high && !was_high {
                hits += 1;
            }
            was_high = high;
        }
        assert_eq!(hits, 4);
    }
}
