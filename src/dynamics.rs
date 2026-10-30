//! Shared pieces for compressors and levellers, after Giannoulis, Massberg and Reiss, "Digital
//! Dynamic Range Compressor Design — A Tutorial and Analysis" (JAES 2012): a soft-knee gain
//! computer in the log domain (their eq. 4), in feed-forward and feedback forms, and one-pole
//! smoothing with separate attack and release (their branching peak detector, eq. 17). Tame's
//! per-bin cuts use the same knee (at ratio ∞, where the cut is the whole excess).

/// A one-pole coefficient for time constant `ms` at `rate` (0 = no smoothing).
pub fn coef(ms: f32, rate: f32) -> f32 {
    if ms <= 0.0 { 0.0 } else { (-1.0 / (ms * 0.001 * rate)).exp() }
}

pub fn to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

pub fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// How far level `x` is over threshold `t` (dB), rounded over a knee `w` dB wide: zero below
/// the knee, the plain excess above it, a parabola between (eq. 4's middle branch).
pub fn excess(x: f32, t: f32, w: f32) -> f32 {
    let d = x - t;
    if 2.0 * d <= -w {
        0.0
    } else if w > 0.0 && 2.0 * d < w {
        (d + w / 2.0) * (d + w / 2.0) / (2.0 * w)
    } else {
        d
    }
}

// (Feed-forward: none of the devices so far is one, but the next ones will be.)

/// The static curve (eq. 4): output level for input level `x`, threshold `t`, ratio `r`, knee `w`.
#[allow(dead_code)]
pub fn curve(x: f32, t: f32, r: f32, w: f32) -> f32 {
    x - reduction(x, t, r, w)
}

/// Feed-forward: the gain reduction (dB, ≥ 0) for input level `x`.
#[allow(dead_code)]
pub fn reduction(x: f32, t: f32, r: f32, w: f32) -> f32 {
    (1.0 - 1.0 / r.max(1.0)) * excess(x, t, w)
}

/// Feedback: the gain reduction for the *output* level `y` that gives ratio `r` from input to
/// output once settled. (With y = x − c and c = (r − 1)(y − t), y = t + (x − t)/r; a feed-forward
/// curve put in a feedback loop could never pass 2:1.)
pub fn feedback_reduction(y: f32, t: f32, r: f32, w: f32) -> f32 {
    (r.max(1.0) - 1.0) * excess(y, t, w)
}

/// One-pole smoothing with an attack and a release: rises with `attack`, falls with `release`.
#[derive(Clone, Copy, Default)]
pub struct Smoother {
    pub y: f32,
}

impl Smoother {
    pub fn next(&mut self, x: f32, attack: f32, release: f32) -> f32 {
        let a = if x > self.y { attack } else { release };
        self.y = a * self.y + (1.0 - a) * x;
        self.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_knee_curve() {
        let (t, r, w) = (-20.0, 4.0, 10.0);
        // Below the knee, untouched; above it, 1/r of the excess comes through.
        assert_eq!(curve(-40.0, t, r, w), -40.0);
        assert!((curve(0.0, t, r, w) - (-15.0)).abs() < 1e-4);
        // Continuous at both knee edges and its slope runs from 1 to 1/r.
        for edge in [t - w / 2.0, t + w / 2.0] {
            assert!((curve(edge - 1e-3, t, r, w) - curve(edge + 1e-3, t, r, w)).abs() < 3e-3);
        }
        let slope = |x: f32| (curve(x + 0.01, t, r, w) - curve(x - 0.01, t, r, w)) / 0.02;
        assert!((slope(t - w) - 1.0).abs() < 1e-3 && (slope(t + w) - 1.0 / r).abs() < 1e-3);
        assert!(slope(t) < 1.0 && slope(t) > 1.0 / r);
    }

    #[test]
    fn feedback_settles_at_the_ratio() {
        // Solve y = x − c(y) by iteration for two input levels well over the threshold.
        let settle = |x: f32| {
            let mut y = x;
            for _ in 0..2000 {
                y += 0.05 * ((x - feedback_reduction(y, -20.0, 3.0, 6.0)) - y);
            }
            y
        };
        let slope = (settle(0.0) - settle(-10.0)) / 10.0;
        assert!((slope - 1.0 / 3.0).abs() < 1e-3, "{slope}");
    }
}
