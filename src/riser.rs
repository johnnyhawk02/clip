//! Riser: a one-knob build-up. Turning Amount up thins the sound with a resonant high-pass,
//! drives it into tape (the Tape device's hysteresis model), widens it, breathes a little noise
//! in under it and sends more and more of it into a tempo-synced delay and a long reverb, so it
//! lifts and washes out ahead of a drop. Each of those follows Amount on its own curve, so the
//! one knob moves them all together.

use serde::{Deserialize, Serialize};

use crate::fx;
use crate::tape;

/// Riser settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// 0..1: how far into the build-up.
    pub amount: f32,
    /// Printed into a selection: rise from nothing to `amount` across it, rather than staying put.
    #[serde(default)]
    pub ramp: bool,
    pub on: bool,
    /// Folded down to its title in the device view (Live: double-click the title bar).
    #[serde(default)]
    pub folded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { amount: 0.5, ramp: true, on: true, folded: false }
    }
}

/// The high-pass sweep's ends (Hz), and how long Amount takes to glide to a new value (s).
const LOW_HZ: f32 = 20.0;
const HIGH_HZ: f32 = 1500.0;
const GLIDE: f32 = 0.03;

/// A two-pole state-variable filter (topology-preserving, so it can sweep without blowing up).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// High-pass output for `x`, with `g` = tan(π fc / sr) and `k` = 1 / Q.
    fn high(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        x - k * v1 - v2
    }
}

pub struct Riser {
    rate: f32,
    /// Amount after its glide.
    x: f32,
    hp: [Svf; 2],
    sat: tape::Saturator,
    /// The input's level (for the noise to follow), and the noise's seed.
    env: f32,
    seed: u32,
    delay: fx::Delay,
    reverb: fx::Reverb,
    /// What goes to the delay and reverb this block, and what comes back (kept, so the audio
    /// thread doesn't allocate once they're grown).
    wet: Vec<[f32; 2]>,
    echoes: Vec<[f32; 2]>,
    verb: Vec<[f32; 2]>,
}

impl Riser {
    pub fn new(rate: u32) -> Riser {
        let mut sat = tape::Saturator::new(rate);
        sat.set_bias(0.7);
        Riser {
            rate: rate as f32,
            x: 0.0,
            hp: [Svf::default(); 2],
            sat,
            env: 0.0,
            seed: 0x2545_f491,
            delay: fx::Delay::new(rate),
            reverb: fx::Reverb::new(rate),
            wet: Vec::new(),
            echoes: Vec::new(),
            verb: Vec::new(),
        }
    }

    /// `beat` is a beat's length in frames.
    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]], beat: f64) {
        let target = s.amount.clamp(0.0, 1.0);
        let glide = 1.0 - (-1.0 / (GLIDE * self.rate)).exp();
        let env_rel = (-1.0 / (0.2 * self.rate)).exp();
        let nyq = 0.45 * self.rate;
        self.wet.clear();
        for f in main.iter_mut() {
            self.x += (target - self.x) * glide;
            let x = self.x;
            // Level of what comes in: fast up, slow down.
            let lvl = f[0].abs().max(f[1].abs());
            self.env = if lvl > self.env { lvl } else { lvl + (self.env - lvl) * env_rel };
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (self.seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let hiss = noise * self.env * 0.2 * x * x * x;
            // The sweep: exponential in Amount, getting more resonant as it goes up.
            let fc = (LOW_HZ * (HIGH_HZ / LOW_HZ).powf(x.powf(1.3))).min(nyq);
            let g = (std::f32::consts::PI * fc / self.rate).tan();
            let k = 1.0 / (0.7 + 2.3 * x * x);
            // Tape: driven harder as it rises, faded in over the first quarter so none is clean.
            let drive = 0.2 + 0.8 * x;
            let push = 1.0 + 1.5 * x;
            let blend = (x * 4.0).min(1.0);
            // (The filter fades in over the first tenth too, so at none the sound comes through as it was.)
            let in_hp = (x * 10.0).min(1.0);
            let mut y = [0.0f32; 2];
            for c in 0..2 {
                let src = f[c] + hiss;
                let h = self.hp[c].high(src, g, k);
                let v = src + (h - src) * in_hp;
                let t = self.sat.record(c, v * push, drive) / push;
                y[c] = v + (t - v) * blend;
            }
            // Widen: more of the sides.
            let (m, sd) = (0.5 * (y[0] + y[1]), 0.5 * (y[0] - y[1]) * (1.0 + x));
            let y = [m + sd, m - sd];
            // Into the wash, more and more, while the dry sound backs off a little under it.
            let send = 0.9 * x * x.sqrt();
            self.wet.push([y[0] * send, y[1] * send]);
            let dry = 1.0 - 0.3 * x * x;
            // A little make-up for the lows the sweep takes away.
            let lift = 1.0 + 0.6 * x;
            *f = [y[0] * dry * lift, y[1] * dry * lift];
        }
        let x = self.x;
        let ds = fx::DelaySettings { beats: 0.5, feedback: 0.25 + 0.45 * x, ping_pong: true, low_cut: 250.0, high_cut: 7000.0, mix: 1.0, on: true, folded: false };
        let rs = fx::ReverbSettings { size: 0.55 + 0.43 * x, damping: 0.35, width: 1.0, predelay: 15.0, mix: 1.0, on: true, folded: false };
        // Echoes, then the echoes and the sound into the reverb.
        self.echoes.clear();
        self.echoes.extend_from_slice(&self.wet);
        self.delay.process(&ds, &mut self.echoes, beat);
        for (w, e) in self.wet.iter_mut().zip(&self.echoes) {
            *w = [w[0] + 0.6 * e[0], w[1] + 0.6 * e[1]];
        }
        self.verb.clear();
        self.verb.extend_from_slice(&self.wet);
        self.reverb.process(&rs, &mut self.verb);
        for ((f, e), r) in main.iter_mut().zip(&self.echoes).zip(&self.verb) {
            f[0] += 0.6 * e[0] + 1.4 * r[0];
            f[1] += 0.6 * e[1] + 1.4 * r[1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drums() -> Vec<[f32; 2]> {
        let mut seed = 7u32;
        (0..44100 * 3)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                let t = (i % 22050) as f32 / 44100.0;
                // A kick (decaying 55 Hz) and a hat.
                let kick = 0.8 * (std::f32::consts::TAU * 55.0 * t).sin() * (-t * 12.0).exp();
                let hat = if i % 11025 < 1500 { 0.3 * n * (1.0 - (i % 11025) as f32 / 1500.0) } else { 0.0 };
                [kick + hat, kick + hat]
            })
            .collect()
    }

    fn rms(b: &[[f32; 2]]) -> f32 {
        (b.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>() / (2 * b.len()) as f32).sqrt()
    }

    #[test]
    fn levels_through_the_sweep() {
        let x = drums();
        let r0 = rms(&x[44100..]);
        for a in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let mut r = Riser::new(44100);
            let mut b = x.clone();
            for chunk in b.chunks_mut(512) {
                r.process(&Settings { amount: a, ..Settings::default() }, chunk, 22050.0);
            }
            let out = rms(&b[44100..]);
            let peak = b.iter().map(|f| f[0].abs().max(f[1].abs())).fold(0.0, f32::max);
            eprintln!("amount {a}: rms {:.2} dB re input, peak {peak:.2}", 20.0 * (out / r0).log10());
            assert!(out.is_finite() && peak < 4.0);
            // Neither silent nor much louder.
            assert!(out / r0 > 0.3 && out / r0 < 2.5, "amount {a}: {}", out / r0);
        }
    }

    #[test]
    fn nothing_much_happens_at_zero() {
        let x = drums();
        let mut r = Riser::new(44100);
        let mut b = x.clone();
        r.process(&Settings { amount: 0.0, ..Settings::default() }, &mut b, 22050.0);
        let diff: Vec<[f32; 2]> = b.iter().zip(&x).map(|(a, b)| [a[0] - b[0], a[1] - b[1]]).collect();
        let d = rms(&diff[44100..]) / rms(&x[44100..]);
        eprintln!("at zero: difference {:.1} dB", 20.0 * d.log10());
        assert!(d < 0.1);
    }
}
