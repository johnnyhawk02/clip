//! Playback through the chosen output device (cpal; the Mac's default unless Preferences ▸
//! Audio picks one). The stream runs for the app's lifetime, rebuilt when the audio setup
//! changes; the UI controls it through a small shared state.

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
    /// The loop has come round since playing started (for `heard_position`).
    pub wrapped: bool,
    /// Song frames per output frame, as last played.
    pub step: f64,
    /// Preferences ▸ Audio's test tone: a quiet 440 Hz sine over whatever plays.
    pub test_tone: bool,
}

/// The audio setup, as Live's Preferences ▸ Audio: devices by name and the sample rate and
/// buffer size asked for (`None` = the Mac's default).
#[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioSetup {
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub rate: Option<u32>,
    #[serde(default)]
    pub buffer: Option<u32>,
}

/// What the output stream is actually running at.
#[derive(Clone, Default)]
pub struct Running {
    pub device: String,
    pub rate: u32,
    /// Frames per buffer (`None` when the device's own default was left).
    pub buffer: Option<u32>,
}

/// The sample rates offered in Preferences (those a device supports).
pub const RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];
/// The buffer sizes offered (those a device supports).
pub const BUFFERS: [u32; 8] = [32, 64, 128, 256, 512, 1024, 2048, 4096];

/// The names of the Mac's output (or input) devices.
pub fn devices(output: bool) -> Vec<String> {
    let host = cpal::default_host();
    let list = if output { host.output_devices() } else { host.input_devices() };
    list.map(|ds| ds.map(|d| d.to_string()).collect()).unwrap_or_default()
}

/// The device named `name` (or the default when `None` or it's gone).
fn find_device(name: Option<&str>, output: bool) -> Option<cpal::Device> {
    let host = cpal::default_host();
    let named = name.and_then(|n| {
        let list = if output { host.output_devices() } else { host.input_devices() };
        list.ok()?.find(|d| d.to_string() == n)
    });
    named.or_else(|| if output { host.default_output_device() } else { host.default_input_device() })
}

/// The sample rates and buffer sizes output device `name` supports, and its default rate.
pub fn output_options(name: Option<&str>) -> (Vec<u32>, Vec<u32>, u32) {
    let Some(d) = find_device(name, true) else { return (Vec::new(), Vec::new(), 44100) };
    let default = d.default_output_config().map_or(44100, |c| c.sample_rate());
    let configs: Vec<_> = d.supported_output_configs().map(|c| c.collect()).unwrap_or_default();
    let rates = RATES.iter().copied().filter(|&r| configs.iter().any(|c| (c.min_sample_rate()..=c.max_sample_rate()).contains(&r))).collect();
    let buffers = BUFFERS
        .iter()
        .copied()
        .filter(|&b| configs.iter().any(|c| matches!(c.buffer_size(), cpal::SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&b)) || matches!(c.buffer_size(), cpal::SupportedBufferSize::Unknown)))
        .collect();
    (rates, buffers, default)
}

/// Input device `name`, for the recorder (the default when it's gone).
pub fn input_device(name: Option<&str>) -> Option<cpal::Device> {
    find_device(name, false)
}

pub struct Player {
    pub shared: Arc<Mutex<Shared>>,
    _stream: Option<cpal::Stream>,
    pub error: Option<String>,
    pub running: Running,
}

impl Player {
    pub fn new(setup: &AudioSetup) -> Player {
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
            wrapped: false,
            step: 1.0,
            test_tone: false,
        }));
        let mut p = Player { shared, _stream: None, error: None, running: Running::default() };
        p.reopen(setup);
        p
    }

    /// (Re)start the output stream with `setup`, carrying on from where playback was.
    pub fn reopen(&mut self, setup: &AudioSetup) {
        self._stream = None;
        match build_stream(self.shared.clone(), setup) {
            Ok((stream, running)) => {
                (self._stream, self.running, self.error) = (Some(stream), running, None);
            }
            Err(e) => {
                // Fall back to the Mac's default rather than staying silent.
                let fallback = build_stream(self.shared.clone(), &AudioSetup { input: setup.input.clone(), ..Default::default() });
                if let Ok((stream, running)) = fallback {
                    (self._stream, self.running) = (Some(stream), running);
                }
                self.error = Some(e);
            }
        }
    }

    pub fn set_test_tone(&self, on: bool) {
        self.shared.lock().unwrap().test_tone = on;
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
            for l in new.every() {
                if let Some(o) = old.every().find(|o| o.track == l.track) {
                    // Only while the chain is the same effects in the same order.
                    if o.devices.len() == l.devices.len() && o.devices.iter().zip(&l.devices).all(|(a, b)| a.0.name() == b.0.name()) {
                        let state = std::mem::take(&mut *o.device_state.lock().unwrap());
                        *l.device_state.lock().unwrap() = state;
                    }
                    // Delay compensation carries on whatever the chain (it crossfades to a new delay).
                    std::mem::swap(&mut *o.comp.lock().unwrap(), &mut *l.comp.lock().unwrap());
                }
            }
        }
        if let (Source::Song(old), Source::Song(new)) = (&s.source, &source) {
            std::mem::swap(&mut *old.bus.lock().unwrap(), &mut *new.bus.lock().unwrap());
        }
        s.source = source;
        s.end = s.end.min(n);
        s.start = s.start.min(s.end);
        if s.pos as usize >= s.end {
            s.playing = false;
            s.pos = s.start as f64;
        }
    }

    /// Play `start..end` (looping it or not) from frame `at` inside it.
    pub fn play_from(&self, start: usize, end: usize, at: usize, looping: bool) {
        self.play(start, end, looping);
        let mut s = self.shared.lock().unwrap();
        if s.playing && at >= s.start && at < s.end {
            s.pos = at as f64;
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
        s.wrapped = false;
    }

    pub fn stop(&self) {
        self.shared.lock().unwrap().playing = false;
    }

    /// Play on within `start..end` from where it is now: from before it, on into it (as Live);
    /// from past its end, round from `start`.
    pub fn set_range(&self, start: usize, end: usize, looping: bool) {
        let mut s = self.shared.lock().unwrap();
        let end = end.min(s.source.frames());
        if start >= end {
            return;
        }
        (s.start, s.end, s.looping) = (start, end, looping);
        if s.pos >= end as f64 {
            s.pos = start as f64;
        }
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

    /// Where the transport is as heard: behind `position` by the song's delay compensation (as
    /// Live's playhead), round the loop's end once it has come round.
    pub fn heard_position(&self) -> usize {
        let s = self.shared.lock().unwrap();
        let late = match &s.source {
            Source::Song(song) => song.latency() as f64 * s.step,
            Source::Buffer(_) => 0.0,
        };
        let at = s.pos - late;
        if at >= s.start as f64 {
            at as usize
        } else if s.wrapped && s.end > s.start {
            (s.end as f64 - (s.start as f64 - at)).max(s.start as f64) as usize
        } else {
            s.start
        }
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

    /// Device `i` of track `t`: a Spacer's cuts right now (dB per band, two rows), when running.
    pub fn spacer_cuts(&self, t: usize, i: usize) -> Option<[[f32; crate::spacer::BANDS]; 2]> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.every().find(|l| l.track == t)?.device_state.lock().unwrap().get(i)?.as_ref()?.spacer_cuts(),
            Source::Buffer(_) => None,
        }
    }

    /// Device `i` of track `t`: a Tame's cut curve and level right now, when running.
    pub fn tame_view(&self, t: usize, i: usize) -> Option<([f32; crate::tame::CURVE], [f32; crate::tame::CURVE])> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.every().find(|l| l.track == t)?.device_state.lock().unwrap().get(i)?.as_ref()?.tame_view(),
            Source::Buffer(_) => None,
        }
    }

    /// Device `i` of track `t`: an Opto's gain reduction right now (dB), when running.
    pub fn opto_gr(&self, t: usize, i: usize) -> Option<f32> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.every().find(|l| l.track == t)?.device_state.lock().unwrap().get(i)?.as_ref()?.opto_gr(),
            Source::Buffer(_) => None,
        }
    }

    /// Device `i` of track `t`: an Auto Filter's cutoff right now (Hz) and sequencer step, when running.
    pub fn filter_view(&self, t: usize, i: usize) -> Option<(f32, Option<usize>)> {
        match &self.shared.lock().unwrap().source {
            Source::Song(song) => song.every().find(|l| l.track == t)?.device_state.lock().unwrap().get(i)?.as_ref()?.filter_view(),
            Source::Buffer(_) => None,
        }
    }

    pub fn take_peaks(&self) -> [f32; 2] {
        std::mem::take(&mut self.shared.lock().unwrap().peak)
    }
}

fn build_stream(shared: Arc<Mutex<Shared>>, setup: &AudioSetup) -> Result<(cpal::Stream, Running), String> {
    let device = find_device(setup.output.as_deref(), true).ok_or("No audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.into();
    if let Some(r) = setup.rate {
        config.sample_rate = r;
    }
    if let Some(b) = setup.buffer {
        config.buffer_size = cpal::BufferSize::Fixed(b);
    }
    let running = Running { device: device.to_string(), rate: config.sample_rate, buffer: setup.buffer };
    let stream = match format {
        SampleFormat::F32 => run::<f32>(&device, config, shared),
        SampleFormat::I16 => run::<i16>(&device, config, shared),
        SampleFormat::I32 => run::<i32>(&device, config, shared),
        SampleFormat::U16 => run::<u16>(&device, config, shared),
        SampleFormat::F64 => run::<f64>(&device, config, shared),
        other => Err(format!("Unsupported output sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, running))
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
    // Scripted test runs can play silently (SOUND_EDIT_MUTE), so nothing reaches the speakers.
    let level = if std::env::var_os("SOUND_EDIT_MUTE").is_some() { 0.0 } else { 1.0 };
    let mut tone = 0.0f64;
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
                s.step = step * s.speed;
                let crossfeed = s.crossfeed && out_ch >= 2;
                // The song is mixed for the whole block up front.
                let frames = data.len() / out_ch.max(1);
                if mix.len() < frames {
                    mix.resize(frames, [0.0; 2]);
                }
                if let (true, Source::Song(song)) = (s.playing, &s.source) {
                    song.render(s.pos, step * s.speed, &mut mix[..frames]);
                }
                // The test tone: −20 dB, 440 Hz, on every channel.
                let tone_on = s.test_tone;
                let mut next_tone = || {
                    if !tone_on {
                        return 0.0;
                    }
                    tone = (tone + 440.0 / out_rate).fract();
                    (tone * std::f64::consts::TAU).sin() as f32 * 0.1
                };
                for (k, frame) in data.chunks_mut(out_ch).enumerate() {
                    let t = next_tone();
                    if !s.playing {
                        xfeed.reset();
                        frame.fill(T::from_sample(t * level));
                        continue;
                    }
                    if s.pos >= s.end as f64 {
                        if s.looping && s.end > s.start {
                            s.pos = s.start as f64;
                            s.wrapped = true;
                            // Mix from the top again for the rest of this block.
                            if let Source::Song(song) = &s.source {
                                song.render(s.pos, step * s.speed, &mut mix[k..frames]);
                            }
                        } else {
                            s.playing = false;
                            s.pos = s.end as f64;
                            frame.fill(T::from_sample(t * level));
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
                        *out = T::from_sample(((v + t) * level).clamp(-1.0, 1.0));
                    }
                    s.pos += step * if matches!(s.source, Source::Song(_)) { s.speed } else { 1.0 };
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
