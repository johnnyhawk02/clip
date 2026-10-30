//! Reverb and Delay, the two effects Live puts on its return tracks to start with.

use serde::{Deserialize, Serialize};

/// Reverb settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReverbSettings {
    /// 0..1: how long the tail is.
    pub size: f32,
    /// 0..1: how quickly the highs die away.
    pub damping: f32,
    /// 0..1: stereo spread.
    pub width: f32,
    /// Pre-delay, ms.
    pub predelay: f32,
    /// 0..1: wet against dry (all wet on a return track).
    pub mix: f32,
    pub on: bool,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        ReverbSettings { size: 0.7, damping: 0.5, width: 1.0, predelay: 10.0, mix: 1.0, on: true }
    }
}

/// Delay settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelaySettings {
    /// Delay time in beats (synced to the song tempo).
    pub beats: f32,
    /// 0..0.95: how much comes round again.
    pub feedback: f32,
    /// Echoes bounce left and right.
    pub ping_pong: bool,
    /// The echoes are filtered to this band (Hz).
    pub low_cut: f32,
    pub high_cut: f32,
    /// 0..1: wet against dry (all wet on a return track).
    pub mix: f32,
    pub on: bool,
}

impl Default for DelaySettings {
    fn default() -> Self {
        DelaySettings { beats: 0.75, feedback: 0.45, ping_pong: true, low_cut: 200.0, high_cut: 6000.0, mix: 1.0, on: true }
    }
}

/// Freeverb's comb and all-pass lengths (at 44.1 kHz), and the right channel's spread.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}

impl Comb {
    fn new(n: usize) -> Comb {
        Comb { buf: vec![0.0; n.max(1)], i: 0, store: 0.0 }
    }

    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * feedback;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

struct AllPass {
    buf: Vec<f32>,
    i: usize,
}

impl AllPass {
    fn new(n: usize) -> AllPass {
        AllPass { buf: vec![0.0; n.max(1)], i: 0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        let y = -x + b;
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

/// A Freeverb-style reverb: eight damped combs and four all-passes per side, with pre-delay.
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    alls: [Vec<AllPass>; 2],
    pre: Vec<[f32; 2]>,
    pre_i: usize,
    rate: f32,
}

impl Reverb {
    pub fn new(rate: u32) -> Reverb {
        let k = rate as f32 / 44100.0;
        let len = |n: usize, side: usize| ((n + side * SPREAD) as f32 * k) as usize;
        Reverb {
            combs: [COMBS.iter().map(|&n| Comb::new(len(n, 0))).collect(), COMBS.iter().map(|&n| Comb::new(len(n, 1))).collect()],
            alls: [ALLPASSES.iter().map(|&n| AllPass::new(len(n, 0))).collect(), ALLPASSES.iter().map(|&n| AllPass::new(len(n, 1))).collect()],
            pre: vec![[0.0; 2]; (rate as usize / 4).max(1)],
            pre_i: 0,
            rate: rate as f32,
        }
    }

    pub fn process(&mut self, s: &ReverbSettings, main: &mut [[f32; 2]]) {
        let feedback = 0.7 + 0.28 * s.size.clamp(0.0, 1.0);
        let damp = 0.4 * s.damping.clamp(0.0, 1.0);
        let pre = ((s.predelay.max(0.0) * 0.001 * self.rate) as usize).min(self.pre.len() - 1);
        let w = s.width.clamp(0.0, 1.0);
        let (w1, w2) = (0.5 * (1.0 + w), 0.5 * (1.0 - w));
        let mix = s.mix.clamp(0.0, 1.0);
        for f in main.iter_mut() {
            // Pre-delay line.
            let n = self.pre.len();
            self.pre[self.pre_i] = *f;
            let d = self.pre[(self.pre_i + n - pre) % n];
            self.pre_i = (self.pre_i + 1) % n;
            let input = (d[0] + d[1]) * 0.015;
            let mut out = [0.0f32; 2];
            for c in 0..2 {
                let mut acc = 0.0;
                for comb in &mut self.combs[c] {
                    acc += comb.process(input, feedback, damp);
                }
                for ap in &mut self.alls[c] {
                    acc = ap.process(acc);
                }
                out[c] = acc;
            }
            let wet = [out[0] * w1 + out[1] * w2, out[1] * w1 + out[0] * w2];
            *f = [f[0] * (1.0 - mix) + wet[0] * mix, f[1] * (1.0 - mix) + wet[1] * mix];
        }
    }
}

/// A tempo-synced stereo delay with filtered feedback and ping-pong.
pub struct Delay {
    buf: Vec<[f32; 2]>,
    i: usize,
    rate: f32,
    /// One-pole filter states on the echoes (low-pass, then high-pass), per side.
    lp: [f32; 2],
    hp: [f32; 2],
    hp_x: [f32; 2],
}

impl Delay {
    pub fn new(rate: u32) -> Delay {
        // Up to 4 s of echo.
        Delay { buf: vec![[0.0; 2]; rate as usize * 4], i: 0, rate: rate as f32, lp: [0.0; 2], hp: [0.0; 2], hp_x: [0.0; 2] }
    }

    /// `beat` is a beat's length in frames.
    pub fn process(&mut self, s: &DelaySettings, main: &mut [[f32; 2]], beat: f64) {
        let n = self.buf.len();
        let d = ((s.beats.max(0.0625) as f64 * beat) as usize).clamp(1, n - 1);
        let fb = s.feedback.clamp(0.0, 0.95);
        let a_lp = 1.0 - (-2.0 * std::f32::consts::PI * s.high_cut.max(100.0) / self.rate).exp();
        let a_hp = (-2.0 * std::f32::consts::PI * s.low_cut.max(10.0) / self.rate).exp();
        let mix = s.mix.clamp(0.0, 1.0);
        for f in main.iter_mut() {
            let e = self.buf[(self.i + n - d) % n];
            // Filter the echoes so each repeat gets darker and thinner.
            let mut echo = [0.0f32; 2];
            for c in 0..2 {
                self.lp[c] += a_lp * (e[c] - self.lp[c]);
                let y = a_hp * (self.hp[c] + self.lp[c] - self.hp_x[c]);
                self.hp_x[c] = self.lp[c];
                self.hp[c] = y;
                echo[c] = y;
            }
            let write = if s.ping_pong {
                // Mono in on the left; each repeat crosses to the other side.
                [0.5 * (f[0] + f[1]) + echo[1] * fb, echo[0] * fb]
            } else {
                [f[0] + echo[0] * fb, f[1] + echo[1] * fb]
            };
            self.buf[self.i] = write;
            self.i = (self.i + 1) % n;
            *f = [f[0] * (1.0 - mix) + echo[0] * mix, f[1] * (1.0 - mix) + echo[1] * mix];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_repeats_after_its_time() {
        let mut d = Delay::new(1000);
        let s = DelaySettings { beats: 1.0, feedback: 0.5, ping_pong: false, low_cut: 10.0, high_cut: 20000.0, mix: 1.0, on: true };
        let mut buf = vec![[0.0f32; 2]; 400];
        buf[0] = [1.0, 1.0];
        d.process(&s, &mut buf, 100.0);
        assert!(buf[50][0].abs() < 1e-3, "silent before the echo");
        assert!(buf[100..110].iter().any(|f| f[0].abs() > 0.1), "an echo after one beat");
        assert!(buf[200..210].iter().any(|f| f[0].abs() > 0.03), "a quieter second echo");
    }

    #[test]
    fn reverb_rings_on_and_stays_finite() {
        let mut r = Reverb::new(44100);
        let mut buf = vec![[0.0f32; 2]; 44100];
        buf[0] = [1.0, 1.0];
        r.process(&ReverbSettings::default(), &mut buf);
        assert!(buf.iter().all(|f| f[0].is_finite() && f[0].abs() < 2.0));
        assert!(buf[20000..].iter().any(|f| f[0].abs() > 1e-4), "still ringing after half a second");
    }
}
