//! Pultec: the passive EQ network of the Pultec EQP-1A, after Barrera's "Pultec EQP-1A Modeling
//! with Wave Digital Filters" (UPF, 2023; SMC 2024). The network's parts and how they connect are
//! taken from that model; here the circuit is solved by modified nodal analysis with trapezoidal
//! companion models for the capacitors and inductor (the same discretisation the WDF uses), at
//! twice the sample rate so the high boost lands near where it should. The low section boosts
//! and cuts a shelf at the same frequency, so turning both up gives the famous "low-end trick":
//! a bump with a dip above it.

use serde::{Deserialize, Serialize};

/// Pultec settings, as on the front panel (knobs 0..10).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Low frequency, Hz: 20, 30, 60 or 100.
    pub low_freq: f32,
    pub low_boost: f32,
    pub low_atten: f32,
    /// High boost frequency, kHz: 3, 4, 5, 8, 10, 12 or 16.
    pub high_freq: f32,
    pub high_boost: f32,
    /// 0..10: sharp to broad.
    pub bandwidth: f32,
    /// High attenuation shelf, kHz: 5, 10 or 20.
    pub atten_sel: f32,
    pub high_atten: f32,
    pub on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { low_freq: 60.0, low_boost: 0.0, low_atten: 0.0, high_freq: 10.0, high_boost: 0.0, bandwidth: 5.0, atten_sel: 10.0, high_atten: 0.0, on: true }
    }
}

pub const LOW_FREQS: [f32; 4] = [20.0, 30.0, 60.0, 100.0];
pub const HIGH_FREQS: [f32; 7] = [3.0, 4.0, 5.0, 8.0, 10.0, 12.0, 16.0];
pub const ATTEN_SELS: [f32; 3] = [5.0, 10.0, 20.0];

/// Pot and part values of the network (Ω, F, H).
const R_BOOST_HI: f64 = 12e3;
const R_Q: f64 = 3.9e3;
const R_Q_OFFSET: f64 = 390.0;
const R_CUT_HI: f64 = 1e3;
const R_BOOST_LO: f64 = 8.2e3;
const R_CUT_LO: f64 = 110e3;
/// L/C of the high boost's resonant pair.
const L_OVER_C: f64 = 12e6;
const R2: f64 = 1e3;
const R3: f64 = 10e3;
/// Make-up for the network's insertion loss (the tube stages' gain in the real unit).
const MAKEUP: f64 = 15.4882;
/// No pot quite reaches 0 Ω.
const R_MIN: f64 = 1e-2;
const OVER: usize = 2;

/// The low section's two capacitors for its frequency.
fn low_caps(f: f32) -> (f64, f64) {
    let c2 = match f as i32 {
        20 => 2.2e-6,
        30 => 1.1e-6,
        60 => 560e-9,
        _ => 330e-9,
    };
    (c2 / 30.0, c2)
}

/// The high attenuation shelf's capacitor.
fn atten_cap(k: f32) -> f64 {
    match k as i32 {
        5 => 270e-9,
        10 => 135e-9,
        _ => 68e-9,
    }
}

#[derive(Clone, Copy)]
enum Kind {
    R,
    C,
    L,
}

/// A two-terminal part between nodes `a` and `b` (0 is ground), with its value.
#[derive(Clone, Copy)]
struct Part {
    a: usize,
    b: usize,
    kind: Kind,
    value: f64,
}

/// Nodes: 1 the input, 2 after the high boost pot, 3 and 4 either side of the cut networks
/// (4 is the top of the low boost network), 5/6 inside the resonant branch, 7 inside the high
/// cut, 8/9 along the low cut leg (9 is the top of R3, whose voltage is the output with node 4's).
const NODES: usize = 9;
/// Unknowns: the node voltages and the input source's current.
const N: usize = NODES + 1;

fn parts(s: &Settings) -> Vec<Part> {
    let k = |v: f32| (v.clamp(0.0, 10.0) / 10.0) as f64;
    let r = |v: f64| v.max(R_MIN);
    let hf = s.high_freq as f64 * 1e3;
    let c1 = (1.0 / (L_OVER_C * (std::f64::consts::TAU * hf).powi(2))).sqrt();
    let (lc1, lc2) = low_caps(s.low_freq);
    let (hb, ha) = (k(s.high_boost), k(s.high_atten));
    use Kind::*;
    vec![
        // High boost: the pot from the input, its wiper across the resonant branch.
        Part { a: 1, b: 2, kind: R, value: r((1.0 - hb) * R_BOOST_HI) },
        Part { a: 2, b: 3, kind: R, value: r(hb * R_BOOST_HI) },
        Part { a: 2, b: 5, kind: C, value: c1 },
        Part { a: 5, b: 6, kind: L, value: L_OVER_C * c1 },
        Part { a: 6, b: 3, kind: R, value: r(k(s.bandwidth) * R_Q) + R_Q_OFFSET },
        // High attenuation: the pot, with the shelf capacitor across its lower part.
        Part { a: 3, b: 7, kind: R, value: r((1.0 - ha) * R_CUT_HI) },
        Part { a: 7, b: 4, kind: R, value: r(ha * R_CUT_HI) },
        Part { a: 7, b: 4, kind: C, value: atten_cap(s.atten_sel) },
        // Low attenuation: its pot across a capacitor, then R2 and R3.
        Part { a: 3, b: 8, kind: C, value: lc1 },
        Part { a: 3, b: 8, kind: R, value: r(k(s.low_atten) * R_CUT_LO) },
        Part { a: 8, b: 9, kind: R, value: R2 },
        Part { a: 9, b: 4, kind: R, value: R3 },
        // Low boost: its pot across a capacitor, to ground.
        Part { a: 4, b: 0, kind: R, value: r(k(s.low_boost) * R_BOOST_LO) },
        Part { a: 4, b: 0, kind: C, value: lc2 },
    ]
}

/// Invert `m` (Gauss-Jordan with partial pivoting).
fn invert(mut m: [[f64; N]; N]) -> [[f64; N]; N] {
    let mut inv = [[0.0; N]; N];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for col in 0..N {
        let p = (col..N).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs())).unwrap();
        m.swap(col, p);
        inv.swap(col, p);
        let d = m[col][col];
        for j in 0..N {
            m[col][j] /= d;
            inv[col][j] /= d;
        }
        for r in 0..N {
            if r != col && m[r][col] != 0.0 {
                let f = m[r][col];
                for j in 0..N {
                    m[r][j] -= f * m[col][j];
                    inv[r][j] -= f * inv[col][j];
                }
            }
        }
    }
    inv
}

/// One side's reactive part states: each one's voltage and current a step ago.
#[derive(Clone, Default)]
struct Side {
    v: Vec<f64>,
    i: Vec<f64>,
    prev_in: f64,
}

pub struct Pultec {
    rate: f64,
    made_for: Option<Settings>,
    parts: Vec<Part>,
    /// Each part's companion conductance, and the system's inverse.
    g: Vec<f64>,
    inv: Box<[[f64; N]; N]>,
    sides: [Side; 2],
}

impl Pultec {
    pub fn new(rate: u32) -> Pultec {
        Pultec { rate: rate as f64 * OVER as f64, made_for: None, parts: Vec::new(), g: Vec::new(), inv: Box::new([[0.0; N]; N]), sides: [Side::default(), Side::default()] }
    }

    /// Build the circuit for `s`: each part's conductance (a capacitor or inductor as its
    /// trapezoidal companion), stamped into the nodal matrix with the input source.
    fn build(&mut self, s: &Settings) {
        let t = 1.0 / self.rate;
        self.parts = parts(s);
        self.g = self.parts.iter().map(|p| match p.kind {
            Kind::R => 1.0 / p.value,
            Kind::C => 2.0 * p.value / t,
            Kind::L => t / (2.0 * p.value),
        }).collect();
        let mut m = [[0.0; N]; N];
        for (p, &g) in self.parts.iter().zip(&self.g) {
            for (x, y, sign) in [(p.a, p.a, 1.0), (p.b, p.b, 1.0), (p.a, p.b, -1.0), (p.b, p.a, -1.0)] {
                if x > 0 && y > 0 {
                    m[x - 1][y - 1] += sign * g;
                }
            }
        }
        // The input: an ideal voltage source at node 1.
        m[0][NODES] = 1.0;
        m[NODES][0] = 1.0;
        *self.inv = invert(m);
        for side in &mut self.sides {
            side.v.resize(self.parts.len(), 0.0);
            side.i.resize(self.parts.len(), 0.0);
        }
        self.made_for = Some(*s);
    }

    /// One step of the circuit at the oversampled rate.
    fn step(&self, side: &mut Side, x: f64) -> f64 {
        let mut b = [0.0; N];
        b[NODES] = x;
        // History currents of the companions, into their nodes.
        for (k, (p, &g)) in self.parts.iter().zip(&self.g).enumerate() {
            let j = match p.kind {
                Kind::R => continue,
                Kind::C => g * side.v[k] + side.i[k],
                Kind::L => -(g * side.v[k] + side.i[k]),
            };
            if p.a > 0 {
                b[p.a - 1] += j;
            }
            if p.b > 0 {
                b[p.b - 1] -= j;
            }
        }
        let mut v = [0.0; N];
        for (r, out) in v.iter_mut().enumerate() {
            *out = self.inv[r].iter().zip(&b).map(|(a, b)| a * b).sum();
        }
        let node = |n: usize| if n == 0 { 0.0 } else { v[n - 1] };
        for (k, (p, &g)) in self.parts.iter().zip(&self.g).enumerate() {
            let vk = node(p.a) - node(p.b);
            match p.kind {
                Kind::R => {}
                Kind::C => side.i[k] = g * vk - (g * side.v[k] + side.i[k]),
                Kind::L => side.i[k] = g * vk + (g * side.v[k] + side.i[k]),
            }
            side.v[k] = vk;
        }
        // The output: the top of R3 (what the make-up stage's grid sees), above the low boost
        // network. (That network's own voltage is what boosts the lows; taking it out, as the
        // reference model's output appears to, left low attenuation doing nothing once low boost
        // was up, and so no low-end trick.)
        node(9) * MAKEUP
    }

    pub fn process(&mut self, s: &Settings, main: &mut [[f32; 2]]) {
        if self.made_for != Some(*s) {
            self.build(s);
        }
        let mut sides = std::mem::take(&mut self.sides);
        for f in main.iter_mut() {
            for (c, side) in sides.iter_mut().enumerate() {
                let x = f[c] as f64;
                let mut acc = 0.0;
                for j in 1..=OVER {
                    let xi = side.prev_in + (x - side.prev_in) * j as f64 / OVER as f64;
                    acc += self.step(side, xi);
                }
                side.prev_in = x;
                f[c] = (acc / OVER as f64) as f32;
            }
        }
        self.sides = sides;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level (dB) a sine at `f` Hz comes out at.
    fn response(s: &Settings, f: f32) -> f32 {
        let mut p = Pultec::new(44100);
        let n = 44100;
        let mut buf: Vec<[f32; 2]> = (0..n).map(|i| {
            let v = 0.1 * (std::f32::consts::TAU * f * i as f32 / 44100.0).sin();
            [v, v]
        }).collect();
        p.process(s, &mut buf);
        let peak = buf[n / 2..].iter().map(|x| x[0].abs()).fold(0.0, f32::max);
        20.0 * (peak / 0.1).log10()
    }

    #[test]
    fn flat_when_off() {
        let s = Settings::default();
        for f in [50.0, 1000.0, 5000.0] {
            let db = response(&s, f);
            eprintln!("flat {f} Hz: {db:.2} dB");
            assert!(db.abs() < 1.5, "{f} Hz: {db} dB");
        }
    }

    #[test]
    fn low_end_trick() {
        let s = Settings { low_freq: 30.0, low_boost: 10.0, low_atten: 10.0, ..Settings::default() };
        let boost = Settings { low_atten: 0.0, ..s };
        let cut = Settings { low_boost: 0.0, ..s };
        for f in [20.0, 40.0, 80.0, 150.0, 200.0, 300.0, 1000.0] {
            eprintln!("{f} Hz: boost {:.1}  cut {:.1}  both {:.1} dB", response(&boost, f), response(&cut, f), response(&s, f));
        }
        assert!(response(&boost, 30.0) > 8.0);
        assert!(response(&cut, 30.0) < -8.0);
        // Both: the lows still up, with a dip above them.
        assert!(response(&s, 40.0) > 5.0 && response(&s, 300.0) < -3.0);
    }

    #[test]
    fn high_boost_peaks_near_its_frequency() {
        let s = Settings { high_freq: 5.0, high_boost: 10.0, bandwidth: 3.0, ..Settings::default() };
        let r: Vec<(f32, f32)> = [1000.0, 3000.0, 4000.0, 5000.0, 6000.0, 8000.0, 12000.0].iter().map(|&f| (f, response(&s, f))).collect();
        eprintln!("high boost 5k: {r:?}");
        let best = r.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(best.1 > 8.0 && (3000.0..=7000.0).contains(&best.0));
        let a = Settings { high_atten: 10.0, atten_sel: 5.0, ..Settings::default() };
        eprintln!("atten 5k at 10k: {:.1} dB", response(&a, 10000.0));
        assert!(response(&a, 10000.0) < -6.0);
    }
}
