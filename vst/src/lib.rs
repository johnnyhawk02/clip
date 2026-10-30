//! Clip's devices as standalone VST3 plugins, built one at a time by cargo feature.
//! The DSP is Clip's own source, included as it stands; nothing here changes how it sounds.

#![allow(dead_code)]

#[path = "../../src/autofilter.rs"]
mod autofilter;
#[path = "../../src/chorus.rs"]
mod chorus;
#[path = "../../src/devices.rs"]
mod devices;
#[path = "../../src/dynamics.rs"]
mod dynamics;
#[path = "../../src/fx.rs"]
mod fx;
#[path = "../../src/opto.rs"]
mod opto;
#[path = "../../src/oversample.rs"]
mod oversample;
#[path = "../../src/phaser.rs"]
mod phaser;
#[path = "../../src/pultec.rs"]
mod pultec;
#[path = "../../src/riser.rs"]
mod riser;
#[path = "../../src/spacer.rs"]
mod spacer;
#[path = "../../src/tame.rs"]
mod tame;
#[path = "../../src/tape.rs"]
mod tape;

mod effects;
mod knobs;

use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{EguiState, create_egui_editor, egui, widgets::generic_ui};

use devices::{Device, State};
use knobs::{Knob, List, Spec};

/// Audio goes through the device this many frames at a time, so knobs move smoothly.
const CHUNK: usize = 64;

/// One of Clip's devices, as a plugin.
pub trait Effect: 'static + Send + Sync {
    const NAME: &'static str;
    const CLASS_ID: [u8; 16];
    const SUBCATEGORIES: &'static [Vst3SubCategory];
    /// Takes a stereo sidechain input.
    const SIDECHAIN: bool = false;
    type Settings: Copy + Send + Sync + 'static;
    fn settings() -> Self::Settings;
    fn device(s: Self::Settings) -> Device;
    fn knobs(list: List<Self::Settings>) -> List<Self::Settings>;
}

pub struct FxParams<S> {
    knobs: Vec<(String, Knob)>,
    specs: Vec<Spec<S>>,
    editor: Arc<EguiState>,
}

// Safety: the pointers are to params in `knobs`, which never changes after `new`, and the Arc
// holding this struct stays put as long as they're used.
unsafe impl<S: 'static + Send + Sync> Params for FxParams<S> {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.knobs.iter().map(|(id, k)| (id.clone(), k.ptr(), String::new())).collect()
    }
}

pub struct ClipFx<E: Effect> {
    params: Arc<FxParams<E::Settings>>,
    state: Option<State>,
    rate: u32,
    /// The song position when the host has none, or isn't playing.
    free_pos: u64,
    /// Audio has gone through since the last reset.
    used: bool,
    effect: PhantomData<E>,
}

impl<E: Effect> Default for ClipFx<E> {
    fn default() -> Self {
        let list = E::knobs(List::new(E::settings()));
        let knobs = list.specs.iter().map(|s| (s.id(), s.knob())).collect::<Vec<_>>();
        let height = (70 + 26 * knobs.len() as u32).min(620);
        ClipFx {
            params: Arc::new(FxParams { knobs, specs: list.specs, editor: EguiState::from_size(440, height) }),
            state: None,
            rate: 44100,
            free_pos: 0,
            used: false,
            effect: PhantomData,
        }
    }
}

impl<E: Effect> Plugin for ClipFx<E> {
    const NAME: &'static str = E::NAME;
    const VENDOR: &'static str = "Johnny";
    const URL: &'static str = "https://johnnyhawk02.github.io/clip/";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = if E::SIDECHAIN {
        &[AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            names: PortNames { aux_inputs: &["Sidechain"], ..PortNames::const_default() },
            ..AudioIOLayout::const_default()
        }]
    } else {
        &[AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        }]
    };

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        create_egui_editor(self.params.editor.clone(), (), |_, _| {}, move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading(E::NAME);
                ui.add_space(6.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    generic_ui::create(ui, params.clone(), setter, generic_ui::GenericSlider);
                });
            });
        })
    }

    fn initialize(&mut self, _: &AudioIOLayout, config: &BufferConfig, context: &mut impl InitContext<Self>) -> bool {
        self.rate = config.sample_rate as u32;
        let device = E::device(E::settings());
        self.state = Some(State::new(&device, self.rate));
        context.set_latency_samples(device.latency() as u32);
        true
    }

    /// Clear echoes and tails (hosts call this around starting, not while playing).
    fn reset(&mut self) {
        if self.used {
            self.state = Some(State::new(&E::device(E::settings()), self.rate));
            self.used = false;
        }
        self.free_pos = 0;
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let Some(state) = self.state.as_mut() else { return ProcessStatus::Normal };
        self.used = true;
        let transport = context.transport();
        let beat = self.rate as f64 * 60.0 / transport.tempo.unwrap_or(120.0).max(1.0);
        let song_pos = transport.pos_samples().filter(|_| transport.playing);
        let start = song_pos.map_or(self.free_pos as f64, |p| p.max(0) as f64);

        let frames = buffer.samples();
        let out = buffer.as_slice();
        let side = aux.inputs.first_mut().map(|b| b.as_slice_immutable());
        let mut main = [[0.0f32; 2]; CHUNK];
        let mut sc = [[0.0f32; 2]; CHUNK];
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            for i in 0..n {
                main[i] = [out[0][done + i], out[1][done + i]];
            }
            if let Some(side) = &side {
                for i in 0..n {
                    sc[i] = [side[0][done + i], side[1][done + i]];
                }
            }
            let mut s = E::settings();
            for ((_, knob), spec) in self.params.knobs.iter().zip(&self.params.specs) {
                spec.put(&mut s, knob.next(n));
            }
            let device = E::device(s);
            state.process(&device, &mut main[..n], E::SIDECHAIN.then_some(&sc[..n]), beat, start + done as f64);
            for i in 0..n {
                out[0][done + i] = main[i][0];
                out[1][done + i] = main[i][1];
            }
            done += n;
        }
        self.free_pos += frames as u64;
        // Reverbs and echoes ring on after the input stops.
        ProcessStatus::KeepAlive
    }
}

impl<E: Effect> Vst3Plugin for ClipFx<E> {
    const VST3_CLASS_ID: [u8; 16] = E::CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = E::SUBCATEGORIES;
}
