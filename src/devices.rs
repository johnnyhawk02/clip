//! Built-in audio effects (devices, as Live calls them) a track can carry, in a chain.

use serde::{Deserialize, Serialize};

use crate::fx;
use crate::spacer;

/// One device in a track's chain, with its settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Device {
    Spacer(spacer::Settings),
    Reverb(fx::ReverbSettings),
    Delay(fx::DelaySettings),
    Spring(fx::SpringSettings),
}

/// The effects there are to add: name and a line on what it does.
pub const ALL: &[(&str, &str)] = &[
    ("Reverb", "A room or hall around the sound"),
    ("Spring", "A spring reverb tank: chirping, boingy echoes (after Parker's and Välimäki's models)"),
    ("Delay", "Echoes in time with the song"),
    ("Spacer", "Ducks this track only at the frequencies another track is playing, so they sit together"),
];

impl Device {
    /// A new device by name (from `ALL`), for a track among `tracks` at index `me`.
    pub fn new(name: &str, me: usize, tracks: usize) -> Option<Device> {
        match name {
            "Spacer" => Some(Device::Spacer(spacer::Settings::new(if me == 0 && tracks > 1 { 1 } else { 0 }))),
            "Reverb" => Some(Device::Reverb(fx::ReverbSettings::default())),
            "Delay" => Some(Device::Delay(fx::DelaySettings::default())),
            "Spring" => Some(Device::Spring(fx::SpringSettings::default())),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Device::Spacer(_) => "Spacer",
            Device::Reverb(_) => "Reverb",
            Device::Delay(_) => "Delay",
            Device::Spring(_) => "Spring",
        }
    }

    pub fn on(&self) -> bool {
        match self {
            Device::Spacer(s) => s.on,
            Device::Reverb(s) => s.on,
            Device::Delay(s) => s.on,
            Device::Spring(s) => s.on,
        }
    }

    pub fn set_on(&mut self, on: bool) {
        match self {
            Device::Spacer(s) => s.on = on,
            Device::Reverb(s) => s.on = on,
            Device::Delay(s) => s.on = on,
            Device::Spring(s) => s.on = on,
        }
    }

    /// The other track it listens to, if it has one.
    pub fn source(&self) -> Option<usize> {
        match self {
            Device::Spacer(s) => Some(s.source),
            Device::Reverb(_) | Device::Delay(_) | Device::Spring(_) => None,
        }
    }

    /// Point it at another track (after tracks move); `None` when its track is gone.
    pub fn remap(mut self, f: &impl Fn(usize) -> Option<usize>) -> Option<Device> {
        match &mut self {
            Device::Spacer(s) => s.source = f(s.source)?,
            Device::Reverb(_) | Device::Delay(_) | Device::Spring(_) => {}
        }
        Some(self)
    }
}

/// A device's running state on the audio thread.
pub enum State {
    Spacer(spacer::Spacer),
    Reverb(fx::Reverb),
    Delay(fx::Delay),
    Spring(fx::Spring),
}

impl State {
    pub fn new(d: &Device, rate: u32) -> State {
        match d {
            Device::Spacer(_) => State::Spacer(spacer::Spacer::new(rate)),
            Device::Reverb(_) => State::Reverb(fx::Reverb::new(rate)),
            Device::Delay(_) => State::Delay(fx::Delay::new(rate)),
            Device::Spring(_) => State::Spring(fx::Spring::new(rate)),
        }
    }

    /// Run device `d` on `main` (stereo frames, in place), with `side` the sound of the track it
    /// listens to (if any); `beat` is a beat's length in frames (for tempo-synced effects).
    pub fn process(&mut self, d: &Device, main: &mut [[f32; 2]], side: Option<&[[f32; 2]]>, beat: f64) {
        match (self, d) {
            (State::Spacer(st), Device::Spacer(s)) => {
                if let Some(side) = side {
                    st.process(s, main, side);
                }
            }
            (State::Reverb(st), Device::Reverb(s)) => st.process(s, main),
            (State::Delay(st), Device::Delay(s)) => st.process(s, main, beat),
            (State::Spring(st), Device::Spring(s)) => st.process(s, main),
            _ => {}
        }
    }

    /// The Spacer's current cuts, for its display.
    pub fn spacer_cuts(&self) -> Option<[[f32; spacer::BANDS]; 2]> {
        match self {
            State::Spacer(s) => Some(s.cut),
            _ => None,
        }
    }
}
