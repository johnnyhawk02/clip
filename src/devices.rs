//! Built-in audio effects (devices, as Live calls them) a track can carry, in a chain.

use serde::{Deserialize, Serialize};

use crate::spacer;

/// One device in a track's chain, with its settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Device {
    Spacer(spacer::Settings),
}

/// The effects there are to add: name and a line on what it does.
pub const ALL: &[(&str, &str)] = &[("Spacer", "Ducks this track only at the frequencies another track is playing, so they sit together")];

impl Device {
    /// A new device by name (from `ALL`), for a track among `tracks` at index `me`.
    pub fn new(name: &str, me: usize, tracks: usize) -> Option<Device> {
        match name {
            "Spacer" => Some(Device::Spacer(spacer::Settings::new(if me == 0 && tracks > 1 { 1 } else { 0 }))),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Device::Spacer(_) => "Spacer",
        }
    }

    pub fn on(&self) -> bool {
        match self {
            Device::Spacer(s) => s.on,
        }
    }

    pub fn set_on(&mut self, on: bool) {
        match self {
            Device::Spacer(s) => s.on = on,
        }
    }

    /// The other track it listens to, if it has one.
    pub fn source(&self) -> Option<usize> {
        match self {
            Device::Spacer(s) => Some(s.source),
        }
    }

    /// Point it at another track (after tracks move); `None` when its track is gone.
    pub fn remap(mut self, f: &impl Fn(usize) -> Option<usize>) -> Option<Device> {
        match &mut self {
            Device::Spacer(s) => s.source = f(s.source)?,
        }
        Some(self)
    }
}

/// A device's running state on the audio thread.
pub enum State {
    Spacer(spacer::Spacer),
}

impl State {
    pub fn new(d: &Device, rate: u32) -> State {
        match d {
            Device::Spacer(_) => State::Spacer(spacer::Spacer::new(rate)),
        }
    }

    /// Run device `d` on `main` (stereo frames, in place), with `side` the sound of the track it
    /// listens to (if any).
    pub fn process(&mut self, d: &Device, main: &mut [[f32; 2]], side: Option<&[[f32; 2]]>) {
        match (self, d) {
            (State::Spacer(st), Device::Spacer(s)) => {
                if let Some(side) = side {
                    st.process(s, main, side);
                }
            }
        }
    }

    /// The Spacer's current cuts, for its display.
    pub fn spacer_cuts(&self) -> Option<[[f32; spacer::BANDS]; 2]> {
        match self {
            State::Spacer(s) => Some(s.cut),
        }
    }
}
