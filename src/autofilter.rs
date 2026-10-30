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
//! samples; the dry sound is delayed to match. Cutoff moves in octaves: the envelope follower
//! (`dynamics::Smoother` on the input's peak, in dB over a 48 dB range) up to ±`ENV_OCT`, the LFO
//! (Tape's, with its random shapes and slew) up to ±`LFO_OCT`.

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
    /// Output gain, dB.
    pub output: f32,
    /// 0..1: dry against wet.
    pub mix: f32,
    pub on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { circuit: 0, kind: 0, freq: 1000.0, res: 0.3, drive: 0.0, env_amount: 0.0, env_attack: 6.0, env_release: 200.0, lfo_amount: 0.0, lfo_rate: 1.0, lfo_shape: 0, lfo_slew: 0.0, output: 0.0, mix: 1.0, on: true }
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
    lfo: tape::Lfo,
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
            lfo: tape::Lfo::default(),
            seed: 0x2545_f491,
            p: None,
            dry: vec![[0.0; 2]; oversample::LATENCY],
            dpos: 0,
            cutoff: 1000.0,
        }
    }

    /// Process `main` (stereo frames, in place); the output is `oversample::LATENCY` frames late.
    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]]) {
        let sr = self.rate;
        let target = Params::target(s);
        let mut p = self.p.unwrap_or(target);
        let a = 1.0 - (-1.0 / (SMOOTH_MS * 0.001 * sr)).exp();
        let (env_a, env_r) = (dynamics::coef(s.env_attack.clamp(0.1, 1000.0), sr), dynamics::coef(s.env_release.clamp(1.0, 5000.0), sr));
        let env_oct = s.env_amount.clamp(-1.0, 1.0) * ENV_OCT;
        let lfo_oct = s.lfo_amount.clamp(0.0, 1.0) * LFO_OCT;
        let lfo_rate = s.lfo_rate.clamp(0.01, 40.0) as f64;
        let hp = s.highpass();
        let top = (0.48 * sr).min(22000.0).log2();
        let os = 4.0 * sr as f64;
        for f in main.iter_mut() {
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
            let lfo = if lfo_oct > 0.0 { self.lfo.next(s.lfo_shape, lfo_rate, s.lfo_slew, sr, &mut rand) } else { 0.0 };
            // A whisper of noise, as a circuit has, so it can start oscillating from silence.
            let hiss = 1e-7 * rand() as f64;
            let oct = (p.oct + env_oct * e + lfo_oct * lfo).clamp(20f32.log2(), top);
            self.cutoff = oct.exp2();
            let g = (std::f64::consts::PI * self.cutoff as f64 / os).tan();
            let gg = g / (1.0 + g);
            let (k, gin) = (p.k as f64, p.gin as f64 * IN_SCALE);
            let back = p.gin as f64 / gin;
            let dry = std::mem::replace(&mut self.dry[self.dpos], *f);
            self.dpos = (self.dpos + 1) % self.dry.len();
            for c in 0..2 {
                let side = &mut self.sides[c];
                let x = f[c] as f64 * gin + hiss;
                let y = match s.circuit {
                    // (Only MS2 so far.)
                    _ => self.over[c].run(x, |v| side.step(v, gg, k, hp)),
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
        a.process(s, &mut y);
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
            a.process(&s, &mut y);
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
            let s = Settings { kind, freq: 800.0, res: 1.0, drive: 24.0, lfo_amount: 1.0, lfo_rate: 30.0, lfo_shape: 2, env_amount: 1.0, env_attack: 0.1, env_release: 5.0, output: 0.0, ..Default::default() };
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
                a.process(&s, chunk);
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
        a.process(&s, &mut y);
        let peak = y.iter().enumerate().max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs())).unwrap().0;
        assert_eq!(peak, 10 + oversample::LATENCY);
        assert!(y[peak][0] > 0.15, "{}", y[peak][0]);
    }
}
