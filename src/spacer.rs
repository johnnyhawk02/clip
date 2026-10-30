//! Spacer, a sidechain spectral ducker in the manner of Trackspacer: one track turns down only
//! the frequencies another track is using, so the two sit together.
//!
//! The sidechain is split into 32 bands (4th-order band-passes: two biquads in series, 30 Hz to
//! 18 kHz, ~0.3 octave apart), each with a power envelope (Attack / Release). The ducked signal
//! runs through 32 cascaded bell filters at the same centres; each bell cuts by an amount that
//! grows with its sidechain band's level. Zero latency, and with a silent sidechain every bell
//! sits at 0 dB, which is transparent. (A port of the Carve prototype in ~/Developer/Plugins.)

use serde::{Deserialize, Serialize};

pub const BANDS: usize = 32;
const F_LO: f64 = 30.0;
const F_HI: f64 = 18000.0;
/// Samples between bell coefficient updates.
const CTRL: usize = 16;
/// Deepest cut, in dB.
const MAX_CUT: f64 = 30.0;
/// cut = amount * 20 log10(1 + SENS * band amplitude)
const SENS: f64 = 30.0;

/// A track's Spacer settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The track whose sound makes room (the sidechain).
    pub source: usize,
    /// 0..1: how deep the cuts go.
    pub amount: f32,
    /// Only bands between these (Hz) duck.
    pub low_cut: f32,
    pub high_cut: f32,
    /// Envelope times, ms.
    pub attack: f32,
    pub release: f32,
    /// 0 Linked, 1 Left/Right, 2 Mid/Side.
    pub stereo: u8,
    /// Output gain, dB.
    pub output: f32,
    /// Off keeps the settings but passes the track through untouched.
    #[serde(default = "on")]
    pub on: bool,
}

fn on() -> bool {
    true
}

impl Settings {
    pub fn new(source: usize) -> Settings {
        Settings { source, amount: 0.6, low_cut: 30.0, high_cut: 18000.0, attack: 8.0, release: 150.0, stereo: 0, output: 0.0, on: true }
    }
}

/// The ducker's running state: filter memories, envelopes and the current cuts.
pub struct Spacer {
    rate: f64,
    freq: [f64; BANDS],
    cosw: [f64; BANDS],
    alpha: [f64; BANDS],
    /// Band-pass coefficients for the sidechain analysis (b0, b1, b2, a1, a2).
    bp: [[f64; 5]; BANDS],
    /// Per channel: two band-pass stages' state per band.
    sc_z: [[[f64; 4]; BANDS]; 2],
    env: [[f64; BANDS]; 2],
    bell: [[[f64; 5]; BANDS]; 2],
    bell_z: [[[f64; 2]; BANDS]; 2],
    /// Current cut per band and analysis channel, dB (for the display).
    pub cut: [[f32; BANDS]; 2],
    ctr: usize,
    gain: f64,
}

impl Spacer {
    pub fn new(rate: u32) -> Spacer {
        let rate = rate.max(1) as f64;
        let ratio = (F_HI / F_LO).powf(1.0 / (BANDS - 1) as f64);
        let bell_bw = ratio.log2() * 1.1;
        let bp_bw = ratio.log2() * 1.6;
        let alpha_for = |w0: f64, bw: f64| w0.sin() * ((std::f64::consts::LN_2 / 2.0) * bw * (w0 / w0.sin())).sinh();
        let mut s = Spacer {
            rate,
            freq: [0.0; BANDS],
            cosw: [0.0; BANDS],
            alpha: [0.0; BANDS],
            bp: [[0.0; 5]; BANDS],
            sc_z: [[[0.0; 4]; BANDS]; 2],
            env: [[0.0; BANDS]; 2],
            bell: [[[1.0, 0.0, 0.0, 0.0, 0.0]; BANDS]; 2],
            bell_z: [[[0.0; 2]; BANDS]; 2],
            cut: [[0.0; BANDS]; 2],
            ctr: 0,
            gain: 1.0,
        };
        for b in 0..BANDS {
            let f = F_LO * ratio.powi(b as i32);
            let w0 = 2.0 * std::f64::consts::PI * f.min(0.45 * rate) / rate;
            s.freq[b] = f;
            s.cosw[b] = w0.cos();
            s.alpha[b] = alpha_for(w0, bell_bw);
            let alpha = alpha_for(w0, bp_bw);
            let a0 = 1.0 + alpha;
            s.bp[b] = [alpha / a0, 0.0, -alpha / a0, -2.0 * s.cosw[b] / a0, (1.0 - alpha) / a0];
        }
        for c in 0..2 {
            for b in 0..BANDS {
                s.set_bell(c, b, 0.0);
            }
        }
        s
    }

    /// The centre frequency of each band.
    #[cfg(test)]
    pub fn freqs(&self) -> [f64; BANDS] {
        self.freq
    }

    fn set_bell(&mut self, c: usize, b: usize, cut_db: f64) {
        let a = 10f64.powf(-cut_db / 40.0);
        let (alpha, cw) = (self.alpha[b], self.cosw[b]);
        let a0 = 1.0 + alpha / a;
        self.bell[c][b] = [(1.0 + alpha * a) / a0, -2.0 * cw / a0, (1.0 - alpha * a) / a0, -2.0 * cw / a0, (1.0 - alpha / a) / a0];
    }

    /// How much band `b` may duck, given Low Cut / High Cut: 1 inside, fading to 0 over half an
    /// octave outside.
    fn weight(&self, b: usize, lo: f64, hi: f64) -> f64 {
        let f = self.freq[b];
        let mut w: f64 = 1.0;
        if f < lo {
            w = (1.0 - 2.0 * (lo / f).log2()).max(0.0);
        }
        if f > hi {
            w = w.min((1.0 - 2.0 * (f / hi).log2()).max(0.0));
        }
        w
    }

    /// Duck `main` (stereo frames, in place) by the sidechain `sc`, frame for frame.
    pub fn process(&mut self, p: &Settings, main: &mut [[f32; 2]], sc: &[[f32; 2]]) {
        let mode = p.stereo.min(2);
        let (lo, hi) = (p.low_cut as f64, p.high_cut as f64);
        let att = (-1.0 / (self.rate * (p.attack.max(0.1) as f64) * 0.001)).exp();
        let rel = (-1.0 / (self.rate * (p.release.max(0.1) as f64) * 0.001)).exp();
        let amount = p.amount.clamp(0.0, 1.0) as f64;
        let target = 10f64.powf(p.output as f64 / 20.0);
        let g_step = 1.0 - (-1.0 / (self.rate * 0.02)).exp();
        let chans = if mode == 0 { 1 } else { 2 };
        for (frame, s) in main.iter_mut().zip(sc) {
            // Sidechain into analysis channels.
            let (sl, sr) = (s[0] as f64, s[1] as f64);
            let a = match mode {
                0 => [0.5 * (sl + sr), 0.0],
                1 => [sl, sr],
                _ => [0.5 * (sl + sr), 0.5 * (sl - sr)],
            };
            for c in 0..chans {
                let x = a[c];
                for b in 0..BANDS {
                    let k = &self.bp[b];
                    let z = &mut self.sc_z[c][b];
                    let u = k[0] * x + z[0];
                    z[0] = k[1] * x - k[3] * u + z[1];
                    z[1] = k[2] * x - k[4] * u;
                    let y = k[0] * u + z[2];
                    z[2] = k[1] * u - k[3] * y + z[3];
                    z[3] = k[2] * u - k[4] * y;
                    // ×2: a sine's mean power to its peak amplitude squared.
                    let pw = 2.0 * y * y;
                    let e = &mut self.env[c][b];
                    let coef = if pw > *e { att } else { rel };
                    *e = pw + coef * (*e - pw);
                }
            }
            self.ctr += 1;
            if self.ctr >= CTRL {
                self.ctr = 0;
                for c in 0..chans {
                    for b in 0..BANDS {
                        let amp = (self.env[c][b] + 1e-20).sqrt();
                        let cut = (amount * self.weight(b, lo, hi) * 20.0 * (1.0 + SENS * amp).log10()).min(MAX_CUT);
                        self.cut[c][b] = cut as f32;
                        self.set_bell(c, b, cut);
                    }
                }
                if chans == 1 {
                    self.cut[1] = self.cut[0];
                }
            }
            // The ducked signal through the bells.
            let (xl, xr) = (frame[0] as f64, frame[1] as f64);
            let mut m = if mode == 2 { [0.5 * (xl + xr), 0.5 * (xl - xr)] } else { [xl, xr] };
            for (c, mc) in m.iter_mut().enumerate() {
                let set = if mode == 0 { 0 } else { c };
                let mut x = *mc;
                for b in 0..BANDS {
                    let k = &self.bell[set][b];
                    let z = &mut self.bell_z[c][b];
                    let y = k[0] * x + z[0];
                    z[0] = k[1] * x - k[3] * y + z[1];
                    z[1] = k[2] * x - k[4] * y;
                    x = y;
                }
                *mc = x;
            }
            let (yl, yr) = if mode == 2 { (m[0] + m[1], m[0] - m[1]) } else { (m[0], m[1]) };
            self.gain += g_step * (target - self.gain);
            *frame = [(yl * self.gain) as f32, (yr * self.gain) as f32];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw(f: f64, rate: f64, n: usize) -> Vec<[f32; 2]> {
        (0..n).map(|i| ((i as f64 * f / rate).fract() * 2.0 - 1.0) as f32 * 0.5).map(|v| [v, v]).collect()
    }

    #[test]
    fn silent_sidechain_is_transparent() {
        let mut s = Spacer::new(44100);
        let mut main = saw(220.0, 44100.0, 4000);
        let orig = main.clone();
        s.process(&Settings::new(0), &mut main, &vec![[0.0; 2]; 4000]);
        let worst = main.iter().zip(&orig).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "changed by {worst}");
    }

    #[test]
    fn ducks_where_the_sidechain_plays() {
        let mut s = Spacer::new(44100);
        let n = 44100;
        let mut main = saw(110.0, 44100.0, n);
        s.process(&Settings::new(0), &mut main, &saw(440.0, 44100.0, n));
        // The band nearest 440 Hz is cut by several dB; one far above it barely.
        let f = s.freqs();
        let near = (0..BANDS).min_by(|&a, &b| (f[a] - 440.0).abs().total_cmp(&(f[b] - 440.0).abs())).unwrap();
        assert!(s.cut[0][near] > 6.0, "cut at 440 Hz: {}", s.cut[0][near]);
        assert!(s.cut[0][3] < 1.0, "cut at {} Hz: {}", f[3], s.cut[0][3]);
    }
}
