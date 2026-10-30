//! Track effects used offline, as the sound-editor effects are: a track device's settings in a
//! dialog, printed into the selection (or the whole clip) when you press Apply.

use super::deviceview::{chorus_panel, delay_panel, phaser_panel, pultec_panel, reverb_panel, riser_panel, spring_panel, tame_panel, opto_panel, autofilter_panel, tape_panel};
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
                Device::Tape(s) => s.mix = 0.35,
                Device::Phaser(_) | Device::Chorus(_) | Device::Pultec(_) | Device::Tame(_) | Device::Opto(_) | Device::AutoFilter(_) => {}
                // Printed, it rises across the selection: all the way, to a full wash.
                Device::Riser(s) => s.amount = 1.0,
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
            // (Tape's and Auto Filter's columns need more room.)
            ui.set_width(match d {
                Device::Tape(_) => DIALOG_W + 420.0,
                Device::AutoFilter(_) => super::deviceview::AUTOFILTER_W,
                _ => DIALOG_W,
            });
            let about = match d {
                // No tail; printed, its cuts look ahead.
                Device::Tame(_) => format!("The track effect, printed into {scope}, its cuts opening just ahead of each resonance."),
                // No tail either.
                Device::Opto(_) | Device::AutoFilter(_) => format!("The track effect, printed into {scope}."),
                _ => format!("The track effect, printed into {scope}. Its tail rings on past the end."),
            };
            dialog_header(ui, &format!("{} Device", d.name()), &about);
            match &mut d {
                Device::Reverb(s) => _ = reverb_panel(ui, s),
                Device::Delay(s) => _ = delay_panel(ui, s),
                Device::Spring(s) => _ = spring_panel(ui, s),
                Device::Tape(s) => _ = tape_panel(ui, s),
                Device::Phaser(s) => _ = phaser_panel(ui, s),
                Device::Chorus(s) => _ = chorus_panel(ui, s),
                Device::Pultec(s) => _ = pultec_panel(ui, s),
                Device::Riser(s) => _ = riser_panel(ui, s, true),
                Device::Tame(s) => _ = tame_panel(ui, s, None),
                Device::Opto(s) => _ = opto_panel(ui, s, None),
                Device::AutoFilter(s) => _ = autofilter_panel(ui, s, None),
                Device::Spacer(_) => {}
            }
            if whole && !matches!(d, Device::Tame(_)) {
                ui.add_space(8.0);
                let text = if matches!(d, Device::Opto(_)) {
                    "Run the clip round first, so it starts already levelling (a loop stays seamless)"
                } else if matches!(d, Device::AutoFilter(_)) {
                    "Run the clip round first, so a loop starts mid-sweep and stays seamless"
                } else {
                    "Wrap the tail round to the start instead (a loop keeps its length)"
                };
                ui.checkbox(&mut wrap, text);
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
            Device::Tape(_) => "Tape Device",
            Device::Phaser(_) => "Phaser Device",
            Device::Chorus(_) => "Chorus Device",
            Device::Pultec(_) => "Pultec Device",
            Device::Riser(_) => "Riser Device",
            Device::Tame(_) => "Tame Device",
            Device::Opto(_) => "Opto Device",
            Device::AutoFilter(_) => "Auto Filter Device",
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
        // A device that delays its output (Auto Filter's oversampling) runs that much further, and
        // that much is cut from the front, so the print lines up. (Tame prints without delay.)
        let late = if matches!(d, Device::Tame(_)) { 0 } else { d.latency() };
        let mut frames: Vec<[f32; 2]> = (0..n * (lead + 1) + late).map(|k| if wrap || k < n * (lead + 1) { [at(0, k % n), at(1, k % n)] } else { [0.0; 2] }).collect();
        match d {
            // Rising: Amount climbs from none at the start of the sound to its setting at the end.
            Device::Riser(r) if r.ramp => {
                for (b, chunk) in frames.chunks_mut(256).enumerate() {
                    let at = (b * 256) % n;
                    let step = Device::Riser(crate::riser::Settings { amount: r.amount * at as f32 / n as f32, ..r });
                    st.process(&step, chunk, None, beat, 0.0);
                }
            }
            // Printed with look-ahead: cuts smoothed both ways in time, lined up with the sound.
            Device::Tame(t) => frames = crate::tame::print(&t, rate, &frames),
            _ => st.process(&d, &mut frames, None, beat, 0.0),
        }
        let mut out = frames.split_off(n * lead + late);
        // (Opto has no tail: just the half-sample its colour stage lags by. Nor has Auto Filter, but
        // for its ringing, cut off at the end as a loop's would be.)
        if !wrap && !matches!(d, Device::Opto(_) | Device::AutoFilter(_)) {
            // Run on through silence until the tail has died away.
            let block = rate as usize / 4;
            for _ in 0..(TAIL_MAX_SECS * 4.0) as usize {
                let mut tail = vec![[0.0f32; 2]; block];
                st.process(&d, &mut tail, None, beat, 0.0);
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
