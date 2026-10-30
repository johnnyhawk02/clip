//! 4x oversampling for nonlinear devices: two halfband FIR stages each way (2x, then 2x again),
//! linear phase, so the delay is a whole number of samples (`LATENCY`) for delay compensation.
//! The halfbands are windowed sincs (Kaiser), every other tap zero.

/// Stage one's half length (63 taps: passband to ~0.42 of the sample rate) and stage two's
/// (17 taps; its images are far away, so short is plenty).
const M1: usize = 31;
const M2: usize = 8;
/// Up and back down, in samples at the base rate: 2·M1 at 2x, plus 2·M2 at 4x.
pub const LATENCY: usize = M1 + M2 / 2;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..40 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
    }
    sum
}

/// One halfband low-pass at its own rate, with its history.
#[derive(Clone)]
struct Halfband {
    /// The nonzero taps: (age, coefficient).
    taps: Vec<(usize, f64)>,
    /// The history, written twice over (at i and i + n) so a read never wraps.
    z: Vec<f64>,
    n: usize,
    i: usize,
}

impl Halfband {
    fn new(m: usize, beta: f64) -> Halfband {
        let mut taps = Vec::new();
        for j in 0..=2 * m {
            let n = j as f64 - m as f64;
            let sinc = if n == 0.0 { 1.0 } else { (std::f64::consts::PI * n / 2.0).sin() / (std::f64::consts::PI * n / 2.0) };
            let r = n / m as f64;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta);
            let c = 0.5 * sinc * w;
            if c.abs() > 1e-12 {
                taps.push((j, c));
            }
        }
        Halfband { taps, z: vec![0.0; 2 * (2 * m + 1)], n: 2 * m + 1, i: 0 }
    }

    fn push(&mut self, x: f64) {
        self.i = (self.i + 1) % self.n;
        self.z[self.i] = x;
        self.z[self.i + self.n] = x;
    }

    fn out(&self) -> f64 {
        let z = &self.z[self.i + self.n - 2 * (self.n / 2)..=self.i + self.n];
        self.taps.iter().map(|&(j, c)| c * z[z.len() - 1 - j]).sum()
    }

    /// One sample in, two out at twice the rate.
    fn up(&mut self, x: f64) -> [f64; 2] {
        self.push(2.0 * x);
        let a = self.out();
        self.push(0.0);
        [a, self.out()]
    }

    /// Two samples in, one out at half the rate (the phase that keeps the delay whole).
    fn down(&mut self, a: f64, b: f64) -> f64 {
        self.push(a);
        let y = self.out();
        self.push(b);
        y
    }
}

/// One channel's 4x oversampler.
#[derive(Clone)]
pub struct Over4 {
    up1: Halfband,
    up2: Halfband,
    down2: Halfband,
    down1: Halfband,
}

impl Default for Over4 {
    fn default() -> Self {
        Over4 { up1: Halfband::new(M1, 8.0), up2: Halfband::new(M2, 7.0), down2: Halfband::new(M2, 7.0), down1: Halfband::new(M1, 8.0) }
    }
}

impl Over4 {
    /// Run `x` through `f` at four times the rate, and back down; `LATENCY` samples late.
    pub fn run(&mut self, x: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
        let half = self.up1.up(x);
        let mut back = [0.0; 2];
        for (o, h) in back.iter_mut().zip(half) {
            let [p, q] = self.up2.up(h);
            let (yp, yq) = (f(p), f(q));
            *o = self.down2.down(yp, yq);
        }
        self.down1.down(back[0], back[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_latency_and_unity() {
        let mut o = Over4::default();
        let out: Vec<f64> = (0..200).map(|i| o.run(if i == 0 { 1.0 } else { 0.0 }, |v| v)).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, LATENCY);
        // A low sine comes through at the same level, LATENCY late.
        let mut o = Over4::default();
        let x = |i: usize| (i as f64 * 0.05).sin();
        let y: Vec<f64> = (0..2000).map(|i| o.run(x(i), |v| v)).collect();
        let err = (500..2000).map(|i| (y[i] - x(i - LATENCY)).abs()).fold(0.0, f64::max);
        assert!(err < 1e-3, "{err}");
    }

    #[test]
    fn images_are_gone() {
        // Squaring a 15 kHz sine at 48 kHz makes 30 kHz, which must not fold back to 18 kHz.
        let mut o = Over4::default();
        let w = 2.0 * std::f64::consts::PI * 15000.0 / 48000.0;
        let y: Vec<f64> = (0..8192).map(|i| o.run((i as f64 * w).sin(), |v| v * v)).collect();
        let w18 = 2.0 * std::f64::consts::PI * 18000.0 / 48000.0;
        let (re, im) = y[4096..].iter().enumerate().fold((0.0, 0.0), |(r, m), (i, v)| (r + v * (i as f64 * w18).cos(), m + v * (i as f64 * w18).sin()));
        let alias = 2.0 * (re * re + im * im).sqrt() / 4096.0;
        assert!(alias < 1e-3, "alias {alias}");
    }
}
