//! Playback through the default output device (cpal). The stream runs for the app's
//! lifetime; the UI controls it through a small shared state.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::audio::Audio;

pub struct Shared {
    pub audio: Arc<Audio>,
    /// Current position in source frames (fractional, for rate conversion).
    pub pos: f64,
    pub start: usize,
    pub end: usize,
    pub playing: bool,
    pub looping: bool,
}

pub struct Player {
    pub shared: Arc<Mutex<Shared>>,
    _stream: Option<cpal::Stream>,
    pub error: Option<String>,
}

impl Player {
    pub fn new() -> Player {
        let shared = Arc::new(Mutex::new(Shared {
            audio: Arc::new(Audio { rate: 44100, channels: vec![Vec::new()] }),
            pos: 0.0,
            start: 0,
            end: 0,
            playing: false,
            looping: false,
        }));
        match build_stream(shared.clone()) {
            Ok(stream) => {
                Player { shared, _stream: Some(stream), error: None }
            }
            Err(e) => Player { shared, _stream: None, error: Some(e) },
        }
    }

    pub fn set_audio(&self, audio: Arc<Audio>) {
        let mut s = self.shared.lock().unwrap();
        let n = audio.frames();
        s.audio = audio;
        s.end = s.end.min(n);
        s.start = s.start.min(s.end);
        if s.pos as usize >= s.end {
            s.playing = false;
            s.pos = s.start as f64;
        }
    }

    pub fn play(&self, start: usize, end: usize, looping: bool) {
        let mut s = self.shared.lock().unwrap();
        let n = s.audio.frames();
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

    pub fn is_playing(&self) -> bool {
        self.shared.lock().unwrap().playing
    }

    pub fn position(&self) -> usize {
        self.shared.lock().unwrap().pos as usize
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
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                // Never block the audio thread: if the UI holds the lock, output silence.
                let Ok(mut s) = shared.try_lock() else {
                    data.fill(T::from_sample(0.0));
                    return;
                };
                let step = s.audio.rate as f64 / out_rate;
                for frame in data.chunks_mut(out_ch) {
                    if !s.playing {
                        frame.fill(T::from_sample(0.0));
                        continue;
                    }
                    if s.pos >= s.end as f64 {
                        if s.looping && s.end > s.start {
                            s.pos = s.start as f64;
                        } else {
                            s.playing = false;
                            s.pos = s.end as f64;
                            frame.fill(T::from_sample(0.0));
                            continue;
                        }
                    }
                    let i0 = s.pos as usize;
                    let t = (s.pos - i0 as f64) as f32;
                    let audio = &s.audio;
                    let last = audio.frames().saturating_sub(1);
                    let i1 = (i0 + 1).min(last);
                    let n_src = audio.num_channels();
                    for (c, out) in frame.iter_mut().enumerate() {
                        let v = if n_src == 0 || i0 > last {
                            0.0
                        } else {
                            let ch = &audio.channels[c.min(n_src - 1)];
                            ch[i0] * (1.0 - t) + ch[i1] * t
                        };
                        *out = T::from_sample(v.clamp(-1.0, 1.0));
                    }
                    s.pos += step;
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
