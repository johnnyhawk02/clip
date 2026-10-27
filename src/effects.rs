//! Effects and generators (echo, reverb, flanger, pitch/tempo…). Each effect takes a clip and
//! returns the processed clip; the caller splices it back into the file.

use std::f32::consts::PI;

use crate::audio::Audio;

fn map_channels(clip: &Audio, f: impl Fn(&[f32]) -> Vec<f32>) -> Audio {
    Audio { rate: clip.rate, channels: clip.channels.iter().map(|c| f(c)).collect() }
}

fn ms_to_frames(rate: u32, ms: f32) -> usize {
    ((ms / 1000.0) * rate as f32).round().max(1.0) as usize
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Repeating echo: `delay_ms` between repeats, each `feedback` times quieter.
pub fn echo(clip: &Audio, delay_ms: f32, feedback: f32, mix: f32) -> Audio {
    let d = ms_to_frames(clip.rate, delay_ms);
    let fb = feedback.clamp(0.0, 0.95);
    map_channels(clip, |c| {
        let mut wet = vec![0.0f32; c.len()];
        for i in d..c.len() {
            wet[i] = c[i - d] + wet[i - d] * fb;
        }
        c.iter().zip(&wet).map(|(x, w)| x + w * mix).collect()
    })
}

/// Schroeder reverb (four combs into two all-passes per channel). `size` 0..1, `mix` 0..1.
pub fn reverb(clip: &Audio, size: f32, mix: f32) -> Audio {
    let scale = clip.rate as f32 / 44100.0;
    let combs_ms = [29.7f32, 37.1, 41.1, 43.7];
    let fb = 0.70 + 0.28 * size.clamp(0.0, 1.0);
    map_channels(clip, |c| {
        let n = c.len();
        let mut wet = vec![0.0f32; n];
        for (k, ms) in combs_ms.iter().enumerate() {
            // Slightly different lengths per comb (and per run) avoid metallic ringing.
            let d = ((ms * (0.6 + size) + k as f32) * 44.1 * scale) as usize;
            let mut buf = vec![0.0f32; d.max(1)];
            let mut idx = 0;
            let mut lp = 0.0f32;
            for i in 0..n {
                let out = buf[idx];
                lp = out * 0.8 + lp * 0.2; // damping
                buf[idx] = c[i] + lp * fb;
                idx = (idx + 1) % buf.len();
                wet[i] += out * 0.25;
            }
        }
        for ms in [5.0f32, 1.7] {
            let d = ((ms * 44.1 * scale) as usize).max(1);
            let mut buf = vec![0.0f32; d];
            let mut idx = 0;
            let g = 0.7;
            for w in wet.iter_mut() {
                let b = buf[idx];
                let y = -g * *w + b;
                buf[idx] = *w + g * y;
                idx = (idx + 1) % d;
                *w = y;
            }
        }
        c.iter().zip(&wet).map(|(x, w)| x * (1.0 - mix * 0.5) + w * mix).collect()
    })
}

/// Flanger: a short delay swept by a sine LFO, mixed with the dry signal.
pub fn flanger(clip: &Audio, depth_ms: f32, rate_hz: f32, feedback: f32, mix: f32) -> Audio {
    let sr = clip.rate as f32;
    let max_d = depth_ms / 1000.0 * sr;
    let base = 0.001 * sr;
    let fb = feedback.clamp(-0.9, 0.9);
    map_channels(clip, |c| {
        let n = c.len();
        let mut buf = vec![0.0f32; n];
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let lfo = 0.5 - 0.5 * (2.0 * PI * rate_hz * i as f32 / sr).cos();
            let d = base + lfo * max_d;
            let p = i as f32 - d;
            let delayed = if p >= 0.0 {
                let i0 = p as usize;
                let t = p - i0 as f32;
                buf[i0] * (1.0 - t) + buf[(i0 + 1).min(i)] * t
            } else {
                0.0
            };
            buf[i] = c[i] + delayed * fb;
            out.push(c[i] * (1.0 - mix * 0.5) + delayed * mix);
        }
        out
    })
}

/// Low-pass "smooth": two cascaded one-pole filters at `cutoff_hz`.
pub fn smooth(clip: &Audio, cutoff_hz: f32) -> Audio {
    let a = 1.0 - (-2.0 * PI * cutoff_hz / clip.rate as f32).exp();
    map_channels(clip, |c| {
        let (mut y1, mut y2) = (c.first().copied().unwrap_or(0.0), c.first().copied().unwrap_or(0.0));
        c.iter()
            .map(|&x| {
                y1 += a * (x - y1);
                y2 += a * (y1 - y2);
                y2
            })
            .collect()
    })
}

/// Noise gate: silences (to exact zero) everything quieter than `threshold_db`, holding the
/// gate open for `hold_ms` after the signal drops so decays aren't chopped off.
pub fn noise_gate(clip: &Audio, threshold_db: f32, hold_ms: f32) -> Audio {
    let thr = db_to_gain(threshold_db);
    let hold = ms_to_frames(clip.rate, hold_ms);
    let n = clip.frames();
    let mut open = vec![false; n];
    let mut last_loud: Option<usize> = None;
    for i in 0..n {
        if clip.channels.iter().any(|c| c[i].abs() >= thr) {
            last_loud = Some(i);
        }
        open[i] = last_loud.is_some_and(|l| i - l <= hold);
    }
    Audio {
        rate: clip.rate,
        channels: clip.channels.iter().map(|c| c.iter().zip(&open).map(|(s, o)| if *o { *s } else { 0.0 }).collect()).collect(),
    }
}

/// Speed change like a tape: pitch and length change together. `percent` 100 = unchanged.
pub fn change_speed(clip: &Audio, percent: f32) -> Audio {
    let factor = (percent / 100.0).clamp(0.1, 10.0) as f64;
    let stretched = Audio { rate: (clip.rate as f64 * factor).round() as u32, channels: clip.channels.clone() };
    let mut out = stretched.resampled(clip.rate);
    out.rate = clip.rate;
    out
}

/// Tempo change that keeps the pitch (WSOLA time-stretch). `percent` 200 = twice as fast.
pub fn change_tempo(clip: &Audio, percent: f32) -> Audio {
    let factor = 100.0 / percent.clamp(10.0, 1000.0) as f64; // output length / input length
    time_stretch(clip, factor)
}

/// Pitch shift by `semitones`, keeping the length.
pub fn pitch_shift(clip: &Audio, semitones: f32) -> Audio {
    let ratio = 2f64.powf(semitones as f64 / 12.0);
    let n = clip.frames();
    // Stretch by the ratio, then play it back faster/slower to restore the length.
    let stretched = time_stretch(clip, ratio);
    let mut out = Audio { rate: (clip.rate as f64 * ratio).round() as u32, channels: stretched.channels }.resampled(clip.rate);
    out.rate = clip.rate;
    for c in &mut out.channels {
        c.resize(n, 0.0);
    }
    out
}

/// Waveform-similarity overlap-add time stretch. `factor` = output length / input length.
pub fn time_stretch(clip: &Audio, factor: f64) -> Audio {
    let n = clip.frames();
    let win = ((clip.rate as f64 * 0.05) as usize).max(64) & !1;
    if n < win * 2 {
        return clip.clone();
    }
    let hop_out = win / 2;
    let hop_in = hop_out as f64 / factor;
    let tol = win / 4;
    let window: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / win as f32).cos()).collect();
    let mono: Vec<f32> = (0..n).map(|i| clip.channels.iter().map(|c| c[i]).sum::<f32>()).collect();

    let out_len = (n as f64 * factor).round() as usize;
    let mut out = vec![vec![0.0f32; out_len + win]; clip.num_channels()];
    let mut norm = vec![0.0f32; out_len + win];
    let mut prev: usize = 0;
    let frames = out_len / hop_out + 1;
    for k in 0..frames {
        let target = (k as f64 * hop_in) as usize;
        if target + win >= n {
            break;
        }
        let pos = if k == 0 {
            0
        } else {
            // Choose the start near `target` that best continues the previous frame.
            let natural = prev + hop_out;
            let lo = target.saturating_sub(tol);
            let hi = (target + tol).min(n - win);
            let mut best = target.min(hi);
            let mut best_score = f32::MIN;
            if natural + hop_out < n {
                for cand in lo..=hi {
                    let mut score = 0.0;
                    let mut j = 0;
                    while j < hop_out {
                        score += mono[cand + j] * mono[natural + j];
                        j += 4;
                    }
                    if score > best_score {
                        best_score = score;
                        best = cand;
                    }
                }
            }
            best
        };
        let o = k * hop_out;
        for (ci, c) in clip.channels.iter().enumerate() {
            for j in 0..win {
                out[ci][o + j] += c[pos + j] * window[j];
            }
        }
        for j in 0..win {
            norm[o + j] += window[j];
        }
        prev = pos;
    }
    for c in &mut out {
        for (s, w) in c.iter_mut().zip(&norm) {
            if *w > 1e-3 {
                *s /= w;
            }
        }
        c.truncate(out_len);
    }
    Audio { rate: clip.rate, channels: out }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wave {
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

impl Wave {
    pub const ALL: [Wave; 4] = [Wave::Sine, Wave::Square, Wave::Triangle, Wave::Sawtooth];
    pub fn label(self) -> &'static str {
        match self {
            Wave::Sine => "Sine",
            Wave::Square => "Square",
            Wave::Triangle => "Triangle",
            Wave::Sawtooth => "Sawtooth",
        }
    }
}

pub fn tone(rate: u32, channels: usize, secs: f32, freq: f32, level_db: f32, wave: Wave) -> Audio {
    let n = (secs * rate as f32) as usize;
    let amp = db_to_gain(level_db);
    let ramp = ms_to_frames(rate, 5.0).min(n / 2).max(1); // avoid clicks at the edges
    let c: Vec<f32> = (0..n)
        .map(|i| {
            let ph = (i as f32 * freq / rate as f32).fract();
            let v = match wave {
                Wave::Sine => (2.0 * PI * ph).sin(),
                Wave::Square => if ph < 0.5 { 1.0 } else { -1.0 },
                Wave::Triangle => 1.0 - 4.0 * (ph - 0.5).abs(),
                Wave::Sawtooth => 2.0 * ph - 1.0,
            };
            let edge = (i.min(n - 1 - i) as f32 / ramp as f32).min(1.0);
            v * amp * edge
        })
        .collect();
    Audio { rate, channels: vec![c; channels.max(1)] }
}

/// Telephone touch-tone (DTMF) frequencies for a key: (low, high) in Hz.
pub fn dtmf_freqs(key: char) -> Option<(f32, f32)> {
    let rows = [697.0, 770.0, 852.0, 941.0];
    let cols = [1209.0, 1336.0, 1477.0, 1633.0];
    let keys = ["123A", "456B", "789C", "*0#D"];
    let key = key.to_ascii_uppercase();
    keys.iter().enumerate().find_map(|(r, row)| row.find(key).map(|c| (rows[r], cols[c])))
}

/// Touch tones for `digits` (0-9, *, #, A-D; a comma adds a longer pause). Unknown characters
/// are skipped.
pub fn dtmf(rate: u32, channels: usize, digits: &str, tone_ms: f32, gap_ms: f32, level_db: f32) -> Audio {
    let amp = db_to_gain(level_db) * 0.5; // two tones summed
    let tone_n = ms_to_frames(rate, tone_ms);
    let gap_n = ms_to_frames(rate, gap_ms);
    let ramp = ms_to_frames(rate, 3.0).min(tone_n / 2).max(1);
    let mut out: Vec<f32> = Vec::new();
    for ch in digits.chars() {
        if ch == ',' {
            out.extend(std::iter::repeat_n(0.0, gap_n * 4));
            continue;
        }
        let Some((lo, hi)) = dtmf_freqs(ch) else { continue };
        for i in 0..tone_n {
            let t = i as f32 / rate as f32;
            let edge = (i.min(tone_n - 1 - i) as f32 / ramp as f32).min(1.0);
            out.push(((2.0 * PI * lo * t).sin() + (2.0 * PI * hi * t).sin()) * amp * edge);
        }
        out.extend(std::iter::repeat_n(0.0, gap_n));
    }
    Audio { rate, channels: vec![out; channels.max(1)] }
}

pub fn noise(rate: u32, channels: usize, secs: f32, level_db: f32) -> Audio {
    let n = (secs * rate as f32) as usize;
    let amp = db_to_gain(level_db);
    let mut seed: u32 = 0x1234_5678;
    let chans = (0..channels.max(1))
        .map(|_| {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
                })
                .collect()
        })
        .collect();
    Audio { rate, channels: chans }
}

pub fn silence(rate: u32, channels: usize, secs: f32) -> Audio {
    let n = (secs * rate as f32) as usize;
    Audio { rate, channels: vec![vec![0.0; n]; channels.max(1)] }
}

/// Mix several sounds (each with a linear gain) into one. All must share a sample rate. The
/// result is as long as the longest input and has as many channels as the widest one; mono
/// inputs are spread across every channel.
pub fn mix(inputs: &[(&Audio, f32)], rate: u32) -> Audio {
    let frames = inputs.iter().map(|(a, _)| a.frames()).max().unwrap_or(0);
    let channels = inputs.iter().map(|(a, _)| a.num_channels()).max().unwrap_or(1).max(1);
    let mut out = vec![vec![0.0f32; frames]; channels];
    for (a, gain) in inputs {
        if a.num_channels() == 0 || *gain == 0.0 {
            continue;
        }
        for (ci, o) in out.iter_mut().enumerate() {
            let src = &a.channels[ci.min(a.num_channels() - 1)];
            for (d, s) in o.iter_mut().zip(src) {
                *d += s * gain;
            }
        }
    }
    Audio { rate, channels: out }
}

/// Mix `clip` into `base` starting at frame `at`, at `level` (linear). Extends the file if
/// the clip runs past the end.
pub fn mix_into(base: &Audio, at: usize, clip: &Audio, level: f32) -> Audio {
    let clip = clip.resampled(base.rate);
    let mut out = base.clone();
    let end = at + clip.frames();
    for (i, c) in out.channels.iter_mut().enumerate() {
        if c.len() < end {
            c.resize(end, 0.0);
        }
        let src = &clip.channels[i.min(clip.num_channels() - 1)];
        for (d, s) in c[at..end].iter_mut().zip(src) {
            *d += s * level;
        }
    }
    out
}

/// Change the sample rate (with a low-pass first when going down, to avoid aliasing).
pub fn convert_rate(clip: &Audio, rate: u32) -> Audio {
    if rate == clip.rate {
        return clip.clone();
    }
    let src = if rate < clip.rate { smooth(clip, rate as f32 * 0.45) } else { clip.clone() };
    src.resampled(rate)
}

/// Mix down to mono, or duplicate mono to stereo.
pub fn convert_channels(clip: &Audio, channels: usize) -> Audio {
    match (clip.num_channels(), channels) {
        (n, c) if n == c => clip.clone(),
        (_, 1) => clip.to_mono(),
        (1, c) => Audio { rate: clip.rate, channels: vec![clip.channels[0].clone(); c] },
        (_, c) => {
            let m = clip.to_mono();
            Audio { rate: clip.rate, channels: vec![m.channels[0].clone(); c] }
        }
    }
}

/// Reduce to `bits` of resolution (to hear what an 8-bit or 12-bit version sounds like).
pub fn quantize(clip: &Audio, bits: u32) -> Audio {
    let steps = (1u64 << (bits.clamp(2, 24) - 1)) as f32 - 1.0;
    map_channels(clip, |c| c.iter().map(|s| (s.clamp(-1.0, 1.0) * steps).round() / steps).collect())
}

/// RBJ biquad, used by the equalizer.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
}

impl Biquad {
    fn new(kind: u8, sr: f32, freq: f32, gain_db: f32, q: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * freq / sr;
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            // low shelf
            0 => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cs + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - k),
                    (a + 1.0) + (a - 1.0) * cs + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - k,
                )
            }
            // high shelf
            2 => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - k),
                    (a + 1.0) - (a - 1.0) * cs + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - k,
                )
            }
            // peaking
            _ => (1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
        };
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [a1 / a0, a2 / a0] }
    }

    fn run(&self, x: &[f32]) -> Vec<f32> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        x.iter()
            .map(|&x0| {
                let y0 = self.b[0] * x0 + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                x2 = x1;
                x1 = x0;
                y2 = y1;
                y1 = y0;
                y0
            })
            .collect()
    }
}

/// Band centres for the equalizer: low shelf, three peaks, high shelf.
pub const EQ_BANDS: [f32; 5] = [100.0, 400.0, 1200.0, 3500.0, 9000.0];

/// Five-band equalizer; `gains_db` lines up with `EQ_BANDS`.
pub fn equalize(clip: &Audio, gains_db: [f32; 5]) -> Audio {
    let sr = clip.rate as f32;
    let filters: Vec<Biquad> = EQ_BANDS
        .iter()
        .zip(gains_db)
        .enumerate()
        .filter(|(_, (_, g))| g.abs() > 0.01)
        .map(|(i, (&f, g))| {
            let kind = match i {
                0 => 0,
                4 => 2,
                _ => 1,
            };
            Biquad::new(kind, sr, f.min(sr * 0.45), g, if kind == 1 { 1.0 } else { 0.707 })
        })
        .collect();
    map_channels(clip, |c| filters.iter().fold(c.to_vec(), |acc, f| f.run(&acc)))
}

/// Linear interpolation through curve points (t in 0..1, sorted by t).
pub fn curve_at(points: &[(f32, f32)], t: f32) -> f32 {
    match points {
        [] => 0.0,
        [only] => only.1,
        _ => {
            if t <= points[0].0 {
                return points[0].1;
            }
            for w in points.windows(2) {
                let ((t0, v0), (t1, v1)) = (w[0], w[1]);
                if t <= t1 {
                    let k = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
                    return v0 + (v1 - v0) * k;
                }
            }
            points[points.len() - 1].1
        }
    }
}

/// Volume envelope: `points` are (position 0..1, gain) pairs.
pub fn envelope(clip: &Audio, points: &[(f32, f32)]) -> Audio {
    let n = clip.frames().max(1) as f32;
    map_channels(clip, |c| c.iter().enumerate().map(|(i, s)| s * curve_at(points, i as f32 / n)).collect())
}

/// Bender: tape-style pitch bend following a curve of (position 0..1, semitones). Bending up
/// plays faster, so the result gets shorter (and longer when bending down).
pub fn bender(clip: &Audio, points: &[(f32, f32)]) -> Audio {
    let n = clip.frames();
    if n < 2 {
        return clip.clone();
    }
    let mut positions = Vec::with_capacity(n);
    let mut p = 0.0f64;
    while p < (n - 1) as f64 {
        positions.push(p);
        let st = curve_at(points, (p / n as f64) as f32) as f64;
        p += 2f64.powf(st / 12.0);
    }
    map_channels(clip, |c| {
        positions
            .iter()
            .map(|&p| {
                let i = p as usize;
                let t = (p - i as f64) as f32;
                c[i] * (1.0 - t) + c[(i + 1).min(n - 1)] * t
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(secs: f32) -> Audio {
        tone(44100, 2, secs, 440.0, -6.0, Wave::Sine)
    }

    #[test]
    fn lengths() {
        let a = sine(2.0);
        let n = a.frames();
        assert_eq!(echo(&a, 200.0, 0.5, 0.5).frames(), n);
        assert_eq!(reverb(&a, 0.5, 0.3).frames(), n);
        assert_eq!(flanger(&a, 3.0, 0.5, 0.5, 0.5).frames(), n);
        assert_eq!(pitch_shift(&a, 5.0).frames(), n);
        let fast = change_tempo(&a, 200.0).frames();
        assert!((fast as i64 - n as i64 / 2).abs() < 100, "{fast}");
        let slow = change_speed(&a, 50.0).frames();
        assert!((slow as i64 - n as i64 * 2).abs() < 10, "{slow}");
        assert_eq!(mix_into(&a, n - 10, &a, 0.5).frames(), 2 * n - 10);
    }

    #[test]
    fn effects_stay_finite_and_bounded() {
        let a = sine(1.0);
        for out in [reverb(&a, 1.0, 1.0), echo(&a, 50.0, 0.95, 1.0), flanger(&a, 10.0, 2.0, 0.9, 1.0), change_tempo(&a, 70.0), pitch_shift(&a, -7.0)] {
            let peak = out.peak(0, out.frames());
            assert!(peak.is_finite() && peak < 20.0, "peak {peak}");
        }
    }

    #[test]
    fn gate_makes_exact_zeros() {
        let mut a = silence(1000, 1, 1.0);
        a.channels[0][100] = 0.5;
        a.channels[0][500] = 0.001; // below -40 dB: gated
        let g = noise_gate(&a, -40.0, 10.0);
        assert_eq!(g.channels[0][100], 0.5);
        assert_eq!(g.channels[0][500], 0.0);
        assert_eq!(g.find_sounds(10), vec![(100, 101)]);
    }

    #[test]
    fn conversions_eq_and_curves() {
        let a = sine(1.0);
        assert_eq!(convert_rate(&a, 22050).frames(), 22050);
        assert_eq!(convert_channels(&a, 1).num_channels(), 1);
        let q = quantize(&a, 8);
        assert!(q.channels[0].iter().all(|s| ((s * 127.0).round() - s * 127.0).abs() < 1e-3));
        // Flat EQ leaves the sound alone; a boost raises the level of a 100 Hz tone.
        let flat = equalize(&a, [0.0; 5]);
        assert_eq!(flat.channels, a.channels);
        let low = tone(44100, 1, 1.0, 100.0, -12.0, Wave::Sine);
        let boosted = equalize(&low, [9.0, 0.0, 0.0, 0.0, 0.0]);
        assert!(boosted.peak(4410, 44100) > low.peak(4410, 44100) * 1.5);
        assert_eq!(curve_at(&[(0.0, 0.0), (1.0, 2.0)], 0.25), 0.5);
        let env = envelope(&a, &[(0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(env.peak(0, env.frames()), 0.0);
        let up = bender(&a, &[(0.0, 12.0), (1.0, 12.0)]).frames();
        assert!((up as i64 - a.frames() as i64 / 2).abs() < 10, "{up}");
    }

    #[test]
    fn dtmf_keys() {
        assert_eq!(dtmf_freqs('5'), Some((770.0, 1336.0)));
        assert_eq!(dtmf_freqs('#'), Some((941.0, 1477.0)));
        assert_eq!(dtmf_freqs('x'), None);
        let a = dtmf(8000, 1, "12x", 100.0, 50.0, -6.0);
        assert_eq!(a.frames(), 2 * (800 + 400));
        assert!(a.peak(0, a.frames()) <= 0.51);
    }

    #[test]
    fn mixes_tracks() {
        let a = Audio { rate: 10, channels: vec![vec![0.5; 4]] };
        let b = Audio { rate: 10, channels: vec![vec![0.25; 6], vec![-0.25; 6]] };
        let m = mix(&[(&a, 1.0), (&b, 2.0)], 10);
        assert_eq!(m.frames(), 6);
        assert_eq!(m.num_channels(), 2);
        assert_eq!(m.channels[0][0], 1.0);
        assert_eq!(m.channels[1][0], 0.0);
        assert_eq!(m.channels[1][5], -0.5);
    }
}
