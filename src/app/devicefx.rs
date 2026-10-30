//! Track effects used offline, as the sound-editor effects are: a track device's settings in a
//! dialog, printed into the selection (or the whole clip) when you press Apply.

use super::deviceview::{delay_panel, reverb_panel, spring_panel};
use super::*;
use crate::devices::{Device, State};

/// Seconds of sound run through before a wrapped clip, so its tail has come round in full.
const WRAP_LEAD_SECS: f64 = 10.0;

impl App {
    /// Open the dialog for device `name` (from `devices::ALL`), with the settings it last had.
    pub(super) fn open_device_fx(&mut self, name: &str) {
        let last = self.device_fx_last.iter().find(|d| d.name() == name).copied();
        // New ones start part wet (a return track's start all wet), as they're heard over the dry sound.
        let mut d = last.or_else(|| {
            let mut d = Device::new(name, 0, 1)?;
            match &mut d {
                Device::Reverb(s) => s.mix = 0.3,
                Device::Spring(s) => s.mix = 0.3,
                Device::Delay(s) => s.mix = 0.35,
                Device::Spacer(_) => {}
            }
            Some(d)
        });
        if let Some(d) = d.as_mut() {
            d.set_on(true);
        }
        self.device_fx = d;
    }

    pub(super) fn device_fx_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.device_fx else { return };
        let whole = self.sel.is_none();
        let scope = if whole { "the whole clip" } else { "the selection" };
        let mut wrap = self.device_fx_wrap;
        let (mut apply, mut cancel) = (false, false);
        let resp = dialog_modal("device-fx").show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            dialog_header(ui, &format!("{} Device", d.name()), &format!("The track effect, printed into {scope}."));
            match &mut d {
                Device::Reverb(s) => _ = reverb_panel(ui, s),
                Device::Delay(s) => _ = delay_panel(ui, s),
                Device::Spring(s) => _ = spring_panel(ui, s),
                Device::Spacer(_) => {}
            }
            if whole {
                ui.add_space(8.0);
                ui.checkbox(&mut wrap, "Wrap the tail round to the start (for loops)");
            }
            (apply, cancel) = dialog_buttons(ui, "Apply", true, |_| {});
        });
        apply |= ctx.input(|i| i.key_pressed(Key::Enter));
        self.device_fx_wrap = wrap;
        self.device_fx_last.retain(|o| o.name() != d.name());
        self.device_fx_last.push(d);
        if cancel || resp.should_close() {
            self.device_fx = None;
        } else if apply {
            self.device_fx = None;
            self.apply_device_fx(d, whole && wrap);
        } else {
            self.device_fx = Some(d);
        }
    }

    /// Run device `d` over the selection (or the whole sound). With `wrap`, the sound is played
    /// round a few times first, so what rings on past its end comes back in at its start.
    pub(super) fn apply_device_fx(&mut self, d: Device, wrap: bool) {
        let beat = self.beat_frames();
        let label = match d {
            Device::Reverb(_) => "Reverb Device",
            Device::Delay(_) => "Delay Device",
            Device::Spring(_) => "Spring Device",
            Device::Spacer(_) => return,
        };
        self.process(label, |a| {
            let n = a.frames();
            let nch = a.num_channels().max(1);
            let at = |c: usize, i: usize| a.channels.get(c.min(nch - 1)).and_then(|v| v.get(i)).copied().unwrap_or(0.0);
            let lead = if wrap && n > 0 { ((WRAP_LEAD_SECS * a.rate as f64) / n as f64).ceil().clamp(1.0, 64.0) as usize } else { 0 };
            let mut frames: Vec<[f32; 2]> = (0..n * (lead + 1)).map(|k| [at(0, k % n.max(1)), at(1, k % n.max(1))]).collect();
            State::new(&d, a.rate).process(&d, &mut frames, None, beat);
            let out = &frames[n * lead..];
            let channels = if nch == 1 {
                vec![out.iter().map(|f| (f[0] + f[1]) * 0.5).collect()]
            } else {
                (0..nch).map(|c| if c < 2 { out.iter().map(|f| f[c]).collect() } else { a.channels[c].clone() }).collect()
            };
            Audio { rate: a.rate, channels }
        });
    }
}
