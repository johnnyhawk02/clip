//! Effect dialogs and floating windows: the Dialog parameter tables, curve, spectrum, mix and
//! record windows.

use super::*;

/// A dialog asking for an effect's or generator's settings before applying it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Dialog {
    Normalize,
    Amplify,
    Split,
    Echo,
    Reverb,
    Flanger,
    Smooth,
    NoiseGate,
    Pitch,
    Tempo,
    Speed,
    InsertSilence,
    InsertTone,
    InsertNoise,
    InsertDtmf,
    MixPaste,
    Convert,
    Equalizer,
    Delay,
    Emphasize,
    Compressor,
    Fade,
    FmSynth,
    OffsetTrack,
    CutGaps,
}

pub(super) struct Field {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) min: f32,
    pub(super) max: f32,
    pub(super) default: f32,
    pub(super) speed: f32,
    pub(super) suffix: &'static str,
    pub(super) decimals: usize,
    /// When set, the field is a pop-up menu of (label, value) instead of a number.
    pub(super) choices: &'static [(&'static str, f32)],
}

pub(super) const fn field(key: &'static str, label: &'static str, min: f32, max: f32, default: f32, speed: f32, suffix: &'static str, decimals: usize) -> Field {
    Field { key, label, min, max, default, speed, suffix, decimals, choices: &[] }
}

pub(super) const fn choice(key: &'static str, label: &'static str, default: f32, choices: &'static [(&'static str, f32)]) -> Field {
    Field { key, label, min: 0.0, max: 0.0, default, speed: 0.0, suffix: "", decimals: 0, choices }
}

/// A curve the user draws over the selection: volume (Envelope) or pitch (Bender).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CurveKind {
    Envelope,
    Bender,
}

impl CurveKind {
    pub(super) fn range(self) -> (f32, f32) {
        match self {
            CurveKind::Envelope => (0.0, 2.0),
            CurveKind::Bender => (-12.0, 12.0),
        }
    }

    pub(super) fn flat(self) -> Vec<(f32, f32)> {
        let v = if self == CurveKind::Envelope { 1.0 } else { 0.0 };
        vec![(0.0, v), (1.0, v)]
    }

    pub(super) fn format(self, v: f32) -> String {
        match self {
            CurveKind::Envelope => format!("{:.0}%", v * 100.0),
            CurveKind::Bender => format!("{v:+.1} st"),
        }
    }
}

impl Dialog {
    pub(super) const ALL: [Dialog; 25] = [
        Dialog::Normalize, Dialog::Amplify, Dialog::Split, Dialog::Echo, Dialog::Reverb, Dialog::Flanger,
        Dialog::Smooth, Dialog::NoiseGate, Dialog::Pitch, Dialog::Tempo, Dialog::Speed,
        Dialog::InsertSilence, Dialog::InsertTone, Dialog::InsertNoise, Dialog::MixPaste,
        Dialog::Convert, Dialog::Equalizer, Dialog::InsertDtmf, Dialog::Delay, Dialog::Emphasize,
        Dialog::Compressor, Dialog::Fade, Dialog::FmSynth, Dialog::OffsetTrack, Dialog::CutGaps,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Dialog::Normalize => "Normalize",
            Dialog::Amplify => "Amplify",
            Dialog::Split => "Split into Sounds",
            Dialog::Echo => "Echo",
            Dialog::Reverb => "Reverb",
            Dialog::Flanger => "Flanger",
            Dialog::Smooth => "Smooth",
            Dialog::NoiseGate => "Noise Gate",
            Dialog::Pitch => "Change Pitch",
            Dialog::Tempo => "Change Tempo",
            Dialog::Speed => "Change Speed",
            Dialog::InsertSilence => "Insert Silence",
            Dialog::InsertTone => "Insert Tone",
            Dialog::InsertNoise => "Insert Noise",
            Dialog::InsertDtmf => "Insert Touch Tones",
            Dialog::MixPaste => "Mix Paste",
            Dialog::Convert => "Convert Format",
            Dialog::Equalizer => "Equalizer",
            Dialog::Delay => "Delay",
            Dialog::Emphasize => "Emphasize",
            Dialog::Compressor => "Compressor",
            Dialog::Fade => "Fade",
            Dialog::FmSynth => "FM Synthesis",
            Dialog::OffsetTrack => "Offset Track",
            Dialog::CutGaps => "Cut Out Gaps",
        }
    }

    pub(super) fn blurb(self, scope: &str) -> String {
        match self {
            Dialog::Normalize => format!("Scale {scope} so its loudest peak reaches this level."),
            Dialog::Amplify => format!("Make {scope} louder or quieter."),
            Dialog::Split => format!(
                "Save every separate sound in {scope} as sound01.wav, sound02.wav… A sound starts at the \
                 first non-zero sample and ends after this many zero samples in a row."
            ),
            Dialog::Echo => format!("Add repeating echoes to {scope}."),
            Dialog::Reverb => format!("Add room reverb to {scope}."),
            Dialog::Flanger => format!("Sweep a short delay across {scope} for a jet-plane effect."),
            Dialog::Smooth => format!("Soften {scope} by removing high frequencies above the cutoff."),
            Dialog::NoiseGate => format!(
                "Silence everything in {scope} quieter than the threshold. Gated parts become exact \
                 zeros, which also helps Split into Sounds."
            ),
            Dialog::Pitch => format!("Raise or lower the pitch of {scope} without changing its length."),
            Dialog::Tempo => format!("Speed up or slow down {scope} without changing its pitch."),
            Dialog::Speed => format!("Play {scope} faster or slower like a tape, changing pitch and length together."),
            Dialog::InsertSilence | Dialog::InsertTone | Dialog::InsertNoise | Dialog::InsertDtmf => {
                "Inserted at the cursor, or replaces the selection.".to_string()
            }
            Dialog::MixPaste => "Mix the clipboard into the sound, starting at the cursor or selection start.".to_string(),
            Dialog::Convert => "Change the whole sound's sample rate, channels or resolution. Choosing 8 or 16 bits \
                                also sets the export format."
                .to_string(),
            Dialog::Equalizer => format!("Boost or cut frequency bands in {scope}."),
            Dialog::Delay => format!("Add one delayed copy of {scope}. Ping-pong bounces it to the other side (stereo)."),
            Dialog::Emphasize => format!("Brighten {scope} by lifting the high frequencies, so speech and detail stand out."),
            Dialog::Compressor => format!("Even out the loud and quiet parts of {scope}: levels above the threshold are turned down by the ratio."),
            Dialog::Fade => format!("Fade {scope} from one level to another, with a choice of curve."),
            Dialog::FmSynth => "Make a tone by frequency modulation (bells, brass, metallic sounds). Inserted at the cursor, or replaces the selection.".to_string(),
            Dialog::CutGaps => format!(
                "Shorten the silent gaps in {scope}: stretches quieter than the threshold and at least the minimum length \
                 are cut down to the kept percentage."
            ),
            Dialog::OffsetTrack => "Move the track later (positive: adds silence at the start) or earlier (negative: removes the start).".to_string(),
        }
    }

    pub(super) fn fields(self) -> &'static [Field] {
        match self {
            Dialog::Normalize => const { &[field("norm_db", "Peak level", -24.0, 0.0, -0.1, 0.1, " dB", 1)] },
            Dialog::Amplify => const { &[field("amp_db", "Amount", -48.0, 24.0, 3.0, 0.1, " dB", 1)] },
            Dialog::Split => const { &[field("gap", "Gap", 1.0, 100_000.0, 10.0, 1.0, " zero samples", 0)] },
            Dialog::Echo => const { &[
                field("echo_delay", "Delay", 10.0, 2000.0, 250.0, 2.0, " ms", 0),
                field("echo_fb", "Repeats", 0.0, 95.0, 40.0, 0.5, " %", 0),
                field("echo_mix", "Echo level", 0.0, 100.0, 50.0, 0.5, " %", 0),
            ] },
            Dialog::Reverb => const { &[
                    choice("rev_preset", "Preset", -1.0, &[
                        ("Custom", -1.0), ("Small room", 0.0), ("Studio", 1.0), ("Hall", 2.0), ("Cathedral", 3.0), ("Plate", 4.0),
                    ]),
                field("rev_size", "Room size", 0.0, 100.0, 50.0, 0.5, " %", 0),
                field("rev_mix", "Reverb level", 0.0, 100.0, 30.0, 0.5, " %", 0),
            ] },
            Dialog::Flanger => const { &[
                field("fl_depth", "Depth", 0.5, 10.0, 3.0, 0.05, " ms", 1),
                field("fl_rate", "Rate", 0.05, 5.0, 0.4, 0.01, " Hz", 2),
                field("fl_fb", "Feedback", -90.0, 90.0, 50.0, 0.5, " %", 0),
                field("fl_mix", "Effect level", 0.0, 100.0, 50.0, 0.5, " %", 0),
            ] },
            Dialog::Smooth => const { &[field("smooth_hz", "Cutoff", 100.0, 20_000.0, 4000.0, 10.0, " Hz", 0)] },
            Dialog::NoiseGate => const { &[
                field("gate_db", "Threshold", -90.0, 0.0, -50.0, 0.2, " dB", 1),
                field("gate_hold", "Hold", 0.0, 500.0, 20.0, 0.5, " ms", 0),
            ] },
            Dialog::Pitch => const { &[field("pitch_st", "Shift", -24.0, 24.0, 0.0, 0.05, " semitones", 1)] },
            Dialog::Tempo => const { &[field("tempo_pct", "Tempo", 25.0, 400.0, 100.0, 0.5, " %", 0)] },
            Dialog::Speed => const { &[field("speed_pct", "Speed", 25.0, 400.0, 100.0, 0.5, " %", 0)] },
            Dialog::InsertSilence => const { &[field("sil_secs", "Length", 0.01, 600.0, 1.0, 0.01, " s", 2)] },
            Dialog::InsertTone => const { &[
                field("tone_hz", "Frequency", 20.0, 20_000.0, 440.0, 1.0, " Hz", 1),
                field("tone_db", "Level", -60.0, 0.0, -6.0, 0.2, " dB", 1),
                field("tone_secs", "Length", 0.01, 600.0, 1.0, 0.01, " s", 2),
            ] },
            Dialog::InsertNoise => const { &[
                field("noise_db", "Level", -60.0, 0.0, -12.0, 0.2, " dB", 1),
                field("noise_secs", "Length", 0.01, 600.0, 1.0, 0.01, " s", 2),
            ] },
            Dialog::MixPaste => const { &[field("mix_db", "Clipboard level", -48.0, 12.0, 0.0, 0.1, " dB", 1)] },
            Dialog::InsertDtmf => const {
                &[
                    field("dtmf_ms", "Tone length", 20.0, 1000.0, 100.0, 1.0, " ms", 0),
                    field("dtmf_gap", "Gap", 10.0, 1000.0, 80.0, 1.0, " ms", 0),
                    field("dtmf_db", "Level", -60.0, 0.0, -6.0, 0.2, " dB", 1),
                ]
            },
            Dialog::Convert => const {
                &[
                    choice("conv_rate", "Sample rate", 0.0, &[
                        ("Keep", 0.0), ("8 kHz", 8000.0), ("11.025 kHz", 11025.0), ("16 kHz", 16000.0),
                        ("22.05 kHz", 22050.0), ("32 kHz", 32000.0), ("44.1 kHz", 44100.0), ("48 kHz", 48000.0),
                        ("96 kHz", 96000.0),
                    ]),
                    choice("conv_ch", "Channels", 0.0, &[("Keep", 0.0), ("Mono", 1.0), ("Stereo", 2.0)]),
                    choice("conv_bits", "Resolution", 0.0, &[("Keep", 0.0), ("8-bit", 8.0), ("12-bit", 12.0), ("16-bit", 16.0)]),
                ]
            },
            Dialog::Delay => const {
                &[
                    field("delay_ms", "Delay", 1.0, 2000.0, 300.0, 1.0, " ms", 0),
                    field("delay_mix", "Level", 0.0, 100.0, 50.0, 0.5, " %", 0),
                    choice("delay_pp", "Ping-pong", 0.0, &[("Off", 0.0), ("On", 1.0)]),
                ]
            },
            Dialog::Emphasize => const { &[field("emph_db", "Amount", 0.0, 18.0, 6.0, 0.1, " dB", 1)] },
            Dialog::Compressor => const {
                &[
                    field("comp_thr", "Threshold", -60.0, 0.0, -18.0, 0.2, " dB", 1),
                    field("comp_ratio", "Ratio", 1.0, 20.0, 4.0, 0.05, " : 1", 1),
                    field("comp_att", "Attack", 0.1, 200.0, 10.0, 0.2, " ms", 1),
                    field("comp_rel", "Release", 5.0, 2000.0, 150.0, 1.0, " ms", 0),
                    field("comp_gain", "Make-up gain", 0.0, 24.0, 6.0, 0.1, " dB", 1),
                ]
            },
            Dialog::Fade => const {
                &[
                    field("fade_from", "From", 0.0, 200.0, 100.0, 0.5, " %", 0),
                    field("fade_to", "To", 0.0, 200.0, 0.0, 0.5, " %", 0),
                    choice("fade_shape", "Curve", 1.0, &[("Linear", 0.0), ("Smooth", 1.0), ("Fast", 2.0), ("Slow", 3.0)]),
                ]
            },
            Dialog::FmSynth => const {
                &[
                    field("fm_hz", "Carrier", 20.0, 5000.0, 220.0, 1.0, " Hz", 1),
                    field("fm_ratio", "Modulator ratio", 0.1, 16.0, 2.0, 0.01, " ×", 2),
                    field("fm_index", "Brightness", 0.0, 20.0, 4.0, 0.05, "", 2),
                    field("fm_att", "Attack", 0.0, 2000.0, 5.0, 0.5, " ms", 0),
                    field("fm_decay", "Decay", 0.0, 99.0, 90.0, 0.2, " %", 0),
                    field("fm_secs", "Length", 0.05, 30.0, 1.5, 0.01, " s", 2),
                    field("fm_db", "Level", -60.0, 0.0, -6.0, 0.2, " dB", 1),
                ]
            },
            Dialog::CutGaps => const {
                &[
                    field("gap_db", "Threshold", -90.0, -10.0, -50.0, 0.2, " dB", 1),
                    field("gap_min", "Minimum gap", 10.0, 10_000.0, 300.0, 1.0, " ms", 0),
                    field("gap_keep", "Keep", 0.0, 100.0, 25.0, 0.5, " %", 0),
                ]
            },
            Dialog::OffsetTrack => const { &[field("offset_secs", "Offset", -60.0, 60.0, 0.5, 0.01, " s", 3)] },
            Dialog::Equalizer => const {
                &[
                    field("eq0", "100 Hz", -12.0, 12.0, 0.0, 0.1, " dB", 1),
                    field("eq1", "400 Hz", -12.0, 12.0, 0.0, 0.1, " dB", 1),
                    field("eq2", "1.2 kHz", -12.0, 12.0, 0.0, 0.1, " dB", 1),
                    field("eq3", "3.5 kHz", -12.0, 12.0, 0.0, 0.1, " dB", 1),
                    field("eq4", "9 kHz", -12.0, 12.0, 0.0, 0.1, " dB", 1),
                ]
            },
        }
    }
}

impl App {
    pub(super) fn apply_dialog(&mut self, d: Dialog) {
        let p = |k: &str| self.param(k);
        let Some(a) = self.audio.clone() else { return };
        let (rate, nch) = (a.rate, a.num_channels());
        match d {
            Dialog::Normalize => self.normalize(),
            Dialog::Amplify => self.gain(p("amp_db")),
            Dialog::Split => self.split_dialog(),
            Dialog::Echo => {
                let (dl, fb, mx) = (p("echo_delay"), p("echo_fb") / 100.0, p("echo_mix") / 100.0);
                self.process("Echo", |c| effects::echo(c, dl, fb, mx));
            }
            Dialog::Reverb => {
                let preset = p("rev_preset");
                let (sz, mx) = if preset >= 0.0 {
                    let (_, size, mix) = effects::REVERB_PRESETS[preset as usize];
                    (size, mix)
                } else {
                    (p("rev_size") / 100.0, p("rev_mix") / 100.0)
                };
                self.process("Reverb", |c| effects::reverb(c, sz, mx));
            }
            Dialog::Flanger => {
                let (dp, rt, fb, mx) = (p("fl_depth"), p("fl_rate"), p("fl_fb") / 100.0, p("fl_mix") / 100.0);
                self.process("Flanger", |c| effects::flanger(c, dp, rt, fb, mx));
            }
            Dialog::Smooth => {
                let hz = p("smooth_hz");
                self.process("Smooth", |c| effects::smooth(c, hz));
            }
            Dialog::NoiseGate => {
                let (db, hold) = (p("gate_db"), p("gate_hold"));
                self.process("Noise Gate", |c| effects::noise_gate(c, db, hold));
            }
            Dialog::Pitch => {
                let st = p("pitch_st");
                self.process("Change Pitch", |c| effects::pitch_shift(c, st));
            }
            Dialog::Tempo => {
                let pct = p("tempo_pct");
                self.process("Change Tempo", |c| effects::change_tempo(c, pct));
            }
            Dialog::Speed => {
                let pct = p("speed_pct");
                self.process("Change Speed", |c| effects::change_speed(c, pct));
            }
            Dialog::InsertSilence => self.insert_clip("Insert Silence", effects::silence(rate, nch, p("sil_secs"))),
            Dialog::InsertTone => {
                let clip = effects::tone(rate, nch, p("tone_secs"), p("tone_hz"), p("tone_db"), self.tone_wave);
                self.insert_clip("Insert Tone", clip);
            }
            Dialog::InsertDtmf => {
                let clip = effects::dtmf(rate, nch, &self.dtmf_digits, p("dtmf_ms"), p("dtmf_gap"), p("dtmf_db"));
                if clip.frames() == 0 {
                    self.status = "No touch-tone keys to insert (use 0–9, *, #, A–D)".into();
                } else {
                    self.insert_clip("Insert Touch Tones", clip);
                }
            }
            Dialog::InsertNoise => self.insert_clip("Insert Noise", effects::noise(rate, nch, p("noise_secs"), p("noise_db"))),
            Dialog::MixPaste => self.mix_paste(),
            Dialog::Delay => {
                let (ms, mx, pp) = (p("delay_ms"), p("delay_mix") / 100.0, p("delay_pp") > 0.5);
                self.process("Delay", |c| effects::delay(c, ms, mx, pp));
            }
            Dialog::Emphasize => {
                let db = p("emph_db");
                self.process("Emphasize", |c| effects::emphasize(c, db));
            }
            Dialog::Compressor => {
                let (t, r, a, rl, g) = (p("comp_thr"), p("comp_ratio"), p("comp_att"), p("comp_rel"), p("comp_gain"));
                self.process("Compressor", |c| effects::compress(c, t, r, a, rl, g));
            }
            Dialog::Fade => {
                let (from, to, shape) = (p("fade_from") / 100.0, p("fade_to") / 100.0, effects::FadeShape::from_index(p("fade_shape") as usize));
                self.process("Fade", |c| effects::fade(c, from, to, shape));
            }
            Dialog::FmSynth => {
                let clip = effects::fm_synth(rate, nch, p("fm_secs"), p("fm_hz"), p("fm_ratio"), p("fm_index"), p("fm_att"), p("fm_decay") / 100.0, p("fm_db"));
                self.insert_clip("FM Synthesis", clip);
            }
            Dialog::OffsetTrack => self.offset_track(p("offset_secs")),
            Dialog::CutGaps => {
                let (s, e) = self.range_or_all();
                let (out, cuts) = effects::cut_gaps(&a.slice(s, e), p("gap_db"), p("gap_min"), p("gap_keep") / 100.0);
                if cuts.is_empty() {
                    self.status = "No gaps found that long and that quiet".into();
                    return;
                }
                let removed: usize = cuts.iter().map(|c| c.1).sum();
                // Later cuts first, so earlier positions stay valid while cues move.
                let shifts: Vec<(usize, usize, usize)> = cuts.iter().rev().map(|&(at, len)| (s + at, len, 0)).collect();
                let new = a.insert(s, e, &out);
                let sel = self.sel.map(|_| (s, s + out.frames()));
                self.apply_edit("Cut Out Gaps", new, sel, &shifts);
                self.status = format!("Cut {} gaps, {} shorter — ⌘Z to undo", cuts.len(), fmt_time(removed as f64 / rate as f64));
            }
            Dialog::Equalizer => {
                let g = [p("eq0"), p("eq1"), p("eq2"), p("eq3"), p("eq4")];
                self.process("Equalizer", |c| effects::equalize(c, g));
            }
            Dialog::Convert => self.convert_format(p("conv_rate") as u32, p("conv_ch") as usize, p("conv_bits") as u32),
        }
    }

    /// Envelope / Bender: draw a curve over the selection, then apply it.
    pub(super) fn curve_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.curve else { return };
        let Some(a) = self.audio.clone() else {
            self.curve = None;
            return;
        };
        let (s, e) = self.range_or_all();
        let scope = if self.sel.is_some() { "the selection" } else { "the whole sound" };
        let (lo, hi) = kind.range();
        let mut apply = false;
        let mut cancel = false;
        let resp = dialog_modal("curve").show(ctx, |ui| {
            ui.set_width(520.0);
            let (title, blurb) = match kind {
                CurveKind::Envelope => ("Envelope", format!("Draw how the volume of {scope} changes over time (0–200%).")),
                CurveKind::Bender => (
                    "Bender",
                    format!("Draw a pitch curve across {scope} (±12 semitones). Like a tape, bending up also speeds it up."),
                ),
            };
            dialog_header(ui, title, &format!("{blurb} Click to add a point, drag to move it, double-click to remove it."));
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(520.0, 180.0), Sense::click_and_drag());
            let p = ui.painter_at(rect.expand(4.0));
            p.rect_filled(rect, 4.0, Color32::from_gray(252));
            // The selection's waveform, faintly, so the curve can be lined up with the sound.
            let w = rect.width() as usize;
            let len = (e - s).max(1);
            for px in 0..w {
                let f0 = s + px * len / w;
                let f1 = (s + (px + 1) * len / w).max(f0 + 1).min(e.max(f0 + 1)).min(a.frames());
                if f0 >= f1 {
                    continue;
                }
                let peak = a.channels.iter().map(|c| c[f0..f1].iter().fold(0.0f32, |m, v| m.max(v.abs()))).fold(0.0, f32::max);
                let h = peak.min(1.0) * rect.height() * 0.48;
                let x = rect.left() + px as f32 + 0.5;
                p.line_segment([Pos2::new(x, rect.center().y - h), Pos2::new(x, rect.center().y + h)], Stroke::new(1.0, Color32::from_gray(215)));
            }
            let to_screen = |(t, v): (f32, f32)| Pos2::new(rect.left() + t * rect.width(), rect.bottom() - (v - lo) / (hi - lo) * rect.height());
            let from_screen = |q: Pos2| {
                (((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0), (lo + (rect.bottom() - q.y) / rect.height() * (hi - lo)).clamp(lo, hi))
            };
            // Reference lines.
            let refs: &[f32] = if kind == CurveKind::Envelope { &[0.0, 1.0, 2.0] } else { &[-12.0, 0.0, 12.0] };
            for &v in refs {
                let y = to_screen((0.0, v)).y;
                p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, Color32::from_gray(if v == refs[1] { 150 } else { 225 })));
                p.text(Pos2::new(rect.left() + 3.0, y), if v == hi { Align2::LEFT_TOP } else { Align2::LEFT_BOTTOM }, kind.format(v), FontId::proportional(10.0), Color32::from_gray(110));
            }
            let points = match kind {
                CurveKind::Envelope => &mut self.env_points,
                CurveKind::Bender => &mut self.bend_points,
            };
            let near = |q: Pos2, pts: &[(f32, f32)]| pts.iter().position(|&pt| to_screen(pt).distance(q) < 7.0);
            if resp.drag_started() {
                let origin = ui.input(|i| i.pointer.press_origin());
                self.curve_drag = origin.and_then(|q| near(q, points)).or_else(|| {
                    // Dragging from empty space adds a point and moves it.
                    let (t, v) = from_screen(origin?);
                    let i = points.iter().position(|&(pt, _)| pt > t)?;
                    points.insert(i, (t, v));
                    Some(i)
                });
            }
            if let (Some(i), Some(q)) = (self.curve_drag, resp.interact_pointer_pos()) {
                if resp.dragged() && i < points.len() {
                    let (t, v) = from_screen(q);
                    let last = points.len() - 1;
                    // End points stay at the ends; inner points stay between their neighbours.
                    let t = if i == 0 {
                        0.0
                    } else if i == last {
                        1.0
                    } else {
                        t.clamp(points[i - 1].0 + 0.001, points[i + 1].0 - 0.001)
                    };
                    points[i] = (t, v);
                }
            }
            if resp.drag_stopped() {
                self.curve_drag = None;
            }
            if resp.double_clicked() {
                if let Some(i) = resp.interact_pointer_pos().and_then(|q| near(q, points)) {
                    if i != 0 && i != points.len() - 1 {
                        points.remove(i);
                    }
                }
            } else if resp.clicked() {
                if let Some(q) = resp.interact_pointer_pos() {
                    if near(q, points).is_none() {
                        let (t, v) = from_screen(q);
                        if let Some(i) = points.iter().position(|&(pt, _)| pt > t) {
                            points.insert(i, (t, v));
                        }
                    }
                }
            }
            let line: Vec<Pos2> = points.iter().map(|&pt| to_screen(pt)).collect();
            p.add(egui::Shape::line(line.clone(), Stroke::new(1.5, Color32::BLACK)));
            for q in line {
                p.rect_filled(Rect::from_center_size(q, Vec2::splat(7.0)), 0.0, Color32::BLACK);
            }
            if let Some(q) = resp.hover_pos() {
                p.text(rect.right_top() + Vec2::new(-4.0, 3.0), Align2::RIGHT_TOP, kind.format(from_screen(q).1), FontId::monospace(10.0), Color32::from_gray(90));
            }
            p.rect_stroke(rect, 4.0, Stroke::new(1.0, LINE), egui::StrokeKind::Outside);
            (apply, cancel) = dialog_buttons(ui, "OK", true, |ui| {
                if ui.button("Reset").clicked() {
                    *points = kind.flat();
                }
                if kind == CurveKind::Envelope {
                    if ui.button("Swell").clicked() {
                        *points = vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)];
                    }
                    if ui.button("Duck").clicked() {
                        *points = vec![(0.0, 1.0), (0.2, 0.3), (0.8, 0.3), (1.0, 1.0)];
                    }
                } else {
                    if ui.button("Dive").clicked() {
                        *points = vec![(0.0, 0.0), (1.0, -12.0)];
                    }
                    if ui.button("Wobble").clicked() {
                        *points = (0..=8).map(|i| (i as f32 / 8.0, if i % 2 == 0 { 0.0 } else { 2.0 })).collect();
                    }
                }
            });
        });
        apply |= ctx.input(|i| i.key_pressed(Key::Enter));
        if cancel || resp.should_close() {
            self.curve = None;
        } else if apply {
            self.curve = None;
            self.apply_curve(kind);
        }
    }

    /// Spectrum window: a frequency plot (2D) or a waterfall over time (3D).
    pub(super) fn spectrum_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_spectrum;
        let src = self.play_audio.clone().or_else(|| self.audio.clone());
        let playing = self.player.is_playing();
        let pos = if playing { self.player.position() } else { self.cursor };
        let mut three_d = self.spectrum_3d;
        let sel = self.sel;
        let cache = &mut self.spectrum_cache;
        let center = self.desk.center();
        egui::Window::new("Spectrum")
            .title_bar(false)
            .frame(card_frame())
            .default_pos(center - Vec2::new(280.0, 150.0))
            .default_size(Vec2::new(560.0, 300.0))
            .resizable(true)
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if window_header(ui, "Spectrum") {
                    open = false;
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut three_d, false, "2D");
                    ui.selectable_value(&mut three_d, true, "3D");
                    ui.label(
                        egui::RichText::new(match (sel, playing) {
                            (Some(_), _) => "Selection",
                            (None, true) => "Following playback",
                            (None, false) => "At the cursor",
                        })
                        .color(MUTED),
                    );
                });
                let Some(a) = src else {
                    ui.label("No sound open.");
                    return;
                };
                let (rect, _) = ui.allocate_exact_size(ui.available_size().max(Vec2::new(200.0, 120.0)), Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 4.0, Color32::WHITE);
                let plot = rect.shrink2(Vec2::new(34.0, 18.0)).translate(Vec2::new(10.0, -2.0));
                let nyq = a.rate as f32 / 2.0;
                let fx = |hz: f32| plot.left() + ((hz.max(20.0) / 20.0).ln() / (nyq / 20.0).ln()) * plot.width();
                let dby = |db: f32, r: Rect| r.top() + (-db / 96.0).clamp(0.0, 1.0) * r.height();
                for hz in [100.0, 1000.0, 10000.0] {
                    if hz < nyq {
                        let x = fx(hz);
                        p.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0, Color32::from_gray(232)));
                        let label = if hz >= 1000.0 { format!("{}k", hz / 1000.0) } else { format!("{hz}") };
                        p.text(Pos2::new(x, plot.bottom() + 3.0), Align2::CENTER_TOP, label, FontId::proportional(10.0), MUTED);
                    }
                }
                let n = a.frames();
                if !three_d {
                    for db in [0.0, -24.0, -48.0, -72.0] {
                        let y = dby(db, plot);
                        p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0, Color32::from_gray(232)));
                        p.text(Pos2::new(plot.left() - 4.0, y), Align2::RIGHT_CENTER, format!("{db:.0}"), FontId::proportional(10.0), MUTED);
                    }
                    // Average several FFTs across a selection; otherwise one at the position.
                    let size = 4096;
                    let (s, e) = sel.unwrap_or((pos, pos + size));
                    let count = ((e.saturating_sub(s)) / size).clamp(1, 16);
                    let mut avg = vec![0.0f32; size / 2];
                    for k in 0..count {
                        let start = s + k * (e.saturating_sub(s).saturating_sub(size)) / count.max(1);
                        for (acc, v) in avg.iter_mut().zip(effects::spectrum(&a, start.min(n), size)) {
                            *acc += v / count as f32;
                        }
                    }
                    let pts: Vec<Pos2> = (1..avg.len())
                        .map(|k| Pos2::new(fx(k as f32 * nyq / avg.len() as f32), dby(avg[k], plot)))
                        .filter(|q| q.x >= plot.left())
                        .collect();
                    let mut fill = pts.clone();
                    fill.push(Pos2::new(plot.right(), plot.bottom()));
                    fill.push(Pos2::new(plot.left(), plot.bottom()));
                    for w in pts.windows(2) {
                        p.add(egui::Shape::convex_polygon(
                            vec![w[0], w[1], Pos2::new(w[1].x, plot.bottom()), Pos2::new(w[0].x, plot.bottom())],
                            Color32::from_gray(228),
                            Stroke::NONE,
                        ));
                    }
                    p.add(egui::Shape::line(pts, Stroke::new(1.2, INK)));
                    if playing && sel.is_none() {
                        ctx.request_repaint();
                    }
                } else {
                    // Waterfall: successive spectra stacked back to front.
                    let (s, e) = sel.unwrap_or((pos, (pos + a.rate as usize * 2).min(n)));
                    let key = (Arc::as_ptr(&a) as usize, (s, e));
                    if cache.as_ref().map(|c| c.0) != Some(key) {
                        let slices = 40;
                        let size = 1024;
                        let data = (0..slices)
                            .map(|k| effects::spectrum(&a, s + k * e.saturating_sub(s).saturating_sub(size) / slices, size))
                            .collect();
                        *cache = Some((key, data));
                    }
                    let data = &cache.as_ref().unwrap().1;
                    let depth = Vec2::new(plot.width() * 0.25, -plot.height() * 0.55);
                    let front = Rect::from_min_max(Pos2::new(plot.left(), plot.top() - depth.y), Pos2::new(plot.right() - depth.x, plot.bottom()));
                    let fxf = |hz: f32| front.left() + ((hz.max(20.0) / 20.0).ln() / (nyq / 20.0).ln()) * front.width();
                    for (k, spec) in data.iter().enumerate().rev() {
                        let off = depth * (k as f32 / data.len() as f32);
                        let base = front.bottom() + off.y;
                        let mut pts: Vec<Pos2> = Vec::new();
                        let bins = spec.len();
                        let mut last_x = f32::MIN;
                        for b in 1..bins {
                            let x = fxf(b as f32 * nyq / bins as f32) + off.x;
                            if x - last_x < 2.0 {
                                continue;
                            }
                            last_x = x;
                            let h = ((spec[b] + 96.0) / 96.0).clamp(0.0, 1.0) * front.height() * 0.8;
                            pts.push(Pos2::new(x, base - h));
                        }
                        if pts.len() < 2 {
                            continue;
                        }
                        // White fill hides the slices behind; then the outline.
                        for w in pts.windows(2) {
                            p.add(egui::Shape::convex_polygon(
                                vec![w[0], w[1], Pos2::new(w[1].x, base), Pos2::new(w[0].x, base)],
                                Color32::WHITE,
                                Stroke::NONE,
                            ));
                        }
                        let shade = (60.0 + 140.0 * k as f32 / data.len() as f32) as u8;
                        p.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_gray(shade))));
                    }
                    p.text(Pos2::new(front.left(), front.bottom() + 14.0), Align2::LEFT_TOP, "front: start of range · back: end", FontId::proportional(10.0), MUTED);
                }
            });
        self.spectrum_3d = three_d;
        self.show_spectrum = open;
    }

    /// PhotoSound ▸ Preferences (⌘,).
    pub(super) fn prefs_dialog(&mut self, ctx: &egui::Context) {
        let Some((bars_before, repitch_before)) = self.prefs_open else { return };
        let mut ok = false;
        let mut cancel = false;
        let mut repitch = repitch_mode();
        let resp = dialog_modal("prefs").show(ctx, |ui| {
            ui.set_width(420.0);
            dialog_header(ui, "Preferences", "");
            dialog_grid("prefsgrid").show(ui, |ui| {
                ui.label("Phrase lines");
                ui.horizontal(|ui| {
                    ui.label("every");
                    ui.add(egui::DragValue::new(&mut self.phrase_bars).range(0..=64));
                    ui.label(if self.phrase_bars == 0 { "bars (off)" } else { "bars" });
                });
                ui.end_row();
                ui.label("Tempo changes");
                ui.vertical(|ui| {
                    ui.radio_value(&mut repitch, true, "Re-Pitch: speed and pitch follow, like a record");
                    ui.radio_value(&mut repitch, false, "Keep pitch: stretch the audio");
                });
                ui.end_row();
            });
            ui.label(egui::RichText::new("Phrase lines are a fine line across the arrangement, counting from bar 1. 0 turns them off. Tempo changes apply to warped clips.").color(MUTED));
            (ok, cancel) = dialog_buttons(ui, "Done", true, |_| {});
        });
        REPITCH.store(repitch, std::sync::atomic::Ordering::Relaxed);
        if cancel || resp.should_close() {
            self.phrase_bars = bars_before;
            REPITCH.store(repitch_before, std::sync::atomic::Ordering::Relaxed);
            self.prefs_open = None;
        } else if ok {
            if std::env::var_os("SOUND_EDIT_ACTIONS").is_none() {
                let p = phrase_pref();
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, self.phrase_bars.to_string());
                if repitch {
                    let _ = std::fs::remove_file(keep_pitch_pref());
                } else {
                    let _ = std::fs::write(keep_pitch_pref(), b"");
                }
            }
            self.prefs_open = None;
            if repitch != repitch_before {
                self.stretch_in_background();
            }
        }
    }

    /// Tracks ▸ Mix: choose tracks and their levels and pan, then mix into a new track or
    /// replace all tracks with the mix.
    pub(super) fn mix_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut include) = self.mix_include.take() else { return };
        include.resize(self.tracks.len(), true);
        let mut ok = false;
        let mut cancel = false;
        let resp = dialog_modal("mix").show(ctx, |ui| {
            ui.set_width(480.0);
            dialog_header(ui, "Mix", "Choose which tracks to mix and set their volume and pan.");
            ui.spacing_mut().slider_width = 110.0;
            egui::Grid::new("mixgrid").num_columns(4).spacing(Vec2::new(14.0, 10.0)).show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("Track").color(MUTED));
                ui.label(egui::RichText::new("Volume").color(MUTED));
                ui.label(egui::RichText::new("Pan").color(MUTED));
                ui.end_row();
                for (t, inc) in self.tracks.iter_mut().zip(include.iter_mut()) {
                    ui.checkbox(inc, "");
                    ui.label(&t.name);
                    ui.add(egui::Slider::new(&mut t.gain, 0.0..=2.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                    ui.add(egui::Slider::new(&mut t.pan, -1.0..=1.0).custom_formatter(|v, _| match v {
                        v if v.abs() < 0.01 => "C".to_string(),
                        v if v < 0.0 => format!("L{:.0}", -v * 100.0),
                        v => format!("R{:.0}", v * 100.0),
                    }));
                    ui.end_row();
                }
            });
            ui.add_space(14.0);
            ui.radio_value(&mut self.mix_to_new, true, "Mix into a new track (keeps the originals)");
            ui.add_space(2.0);
            ui.radio_value(&mut self.mix_to_new, false, "Replace all tracks with the mix");
            (ok, cancel) = dialog_buttons(ui, "Mix", include.iter().any(|i| *i), |_| {});
        });
        if cancel || resp.should_close() {
            self.refresh_playback();
            return;
        }
        if !ok {
            self.mix_include = Some(include);
            return;
        }
        let rate = self.rate();
        let rendered: Vec<(Audio, f32, f32)> =
            self.tracks.iter().zip(&include).filter(|(_, i)| **i).map(|(t, _)| (self.render_track(t), t.gain, t.pan)).collect();
        let inputs: Vec<(&Audio, f32, f32)> = rendered.iter().map(|(a, g, p)| (a, *g, *p)).collect();
        let mix = effects::mix_panned(&inputs, rate);
        let mix = Arc::new(if self.auto_gain { auto_gain(mix) } else { mix });
        if self.mix_to_new {
            let mut t = Track::new("Mix");
            t.clips.push(Clip::new("Mix", mix, 0));
            self.add_track(t, "Mix");
            self.status = "Mixed into a new track — ⌘Z to undo".into();
        } else {
            self.push_undo("Mix");
            let mut t = Track::new("Mix");
            t.clips.push(Clip::new("Mix", mix, 0));
            self.tracks = vec![t];
            self.active = 0;
            self.sel_clip = Some((0, 0));
            self.refresh_playback();
            self.dirty = true;
            self.status = "Replaced the tracks with their mix — ⌘Z to undo".into();
        }
    }

    /// Record window: record from the default input, then insert at the cursor.
    pub(super) fn record_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_record;
        let recording = self.recorder.is_some();
        if let Some(r) = &self.recorder {
            self.rec_meter = r.take_peak().max(self.rec_meter * 0.85);
            ctx.request_repaint();
        } else {
            self.rec_meter *= 0.85;
        }
        let mut toggle = false;
        self.place("Record", Pos2::new(self.desk.center().x - 135.0, self.desk.top() + 40.0))
            .title_bar(false)
            .frame(card_frame())
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(270.0);
                if window_header(ui, "Record") {
                    open = false;
                }
                ui.horizontal(|ui| {
                    let label = if recording { "■ Stop" } else { "● Record" };
                    let btn = Button::new(egui::RichText::new(label).size(15.0).color(INK))
                        .min_size(Vec2::new(110.0, 32.0));
                    toggle = ui.add(btn).clicked();
                    let secs = self.recorder.as_ref().map_or(0.0, |r| r.seconds());
                    ui.label(egui::RichText::new(fmt_time(secs)).monospace().size(18.0));
                });
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(270.0, 8.0), Sense::hover());
                draw_meter(ui.painter(), r, self.rec_meter);
                ui.add_space(8.0);
                ui.label(egui::RichText::new(if self.audio.is_some() {
                    "Records from the Mac's current input. The recording goes in at the cursor, or replaces the selection."
                } else {
                    "Records from the Mac's current input into a new sound."
                }).color(MUTED));
            });
        if toggle {
            if recording {
                self.stop_recording();
            } else {
                self.start_recording();
            }
        }
        if !open && self.recorder.is_some() {
            self.stop_recording();
        }
        self.show_record = open;
    }

    pub(super) fn dialog_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.dialog else { return };
        let scope = if self.sel.is_some() { "the selection" } else { "the whole sound" };
        let mut apply = false;
        let mut cancel = false;
        let resp = dialog_modal("dialog").show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            ui.spacing_mut().interact_size.x = FIELD_W;
            dialog_header(ui, kind.title(), &kind.blurb(scope));
            dialog_grid("fields").show(ui, |ui| {
                for f in kind.fields().iter().filter(|_| kind != Dialog::Equalizer) {
                    ui.label(f.label);
                    let v = self.params.entry(f.key).or_insert(f.default);
                    if !f.choices.is_empty() {
                        let current = f.choices.iter().find(|c| c.1 == *v).map_or("?", |c| c.0);
                        egui::ComboBox::from_id_salt(f.key).width(FIELD_W).selected_text(current).show_ui(ui, |ui| {
                            for &(label, value) in f.choices {
                                ui.selectable_value(v, value, label);
                            }
                        });
                        ui.end_row();
                        continue;
                    }
                    ui.add(
                        egui::DragValue::new(v)
                            .range(f.min..=f.max)
                            .speed(f.speed)
                            .suffix(f.suffix)
                            .fixed_decimals(f.decimals),
                    );
                    ui.end_row();
                }
                if kind == Dialog::InsertDtmf {
                    ui.label("Keys");
                    ui.add_sized([FIELD_W, 22.0], egui::TextEdit::singleline(&mut self.dtmf_digits).hint_text("e.g. 555-0123"))
                        .on_hover_text("0–9, *, #, A–D. A comma adds a pause; other characters are ignored.");
                    ui.end_row();
                }
                if kind == Dialog::InsertTone {
                    ui.label("Waveform");
                    egui::ComboBox::from_id_salt("wave").width(FIELD_W).selected_text(self.tone_wave.label()).show_ui(ui, |ui| {
                        for w in Wave::ALL {
                            ui.selectable_value(&mut self.tone_wave, w, w.label());
                        }
                    });
                    ui.end_row();
                }
            });
            if kind == Dialog::Equalizer {
                // Graphic-EQ style: a vertical slider per band, evenly spaced.
                ui.columns(kind.fields().len(), |cols| {
                    for (ui, f) in cols.iter_mut().zip(kind.fields()) {
                        ui.vertical_centered(|ui| {
                            let v = self.params.entry(f.key).or_insert(f.default);
                            ui.spacing_mut().slider_width = 130.0;
                            // Centre the slider in its column, like the labels under it.
                            let pad = (ui.available_width() - ui.spacing().interact_size.y) / 2.0;
                            ui.horizontal(|ui| {
                                ui.add_space(pad.max(0.0));
                                ui.add(egui::Slider::new(v, f.min..=f.max).vertical().show_value(false));
                            });
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("{:+.1} dB", *v)).size(12.0).color(INK));
                            ui.label(egui::RichText::new(f.label).size(12.0).color(MUTED));
                        });
                    }
                });
            }
            if kind == Dialog::Split {
                if let Some(a) = &self.audio {
                    let (s, e) = self.sel.unwrap_or((0, a.frames()));
                    let n = a.slice(s, e).find_sounds(self.split_gap()).len();
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(format!("{n} sound{} found. Saved as {}.", if n == 1 { "" } else { "s" }, self.wav_format.label())).color(MUTED));
                }
            }
            let ok = if kind == Dialog::Split { "Choose Folder…" } else { "OK" };
            let params = &mut self.params;
            (apply, cancel) = dialog_buttons(ui, ok, true, |ui| {
                if kind == Dialog::Equalizer && ui.button("Flat").clicked() {
                    for f in kind.fields() {
                        params.insert(f.key, 0.0);
                    }
                }
            });
        });
        apply |= ctx.input(|i| i.key_pressed(Key::Enter));
        if cancel || resp.should_close() {
            self.dialog = None;
        } else if apply {
            self.dialog = None;
            self.apply_dialog(kind);
        }
    }
}
