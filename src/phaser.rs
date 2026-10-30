//! Phaser: the 1974 MXR Phase 90, after Giampiccolo et al., "Wave Digital Model of the MXR
//! Phase 90 Based on a Time-Varying Resistor Approximation of JFET Elements" (DAFx24). Four
//! first-order all-pass stages (eq. 8) whose break frequency (eq. 10) is set by a JFET acting
//! as a resistor in parallel with R5: its channel resistance follows the LFO on its gate and
//! the signal across it (eqs. 14-16, with the 2N5952's values from Table 1, and the voltage
//! across it taken from the previous sample as the paper does). The shifted signal is summed
//! with the dry one, so the notches sweep. The buffers' high-passes come from figs. 1 and 3.

use serde::{Deserialize, Serialize};

/// Phaser settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// LFO speed, Hz (the pedal's one knob).
    pub speed: f32,
    /// 0..1: wet against dry. At 1 it's the pedal (which already mixes in the dry signal).
    pub mix: f32,
    pub on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { speed: 0.5, mix: 1.0, on: true }
    }
}

/// 2N5952 JFET (Table 1).
const VP: f64 = -2.021;
const IS0: f64 = 5.367e-3;
const LAMBDA: f64 = 4e-3;
/// Shifting unit (fig. 2): R5 and C2.
const R5: f64 = 24e3;
const C2: f64 = 47e-9;
/// The LFO on the gates, relative to Vref (5.1 V): the paper's 3.25 V ± 0.15 V triangle with a
/// 65% rising part.
const LFO_CENTRE: f64 = 3.25 - 5.1;
const LFO_DEPTH: f64 = 0.15;
const LFO_DUTY: f64 = 0.65;
const STAGES: usize = 4;

/// The JFET's drain-source resistance for gate voltage `vgs` and channel voltage `v2`
/// (eqs. 14-16; the channel conducts either way, so by its size).
fn r_jfet(vgs: f64, v2: f64) -> f64 {
    let v2 = v2.abs();
    let over = vgs - VP;
    if over < 0.0 {
        f64::INFINITY
    } else if over >= v2 {
        VP * VP / (2.0 * IS0 * (over - v2 / 2.0)).max(1e-12)
    } else {
        // (At v2 → 0 this joins the ohmic region's resistance; v2 > 0 here.)
        v2 / (IS0 * (1.0 - vgs / VP).powi(2) * (1.0 + LAMBDA * v2)).max(1e-12)
    }
}

#[derive(Clone, Copy, Default)]
struct Side {
    /// All-pass states, and the voltage across each JFET a sample ago.
    ap: [f64; STAGES],
    v2: [f64; STAGES],
    /// Input buffer and output high-pass states (x, y).
    hp_in: (f64, f64),
    hp_out: (f64, f64),
}

pub struct Phaser {
    rate: f64,
    lfo: f64,
    sides: [Side; 2],
}

impl Phaser {
    pub fn new(rate: u32) -> Phaser {
        Phaser { rate: rate as f64, lfo: 0.0, sides: [Side::default(); 2] }
    }

    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]]) {
        let ts = 1.0 / self.rate;
        // Input buffer: C1 into R1 + R2 (33 Hz); output: C3 into R7 ∥ R6 (the paper's 22 Hz).
        let hp = |fc: f64| (-2.0 * std::f64::consts::PI * fc * ts).exp();
        let (a_in, a_out) = (hp(1.0 / (2.0 * std::f64::consts::PI * (10e3 + 470e3) * 0.01e-6)), hp(22.0));
        let step = s.speed.clamp(0.05, 12.0) as f64 * ts;
        let mix = s.mix.clamp(0.0, 1.0);
        for f in main.iter_mut() {
            // Triangle LFO, rising for 65% of its cycle.
            self.lfo = (self.lfo + step) % 1.0;
            let tri = if self.lfo < LFO_DUTY { self.lfo / LFO_DUTY } else { 1.0 - (self.lfo - LFO_DUTY) / (1.0 - LFO_DUTY) };
            let vgs = LFO_CENTRE + LFO_DEPTH * (2.0 * tri - 1.0);
            for (c, side) in self.sides.iter_mut().enumerate() {
                let x = f[c] as f64;
                let v1 = a_in * (side.hp_in.1 + x - side.hp_in.0);
                side.hp_in = (x, v1);
                let mut y = v1;
                for k in 0..STAGES {
                    // Break frequency where the stage shifts by π/2 (eq. 10 with ϕ = π/2).
                    let rj = r_jfet(vgs, side.v2[k]);
                    let r = if rj.is_finite() { R5 * rj / (R5 + rj) } else { R5 };
                    let wb = 1.0 / (r * C2);
                    let t = (wb * ts / 2.0).tan();
                    let c1 = -(1.0 - t) / (1.0 + t);
                    // A(z) = (c1 + z⁻¹) / (1 + c1 z⁻¹), transposed direct form.
                    let out = c1 * y + side.ap[k];
                    side.ap[k] = y - c1 * out;
                    // The voltage across the JFET: the stage's input high-passed by C2 into R,
                    // which is half the input less half the all-pass output.
                    side.v2[k] = 0.5 * (y - out);
                    y = out;
                }
                // Output stage: dry and shifted summed equally, then the 22 Hz high-pass.
                let sum = 0.5 * (v1 + y);
                let o = a_out * (side.hp_out.1 + sum - side.hp_out.0);
                side.hp_out = (sum, o);
                f[c] = (x * (1.0 - mix as f64) + o * mix as f64) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jfet_resistance_sweeps_and_joins_up() {
        // Across the LFO's range the channel goes from almost open to a few kΩ.
        let lo = r_jfet(LFO_CENTRE - LFO_DEPTH, 0.0);
        let hi = r_jfet(LFO_CENTRE + LFO_DEPTH, 0.0);
        eprintln!("R_jfet {lo:.0} .. {hi:.0} Ω");
        assert!(hi < lo && hi > 100.0);
        // The ohmic and saturated forms meet where the regions do.
        let vgs = -1.8;
        let edge = vgs - VP;
        let (a, b) = (r_jfet(vgs, edge - 1e-6), r_jfet(vgs, edge + 1e-6));
        assert!((a - b).abs() / a < 0.05, "{a} vs {b}");
    }

    #[test]
    fn notch_moves() {
        // A sine near the sweep's middle goes up and down in level as the notches pass it.
        let mut p = Phaser::new(44100);
        let s = Settings { speed: 2.0, ..Settings::default() };
        let mut buf: Vec<[f32; 2]> = (0..44100).map(|i| {
            let v = 0.5 * (std::f32::consts::TAU * 700.0 * i as f32 / 44100.0).sin();
            [v, v]
        }).collect();
        p.process(&s, &mut buf);
        let env: Vec<f32> = buf[4410..].chunks(441).map(|c| c.iter().map(|f| f[0].abs()).fold(0.0, f32::max)).collect();
        let (mn, mx) = env.iter().fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        eprintln!("700 Hz level {mn:.3} .. {mx:.3}");
        assert!(mx.is_finite() && mn < mx * 0.6 && mx < 1.0);
    }
}
