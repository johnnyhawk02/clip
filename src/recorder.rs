//! Recording from the default input device (cpal).

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::audio::Audio;

#[derive(Default)]
struct Shared {
    /// Interleaved samples captured so far.
    samples: Vec<f32>,
    /// Peak level since the UI last read it.
    peak: f32,
}

pub struct Recorder {
    _stream: cpal::Stream,
    shared: Arc<Mutex<Shared>>,
    pub rate: u32,
    pub channels: usize,
}

impl Recorder {
    /// Start capturing from the default input straight away.
    pub fn start() -> Result<Recorder, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("No audio input device found")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let (rate, channels) = (config.sample_rate, config.channels as usize);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, shared.clone()),
            SampleFormat::I16 => build::<i16>(&device, config, shared.clone()),
            SampleFormat::I32 => build::<i32>(&device, config, shared.clone()),
            SampleFormat::U16 => build::<u16>(&device, config, shared.clone()),
            other => Err(format!("Unsupported input sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recorder { _stream: stream, shared, rate, channels: channels.max(1) })
    }

    pub fn seconds(&self) -> f64 {
        let n = self.shared.lock().unwrap().samples.len();
        n as f64 / self.channels as f64 / self.rate as f64
    }

    /// Peak input level since the last call (0..1).
    pub fn take_peak(&self) -> f32 {
        let mut s = self.shared.lock().unwrap();
        std::mem::take(&mut s.peak)
    }

    /// Stop recording and return what was captured.
    pub fn finish(self) -> Audio {
        let samples = std::mem::take(&mut self.shared.lock().unwrap().samples);
        drop(self._stream);
        deinterleave(&samples, self.channels, self.rate)
    }
}

pub fn deinterleave(samples: &[f32], channels: usize, rate: u32) -> Audio {
    let channels = channels.max(1);
    let mut out = vec![Vec::with_capacity(samples.len() / channels); channels];
    for frame in samples.chunks_exact(channels) {
        for (c, s) in out.iter_mut().zip(frame) {
            c.push(*s);
        }
    }
    Audio { rate, channels: out }
}

fn build<T>(device: &cpal::Device, config: StreamConfig, shared: Arc<Mutex<Shared>>) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut s = shared.lock().unwrap();
                for &x in data {
                    let v = x.to_sample::<f32>();
                    s.peak = s.peak.max(v.abs());
                    s.samples.push(v);
                }
            },
            |err| eprintln!("input stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn deinterleaves() {
        let a = super::deinterleave(&[1.0, -1.0, 0.5, -0.5, 0.25, -0.25], 2, 8000);
        assert_eq!(a.channels, vec![vec![1.0, 0.5, 0.25], vec![-1.0, -0.5, -0.25]]);
        assert_eq!(a.rate, 8000);
    }
}
