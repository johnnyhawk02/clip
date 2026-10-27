//! Audio buffer, decoding (symphonia), WAV export (hound) and editing operations.

use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Planar f32 audio. Every channel has the same length.
#[derive(Clone)]
pub struct Audio {
    pub rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, |c| c.len())
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }

    pub fn bytes(&self) -> usize {
        self.frames() * self.num_channels() * 4
    }

    pub fn empty_like(&self) -> Audio {
        Audio { rate: self.rate, channels: vec![Vec::new(); self.num_channels()] }
    }

    /// Copy of frames in `a..b`.
    pub fn slice(&self, a: usize, b: usize) -> Audio {
        Audio {
            rate: self.rate,
            channels: self.channels.iter().map(|c| c[a..b].to_vec()).collect(),
        }
    }

    /// Keep only `a..b`.
    pub fn crop(&self, a: usize, b: usize) -> Audio {
        self.slice(a, b)
    }

    /// Remove `a..b`.
    pub fn delete(&self, a: usize, b: usize) -> Audio {
        Audio {
            rate: self.rate,
            channels: self
                .channels
                .iter()
                .map(|c| {
                    let mut v = Vec::with_capacity(c.len() - (b - a));
                    v.extend_from_slice(&c[..a]);
                    v.extend_from_slice(&c[b..]);
                    v
                })
                .collect(),
        }
    }

    /// Insert `clip` at frame `at`, replacing `at..replace_end` (pass `at` to just insert).
    /// The clip's channels are mapped onto ours (mono clip fills every channel).
    pub fn insert(&self, at: usize, replace_end: usize, clip: &Audio) -> Audio {
        let clip = clip.resampled(self.rate);
        Audio {
            rate: self.rate,
            channels: self
                .channels
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let src = &clip.channels[i.min(clip.num_channels().saturating_sub(1))];
                    let mut v = Vec::with_capacity(c.len() + src.len());
                    v.extend_from_slice(&c[..at]);
                    v.extend_from_slice(src);
                    v.extend_from_slice(&c[replace_end..]);
                    v
                })
                .collect(),
        }
    }

    /// Apply `f(sample, position 0..1 within range)` to every sample in `a..b`.
    pub fn map_range(&self, a: usize, b: usize, f: impl Fn(f32, f32) -> f32) -> Audio {
        let mut out = self.clone();
        let len = (b - a).max(1) as f32;
        for c in &mut out.channels {
            for (i, s) in c[a..b].iter_mut().enumerate() {
                *s = f(*s, i as f32 / len);
            }
        }
        out
    }

    pub fn fade_in(&self, a: usize, b: usize) -> Audio {
        // Equal-power-ish curve sounds more natural than linear.
        self.map_range(a, b, |s, t| s * (t * std::f32::consts::FRAC_PI_2).sin())
    }

    pub fn fade_out(&self, a: usize, b: usize) -> Audio {
        self.map_range(a, b, |s, t| s * (t * std::f32::consts::FRAC_PI_2).cos())
    }

    pub fn gain(&self, a: usize, b: usize, g: f32) -> Audio {
        self.map_range(a, b, |s, _| s * g)
    }

    pub fn silence(&self, a: usize, b: usize) -> Audio {
        self.map_range(a, b, |_, _| 0.0)
    }

    pub fn reverse(&self, a: usize, b: usize) -> Audio {
        let mut out = self.clone();
        for c in &mut out.channels {
            c[a..b].reverse();
        }
        out
    }

    /// Find the separate sounds in a recording. A sound starts at the first frame that
    /// isn't exactly zero, and ends once `min_gap` consecutive zero frames follow it.
    /// Returned ranges run from the first to the last non-zero frame (end exclusive).
    pub fn find_sounds(&self, min_gap: usize) -> Vec<(usize, usize)> {
        let min_gap = min_gap.max(1);
        let is_zero = |i: usize| self.channels.iter().all(|c| c[i] == 0.0);
        let mut sounds = Vec::new();
        let mut start: Option<usize> = None;
        let mut zeros = 0;
        for i in 0..self.frames() {
            if is_zero(i) {
                if let Some(s) = start {
                    zeros += 1;
                    if zeros >= min_gap {
                        sounds.push((s, i + 1 - zeros));
                        start = None;
                    }
                }
            } else {
                if start.is_none() {
                    start = Some(i);
                }
                zeros = 0;
            }
        }
        if let Some(s) = start {
            sounds.push((s, self.frames() - zeros));
        }
        sounds
    }

    /// The zero crossing (of all channels summed) nearest to `f`, searching `radius` frames
    /// either way.
    pub fn nearest_zero_crossing(&self, f: usize, radius: usize) -> Option<usize> {
        let n = self.frames();
        let m = |i: usize| self.channels.iter().map(|c| c[i]).sum::<f32>();
        let crossing = |i: usize| i > 0 && i < n && (m(i) == 0.0 || (m(i - 1) < 0.0) != (m(i) < 0.0));
        (0..=radius).find_map(|d| {
            if crossing(f + d) {
                Some(f + d)
            } else if d <= f && crossing(f - d) {
                Some(f - d)
            } else {
                None
            }
        })
    }

    pub fn peak(&self, a: usize, b: usize) -> f32 {
        self.channels
            .iter()
            .flat_map(|c| c[a..b].iter())
            .fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Scale `a..b` so its peak hits `target_db` dBFS. Returns None if the range is silent.
    pub fn normalize(&self, a: usize, b: usize, target_db: f32) -> Option<Audio> {
        let peak = self.peak(a, b);
        if peak <= 1e-9 {
            return None;
        }
        let target = 10f32.powf(target_db / 20.0);
        Some(self.gain(a, b, target / peak))
    }

    /// Down-mix to mono (average of channels).
    pub fn to_mono(&self) -> Audio {
        let n = self.num_channels().max(1) as f32;
        let frames = self.frames();
        let mut m = vec![0.0f32; frames];
        for c in &self.channels {
            for (o, s) in m.iter_mut().zip(c) {
                *o += s / n;
            }
        }
        Audio { rate: self.rate, channels: vec![m] }
    }

    /// Linear-interpolation resample (used only when pasting clips between files).
    pub fn resampled(&self, rate: u32) -> Audio {
        if rate == self.rate || self.frames() == 0 {
            return self.clone();
        }
        let ratio = self.rate as f64 / rate as f64;
        let out_len = ((self.frames() as f64) / ratio).floor() as usize;
        Audio {
            rate,
            channels: self
                .channels
                .iter()
                .map(|c| {
                    (0..out_len)
                        .map(|i| {
                            let p = i as f64 * ratio;
                            let i0 = p as usize;
                            let i1 = (i0 + 1).min(c.len() - 1);
                            let t = (p - i0 as f64) as f32;
                            c[i0] * (1.0 - t) + c[i1] * t
                        })
                        .collect()
                })
                .collect(),
        }
    }
}

/// Decode any format symphonia supports (wav, mp3, flac, ogg/vorbis, aac/m4a, aiff, caf...).
pub fn load(path: &Path) -> Result<Audio, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Can't open file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("Unsupported format: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("No audio track found")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("Missing codec parameters")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("Unsupported codec: {e}"))?;
    let track_id = track.id;

    let mut rate = 0u32;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::ResetRequired) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                if channels.first().is_some_and(|c| !c.is_empty()) {
                    break; // keep what decoded so far
                }
                return Err(format!("Read error: {e}"));
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let n_ch = buf.spec().channels().count().max(1);
                if channels.is_empty() {
                    rate = buf.spec().rate();
                    channels = vec![Vec::new(); n_ch];
                }
                scratch.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut scratch);
                for frame in scratch.chunks_exact(n_ch) {
                    for (ch, s) in channels.iter_mut().zip(frame) {
                        ch.push(*s);
                    }
                }
            }
            Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
            Err(e) => return Err(format!("Decode error: {e}")),
        }
    }

    if channels.is_empty() || rate == 0 {
        return Err("File contains no audio".into());
    }
    Ok(Audio { rate, channels })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WavFormat {
    Pcm8,
    Pcm16,
    Pcm24,
    Float32,
}

impl WavFormat {
    pub fn label(self) -> &'static str {
        match self {
            WavFormat::Pcm8 => "8-bit PCM",
            WavFormat::Pcm16 => "16-bit PCM",
            WavFormat::Pcm24 => "24-bit PCM",
            WavFormat::Float32 => "32-bit float",
        }
    }
}

pub fn save_wav(audio: &Audio, path: &Path, fmt: WavFormat) -> Result<(), String> {
    let (bits, sample_format) = match fmt {
        WavFormat::Pcm8 => (8, hound::SampleFormat::Int),
        WavFormat::Pcm16 => (16, hound::SampleFormat::Int),
        WavFormat::Pcm24 => (24, hound::SampleFormat::Int),
        WavFormat::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec {
        channels: audio.num_channels() as u16,
        sample_rate: audio.rate,
        bits_per_sample: bits,
        sample_format,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let scale = match fmt {
        WavFormat::Pcm8 => 127.0,
        WavFormat::Pcm16 => i16::MAX as f32,
        WavFormat::Pcm24 => 8_388_607.0,
        WavFormat::Float32 => 1.0,
    };
    for i in 0..audio.frames() {
        for c in &audio.channels {
            let s = c[i];
            let r = match fmt {
                WavFormat::Float32 => w.write_sample(s),
                _ => w.write_sample((s.clamp(-1.0, 1.0) * scale).round() as i32),
            };
            r.map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

/// A named position in the sound, saved in WAV files as cue points.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub frame: usize,
    pub name: String,
}

/// A named stretch of the sound, saved in WAV files as a cue point with a length (`ltxt`).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub name: String,
}

/// Everything saved alongside the audio in a WAV file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Markers {
    pub cues: Vec<Cue>,
    pub regions: Vec<Region>,
    /// Loop points (start, end), saved in a `smpl` chunk.
    pub loop_points: Option<(usize, usize)>,
}

impl Markers {
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty() && self.regions.is_empty() && self.loop_points.is_none()
    }
}

/// Append cue (`cue `), label/region (`LIST adtl`) and loop (`smpl`) chunks to a finished WAV file.
pub fn write_wav_markers(path: &Path, m: &Markers, rate: u32) -> Result<(), String> {
    if m.is_empty() {
        return Ok(());
    }
    let mut bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let u32le = |v: usize| (v as u32).to_le_bytes();
    // Cue ids: cues first, then regions.
    let points: Vec<(usize, &str, Option<usize>)> = m
        .cues
        .iter()
        .map(|c| (c.frame, c.name.as_str(), None))
        .chain(m.regions.iter().map(|r| (r.start, r.name.as_str(), Some(r.end - r.start))))
        .collect();
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    if !points.is_empty() {
        let mut cue = Vec::new();
        cue.extend(u32le(points.len()));
        for (i, (frame, _, _)) in points.iter().enumerate() {
            cue.extend(u32le(i + 1)); // id
            cue.extend(u32le(*frame)); // position
            cue.extend(b"data");
            cue.extend(u32le(0)); // chunk start
            cue.extend(u32le(0)); // block start
            cue.extend(u32le(*frame)); // sample offset
        }
        let mut adtl = b"adtl".to_vec();
        let mut sub = |id: &[u8; 4], body: Vec<u8>| {
            adtl.extend(id);
            adtl.extend(u32le(body.len()));
            adtl.extend(&body);
            if body.len() % 2 == 1 {
                adtl.push(0);
            }
        };
        for (i, (_, name, len)) in points.iter().enumerate() {
            let mut text = name.as_bytes().to_vec();
            text.push(0);
            let mut labl = u32le(i + 1).to_vec();
            labl.extend(&text);
            sub(b"labl", labl);
            if let Some(len) = len {
                let mut ltxt = u32le(i + 1).to_vec();
                ltxt.extend(u32le(*len));
                ltxt.extend(b"rgn ");
                ltxt.extend([0u8; 8]); // country, language, dialect, code page
                sub(b"ltxt", ltxt);
            }
        }
        chunks.push((*b"cue ", cue));
        chunks.push((*b"LIST", adtl));
    }
    if let Some((s, e)) = m.loop_points {
        let mut smpl = Vec::new();
        for v in [0, 0, (1_000_000_000.0 / rate as f64) as usize, 60, 0, 0, 0, 1, 0] {
            smpl.extend(u32le(v)); // manufacturer, product, period, unity note, ..., 1 loop
        }
        for v in [0, 0, s, e.saturating_sub(1), 0, 0] {
            smpl.extend(u32le(v)); // id, forward loop, start, end (inclusive), fraction, play count
        }
        chunks.push((*b"smpl", smpl));
    }
    for (id, body) in chunks {
        bytes.extend(id);
        bytes.extend(u32le(body.len()));
        bytes.extend(&body);
        if body.len() % 2 == 1 {
            bytes.push(0);
        }
    }
    let riff = u32le(bytes.len() - 8);
    bytes[4..8].copy_from_slice(&riff);
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Read cue points, regions and loop points from a WAV file. Returns nothing for other files.
pub fn read_wav_markers(path: &Path) -> Markers {
    let Ok(bytes) = std::fs::read(path) else { return Markers::default() };
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Markers::default();
    }
    let rd = |b: &[u8], at: usize| b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let mut positions: Vec<(usize, usize)> = Vec::new();
    let mut labels: std::collections::HashMap<usize, String> = Default::default();
    let mut lengths: std::collections::HashMap<usize, usize> = Default::default();
    let mut m = Markers::default();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = rd(&bytes, at + 4);
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"cue " {
            let n = rd(body, 0);
            for i in 0..n {
                let base = 4 + i * 24;
                if base + 24 <= body.len() {
                    positions.push((rd(body, base), rd(body, base + 20)));
                }
            }
        } else if id == b"LIST" && body.get(0..4) == Some(b"adtl") {
            let mut p = 4;
            while p + 8 <= body.len() {
                let sub = &body[p..p + 4];
                let len = rd(body, p + 4);
                if sub == b"labl" && p + 12 <= body.len() {
                    let text = &body[p + 12..(p + 8 + len).min(body.len())];
                    let name = String::from_utf8_lossy(text).trim_end_matches('\0').to_string();
                    labels.insert(rd(body, p + 8), name);
                } else if sub == b"ltxt" && p + 16 <= body.len() {
                    lengths.insert(rd(body, p + 8), rd(body, p + 12));
                }
                p += 8 + len + (len % 2);
            }
        } else if id == b"smpl" && rd(body, 28) > 0 && body.len() >= 60 {
            let (s, e) = (rd(body, 44), rd(body, 48));
            if e >= s {
                m.loop_points = Some((s, e + 1));
            }
        }
        at += 8 + size + (size % 2);
    }
    for (id, frame) in positions {
        let name = labels.get(&id).cloned();
        match lengths.get(&id) {
            Some(&len) if len > 0 => m.regions.push(Region {
                start: frame,
                end: frame + len,
                name: name.unwrap_or_else(|| format!("Region {id}")),
            }),
            _ => m.cues.push(Cue { frame, name: name.unwrap_or_else(|| format!("Cue {id}")) }),
        }
    }
    m.cues.sort_by_key(|c| c.frame);
    m.regions.sort_by_key(|r| r.start);
    m
}

/// Write an AIFF file (16-bit, or 8/24-bit; float formats are written as 24-bit).
pub fn save_aiff(audio: &Audio, path: &Path, fmt: WavFormat) -> Result<(), String> {
    let bits: u16 = match fmt {
        WavFormat::Pcm8 => 8,
        WavFormat::Pcm16 => 16,
        WavFormat::Pcm24 | WavFormat::Float32 => 24,
    };
    let bytes_per = (bits / 8) as usize;
    let (n, ch) = (audio.frames(), audio.num_channels());
    let data_len = n * ch * bytes_per;
    let mut out = Vec::with_capacity(54 + data_len);
    let be32 = |v: usize| (v as u32).to_be_bytes();
    out.extend(b"FORM");
    out.extend(be32(4 + 26 + 16 + data_len));
    out.extend(b"AIFF");
    out.extend(b"COMM");
    out.extend(be32(18));
    out.extend((ch as u16).to_be_bytes());
    out.extend(be32(n));
    out.extend(bits.to_be_bytes());
    out.extend(extended80(audio.rate as f64));
    out.extend(b"SSND");
    out.extend(be32(8 + data_len));
    out.extend(be32(0));
    out.extend(be32(0));
    let scale = ((1i64 << (bits - 1)) - 1) as f32;
    for i in 0..n {
        for c in &audio.channels {
            let v = (c[i].clamp(-1.0, 1.0) * scale).round() as i32;
            out.extend(&v.to_be_bytes()[4 - bytes_per..]);
        }
    }
    std::fs::write(path, out).map_err(|e| e.to_string())
}

/// IEEE 754 80-bit extended float, as AIFF stores the sample rate.
fn extended80(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if v <= 0.0 {
        return out;
    }
    let exp = v.log2().floor() as i32;
    let mantissa = (v / 2f64.powi(exp) * (1u64 << 63) as f64) as u64;
    out[0..2].copy_from_slice(&((exp + 16383) as u16).to_be_bytes());
    out[2..10].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Save as WAV or AIFF depending on the file extension.
pub fn save_any(audio: &Audio, path: &Path, fmt: WavFormat) -> Result<(), String> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("aif") | Some("aiff") => save_aiff(audio, path, fmt),
        _ => save_wav(audio, path, fmt),
    }
}

/// Min/max summary of a channel in fixed-size blocks, for fast zoomed-out waveform drawing.
pub struct Peaks {
    pub block: usize,
    /// Per channel: (min, max) per block.
    pub data: Vec<Vec<(f32, f32)>>,
}

impl Peaks {
    pub const BLOCK: usize = 256;

    pub fn build(audio: &Audio) -> Peaks {
        let data = audio
            .channels
            .iter()
            .map(|c| {
                c.chunks(Self::BLOCK)
                    .map(|b| {
                        b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)))
                    })
                    .collect()
            })
            .collect();
        Peaks { block: Self::BLOCK, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Audio {
        Audio { rate: 1000, channels: vec![(0..n).map(|i| i as f32 / n as f32).collect(); 2] }
    }

    #[test]
    fn edits_keep_lengths_consistent() {
        let a = ramp(100);
        assert_eq!(a.crop(10, 30).frames(), 20);
        assert_eq!(a.delete(10, 30).frames(), 80);
        let clip = a.slice(0, 5);
        assert_eq!(a.insert(50, 50, &clip).frames(), 105);
        assert_eq!(a.insert(50, 60, &clip).frames(), 95);
        let n = a.normalize(0, 100, 0.0).unwrap();
        assert!((n.peak(0, 100) - 1.0).abs() < 1e-5);
        let f = a.fade_in(0, 100);
        assert_eq!(f.channels[0][0], 0.0);
    }

    #[test]
    fn find_sounds_splits_on_zero_runs() {
        let mut c = vec![0.0f32; 10];
        c.extend([0.5, -0.2, 0.0, 0.3]); // sound 1: 10..14 (short gap of one zero inside)
        c.extend(vec![0.0; 10]);
        c.extend([0.1, 0.0, 0.0, 0.0, 0.2]); // sound 2: 24..29 (gap shorter than 10)
        c.extend(vec![0.0; 9]);
        c.extend([0.4]); // still sound 2: only 9 zeros before it
        c.extend(vec![0.0; 3]); // file ends mid-gap
        let a = Audio { rate: 1000, channels: vec![c] };
        assert_eq!(a.find_sounds(10), vec![(10, 14), (24, 39)]);
        assert_eq!(a.find_sounds(3), vec![(10, 14), (24, 25), (28, 29), (38, 39)]);
    }

    #[test]
    fn wav_roundtrip() {
        let a = ramp(500);
        let dir = std::env::temp_dir().join("sound_edit_test.wav");
        save_wav(&a, &dir, WavFormat::Pcm16).unwrap();
        let b = load(&dir).unwrap();
        assert_eq!(b.frames(), 500);
        save_wav(&a, &dir, WavFormat::Pcm8).unwrap();
        let c = load(&dir).unwrap();
        assert_eq!(c.frames(), 500);
        assert!((c.channels[0][250] - a.channels[0][250]).abs() < 0.02);
        assert_eq!(b.num_channels(), 2);
        assert_eq!(b.rate, 1000);
        std::fs::remove_file(dir).ok();
    }

    #[test]
    fn decodes_sample_files() {
        for name in ["test-speech.aiff", "test-speech.m4a"] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name);
            if p.exists() {
                let a = load(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(a.duration_secs() > 1.0, "{name} too short");
                println!("{name}: {} Hz, {} ch, {:.2}s", a.rate, a.num_channels(), a.duration_secs());
            }
        }
    }

    #[test]
    fn wav_cues_roundtrip() {
        let a = ramp(1000);
        let path = std::env::temp_dir().join("sound_edit_cues.wav");
        save_wav(&a, &path, WavFormat::Pcm16).unwrap();
        let m = Markers {
            cues: vec![Cue { frame: 10, name: "Start".into() }, Cue { frame: 500, name: "Middle bit".into() }],
            regions: vec![Region { start: 100, end: 300, name: "Verse".into() }],
            loop_points: Some((200, 800)),
        };
        write_wav_markers(&path, &m, 1000).unwrap();
        assert_eq!(read_wav_markers(&path), m);
        // The audio still reads back correctly with the extra chunks.
        assert_eq!(load(&path).unwrap().frames(), 1000);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn zero_crossings() {
        // 100 Hz sine at 1 kHz: crosses zero every 5 frames.
        let c: Vec<f32> = (0..100).map(|i| (i as f32 * 0.2 * std::f32::consts::PI + 0.3).sin()).collect();
        let a = Audio { rate: 1000, channels: vec![c] };
        let z = a.nearest_zero_crossing(12, 10).unwrap();
        assert!((z as i64 - 12).abs() <= 3, "{z}");
        assert!(a.channels[0][z - 1].signum() != a.channels[0][z].signum());
        assert_eq!(Audio { rate: 1000, channels: vec![vec![0.5; 50]] }.nearest_zero_crossing(20, 10), None);
    }

    #[test]
    fn aiff_roundtrip() {
        let a = ramp(400);
        let path = std::env::temp_dir().join("sound_edit_test.aiff");
        save_any(&a, &path, WavFormat::Pcm16).unwrap();
        let b = load(&path).unwrap();
        assert_eq!((b.frames(), b.num_channels(), b.rate), (400, 2, 1000));
        assert!((b.channels[1][200] - a.channels[1][200]).abs() < 1e-3);
        std::fs::remove_file(path).ok();
    }
}
