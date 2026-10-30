//! Opto, an optical leveller in the manner of the LA-2A: a light and a light-dependent resistor
//! do the turning down, so it moves smoothly and lets go slowly, and the longer and harder it has
//! been working, the more slowly it lets go.
//!
//! Detection is feedback, as in the hardware: the detector hears the output (after the cell),
//! full-wave rectified and smoothed (Eichas and Zölzer's peak detector, eq. 9: the LED drive),
//! both sides linked. The gain computer is `dynamics::feedback_reduction`, a soft knee in dB set so
//! the loop settles at 3:1 (Compress) or about 12:1 (Limit: set to 10.5, see the ratio test).
//! Peak Reduction moves the threshold down.
//!
//! The cell follows Eichas and Zölzer, "Modeling of an optocoupler-based audio dynamic range
//! control circuit" (SPIE 2016): their measurements show the cell darkening quickly (the
//! resistance falls within ~5 ms, the drive capacitor setting the attack) but brightening ~100
//! times more slowly, and they smooth the gain with weighted first-order low-passes. Here the
//! reduction (dB, the log of the resistance) is the weighted sum of two such stages: a fast one
//! (10 ms attack, 30 ms release) and a slow one (150 ms attack, so short peaks barely reach it;
//! a 1–15 s release). The slow stage's release lengthens with a memory of how much reduction
//! there has been lately (charged over ~5 s, forgotten over ~30 s), as the cell's phosphor
//! and photoresistor do. So it recovers about half way in ~60 ms, then creeps back.
//!
//! Colour is the valve stage after the cell: Tape's record-head saturation (oversampled
//! hysteresis) at a gentle drive, before the make-up gain. No look-ahead, so no latency.

use serde::{Deserialize, Serialize};

use crate::dynamics::{self, Smoother};
use crate::tape;

/// Opto's settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// 0..1: how far the threshold comes down (how much it levels).
    pub peak_reduction: f32,
    /// Make-up gain, dB.
    pub gain: f32,
    /// Limit: a higher ratio and a harder knee.
    pub limit: bool,
    /// 0..1: valve and transformer warmth.
    pub colour: f32,
    /// 0..1: dry against wet.
    pub mix: f32,
    pub on: bool,
    /// Folded down to its title in the device view (Live: double-click the title bar).
    #[serde(default)]
    pub folded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { peak_reduction: 0.4, gain: 0.0, limit: false, colour: 0.3, mix: 1.0, on: true, folded: false }
    }
}

impl Settings {
    /// Threshold (dB, full-scale sine peak = 0): +6 at no Peak Reduction down to −44 at full.
    fn threshold(&self) -> f32 {
        6.0 - 50.0 * self.peak_reduction.clamp(0.0, 1.0)
    }

    /// Ratio and knee width (dB).
    fn curve(&self) -> (f32, f32) {
        if self.limit { (10.5, 4.0) } else { (3.0, 10.0) }
    }
}

/// The cell's time constants, ms.
const DET_ATTACK: f32 = 0.3;
const DET_RELEASE: f32 = 20.0;
const FAST_ATTACK: f32 = 10.0;
const FAST_RELEASE: f32 = 30.0;
const SLOW_ATTACK: f32 = 150.0;
/// The slow stage's release, s: this with no memory, up to `SLOW_MAX` with full memory.
const SLOW_MIN: f32 = 1.0;
const SLOW_MAX: f32 = 15.0;
/// How much of the reduction is the fast stage's.
const FAST_SHARE: f32 = 0.6;
/// Memory: full after this much reduction (dB) held, charged and forgotten over these (s).
const MEM_FULL: f32 = 10.0;
const MEM_CHARGE: f32 = 5.0;
const MEM_FORGET: f32 = 30.0;

/// The light and the photoresistor: reduction in dB from the gain computer's target.
#[derive(Clone, Copy, Default)]
pub(crate) struct Cell {
    fast: Smoother,
    slow: Smoother,
    pub(crate) memory: f32,
}

struct Coefs {
    fast_a: f32,
    fast_r: f32,
    slow_a: f32,
    mem_up: f32,
    mem_down: f32,
    rate: f32,
}

impl Coefs {
    fn new(rate: f32) -> Coefs {
        Coefs {
            fast_a: dynamics::coef(FAST_ATTACK, rate),
            fast_r: dynamics::coef(FAST_RELEASE, rate),
            slow_a: dynamics::coef(SLOW_ATTACK, rate),
            mem_up: dynamics::coef(MEM_CHARGE * 1000.0, rate),
            mem_down: dynamics::coef(MEM_FORGET * 1000.0, rate),
            rate,
        }
    }
}

impl Cell {
    fn step(&mut self, target: f32, k: &Coefs) -> f32 {
        let fast = self.fast.next(target, k.fast_a, k.fast_r);
        // (The release is many thousands of samples long, so 1 − 1/n stands in for e^(−1/n).)
        let slow_r = 1.0 - 1.0 / ((SLOW_MIN + (SLOW_MAX - SLOW_MIN) * self.memory) * k.rate);
        let slow = self.slow.next(target, k.slow_a, slow_r);
        let m = (target / MEM_FULL).clamp(0.0, 1.0);
        let a = if m > self.memory { k.mem_up } else { k.mem_down };
        self.memory = a * self.memory + (1.0 - a) * m;
        FAST_SHARE * fast + (1.0 - FAST_SHARE) * slow
    }
}

pub struct Opto {
    k: Coefs,
    det: Smoother,
    cell: Cell,
    /// The reduction now, dB (for the meter too).
    pub gr: f32,
    sat: tape::Saturator,
    det_a: f32,
    det_r: f32,
}

impl Opto {
    pub fn new(rate: u32) -> Opto {
        let mut sat = tape::Saturator::new(rate);
        sat.set_bias(0.8);
        let r = rate as f32;
        Opto { k: Coefs::new(r), det: Smoother::default(), cell: Cell::default(), gr: 0.0, sat, det_a: dynamics::coef(DET_ATTACK, r), det_r: dynamics::coef(DET_RELEASE, r) }
    }

    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]]) {
        let t = s.threshold();
        let (ratio, knee) = s.curve();
        let makeup = dynamics::from_db(s.gain);
        let colour = s.colour.clamp(0.0, 1.0);
        let (drive, blend) = (0.35 * colour, (colour * 3.0).min(1.0));
        let mix = s.mix.clamp(0.0, 1.0);
        for f in main.iter_mut() {
            let g = dynamics::from_db(-self.gr);
            let y = [f[0] * g, f[1] * g];
            // The detector hears the output: rectified, both sides, smoothed.
            let env = self.det.next(y[0].abs().max(y[1].abs()), self.det_a, self.det_r);
            let target = dynamics::feedback_reduction(dynamics::to_db(env), t, ratio, knee);
            self.gr = self.cell.step(target, &self.k);
            for c in 0..2 {
                let v = if colour > 0.0 { y[c] + (self.sat.record(c, y[c], drive) - y[c]) * blend } else { y[c] };
                f[c] = f[c] + (v * makeup - f[c]) * mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48000;

    fn tone(amp: f32, secs: f32, from: usize) -> Vec<[f32; 2]> {
        let n = (secs * SR as f32) as usize;
        (0..n).map(|i| {
            let v = amp * (2.0 * std::f32::consts::PI * 1000.0 * (from + i) as f32 / SR as f32).sin();
            [v, v]
        }).collect()
    }

    /// Run `blocks` of (amplitude, seconds) through, returning the reduction at every sample.
    fn run(s: &Settings, blocks: &[(f32, f32)]) -> (Opto, Vec<f32>) {
        let mut o = Opto::new(SR);
        let mut gr = Vec::new();
        let mut at = 0;
        for &(amp, secs) in blocks {
            let mut b = tone(amp, secs, at);
            at += b.len();
            for f in b.chunks_mut(1) {
                o.process(s, f);
                gr.push(o.gr);
            }
        }
        (o, gr)
    }

    fn ms(n: usize) -> f32 {
        n as f32 * 1000.0 / SR as f32
    }

    fn plain() -> Settings {
        Settings { colour: 0.0, ..Default::default() }
    }

    /// Settled level in to level out (dB): the ratio, well over the threshold.
    #[test]
    fn ratio() {
        // (Limit, set to 10.5:1, measures firmer: its 4 dB knee and the detector's ripple, multiplied
        // by the loop's high gain, push the settled reduction up a little more at higher levels.)
        for (limit, lo, hi) in [(false, 2.6, 3.4), (true, 10.0, 16.0)] {
            let s = Settings { limit, peak_reduction: 1.0, ..plain() };
            let level = |db: f32| {
                let (_, gr) = run(&s, &[(dynamics::from_db(db), 3.0)]);
                db - gr.last().unwrap()
            };
            let r = 10.0 / (level(-10.0) - level(-20.0));
            eprintln!("ratio {} -> {r:.2}", if limit { "limit" } else { "compress" });
            assert!((lo..=hi).contains(&r), "{r}");
        }
    }

    #[test]
    fn attack_about_10_ms() {
        let (_, gr) = run(&Settings { peak_reduction: 0.8, ..plain() }, &[(1.0, 0.5)]);
        // Against the reduction 40 ms in (before the slow stage has much of a say).
        let fin = gr[(0.04 * SR as f32) as usize];
        let t63 = ms(gr.iter().position(|&g| g >= 0.63 * fin).unwrap());
        let t90 = ms(gr.iter().position(|&g| g >= 0.9 * fin).unwrap());
        eprintln!("attack: {fin:.1} dB, 63% in {t63:.1} ms, 90% in {t90:.1} ms");
        assert!((3.0..=15.0).contains(&t63), "{t63}");
        assert!(t90 < 30.0, "{t90}");
    }

    /// Time from the drop to `frac` of the reduction recovered, ms.
    fn recovery(gr: &[f32], drop: usize, frac: f32) -> f32 {
        let start = gr[drop - 1];
        ms(gr[drop..].iter().position(|&g| g <= start * (1.0 - frac)).unwrap_or(gr.len() - drop))
    }

    #[test]
    fn two_stage_release_with_memory() {
        let s = Settings { peak_reduction: 0.8, ..plain() };
        let quiet = dynamics::from_db(-60.0);
        let (_, short) = run(&s, &[(1.0, 1.0), (quiet, 20.0)]);
        let (_, long) = run(&s, &[(1.0, 15.0), (quiet, 20.0)]);
        let (ds, dl) = ((1.0 * SR as f32) as usize, (15.0 * SR as f32) as usize);
        let half_short = recovery(&short, ds, 0.5);
        let half_long = recovery(&long, dl, 0.5);
        let most_short = recovery(&short, ds, 0.9) / 1000.0;
        let most_long = recovery(&long, dl, 0.9) / 1000.0;
        eprintln!("release after 1 s: 50% in {half_short:.0} ms, 90% in {most_short:.2} s; after 15 s: 50% in {half_long:.0} ms, 90% in {most_long:.2} s (from {:.1} / {:.1} dB)", short[ds - 1], long[dl - 1]);
        assert!((40.0..=80.0).contains(&half_short), "{half_short}");
        assert!((40.0..=100.0).contains(&half_long), "{half_long}");
        // The slow tail: seconds, and much longer after a long hard stretch.
        assert!((1.0..=6.0).contains(&most_short), "{most_short}");
        assert!(most_long > 2.0 * most_short && most_long < 30.0, "{most_long}");
    }

    #[test]
    fn short_peaks_let_go_quickly() {
        let s = Settings { peak_reduction: 0.8, ..plain() };
        let quiet = dynamics::from_db(-60.0);
        let (_, gr) = run(&s, &[(1.0, 0.03), (quiet, 3.0)]);
        let (_, long) = run(&s, &[(1.0, 1.0), (quiet, 3.0)]);
        let d = (0.03 * SR as f32) as usize;
        let most = recovery(&gr, d, 0.9);
        let most_long = recovery(&long, SR as usize, 0.9);
        eprintln!("after a 30 ms peak: 90% back in {most:.0} ms (after 1 s: {most_long:.0} ms)");
        assert!(most < 1000.0 && most < most_long / 2.0, "{most}");
    }

    #[test]
    fn stays_bounded_and_quiet_passes() {
        for limit in [false, true] {
            let s = Settings { limit, peak_reduction: 1.0, colour: 1.0, gain: 10.0, ..Default::default() };
            let mut o = Opto::new(SR);
            let mut b = tone(2.0, 2.0, 0);
            o.process(&s, &mut b);
            // (The first few milliseconds are through before it has turned down.)
            assert!(b.iter().all(|f| f[0].is_finite() && f[0].abs() < 2.0 * 3.2 * 1.2));
            assert!(b[48000..].iter().all(|f| f[0].abs() < 0.3));
        }
        // Under the threshold nothing changes (colour off).
        let mut o = Opto::new(SR);
        let src = tone(0.01, 0.5, 0);
        let mut b = src.clone();
        o.process(&Settings { peak_reduction: 0.0, ..plain() }, &mut b);
        assert!(src.iter().zip(&b).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6));
    }
}
