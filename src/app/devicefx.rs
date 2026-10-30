//! Track effects used offline, as the sound-editor effects are: a track device's settings in a
//! dialog, printed into the selection (or the whole clip) when you press Apply.

use super::deviceview::{delay_panel, reverb_panel, spring_panel};
use super::*;
use crate::devices::{Device, State};

/// Seconds of sound run through before a wrapped clip, so its tail has come round in full.
const WRAP_LEAD_SECS: f64 = 10.0;
/// The longest tail let ring on past the end, and the level it rings down to (-80 dB).
const TAIL_MAX_SECS: f64 = 30.0;
const TAIL_FLOOR: f32 = 1e-4;

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
            dialog_header(ui, &format!("{} Device", d.name()), &format!("The track effect, printed into {scope}. Its tail rings on past the end."));
            match &mut d {
                Device::Reverb(s) => _ = reverb_panel(ui, s),
                Device::Delay(s) => _ = delay_panel(ui, s),
                Device::Spring(s) => _ = spring_panel(ui, s),
                Device::Spacer(_) => {}
            }
            if whole {
                ui.add_space(8.0);
                ui.checkbox(&mut wrap, "Wrap the tail round to the start instead (a loop keeps its length)");
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

    /// Run device `d` over the selection (or the whole sound). Its tail rings on past the end:
    /// mixed over what follows, and lengthening the sound if it runs past its end. With `wrap`,
    /// the sound is played round a few times first instead, so the tail comes back in at its
    /// start and a loop keeps its length.
    pub(super) fn apply_device_fx(&mut self, d: Device, wrap: bool) {
        let label = match d {
            Device::Reverb(_) => "Reverb Device",
            Device::Delay(_) => "Delay Device",
            Device::Spring(_) => "Spring Device",
            Device::Spacer(_) => return,
        };
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        if e <= s {
            return;
        }
        let part = a.slice(s, e);
        let (n, nch, rate) = (part.frames(), part.num_channels().max(1), a.rate);
        let at = |c: usize, i: usize| part.channels.get(c.min(nch - 1)).and_then(|v| v.get(i)).copied().unwrap_or(0.0);
        let mut st = State::new(&d, rate);
        let beat = self.beat_frames();
        let lead = if wrap { ((WRAP_LEAD_SECS * rate as f64) / n as f64).ceil().clamp(1.0, 64.0) as usize } else { 0 };
        let mut frames: Vec<[f32; 2]> = (0..n * (lead + 1)).map(|k| [at(0, k % n), at(1, k % n)]).collect();
        st.process(&d, &mut frames, None, beat);
        let mut out = frames.split_off(n * lead);
        if !wrap {
            // Run on through silence until the tail has died away.
            let block = rate as usize / 4;
            for _ in 0..(TAIL_MAX_SECS * 4.0) as usize {
                let mut tail = vec![[0.0f32; 2]; block];
                st.process(&d, &mut tail, None, beat);
                let loud = tail.iter().any(|f| f[0].abs().max(f[1].abs()) > TAIL_FLOOR);
                out.extend(tail);
                if !loud {
                    break;
                }
            }
            // Trim the silence at the very end.
            let last = out.iter().rposition(|f| f[0].abs().max(f[1].abs()) > TAIL_FLOOR).map_or(n, |i| (i + 1).max(n));
            out.truncate(last);
        }
        let to_audio = |fr: &[[f32; 2]]| Audio {
            rate,
            channels: if nch == 1 {
                vec![fr.iter().map(|f| (f[0] + f[1]) * 0.5).collect()]
            } else {
                (0..nch).map(|c| if c < 2 { fr.iter().map(|f| f[c]).collect() } else { vec![0.0; fr.len()] }).collect()
            },
        };
        let mut new = a.insert(s, e, &to_audio(&out[..n]));
        if out.len() > n {
            new = effects::mix_into(&new, e, &to_audio(&out[n..]), 1.0);
        }
        let sel = self.sel.map(|_| (s, e + out.len() - n));
        self.apply_edit(label, new, sel, &[]);
    }
}
