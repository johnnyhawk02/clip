//! Playback through the default output device (cpal). The stream runs for the app's
//! lifetime; the UI controls it through a small shared state.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::audio::Audio;
use crate::crossfeed::Crossfeed;
use crate::engine::Song;

/// What the player plays: a finished buffer (the clip editor), or the song, mixed live.
pub enum Source {
    Buffer(Arc<Audio>),
    Song(Arc<Song>),
}

impl Source {
    fn frames(&self) -> usize {
        match self {
            Source::Buffer(a) => a.frames(),
            Source::Song(s) => s.end,
        }
    }

    fn rate(&self) -> u32 {
        match self {
            Source::Buffer(a) => a.rate,
            Source::Song(s) => s.rate,
        }
    }
}

pub struct Shared {
    pub source: Source,
    /// Current position in source frames (fractional, for rate conversion).
    pub pos: f64,
    /// Song frames per source frame of time: above 1 while a faster tempo is being dragged in,
    /// before the song is laid out for it (the song re-pitches live, as on a record).
    pub speed: f64,
    /// Loudest output per side since the meters last looked.
    pub peak: [f32; 2],
    pub start: usize,
    pub end: usize,
    pub playing: bool,
    pub looping: bool,
    /// Headphone crossfeed on what's heard.
    pub crossfeed: bool,
}

pub struct Player {
    pub shared: Arc<Mutex<Shared>>,
    _stream: Option<cpal::Stream>,
    pub error: Option<String>,
}

impl Player {
    pub fn new() -> Player {
        let shared = Arc::new(Mutex::new(Shared {
            source: Source::Buffer(Arc::new(Audio { rate: 44100, channels: vec![Vec::new()] })),
            pos: 0.0,
            speed: 1.0,
            peak: [0.0; 2],
            start: 0,
            end: 0,
            playing: false,
            looping: false,
            crossfeed: false,
        }));
        match build_stream(shared.clone()) {
            Ok(stream) => {
                Player { shared, _stream: Some(stream), error: None }
            }
            Err(e) => Player { shared, _stream: None, error: Some(e) },
        }
    }

    pub fn set_audio(&self, audio: Arc<Audio>) {
        self.set_source(Source::Buffer(audio));
    }

    pub fn set_song(&self, song: Arc<Song>) {
        self.set_source(Source::Song(song));
    }

    fn set_source(&self, source: Source) {
        let mut s = self.shared.lock().unwrap();
        let n = source.frames();
        // A new song keeps each track's Spacer running (its envelopes), so edits don't glitch it.
        if let (Source::Song(old), Source::Song(new)) = (&s.source, &source) {
            for l in &new.layers {
                if let Some(o) = old.layers.iter().find(|o| o.track == l.track) {
                    let state = o.spacer_state.lock().unwrap().take();
                    *l.spacer_state.lock().unwrap() = state;
                }
            }
        }
        s.source = source;
        s.end = s.end.min(n);
        s.start = s.start.min(s.end);
        if s.pos as usize >= s.end {
            s.playing = false;
            s.pos = s.start as f64;
        }
    }

    pub fn play(&self, start: usize, end: usize, looping: bool) {
        let mut s = self.shared.lock().unwrap();
        let n = s.source.frames();
        let end = end.min(n);
        if start >= end {
            return;
        }
        s.start = start;
        s.end = end;
        s.pos = start as f64;
        s.looping = looping;
        s.playing = true;
    }

    pub fn stop(&self) {
        self.shared.lock().unwrap().playing = false;
    }

    pub fn set_looping(&self, looping: bool) {
        self.shared.lock().unwrap().looping = looping;
    }

    pub fn set_crossfeed(&self, on: bool) {
        self.shared.lock().unwrap().crossfeed = on;
    }

    pub fn is_playing(&self) -> bool {
        self.shared.lock().unwrap().playing
    }

    pub fn position(&self) -> usize {
        self.shared.lock().unwrap().pos as usize
    }

    /// Play the song this much faster (a tempo being dragged in; see `Shared::speed`).
    pub fn set_speed(&self, speed: f64) {
        self.shared.lock().unwrap().speed = speed;
    }

    /// The song was laid out again at a new tempo: frame positions scale by `r` and the speed
    /// goes back to 1, so the playhead stays on the same beat.
    pub fn rescale(&self, r: f64) {
        let mut s = self.shared.lock().unwrap();
        s.pos *= r;
        s.start = (s.start as f64 * r).round() as usize;
        s.end = (s.end as f64 * r).round() as usize;
        s.speed = 1.0;
    }

    /// The loudest output on each side since the last call (for the level meters).
    /// Each heard track's loudest level since the last look (song playback only).
    pub fn take_track_peaks(&self) -> Vec<(usize, f32)> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.take_track_peaks(),
            Source::Buffer(_) => Vec::new(),
        }
    }

    /// Track `t`'s Spacer cuts right now (dB per band, two rows), when it's running.
    pub fn spacer_cuts(&self, t: usize) -> Option<[[f32; crate::spacer::BANDS]; 2]> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.layers.iter().find(|l| l.track == t)?.spacer_state.lock().unwrap().as_ref().map(|s| s.cut),
            Source::Buffer(_) => None,
        }
    }

    pub fn take_peaks(&self) -> [f32; 2] {
        std::mem::take(&mut self.shared.lock().unwrap().peak)
    }
}

fn build_stream(shared: Arc<Mutex<Shared>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => run::<f32>(&device, config, shared),
        SampleFormat::I16 => run::<i16>(&device, config, shared),
        SampleFormat::I32 => run::<i32>(&device, config, shared),
        SampleFormat::U16 => run::<u16>(&device, config, shared),
        SampleFormat::F64 => run::<f64>(&device, config, shared),
        other => Err(format!("Unsupported output sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn run<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let out_ch = config.channels as usize;
    let out_rate = config.sample_rate as f64;
    let mut xfeed = Crossfeed::new(out_rate);
    let mut mix: Vec<[f32; 2]> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                // Never block the audio thread: if the UI holds the lock, output silence.
                let Ok(mut s) = shared.try_lock() else {
                    data.fill(T::from_sample(0.0));
                    return;
                };
                let step = s.source.rate() as f64 / out_rate;
                let crossfeed = s.crossfeed && out_ch >= 2;
                // The song is mixed for the whole block up front.
                let frames = data.len() / out_ch.max(1);
                if mix.len() < frames {
                    mix.resize(frames, [0.0; 2]);
                }
                if let (true, Source::Song(song)) = (s.playing, &s.source) {
                    song.render(s.pos, step * s.speed, &mut mix[..frames]);
                }
                for (k, frame) in data.chunks_mut(out_ch).enumerate() {
                    if !s.playing {
                        xfeed.reset();
                        frame.fill(T::from_sample(0.0));
                        continue;
                    }
                    if s.pos >= s.end as f64 {
                        if s.looping && s.end > s.start {
                            s.pos = s.start as f64;
                            // Mix from the top again for the rest of this block.
                            if let Source::Song(song) = &s.source {
                                song.render(s.pos, step * s.speed, &mut mix[k..frames]);
                            }
                        } else {
                            s.playing = false;
                            s.pos = s.end as f64;
                            frame.fill(T::from_sample(0.0));
                            continue;
                        }
                    }
                    let (l, r) = match &s.source {
                        Source::Song(_) => (mix[k][0], mix[k][1]),
                        Source::Buffer(audio) => {
                            let i0 = s.pos as usize;
                            let t = (s.pos - i0 as f64) as f32;
                            let last = audio.frames().saturating_sub(1);
                            let i1 = (i0 + 1).min(last);
                            let n_src = audio.num_channels();
                            let sample = |c: usize| {
                                if n_src == 0 || i0 > last {
                                    0.0
                                } else {
                                    let ch = &audio.channels[c.min(n_src - 1)];
                                    ch[i0] * (1.0 - t) + ch[i1] * t
                                }
                            };
                            (sample(0), sample(1))
                        }
                    };
                    s.peak = [s.peak[0].max(l.abs()), s.peak[1].max(r.abs())];
                    let (l, r) = if crossfeed { xfeed.process(l, r) } else { (l, r) };
                    for (c, out) in frame.iter_mut().enumerate() {
                        let v = match c {
                            0 => l,
                            1 => r,
                            _ => 0.0,
                        };
                        *out = T::from_sample(v.clamp(-1.0, 1.0));
                    }
                    s.pos += step * if matches!(s.source, Source::Song(_)) { s.speed } else { 1.0 };
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
