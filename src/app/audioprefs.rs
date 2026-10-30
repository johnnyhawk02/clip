//! Preferences ▸ Audio, as Live's: the output and input devices, sample rate and buffer size,
//! the latency they give, and a test tone. Changes are heard straight away (the output stream
//! is rebuilt); Cancel puts back the setup the window opened with.

use super::*;
use crate::player::{self, AudioSetup};

/// The devices and what the output device supports, read when the tab opens (asking Core
/// Audio every frame would be slow).
#[derive(Default)]
pub(super) struct AudioLists {
    outputs: Vec<String>,
    inputs: Vec<String>,
    rates: Vec<u32>,
    buffers: Vec<u32>,
    default_rate: u32,
}

/// A file holding the audio setup (JSON).
fn audio_pref() -> PathBuf {
    crossfeed_pref().with_file_name("audio-setup.json")
}

/// The saved audio setup (the Mac's defaults when there is none).
pub(super) fn load_audio_setup() -> AudioSetup {
    std::fs::read(audio_pref()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

impl App {
    fn refresh_audio_lists(&mut self) {
        let (rates, buffers, default_rate) = player::output_options(self.audio_setup.output.as_deref());
        self.audio_lists = Some(AudioLists { outputs: player::devices(true), inputs: player::devices(false), rates, buffers, default_rate });
    }

    /// Restart the output with the current setup.
    fn apply_audio_setup(&mut self) {
        self.player.reopen(&self.audio_setup);
        self.status = match &self.player.error {
            Some(e) => format!("Couldn't use that audio setup ({e}): using the Mac's default output"),
            None => format!("Audio output: {} at {} Hz", self.player.running.device, self.player.running.rate),
        };
    }

    /// The Audio tab's controls.
    pub(super) fn audio_prefs(&mut self, ui: &mut egui::Ui) {
        if self.audio_lists.is_none() {
            self.refresh_audio_lists();
        }
        let before = self.audio_setup.clone();
        let lists = self.audio_lists.take().unwrap_or_default();
        let mut setup = before.clone();
        let default_name = "Default (as the Mac's Sound settings)";
        let running = self.player.running.clone();
        dialog_grid("audioprefs").show(ui, |ui| {
            ui.label("Driver Type");
            ui.label("CoreAudio");
            ui.end_row();
            for (label, output, list) in [("Audio Input Device", false, &lists.inputs), ("Audio Output Device", true, &lists.outputs)] {
                ui.label(label);
                let pick = if output { &mut setup.output } else { &mut setup.input };
                egui::ComboBox::from_id_salt(label).width(230.0).selected_text(pick.clone().unwrap_or(default_name.into())).show_ui(ui, |ui| {
                    ui.selectable_value(pick, None, default_name);
                    for n in list {
                        ui.selectable_value(pick, Some(n.clone()), n);
                    }
                });
                ui.end_row();
            }
            ui.label("Sample Rate");
            let rate_text = |r: Option<u32>| match r {
                Some(r) => format!("{r} Hz"),
                None => format!("Device default ({} Hz)", lists.default_rate),
            };
            egui::ComboBox::from_id_salt("audiorate").width(230.0).selected_text(rate_text(setup.rate)).show_ui(ui, |ui| {
                ui.selectable_value(&mut setup.rate, None, rate_text(None));
                for &r in &lists.rates {
                    ui.selectable_value(&mut setup.rate, Some(r), rate_text(Some(r)));
                }
            });
            ui.end_row();
            ui.label("Buffer Size");
            let rate = running.rate.max(1) as f32;
            let buf_text = |b: Option<u32>| match b {
                Some(b) => format!("{b} Samples ({:.1} ms)", b as f32 / rate * 1000.0),
                None => "Device default".into(),
            };
            egui::ComboBox::from_id_salt("audiobuf").width(230.0).selected_text(buf_text(setup.buffer)).show_ui(ui, |ui| {
                ui.selectable_value(&mut setup.buffer, None, buf_text(None));
                for &b in &lists.buffers {
                    ui.selectable_value(&mut setup.buffer, Some(b), buf_text(Some(b)));
                }
            });
            ui.end_row();
            ui.label("Output Latency");
            let late = running.buffer.map_or("set by the device".into(), |b| format!("{:.1} ms", b as f32 / rate * 1000.0));
            ui.label(egui::RichText::new(late).color(muted()));
            ui.end_row();
            ui.label("Test Tone");
            let mut tone = self.test_tone;
            if ui.checkbox(&mut tone, "440 Hz, −20 dB").changed() {
                self.test_tone = tone;
                self.player.set_test_tone(tone);
            }
            ui.end_row();
        });
        ui.label(egui::RichText::new(format!("Playing through {} at {} Hz.", running.device, running.rate)).color(muted()));
        self.audio_lists = Some(lists);
        if setup != before {
            if setup.output != before.output {
                // Another device supports other rates and sizes: start it at its own.
                (setup.rate, setup.buffer) = (None, None);
            }
            self.audio_setup = setup;
            if self.audio_setup.output != before.output || self.audio_setup.rate != before.rate || self.audio_setup.buffer != before.buffer {
                self.apply_audio_setup();
            }
            self.refresh_audio_lists();
        }
    }

    /// Preferences closed: keep the setup (Done) or put back the one it opened with (Cancel).
    pub(super) fn close_audio_prefs(&mut self, keep: bool, before: AudioSetup) {
        self.audio_lists = None;
        if self.test_tone {
            self.test_tone = false;
            self.player.set_test_tone(false);
        }
        if !keep {
            if self.audio_setup != before {
                self.audio_setup = before;
                self.apply_audio_setup();
            }
            return;
        }
        if std::env::var_os("SOUND_EDIT_ACTIONS").is_none() {
            if let Ok(json) = serde_json::to_vec_pretty(&self.audio_setup) {
                let p = audio_pref();
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, json);
            }
        }
    }
}
