//! The editor window: classic-Mac styled document view with floating Tools, Controls and Info
//! windows, effect dialogs, menus and shortcuts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self, Align2, Button, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2,
};

use crate::actions::{self, Action};
use crate::audio::{self, Audio, Cue, Peaks, WavFormat};
use crate::effects::{self, Wave};
use crate::menu::NativeMenu;
use crate::player::Player;
use crate::recorder::Recorder;

type Sel = (usize, usize);

/// Undo history is capped by memory rather than step count, since each step is a full copy.
const UNDO_BUDGET_BYTES: usize = 2 << 30;

struct Snapshot {
    audio: Arc<Audio>,
    cues: Vec<Cue>,
    sel: Option<Sel>,
    cursor: usize,
    label: &'static str,
}

/// Pencil drawing only works this close in (frames per pixel), so each stroke is precise.
const PENCIL_MAX_SPP: f64 = 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Select,
    Hand,
    Zoom,
    Pencil,
}

struct PencilStroke {
    channel: usize,
    last: Option<(usize, f32)>,
}

/// A dialog asking for an effect's or generator's settings before applying it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialog {
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
    MixPaste,
    Convert,
    Equalizer,
}

struct Field {
    key: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    speed: f32,
    suffix: &'static str,
    decimals: usize,
    /// When set, the field is a pop-up menu of (label, value) instead of a number.
    choices: &'static [(&'static str, f32)],
}

const fn field(key: &'static str, label: &'static str, min: f32, max: f32, default: f32, speed: f32, suffix: &'static str, decimals: usize) -> Field {
    Field { key, label, min, max, default, speed, suffix, decimals, choices: &[] }
}

const fn choice(key: &'static str, label: &'static str, default: f32, choices: &'static [(&'static str, f32)]) -> Field {
    Field { key, label, min: 0.0, max: 0.0, default, speed: 0.0, suffix: "", decimals: 0, choices }
}

/// A curve the user draws over the selection: volume (Envelope) or pitch (Bender).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CurveKind {
    Envelope,
    Bender,
}

impl CurveKind {
    fn range(self) -> (f32, f32) {
        match self {
            CurveKind::Envelope => (0.0, 2.0),
            CurveKind::Bender => (-12.0, 12.0),
        }
    }

    fn flat(self) -> Vec<(f32, f32)> {
        let v = if self == CurveKind::Envelope { 1.0 } else { 0.0 };
        vec![(0.0, v), (1.0, v)]
    }

    fn format(self, v: f32) -> String {
        match self {
            CurveKind::Envelope => format!("{:.0}%", v * 100.0),
            CurveKind::Bender => format!("{v:+.1} st"),
        }
    }
}

impl Dialog {
    const ALL: [Dialog; 17] = [
        Dialog::Normalize, Dialog::Amplify, Dialog::Split, Dialog::Echo, Dialog::Reverb, Dialog::Flanger,
        Dialog::Smooth, Dialog::NoiseGate, Dialog::Pitch, Dialog::Tempo, Dialog::Speed,
        Dialog::InsertSilence, Dialog::InsertTone, Dialog::InsertNoise, Dialog::MixPaste,
        Dialog::Convert, Dialog::Equalizer,
    ];

    fn title(self) -> &'static str {
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
            Dialog::MixPaste => "Mix Paste",
            Dialog::Convert => "Convert Format",
            Dialog::Equalizer => "Equalizer",
        }
    }

    fn blurb(self, scope: &str) -> String {
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
            Dialog::InsertSilence | Dialog::InsertTone | Dialog::InsertNoise => {
                "Inserted at the cursor, or replaces the selection.".to_string()
            }
            Dialog::MixPaste => "Mix the clipboard into the sound, starting at the cursor or selection start.".to_string(),
            Dialog::Convert => "Change the whole sound's sample rate, channels or resolution. Choosing 8 or 16 bits \
                                also sets the export format."
                .to_string(),
            Dialog::Equalizer => format!("Boost or cut frequency bands in {scope}."),
        }
    }

    fn fields(self) -> &'static [Field] {
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

enum Pending {
    New,
    Open(PathBuf),
    Close,
}

pub struct App {
    audio: Option<Arc<Audio>>,
    peaks: Option<Peaks>,
    path: Option<PathBuf>,
    dirty: bool,

    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,

    sel: Option<Sel>,
    cursor: usize,
    drag_anchor: Option<usize>,
    /// Visible range, in frames.
    view_start: f64,
    view_len: f64,
    scrollbar_drag: bool,

    player: Player,
    looping: bool,
    play_origin: usize,

    clipboard: Option<Audio>,
    wav_format: WavFormat,
    params: HashMap<&'static str, f32>,
    tone_wave: Wave,

    status: String,
    loading: Option<(PathBuf, Receiver<Result<Audio, String>>)>,
    pending: Option<Pending>,
    allow_close: bool,
    automation: Option<Automation>,
    menu: NativeMenu,
    dialog: Option<Dialog>,

    tool: Tool,
    pencil: Option<PencilStroke>,
    zoom_band: Option<(usize, usize)>,
    show_tools: bool,
    show_controls: bool,
    show_info: bool,
    meter: Vec<f32>,
    /// (audio pointer, selection) -> peak level, so Info doesn't rescan every frame.
    info_peak: Option<((usize, Option<Sel>), f32)>,
    /// The area behind the document window; floating windows are placed relative to it.
    desk: Rect,
    cues: Vec<Cue>,
    cue_drag: Option<usize>,
    show_cues: bool,
    recorder: Option<Recorder>,
    show_record: bool,
    rec_meter: f32,
    curve: Option<CurveKind>,
    env_points: Vec<(f32, f32)>,
    bend_points: Vec<(f32, f32)>,
    curve_drag: Option<usize>,
    /// Frames drawn so far.
    frame_count: u64,
    /// Per floating window: the frame until which it is held at its starting position (set the
    /// first time it opens, while egui is still working out its size).
    place_until: HashMap<&'static str, u64>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        classic_style(&cc.egui_ctx);
        let player = Player::new();
        let status = match &player.error {
            Some(e) => format!("Audio output unavailable: {e}"),
            None => "Drop an audio file here, or press ⌘O to open one.".into(),
        };
        let mut app = App {
            audio: None,
            peaks: None,
            path: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            sel: None,
            cursor: 0,
            drag_anchor: None,
            view_start: 0.0,
            view_len: 1.0,
            scrollbar_drag: false,
            player,
            looping: false,
            play_origin: 0,
            clipboard: None,
            wav_format: WavFormat::Pcm16,
            params: Dialog::ALL.iter().flat_map(|d| d.fields()).map(|f| (f.key, f.default)).collect(),
            tone_wave: Wave::Sine,
            status,
            loading: None,
            pending: None,
            allow_close: false,
            automation: Automation::from_env(),
            menu: NativeMenu::install(&cc.egui_ctx),
            dialog: None,
            tool: Tool::Select,
            pencil: None,
            zoom_band: None,
            show_tools: true,
            show_controls: true,
            show_info: false,
            meter: Vec::new(),
            info_peak: None,
            desk: Rect::NOTHING,
            cues: Vec::new(),
            cue_drag: None,
            show_cues: false,
            recorder: None,
            show_record: false,
            rec_meter: 0.0,
            curve: None,
            env_points: CurveKind::Envelope.flat(),
            bend_points: CurveKind::Bender.flat(),
            curve_drag: None,
            frame_count: 0,
            place_until: HashMap::new(),
        };
        if let Some(p) = initial {
            app.start_load(p, &cc.egui_ctx);
        }
        app
    }

    // ---------- file handling ----------

    fn request_open(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.dirty {
            self.pending = Some(Pending::Open(path));
        } else {
            self.start_load(path, ctx);
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Audio",
                &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf", "mkv", "webm"],
            )
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(p) = picked {
            self.request_open(p, ctx);
        }
    }

    fn start_load(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let p = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(audio::load(&p));
            ctx.request_repaint();
        });
        self.status = format!("Loading {}…", file_name(&path));
        self.loading = Some((path, rx));
    }

    fn poll_loading(&mut self) {
        let Some((path, rx)) = &self.loading else { return };
        let Ok(result) = rx.try_recv() else { return };
        let path = path.clone();
        self.loading = None;
        match result {
            Ok(a) if a.frames() > 0 => {
                self.status = format!(
                    "Opened {} — {} Hz, {} ch, {}",
                    file_name(&path),
                    a.rate,
                    a.num_channels(),
                    fmt_time(a.duration_secs())
                );
                self.player.stop();
                self.undo.clear();
                self.redo.clear();
                self.sel = None;
                self.cursor = 0;
                self.dirty = false;
                self.cues = audio::read_wav_cues(&path);
                self.path = Some(path);
                self.set_audio(Arc::new(a));
                self.zoom_fit();
            }
            Ok(_) => self.status = format!("{} contains no audio", file_name(&path)),
            Err(e) => self.status = format!("Couldn't open {}: {e}", file_name(&path)),
        }
    }

    fn set_audio(&mut self, a: Arc<Audio>) {
        // If the whole file was in view, keep it that way after the length changes.
        let was_full = self.view_start <= 0.0 && self.view_len >= self.frames() as f64;
        self.peaks = Some(Peaks::build(&a));
        self.player.set_audio(a.clone());
        let n = a.frames();
        self.cursor = self.cursor.min(n);
        self.sel = self.sel.map(|(s, e)| (s.min(n), e.min(n))).filter(|(s, e)| e > s);
        self.audio = Some(a);
        if was_full {
            self.zoom_fit();
        }
        self.clamp_view();
    }

    fn export(&mut self, selection_only: bool) {
        let Some(a) = self.audio.clone() else { return };
        let (a, suffix) = match (selection_only, self.sel) {
            (true, Some((s, e))) => (Arc::new(a.slice(s, e)), "-selection"),
            _ => (a, "-edit"),
        };
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        let mut dialog = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .set_file_name(format!("{stem}{suffix}.wav"));
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut out) = dialog.save_file() else { return };
        if out.extension().is_none() {
            out.set_extension("wav");
        }
        let (lo, hi) = match (selection_only, self.sel) {
            (true, Some(r)) => r,
            _ => (0, usize::MAX),
        };
        let cues: Vec<Cue> = self
            .cues
            .iter()
            .filter(|c| c.frame >= lo && c.frame < hi)
            .map(|c| Cue { frame: c.frame - lo, name: c.name.clone() })
            .collect();
        match audio::save_wav(&a, &out, self.wav_format).and_then(|_| audio::write_wav_cues(&out, &cues)) {
            Ok(()) => {
                self.status = format!(
                    "Exported {} ({}, {})",
                    out.display(),
                    self.wav_format.label(),
                    fmt_time(a.duration_secs())
                );
                if suffix == "-edit" {
                    self.dirty = false;
                }
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    // ---------- editing ----------

    fn frames(&self) -> usize {
        self.audio.as_ref().map_or(0, |a| a.frames())
    }

    /// Selection, or the whole file when nothing is selected.
    fn range_or_all(&self) -> Sel {
        self.sel.unwrap_or((0, self.frames()))
    }

    fn push_undo(&mut self, label: &'static str) {
        let Some(a) = self.audio.clone() else { return };
        self.undo.push(Snapshot { audio: a, cues: self.cues.clone(), sel: self.sel, cursor: self.cursor, label });
        self.redo.clear();
        // Drop oldest steps beyond the memory budget (always keep at least one).
        let mut total: usize = self.undo.iter().map(|s| s.audio.bytes()).sum();
        while total > UNDO_BUDGET_BYTES && self.undo.len() > 1 {
            total -= self.undo.remove(0).audio.bytes();
        }
    }

    fn apply(&mut self, label: &'static str, new: Audio, sel: Option<Sel>) {
        self.apply_edit(label, new, sel, &[]);
    }

    /// Like `apply`, for edits that change the length: each (at, removed, inserted) moves the
    /// cue points to follow the audio.
    fn apply_edit(&mut self, label: &'static str, new: Audio, sel: Option<Sel>, shifts: &[(usize, usize, usize)]) {
        self.push_undo(label);
        for &(at, removed, inserted) in shifts {
            self.shift_cues(at, removed, inserted);
        }
        self.sel = sel;
        let was_playing = self.player.is_playing();
        self.set_audio(Arc::new(new));
        if was_playing && self.player.position() >= self.frames() {
            self.player.stop();
        }
        self.dirty = true;
        self.status = format!("{label} — ⌘Z to undo");
    }

    /// `removed` frames at `at` were replaced by `inserted` frames. Cues inside the replaced
    /// part scale into the new part (or go, if nothing replaced it); later cues move along.
    fn shift_cues(&mut self, at: usize, removed: usize, inserted: usize) {
        if removed == inserted {
            return;
        }
        self.cues.retain_mut(|c| {
            if c.frame < at {
                true
            } else if c.frame < at + removed {
                if inserted == 0 {
                    return false;
                }
                c.frame = at + ((c.frame - at) as f64 * inserted as f64 / removed as f64) as usize;
                true
            } else {
                c.frame = c.frame + inserted - removed;
                true
            }
        });
    }

    fn undo(&mut self) {
        let Some(snap) = self.undo.pop() else { return };
        let cur = self.audio.clone().unwrap();
        self.redo.push(Snapshot { audio: cur, cues: std::mem::replace(&mut self.cues, snap.cues), sel: self.sel, cursor: self.cursor, label: snap.label });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.set_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Undid {}", snap.label);
    }

    fn redo(&mut self) {
        let Some(snap) = self.redo.pop() else { return };
        let cur = self.audio.clone().unwrap();
        self.undo.push(Snapshot { audio: cur, cues: std::mem::replace(&mut self.cues, snap.cues), sel: self.sel, cursor: self.cursor, label: snap.label });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.set_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Redid {}", snap.label);
    }

    fn trim(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.cursor = 0;
        let n = a.frames();
        self.apply_edit("Trim to selection", a.crop(s, e), None, &[(e, n - e, 0), (0, s, 0)]);
        self.zoom_fit();
    }

    fn delete(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        if e - s >= a.frames() {
            self.status = "Can't delete everything — use Trim instead".into();
            return;
        }
        self.cursor = s;
        self.apply_edit("Delete selection", a.delete(s, e), None, &[(s, e - s, 0)]);
    }

    fn copy(&mut self, ctx: &egui::Context) -> bool {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return false };
        let clip = a.slice(s, e);
        // Put a text marker on the system clipboard so ⌘V reliably produces a paste event.
        ctx.copy_text(format!("[Sound Edit clip, {}]", fmt_time(clip.duration_secs())));
        self.status = format!("Copied {}", fmt_time(clip.duration_secs()));
        self.clipboard = Some(clip);
        true
    }

    fn cut(&mut self, ctx: &egui::Context) {
        if self.copy(ctx) {
            let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
            self.cursor = s;
            if e - s >= a.frames() {
                self.apply_edit("Cut", a.empty_like(), None, &[(s, e - s, 0)]);
            } else {
                self.apply_edit("Cut", a.delete(s, e), None, &[(s, e - s, 0)]);
            }
        }
    }

    fn paste(&mut self) {
        let (Some(a), Some(clip)) = (self.audio.clone(), self.clipboard.clone()) else { return };
        let (at, end) = self.sel.unwrap_or((self.cursor, self.cursor));
        let new = a.insert(at, end, &clip);
        let pasted_len = new.frames() + (end - at) - a.frames();
        self.cursor = at;
        self.apply_edit("Paste", new, Some((at, at + pasted_len)), &[(at, end - at, pasted_len)]);
    }

    fn fade(&mut self, fade_in: bool) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        if fade_in {
            self.apply("Fade in", a.fade_in(s, e), self.sel);
        } else {
            self.apply("Fade out", a.fade_out(s, e), self.sel);
        }
    }

    fn normalize(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        match a.normalize(s, e, self.param("norm_db")) {
            Some(n) => self.apply("Normalize", n, self.sel),
            None => self.status = "Nothing to normalize — range is silent".into(),
        }
    }

    fn gain(&mut self, db: f32) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Amplify", a.gain(s, e, 10f32.powf(db / 20.0)), self.sel);
    }

    fn silence(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.apply("Silence", a.silence(s, e), self.sel);
    }

    fn reverse(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Backwards", a.reverse(s, e), self.sel);
    }

    fn to_mono(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        if a.num_channels() > 1 {
            self.apply("Convert to mono", a.to_mono(), self.sel);
        }
    }

    fn param(&self, key: &str) -> f32 {
        self.params.get(key).copied().unwrap_or(0.0)
    }

    fn split_gap(&self) -> usize {
        self.param("gap").max(1.0) as usize
    }

    /// Run an effect on the selection (or whole sound) and splice the result back in.
    /// Effects may change the length (tempo, speed); the selection follows.
    fn process(&mut self, label: &'static str, f: impl FnOnce(&Audio) -> Audio) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        if e <= s {
            return;
        }
        let out = f(&a.slice(s, e));
        let len = out.frames();
        let sel = self.sel.map(|_| (s, s + len));
        self.apply_edit(label, a.insert(s, e, &out), sel, &[(s, e - s, len)]);
    }

    /// Insert a generated clip at the cursor, replacing the selection if there is one.
    fn insert_clip(&mut self, label: &'static str, clip: Audio) {
        let Some(a) = self.audio.clone() else { return };
        let (at, end) = self.sel.unwrap_or((self.cursor, self.cursor));
        let new = a.insert(at, end, &clip);
        let len = new.frames() + (end - at) - a.frames();
        self.cursor = at;
        self.apply_edit(label, new, Some((at, at + len)), &[(at, end - at, len)]);
    }

    fn mix_paste(&mut self) {
        let (Some(a), Some(clip)) = (self.audio.clone(), self.clipboard.clone()) else { return };
        let at = self.sel.map_or(self.cursor, |(s, _)| s);
        let level = 10f32.powf(self.param("mix_db") / 20.0);
        let len = clip.resampled(a.rate).frames();
        self.apply("Mix Paste", effects::mix_into(&a, at, &clip, level), Some((at, at + len)));
    }

    fn apply_dialog(&mut self, d: Dialog) {
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
                let (sz, mx) = (p("rev_size") / 100.0, p("rev_mix") / 100.0);
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
            Dialog::InsertNoise => self.insert_clip("Insert Noise", effects::noise(rate, nch, p("noise_secs"), p("noise_db"))),
            Dialog::MixPaste => self.mix_paste(),
            Dialog::Equalizer => {
                let g = [p("eq0"), p("eq1"), p("eq2"), p("eq3"), p("eq4")];
                self.process("Equalizer", |c| effects::equalize(c, g));
            }
            Dialog::Convert => self.convert_format(p("conv_rate") as u32, p("conv_ch") as usize, p("conv_bits") as u32),
        }
    }

    /// Convert the whole sound's format (0 = keep that property).
    fn convert_format(&mut self, rate: u32, channels: usize, bits: u32) {
        let Some(a) = self.audio.clone() else { return };
        let mut out = (*a).clone();
        if channels > 0 {
            out = effects::convert_channels(&out, channels);
        }
        if rate > 0 && rate != out.rate {
            out = effects::convert_rate(&out, rate);
        }
        if bits > 0 {
            out = effects::quantize(&out, bits);
            match bits {
                8 => self.wav_format = WavFormat::Pcm8,
                16 => self.wav_format = WavFormat::Pcm16,
                _ => {}
            }
        }
        // Positions are in frames, so cues, cursor, selection and view follow the new rate.
        let k = out.rate as f64 / a.rate as f64;
        let scale = |f: usize| (f as f64 * k) as usize;
        let (cursor, view_start, view_len) = (self.cursor, self.view_start, self.view_len);
        let sel = self.sel.map(|(s, e)| (scale(s), scale(e)));
        self.apply("Convert Format", out, sel);
        for c in &mut self.cues {
            c.frame = scale(c.frame);
        }
        self.cursor = scale(cursor);
        self.view_start = view_start * k;
        self.view_len = view_len * k;
        self.clamp_view();
        let a = self.audio.as_ref().unwrap();
        self.status = format!("Converted to {} Hz, {} ch — ⌘Z to undo", a.rate, a.num_channels());
    }

    fn apply_curve(&mut self, kind: CurveKind) {
        match kind {
            CurveKind::Envelope => {
                let pts = self.env_points.clone();
                self.process("Envelope", |c| effects::envelope(c, &pts));
            }
            CurveKind::Bender => {
                let pts = self.bend_points.clone();
                self.process("Bender", |c| effects::bender(c, &pts));
            }
        }
    }

    fn add_cue(&mut self) {
        let at = self.sel.map_or(self.cursor, |(s, _)| s);
        if self.cues.iter().any(|c| c.frame == at) {
            return;
        }
        let n = (1..).find(|i| !self.cues.iter().any(|c| c.name == format!("Cue {i}"))).unwrap();
        self.push_undo("Cue Point");
        self.cues.push(Cue { frame: at, name: format!("Cue {n}") });
        self.cues.sort_by_key(|c| c.frame);
        self.dirty = true;
        self.status = format!("Added Cue {n} — name it in Window ▸ Cues");
    }

    fn jump_cue(&mut self, forward: bool) {
        let from = self.sel.map_or(self.cursor, |(s, _)| s);
        let target = if forward {
            self.cues.iter().map(|c| c.frame).find(|&f| f > from)
        } else {
            self.cues.iter().rev().map(|c| c.frame).find(|&f| f < from)
        };
        if let Some(f) = target {
            self.go_to(f);
        }
    }

    /// Move the insertion point to `f` and scroll it into view.
    fn go_to(&mut self, f: usize) {
        self.sel = None;
        self.cursor = f;
        self.play_origin = f;
        if (f as f64) < self.view_start || (f as f64) > self.view_start + self.view_len {
            self.view_start = f as f64 - self.view_len * 0.1;
            self.clamp_view();
        }
    }

    fn start_recording(&mut self) {
        self.player.stop();
        match Recorder::start() {
            Ok(r) => {
                self.status = format!("Recording at {} Hz, {} ch…", r.rate, r.channels);
                self.recorder = Some(r);
            }
            Err(e) => self.status = format!("Can't record: {e}"),
        }
    }

    fn stop_recording(&mut self) {
        let Some(r) = self.recorder.take() else { return };
        let clip = r.finish();
        if clip.frames() == 0 {
            self.status = "Nothing was recorded".into();
            return;
        }
        let secs = clip.duration_secs();
        if self.audio.is_none() {
            self.new_sound();
            self.set_audio(Arc::new(clip));
            self.zoom_fit();
            self.dirty = true;
            self.status = format!("Recorded {} into a new sound", fmt_time(secs));
        } else {
            self.insert_clip("Record", clip);
            self.status = format!("Recorded {} — ⌘Z to undo", fmt_time(secs));
        }
    }

    fn new_sound(&mut self) {
        self.player.stop();
        self.undo.clear();
        self.redo.clear();
        self.sel = None;
        self.cursor = 0;
        self.dirty = false;
        self.path = None;
        self.cues.clear();
        self.set_audio(Arc::new(Audio { rate: 44100, channels: vec![Vec::new(); 2] }));
        self.zoom_fit();
        self.status = "New sound (44.1 kHz stereo). Use the Insert menu, Paste or Mix Paste to add audio.".into();
    }

    /// Cut the recording (or the selection) into separate sounds and save each one as
    /// soundNN.wav in a folder the user picks.
    fn split_dialog(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        let count = a.slice(s, e).find_sounds(self.split_gap()).len();
        if count == 0 {
            self.status = "No sounds found — the range is completely silent".into();
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title(format!("Choose a folder for {count} sounds"));
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(dir) = dialog.pick_folder() else { return };
        let names = split_names(count);
        let existing = names.iter().filter(|n| dir.join(n).exists()).count();
        if existing > 0 {
            let ok = rfd::MessageDialog::new()
                .set_title("Replace files?")
                .set_description(format!("{existing} of the files (e.g. {}) already exist in that folder. Replace them?", names[0]))
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show();
            if ok != rfd::MessageDialogResult::Ok {
                return;
            }
        }
        self.split_to(&dir);
    }

    fn split_to(&mut self, dir: &Path) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        let src = a.slice(s, e);
        let sounds = src.find_sounds(self.split_gap());
        let names = split_names(sounds.len());
        for ((a0, a1), name) in sounds.iter().zip(&names) {
            if let Err(err) = audio::save_wav(&src.slice(*a0, *a1), &dir.join(name), self.wav_format) {
                self.status = format!("Couldn't save {name}: {err}");
                return;
            }
        }
        self.status = match names.as_slice() {
            [only] => format!("Saved 1 sound ({only}) to {}", dir.display()),
            [first, .., last] => format!("Saved {} sounds ({first} … {last}) to {}", names.len(), dir.display()),
            [] => "No sounds found".into(),
        };
    }

    // ---------- transport ----------

    fn toggle_play(&mut self) {
        if self.audio.is_none() {
            return;
        }
        if self.player.is_playing() {
            // Pause: leave the cursor where playback got to.
            self.cursor = self.player.position().min(self.frames());
            self.player.stop();
            return;
        }
        let n = self.frames();
        let (start, end) = match self.sel {
            Some((s, e)) if self.cursor > s && self.cursor < e => (self.cursor, e),
            Some((s, e)) => (s, e),
            None if self.cursor >= n => (0, n),
            None => (self.cursor, n),
        };
        self.play_origin = start;
        self.player.play(start, end, self.looping && self.sel.is_some());
    }

    fn stop(&mut self) {
        self.player.stop();
        self.cursor = self.sel.map_or(self.play_origin, |(s, _)| s);
    }

    // ---------- view ----------

    fn zoom_fit(&mut self) {
        self.view_start = 0.0;
        self.view_len = self.frames().max(1) as f64;
    }

    fn zoom_to_selection(&mut self) {
        if let Some((s, e)) = self.sel {
            let pad = (e - s) as f64 * 0.05;
            self.view_start = s as f64 - pad;
            self.view_len = (e - s) as f64 + 2.0 * pad;
            self.clamp_view();
        }
    }

    /// Zoom by `factor` (<1 zooms in) keeping frame `anchor` fixed on screen.
    fn zoom(&mut self, factor: f64, anchor: f64) {
        let new_len = (self.view_len * factor).clamp(32.0, self.frames().max(32) as f64);
        let t = (anchor - self.view_start) / self.view_len;
        self.view_start = anchor - t * new_len;
        self.view_len = new_len;
        self.clamp_view();
    }

    fn clamp_view(&mut self) {
        let n = self.frames().max(1) as f64;
        self.view_len = self.view_len.clamp(32f64.min(n), n);
        self.view_start = self.view_start.clamp(0.0, (n - self.view_len).max(0.0));
    }

    // ---------- UI ----------

    fn enabled(&self, a: Action) -> bool {
        let has = self.audio.is_some();
        let sel = has && self.sel.is_some();
        let not_empty = self.frames() > 0;
        match a {
            Action::New | Action::Open | Action::Quit => true,
            Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Loop => true,
            Action::ShowCues | Action::Record => true,
            Action::AddCue => has,
            Action::NextCue | Action::PrevCue => has && !self.cues.is_empty(),
            Action::ShowTools | Action::ShowControls | Action::ShowInfo => true,
            Action::ToolSelect | Action::ToolHand | Action::ToolZoom | Action::ToolPencil => true,
            Action::Undo => !self.undo.is_empty(),
            Action::Redo => !self.redo.is_empty(),
            Action::Paste | Action::MixPaste => has && self.clipboard.is_some(),
            Action::InsertSilence | Action::InsertTone | Action::InsertNoise | Action::SelectAll => has,
            Action::ToMono => self.audio.as_ref().is_some_and(|a| a.num_channels() > 1),
            Action::ExportSelection
            | Action::Cut
            | Action::Copy
            | Action::Delete
            | Action::SelectNone
            | Action::Trim
            | Action::Silence
            | Action::FadeIn
            | Action::FadeOut
            | Action::ZoomSel => sel,
            _ => has && not_empty,
        }
    }

    fn checked(&self, a: Action) -> bool {
        match a {
            Action::Loop => self.looping,
            Action::FormatPcm8 => self.wav_format == WavFormat::Pcm8,
            Action::FormatPcm16 => self.wav_format == WavFormat::Pcm16,
            Action::FormatPcm24 => self.wav_format == WavFormat::Pcm24,
            Action::FormatFloat32 => self.wav_format == WavFormat::Float32,
            Action::ToolSelect => self.tool == Tool::Select,
            Action::ToolHand => self.tool == Tool::Hand,
            Action::ToolZoom => self.tool == Tool::Zoom,
            Action::ToolPencil => self.tool == Tool::Pencil,
            Action::ShowTools => self.show_tools,
            Action::ShowControls => self.show_controls,
            Action::ShowInfo => self.show_info,
            Action::ShowCues => self.show_cues,
            Action::Record => self.recorder.is_some(),
            _ => false,
        }
    }

    fn menu_label(&self, a: Action) -> String {
        match a {
            Action::Undo => self.undo.last().map_or("Undo".into(), |s| format!("Undo {}", s.label)),
            Action::Redo => self.redo.last().map_or("Redo".into(), |s| format!("Redo {}", s.label)),
            _ => a.label().to_string(),
        }
    }

    fn run(&mut self, a: Action, ctx: &egui::Context) {
        if !self.enabled(a) {
            return;
        }
        let mid = self.view_start + self.view_len / 2.0;
        match a {
            Action::New => {
                if self.dirty {
                    self.pending = Some(Pending::New);
                } else {
                    self.new_sound();
                }
            }
            Action::Open => self.open_dialog(ctx),
            Action::Export => self.export(false),
            Action::ExportSelection => self.export(true),
            Action::FormatPcm8 => self.wav_format = WavFormat::Pcm8,
            Action::FormatPcm16 => self.wav_format = WavFormat::Pcm16,
            Action::FormatPcm24 => self.wav_format = WavFormat::Pcm24,
            Action::FormatFloat32 => self.wav_format = WavFormat::Float32,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Cut => self.cut(ctx),
            Action::Copy => {
                self.copy(ctx);
            }
            Action::Paste => self.paste(),
            Action::MixPaste => self.dialog = Some(Dialog::MixPaste),
            Action::Delete => self.delete(),
            Action::SelectAll => self.sel = Some((0, self.frames())).filter(|(_, e)| *e > 0),
            Action::SelectNone => self.sel = None,
            Action::Trim => self.trim(),
            Action::Silence => self.silence(),
            Action::InsertSilence => self.dialog = Some(Dialog::InsertSilence),
            Action::InsertTone => self.dialog = Some(Dialog::InsertTone),
            Action::InsertNoise => self.dialog = Some(Dialog::InsertNoise),
            Action::FadeIn => self.fade(true),
            Action::FadeOut => self.fade(false),
            Action::Normalize => self.dialog = Some(Dialog::Normalize),
            Action::Gain => self.dialog = Some(Dialog::Amplify),
            Action::Reverse => self.reverse(),
            Action::Echo => self.dialog = Some(Dialog::Echo),
            Action::Reverb => self.dialog = Some(Dialog::Reverb),
            Action::Flanger => self.dialog = Some(Dialog::Flanger),
            Action::Smooth => self.dialog = Some(Dialog::Smooth),
            Action::NoiseGate => self.dialog = Some(Dialog::NoiseGate),
            Action::Pitch => self.dialog = Some(Dialog::Pitch),
            Action::Tempo => self.dialog = Some(Dialog::Tempo),
            Action::Speed => self.dialog = Some(Dialog::Speed),
            Action::ToMono => self.to_mono(),
            Action::Split => self.dialog = Some(Dialog::Split),
            Action::PlayPause => self.toggle_play(),
            Action::Stop => self.stop(),
            Action::Loop => {
                self.looping = !self.looping;
                self.player.set_looping(self.looping && self.sel.is_some());
            }
            Action::GoStart => {
                self.cursor = 0;
                self.sel = None;
                self.view_start = 0.0;
                self.clamp_view();
            }
            Action::GoEnd => {
                self.cursor = self.frames();
                self.sel = None;
                self.view_start = self.frames() as f64;
                self.clamp_view();
            }
            Action::ZoomIn => self.zoom(0.5, mid),
            Action::ZoomOut => self.zoom(2.0, mid),
            Action::ZoomSel => self.zoom_to_selection(),
            Action::ZoomFit => self.zoom_fit(),
            Action::ToolSelect => self.tool = Tool::Select,
            Action::ToolHand => self.tool = Tool::Hand,
            Action::ToolZoom => self.tool = Tool::Zoom,
            Action::ToolPencil => self.tool = Tool::Pencil,
            Action::ShowTools => self.show_tools = !self.show_tools,
            Action::ShowControls => self.show_controls = !self.show_controls,
            Action::ShowInfo => self.show_info = !self.show_info,
            Action::ShowCues => self.show_cues = !self.show_cues,
            Action::Convert => self.dialog = Some(Dialog::Convert),
            Action::Equalizer => self.dialog = Some(Dialog::Equalizer),
            Action::Envelope => self.curve = Some(CurveKind::Envelope),
            Action::Bender => self.curve = Some(CurveKind::Bender),
            Action::Record => {
                if self.recorder.is_some() {
                    self.stop_recording();
                } else {
                    self.show_record = true;
                }
            }
            Action::AddCue => self.add_cue(),
            Action::NextCue => self.jump_cue(true),
            Action::PrevCue => self.jump_cue(false),
        }
    }

    /// Plain-key shortcuts. ⌘ shortcuts come through the native menu bar instead.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let none = Modifiers::NONE;
        let keys: &[(Modifiers, Key, Action)] = &[
            (none, Key::Space, Action::PlayPause),
            (none, Key::Delete, Action::Delete),
            (none, Key::Backspace, Action::Delete),
            (none, Key::Home, Action::GoStart),
            (none, Key::End, Action::GoEnd),
            (none, Key::Equals, Action::ZoomIn),
            (Modifiers::SHIFT, Key::Equals, Action::ZoomIn),
            (none, Key::Plus, Action::ZoomIn),
            (none, Key::Minus, Action::ZoomOut),
            (none, Key::F, Action::ZoomFit),
            (none, Key::L, Action::Loop),
            (none, Key::S, Action::ToolSelect),
            (none, Key::H, Action::ToolHand),
            (none, Key::Z, Action::ToolZoom),
            (none, Key::P, Action::ToolPencil),
            (none, Key::OpenBracket, Action::PrevCue),
            (none, Key::CloseBracket, Action::NextCue),
        ];
        for &(m, k, a) in keys {
            if ctx.input_mut(|i| i.consume_key(m, k)) {
                self.run(a, ctx);
            }
        }
        if ctx.input_mut(|i| i.consume_key(none, Key::Escape)) {
            if self.player.is_playing() {
                self.stop();
            } else {
                self.sel = None;
            }
        }
    }

    /// One square palette button for `a`, drawn icon or glyph.
    fn palette_button(&mut self, ui: &mut egui::Ui, a: Action, size: Vec2) {
        let ctx = ui.ctx().clone();
        let playing = a == Action::PlayPause && self.player.is_playing();
        let mut tip = if playing { "Pause".to_string() } else { self.menu_label(a).trim_end_matches('…').to_string() };
        if let Some(k) = a.shortcut() {
            tip += &format!("  ({k})");
        }
        if !a.help().is_empty() {
            tip += &format!("\n{}", a.help());
        }
        let selected = self.checked(a);
        let enabled = self.enabled(a);
        let resp = ui.add_enabled(enabled, Button::new("").min_size(size).selected(selected));
        let color = if selected {
            Color32::WHITE
        } else if enabled {
            Color32::BLACK
        } else {
            Color32::from_gray(150)
        };
        paint_icon(ui.painter(), resp.rect, a, color, playing);
        if resp.on_hover_text(tip).on_disabled_hover_text(a.label()).clicked() {
            self.run(a, &ctx);
        }
    }

    /// Starting position for a floating window, applied for the first few frames (once the
    /// window size is known) and then left to the user.
    fn place(&mut self, name: &'static str, pos: Pos2) -> egui::Window<'static> {
        let w = egui::Window::new(name);
        let until = *self.place_until.entry(name).or_insert(self.frame_count + 3);
        // Unconstrained while placing: on the first frames an auto-sized window doesn't know its
        // size yet, and constraining would push it up the screen.
        if self.frame_count < until && self.desk.is_positive() {
            w.constrain(false).current_pos(pos)
        } else {
            w.constrain_to(self.desk).default_pos(pos)
        }
    }

    /// Floating Tools palette: editing tools on top, common commands below.
    fn tools_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_tools;
        self.place("Tools", self.desk.min + Vec2::new(12.0, 12.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::Grid::new("palette").spacing(Vec2::new(2.0, 2.0)).show(ui, |ui| {
                    for row in actions::PALETTE {
                        match row {
                            Some(pair) => {
                                for a in pair {
                                    self.palette_button(ui, *a, Vec2::new(30.0, 28.0));
                                }
                            }
                            None => {
                                ui.add_space(2.0);
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        self.show_tools = open;
    }

    /// Floating tape-deck style Controls: transport, counter and level meters.
    fn controls_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_controls;
        // Level meters follow the playhead and fall back smoothly.
        let n_ch = self.audio.as_ref().map_or(2, |a| a.num_channels().max(1));
        self.meter.resize(n_ch, 0.0);
        let playing = self.player.is_playing();
        let pos = self.player.position();
        for (c, m) in self.meter.iter_mut().enumerate() {
            let now = match (&self.audio, playing) {
                (Some(a), true) if pos < a.frames() => {
                    let end = (pos + 2048).min(a.frames());
                    a.channels[c][pos..end].iter().fold(0.0f32, |p, s| p.max(s.abs()))
                }
                _ => 0.0,
            };
            *m = now.max(*m * 0.85);
        }
        if self.meter.iter().any(|m| *m > 0.001) {
            ctx.request_repaint();
        }

        self.place("Controls", Pos2::new(self.desk.left() + 112.0, self.desk.bottom() - 138.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(3.0, 4.0);
                egui::Grid::new("transport").spacing(Vec2::new(3.0, 0.0)).show(ui, |ui| {
                    for a in [Action::GoStart, Action::PlayPause, Action::Stop, Action::Record, Action::Loop, Action::GoEnd] {
                        self.palette_button(ui, a, Vec2::new(32.0, 26.0));
                    }
                });
                let rate = self.audio.as_ref().map_or(44100, |a| a.rate) as f64;
                let at = if playing { pos } else { self.cursor };
                let (counter_rect, _) = ui.allocate_exact_size(Vec2::new(202.0, 30.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(counter_rect, 0.0, Color32::from_rgb(20, 30, 20));
                p.rect_stroke(counter_rect, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
                p.text(
                    counter_rect.left_center() + Vec2::new(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    fmt_time(at as f64 / rate),
                    FontId::monospace(20.0),
                    Color32::from_rgb(120, 255, 120),
                );
                if let Some((s, e)) = self.sel {
                    p.text(
                        counter_rect.right_center() - Vec2::new(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        format!("sel {}", fmt_time((e - s) as f64 / rate)),
                        FontId::monospace(11.0),
                        Color32::from_rgb(90, 200, 90),
                    );
                }
                // Segmented level meters, one per channel.
                for (c, level) in self.meter.clone().iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(counter_rect.width(), 9.0), Sense::hover());
                    let p = ui.painter();
                    let segs = 32;
                    let db = 20.0 * level.max(1e-6).log10();
                    let lit = (((db + 48.0) / 48.0).clamp(0.0, 1.0) * segs as f32).round() as usize;
                    let w = (r.width() - 12.0) / segs as f32;
                    p.text(r.left_center(), Align2::LEFT_CENTER, if c == 0 { "L" } else { "R" }, FontId::monospace(9.0), Color32::BLACK);
                    for i in 0..segs {
                        let x = r.left() + 12.0 + i as f32 * w;
                        let seg = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w - 1.0, r.height()));
                        let on = i < lit;
                        let col = match (on, i >= segs - 4) {
                            (true, true) => Color32::from_rgb(220, 0, 0),
                            (true, false) => Color32::BLACK,
                            (false, _) => Color32::from_gray(200),
                        };
                        p.rect_filled(seg, 0.0, col);
                    }
                }
            });
        self.show_controls = open;
    }

    /// Floating Info window with details about the sound and the selection.
    fn info_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_info;
        let peak = self.audio.as_ref().map(|a| {
            let key = (Arc::as_ptr(a) as usize, self.sel);
            match self.info_peak {
                Some((k, v)) if k == key => v,
                _ => {
                    let (s, e) = self.sel.unwrap_or((0, a.frames()));
                    let v = a.peak(s, e);
                    self.info_peak = Some((key, v));
                    v
                }
            }
        });
        self.place("Info", Pos2::new(self.desk.left() + 350.0, self.desk.bottom() - 138.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let Some(a) = &self.audio else {
                    ui.label("No sound open.");
                    return;
                };
                let r = a.rate as f64;
                egui::Grid::new("info").num_columns(2).spacing(Vec2::new(12.0, 2.0)).show(ui, |ui| {
                    ui.label("Sound");
                    ui.label(self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into()));
                    ui.end_row();
                    ui.label("Format");
                    ui.label(format!(
                        "{} Hz, {}",
                        a.rate,
                        match a.num_channels() {
                            1 => "mono".to_string(),
                            2 => "stereo".to_string(),
                            n => format!("{n} channels"),
                        }
                    ));
                    ui.end_row();
                    ui.label("Length");
                    ui.label(format!("{}  ({} samples)", fmt_time(a.duration_secs()), a.frames()));
                    ui.end_row();
                    match self.sel {
                        Some((s, e)) => {
                            ui.label("Selection");
                            ui.label(format!("{} – {}", fmt_time(s as f64 / r), fmt_time(e as f64 / r)));
                            ui.end_row();
                            ui.label("Sel. length");
                            ui.label(format!("{}  ({} samples)", fmt_time((e - s) as f64 / r), e - s));
                            ui.end_row();
                        }
                        None => {
                            ui.label("Cursor");
                            ui.label(fmt_time(self.cursor as f64 / r));
                            ui.end_row();
                        }
                    }
                    if let Some(p) = peak {
                        ui.label("Peak");
                        ui.label(if p > 0.0 { format!("{:.1} dB", 20.0 * p.log10()) } else { "silent".into() });
                        ui.end_row();
                    }
                    ui.label("Export as");
                    ui.label(self.wav_format.label());
                    ui.end_row();
                });
            });
        self.show_info = open;
    }

    /// Envelope / Bender: draw a curve over the selection, then apply it.
    fn curve_window(&mut self, ctx: &egui::Context) {
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
        let resp = egui::Modal::new(egui::Id::unique("curve")).show(ctx, |ui| {
            ui.set_width(480.0);
            match kind {
                CurveKind::Envelope => {
                    ui.heading("Envelope");
                    ui.label(format!("Draw how the volume of {scope} changes over time (0–200%)."));
                }
                CurveKind::Bender => {
                    ui.heading("Bender");
                    ui.label(format!(
                        "Draw a pitch curve across {scope} (±12 semitones). Like a tape, bending up also \
                         speeds it up and bending down slows it down."
                    ));
                }
            }
            ui.label("Click to add a point, drag to move it, double-click a point to remove it.");
            ui.add_space(4.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(480.0, 170.0), Sense::click_and_drag());
            let p = ui.painter_at(rect.expand(4.0));
            p.rect_filled(rect, 0.0, Color32::WHITE);
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
            p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                apply = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
                ui.add_space(20.0);
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

    /// Floating list of cue points: rename, jump to, delete.
    fn cues_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_cues;
        let rate = self.audio.as_ref().map_or(44100, |a| a.rate) as f64;
        let mut go: Option<usize> = None;
        let mut remove: Option<usize> = None;
        let mut add = false;
        let mut renamed = false;
        self.place("Cues", Pos2::new(self.desk.right() - 330.0, self.desk.bottom() - 138.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if self.cues.is_empty() {
                    ui.label("No cue points yet.");
                    ui.label("Insert ▸ Cue Point (⌘M) adds one at the cursor.");
                }
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    egui::Grid::new("cues").num_columns(4).spacing(Vec2::new(6.0, 3.0)).show(ui, |ui| {
                        for (i, c) in self.cues.iter_mut().enumerate() {
                            if ui.add_sized([120.0, 20.0], egui::TextEdit::singleline(&mut c.name)).changed() {
                                renamed = true;
                            }
                            ui.label(egui::RichText::new(fmt_time(c.frame as f64 / rate)).monospace());
                            if ui.small_button("Go").clicked() {
                                go = Some(c.frame);
                            }
                            if ui.small_button("×").on_hover_text("Remove this cue").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(2.0);
                add = ui.add_enabled(self.audio.is_some(), Button::new("Add at Cursor")).clicked();
            });
        self.show_cues = open;
        if renamed {
            self.dirty = true;
        }
        if let Some(f) = go {
            self.go_to(f);
        }
        if let Some(i) = remove {
            self.push_undo("Remove Cue");
            self.cues.remove(i);
            self.dirty = true;
        }
        if add {
            self.add_cue();
        }
    }

    /// Record window: record from the default input, then insert at the cursor.
    fn record_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_record;
        let recording = self.recorder.is_some();
        if let Some(r) = &self.recorder {
            self.rec_meter = r.take_peak().max(self.rec_meter * 0.85);
            ctx.request_repaint();
        } else {
            self.rec_meter *= 0.85;
        }
        let mut toggle = false;
        self.place("Record", Pos2::new(self.desk.center().x - 130.0, self.desk.center().y - 80.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(250.0);
                ui.horizontal(|ui| {
                    let label = if recording { "■ Stop" } else { "● Record" };
                    let btn = Button::new(egui::RichText::new(label).size(16.0).color(if recording { Color32::BLACK } else { Color32::from_rgb(200, 0, 0) }))
                        .min_size(Vec2::new(110.0, 32.0));
                    toggle = ui.add(btn).clicked();
                    let secs = self.recorder.as_ref().map_or(0.0, |r| r.seconds());
                    ui.label(egui::RichText::new(fmt_time(secs)).monospace().size(18.0));
                });
                let (r, _) = ui.allocate_exact_size(Vec2::new(250.0, 10.0), Sense::hover());
                let p = ui.painter();
                let segs = 34;
                let db = 20.0 * self.rec_meter.max(1e-6).log10();
                let lit = (((db + 48.0) / 48.0).clamp(0.0, 1.0) * segs as f32).round() as usize;
                let w = r.width() / segs as f32;
                for i in 0..segs {
                    let seg = Rect::from_min_size(Pos2::new(r.left() + i as f32 * w, r.top()), Vec2::new(w - 1.0, r.height()));
                    let col = match (i < lit, i >= segs - 4) {
                        (true, true) => Color32::from_rgb(220, 0, 0),
                        (true, false) => Color32::BLACK,
                        _ => Color32::from_gray(200),
                    };
                    p.rect_filled(seg, 0.0, col);
                }
                ui.label(if self.audio.is_some() {
                    "Records from the Mac's current input. The recording goes in at the cursor, or replaces the selection."
                } else {
                    "Records from the Mac's current input into a new sound."
                });
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

    fn dialog_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.dialog else { return };
        let scope = if self.sel.is_some() { "the selection" } else { "the whole sound" };
        let mut apply = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(kind.title());
            ui.label(kind.blurb(scope));
            ui.add_space(6.0);
            egui::Grid::new("fields").num_columns(2).spacing(Vec2::new(10.0, 6.0)).show(ui, |ui| {
                for f in kind.fields().iter().filter(|_| kind != Dialog::Equalizer) {
                    ui.label(f.label);
                    let v = self.params.entry(f.key).or_insert(f.default);
                    if !f.choices.is_empty() {
                        let current = f.choices.iter().find(|c| c.1 == *v).map_or("?", |c| c.0);
                        egui::ComboBox::from_id_salt(f.key).selected_text(current).show_ui(ui, |ui| {
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
                if kind == Dialog::InsertTone {
                    ui.label("Waveform");
                    egui::ComboBox::from_id_salt("wave").selected_text(self.tone_wave.label()).show_ui(ui, |ui| {
                        for w in Wave::ALL {
                            ui.selectable_value(&mut self.tone_wave, w, w.label());
                        }
                    });
                    ui.end_row();
                }
            });
            if kind == Dialog::Equalizer {
                // Graphic-EQ style: a vertical slider per band.
                ui.horizontal(|ui| {
                    for f in kind.fields() {
                        ui.vertical(|ui| {
                            ui.set_width(56.0);
                            let v = self.params.entry(f.key).or_insert(f.default);
                            ui.add(egui::Slider::new(v, f.min..=f.max).vertical().show_value(false));
                            ui.label(egui::RichText::new(format!("{:+.1}", *v)).monospace().size(10.0));
                            ui.label(egui::RichText::new(f.label).size(11.0));
                        });
                    }
                });
                if ui.small_button("Flat").clicked() {
                    for f in kind.fields() {
                        self.params.insert(f.key, 0.0);
                    }
                }
            }
            if kind == Dialog::Split {
                if let Some(a) = &self.audio {
                    let (s, e) = self.sel.unwrap_or((0, a.frames()));
                    let n = a.slice(s, e).find_sounds(self.split_gap()).len();
                    ui.label(format!("{n} sound{} found. Saved as {}.", if n == 1 { "" } else { "s" }, self.wav_format.label()));
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = if kind == Dialog::Split { "Choose Folder…" } else { "OK" };
                apply = ui.button(ok).clicked();
                cancel = ui.button("Cancel").clicked();
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if let Some(a) = &self.audio {
                let r = a.rate as f64;
                ui.label(format!("{} Hz · {} ch · {}", a.rate, a.num_channels(), fmt_time(a.duration_secs())));
                ui.separator();
                match self.sel {
                    Some((s, e)) => ui.label(format!("Sel {} – {} ({})", fmt_time(s as f64 / r), fmt_time(e as f64 / r), fmt_time((e - s) as f64 / r))),
                    None => ui.label(format!("Cursor {}", fmt_time(self.cursor as f64 / r))),
                };
                ui.separator();
            }
            if self.loading.is_some() {
                ui.spinner();
            }
            ui.label(&self.status);
        });
    }

    fn waveform(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let Some(n) = self.audio.as_ref().map(|a| a.frames()) else {
            ui.painter().rect_filled(full, 0.0, Color32::WHITE);
            ui.painter().text(
                full.center(),
                Align2::CENTER_CENTER,
                if self.loading.is_some() { "Loading…" } else { "Drop a sound here, or choose File ▸ Open\n(WAV, MP3, FLAC, OGG, M4A, AIFF…)" },
                FontId::proportional(16.0),
                Color32::from_gray(120),
            );
            return;
        };
        let rate = self.audio.as_ref().unwrap().rate as f64;
        let n_ch = self.audio.as_ref().unwrap().num_channels().max(1);

        let ruler = Rect::from_min_size(full.min, Vec2::new(full.width(), 18.0));
        let scroll = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - 15.0), full.max);
        let wave = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.right(), scroll.top()));
        let lane_h = wave.height() / n_ch as f32;
        let spp = self.view_len / wave.width() as f64; // frames per pixel

        // --- interaction on the waveform ---
        let resp = ui.allocate_rect(Rect::from_min_max(ruler.min, wave.max), Sense::click_and_drag());
        let x_to_frame = |x: f32, vs: f64, vl: f64| -> usize {
            let t = ((x - wave.left()) / wave.width()) as f64;
            (vs + t * vl).round().clamp(0.0, n as f64) as usize
        };

        if resp.hovered() {
            let (zoom, scroll_delta, hover) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zoom != 1.0 {
                let anchor = hover.map_or(self.view_start + self.view_len / 2.0, |p| {
                    self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len
                });
                self.zoom(1.0 / zoom as f64, anchor);
            }
            let d = if scroll_delta.x.abs() > scroll_delta.y.abs() { scroll_delta.x } else { scroll_delta.y };
            if d != 0.0 {
                self.view_start -= d as f64 / wave.width() as f64 * self.view_len;
                self.clamp_view();
            }
        }

        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        // Where the button went down: egui only reports a drag after the pointer has moved a
        // few pixels, and anchoring there would shift the start of the drag.
        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        match self.tool {
            Tool::Select => {
                // Cue flags in the ruler can be dragged along, or clicked to jump there.
                let flag_at = |q: Pos2, cues: &[Cue], vs: f64, vl: f64| {
                    (q.y <= ruler.bottom()).then(|| {
                        cues.iter().position(|c| (wave.left() + ((c.frame as f64 - vs) / vl) as f32 * wave.width() - q.x).abs() < 6.0)
                    })?
                };
                let hover_flag = resp.hover_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len));
                if hover_flag.is_some() || self.cue_drag.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                } else if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
                if resp.drag_started() {
                    if let Some(i) = press.and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)) {
                        self.push_undo("Move Cue");
                        self.cue_drag = Some(i);
                    }
                }
                if let Some(i) = self.cue_drag {
                    if let Some(q) = resp.interact_pointer_pos() {
                        if let Some(c) = self.cues.get_mut(i) {
                            c.frame = x_to_frame(q.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                        }
                    }
                    if resp.drag_stopped() {
                        self.cue_drag = None;
                        self.cues.sort_by_key(|c| c.frame);
                        self.dirty = true;
                    }
                } else if resp.clicked() && resp.interact_pointer_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)).is_some() {
                    let i = resp.interact_pointer_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)).unwrap();
                    let f = self.cues[i].frame;
                    self.go_to(f);
                } else {
                if resp.drag_started() {
                    if let Some(p) = press {
                        let f = x_to_frame(p.x, self.view_start, self.view_len);
                        self.drag_anchor = Some(match (shift, self.sel) {
                            // Shift-drag extends the existing selection from its far end.
                            (true, Some((s, e))) => if f.abs_diff(s) < f.abs_diff(e) { e } else { s },
                            (true, None) => self.cursor,
                            _ => f,
                        });
                    }
                }
                if resp.dragged() {
                    if let (Some(anchor), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos()) {
                        // Auto-scroll when dragging past the edges.
                        if p.x < wave.left() {
                            self.view_start -= self.view_len * 0.02;
                            self.clamp_view();
                        } else if p.x > wave.right() {
                            self.view_start += self.view_len * 0.02;
                            self.clamp_view();
                        }
                        let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                        let (s, e) = (anchor.min(f), anchor.max(f));
                        if e > s {
                            self.sel = Some((s, e));
                            self.cursor = s;
                            if !self.player.is_playing() {
                                self.play_origin = s;
                            }
                        }
                    }
                }
                if resp.drag_stopped() {
                    self.drag_anchor = None;
                    if self.player.is_playing() {
                        if let Some((s, e)) = self.sel {
                            self.player.play(s, e, self.looping);
                            self.play_origin = s;
                        }
                    }
                }
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let f = x_to_frame(p.x, self.view_start, self.view_len);
                        if shift {
                            let (a, b) = match self.sel {
                                Some((s, e)) => if f.abs_diff(s) < f.abs_diff(e) { (f, e) } else { (s, f) },
                                None => (self.cursor, f),
                            };
                            if a.max(b) > a.min(b) {
                                self.sel = Some((a.min(b), a.max(b)));
                            }
                        } else {
                            self.sel = None;
                            self.cursor = f;
                            self.play_origin = f;
                            if self.player.is_playing() {
                                self.player.play(f, n, false);
                            }
                        }
                    }
                }
                if resp.double_clicked() && n > 0 {
                    self.sel = Some((0, n));
                }
                }
            }
            Tool::Hand => {
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                }
                if resp.dragged() {
                    self.view_start -= resp.drag_delta().x as f64 / wave.width() as f64 * self.view_len;
                    self.clamp_view();
                }
            }
            Tool::Zoom => {
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(if alt { egui::CursorIcon::ZoomOut } else { egui::CursorIcon::ZoomIn });
                }
                if resp.drag_started() {
                    self.drag_anchor = press.map(|p| x_to_frame(p.x, self.view_start, self.view_len));
                }
                if resp.dragged() {
                    if let (Some(a), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos()) {
                        let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                        self.zoom_band = Some((a.min(f), a.max(f)));
                    }
                }
                if resp.drag_stopped() {
                    if let Some((s, e)) = self.zoom_band.take() {
                        if e > s + 8 {
                            self.view_start = s as f64;
                            self.view_len = (e - s) as f64;
                            self.clamp_view();
                        }
                    }
                    self.drag_anchor = None;
                }
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let f = self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len;
                        self.zoom(if alt { 2.0 } else { 0.5 }, f);
                    }
                }
            }
            Tool::Pencil => {
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                let value_at = |y: f32, ch: usize| {
                    let mid = wave.top() + ch as f32 * lane_h + lane_h / 2.0;
                    ((mid - y) / (lane_h / 2.0 * 0.92)).clamp(-1.0, 1.0)
                };
                if (resp.drag_started() || resp.clicked()) && self.pencil.is_none() {
                    if spp > PENCIL_MAX_SPP || n == 0 {
                        self.status = "Zoom in closer to draw with the pencil".into();
                    } else if let Some(p) = press {
                        let channel = (((p.y - wave.top()) / lane_h) as usize).min(n_ch - 1);
                        self.push_undo("Pencil");
                        self.pencil = Some(PencilStroke { channel, last: None });
                        let f = x_to_frame(p.x, self.view_start, self.view_len).min(n - 1);
                        self.pencil_to(f, value_at(p.y, channel));
                    }
                }
                if resp.dragged() {
                    if let (Some(ch), Some(p)) = (self.pencil.as_ref().map(|s| s.channel), resp.interact_pointer_pos()) {
                        let f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len).min(n - 1);
                        self.pencil_to(f, value_at(p.y, ch));
                    }
                }
                if (resp.drag_stopped() || resp.clicked()) && self.pencil.take().is_some() {
                    let a = self.audio.clone().unwrap();
                    self.set_audio(a);
                    self.dirty = true;
                    self.status = "Pencil — ⌘Z to undo".into();
                }
            }
        }

        // --- scrollbar ---
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let thumb_w = (self.view_len / n.max(1) as f64 * scroll.width() as f64).max(16.0).min(scroll.width() as f64) as f32;
        let track = scroll.width() - thumb_w;
        let max_start = (n as f64 - self.view_len).max(0.0);
        let thumb_x = if max_start > 0.0 { (self.view_start / max_start) as f32 * track } else { 0.0 };
        let thumb = Rect::from_min_size(Pos2::new(scroll.left() + thumb_x, scroll.top()), Vec2::new(thumb_w, scroll.height()));
        if sresp.drag_started() {
            self.scrollbar_drag = true;
        }
        if sresp.drag_stopped() {
            self.scrollbar_drag = false;
        }
        if (sresp.dragged() || sresp.clicked()) && track > 0.0 {
            if let Some(p) = sresp.interact_pointer_pos() {
                let t = ((p.x - scroll.left() - thumb_w / 2.0) / track).clamp(0.0, 1.0) as f64;
                self.view_start = t * max_start;
                self.clamp_view();
            }
        }

        // Follow the playhead while playing.
        let playing = self.player.is_playing();
        let play_pos = self.player.position();
        if playing {
            ui.ctx().request_repaint();
            if self.drag_anchor.is_none() && !self.scrollbar_drag && self.tool != Tool::Hand {
                let p = play_pos as f64;
                if p > self.view_start + self.view_len || p < self.view_start {
                    self.view_start = p - self.view_len * 0.05;
                    self.clamp_view();
                }
            }
        }

        // --- painting: black waveform on white, selection shown inverted ---
        let audio = self.audio.clone().unwrap();
        let painter = ui.painter_at(full);
        let vs = self.view_start;
        let vl = self.view_len;
        let frame_to_x = |f: f64| wave.left() + ((f - vs) / vl) as f32 * wave.width();
        let black = Color32::BLACK;

        painter.rect_filled(wave, 0.0, Color32::WHITE);
        painter.rect_filled(ruler, 0.0, Color32::from_gray(214));
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, black));
        if let Some((s, e)) = self.sel {
            let (x0, x1) = (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right()));
            if x1 > x0 {
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, ruler.bottom() - 1.0)), 0.0, Color32::from_gray(170));
            }
        }

        // Time ruler with "nice" tick spacing (~90 px apart) and minor ticks.
        let secs_per_px = vl / rate / wave.width() as f64;
        let nice = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = *nice.iter().find(|&&s| s / secs_per_px >= 90.0).unwrap_or(&1200.0);
        let minor = step / 5.0;
        let mut t = (vs / rate / minor).floor() * minor;
        let mut k = (t / minor).round() as i64;
        while t <= (vs + vl) / rate {
            let x = frame_to_x(t * rate);
            if x >= wave.left() {
                let major = k % 5 == 0;
                let h = if major { 7.0 } else { 3.0 };
                painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, black));
                if major {
                    painter.line_segment([Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, Color32::from_gray(232)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + 1.0), Align2::LEFT_TOP, fmt_tick(t, step), FontId::proportional(11.0), black);
                }
            }
            k += 1;
            t = k as f64 * minor;
        }

        // Selection: inverted (black) background.
        let sel_px = self.sel.map(|(s, e)| (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right())));
        if let Some((x0, x1)) = sel_px {
            if x1 > x0 {
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, wave.top()), Pos2::new(x1, wave.bottom())), 0.0, black);
            }
        }

        // Waveforms, one lane per channel.
        let peaks = self.peaks.as_ref().unwrap();
        for (ci, ch) in audio.channels.iter().enumerate() {
            let top = wave.top() + ci as f32 * lane_h;
            let mid = top + lane_h / 2.0;
            let amp = lane_h / 2.0 * 0.92;
            let in_sel = |f: usize| matches!(self.sel, Some((s, e)) if f >= s && f < e);
            let color_at = |f: usize| if in_sel(f) { Color32::WHITE } else { black };
            // Centre line, drawn per segment so it inverts inside the selection.
            painter.line_segment([Pos2::new(wave.left(), mid), Pos2::new(wave.right(), mid)], Stroke::new(1.0, Color32::from_gray(170)));
            if ci > 0 {
                painter.line_segment([Pos2::new(wave.left(), top), Pos2::new(wave.right(), top)], Stroke::new(1.0, black));
            }
            if spp <= 1.5 {
                // Zoomed in: connect individual samples.
                let first = vs.floor().max(0.0) as usize;
                let last = ((vs + vl).ceil() as usize + 1).min(n);
                let mut prev: Option<Pos2> = None;
                for (i, &s) in ch.iter().enumerate().take(last).skip(first) {
                    let p = Pos2::new(frame_to_x(i as f64), mid - s.clamp(-1.0, 1.0) * amp);
                    if let Some(q) = prev {
                        painter.line_segment([q, p], Stroke::new(1.0, color_at(i)));
                    }
                    if spp < 0.12 {
                        painter.rect_filled(Rect::from_center_size(p, Vec2::splat(3.0)), 0.0, color_at(i));
                    }
                    prev = Some(p);
                }
            } else {
                let blocks = &peaks.data[ci];
                let w = wave.width().ceil() as usize;
                for px in 0..w {
                    let f0 = (vs + px as f64 * spp).max(0.0) as usize;
                    let f1 = ((vs + (px + 1) as f64 * spp) as usize).min(n);
                    if f0 >= f1 {
                        continue;
                    }
                    let (lo, hi) = if spp >= peaks.block as f64 * 2.0 {
                        blocks[f0 / peaks.block..(f1 / peaks.block).max(f0 / peaks.block + 1).min(blocks.len())]
                            .iter()
                            .fold((f32::MAX, f32::MIN), |(a, b), &(l, h)| (a.min(l), b.max(h)))
                    } else {
                        ch[f0..f1].iter().fold((f32::MAX, f32::MIN), |(a, b), &s| (a.min(s), b.max(s)))
                    };
                    let x = wave.left() + px as f32 + 0.5;
                    let y0 = mid - hi.clamp(-1.0, 1.0) * amp;
                    let y1 = (mid - lo.clamp(-1.0, 1.0) * amp).max(y0 + 1.0);
                    painter.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], Stroke::new(1.0, color_at(f0)));
                }
            }
        }

        // Zoom tool rubber band.
        if let Some((s, e)) = self.zoom_band {
            let r = Rect::from_min_max(Pos2::new(frame_to_x(s as f64), wave.top() + 1.0), Pos2::new(frame_to_x(e as f64), wave.bottom() - 1.0));
            for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
                painter.add(egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, Color32::from_gray(80)), 3.0, 3.0));
            }
        }

        // Cue points: a dotted line through the sound and a labelled flag in the ruler.
        for c in &self.cues {
            let x = frame_to_x(c.frame as f64);
            if x < wave.left() - 1.0 || x > wave.right() + 1.0 {
                continue;
            }
            let in_sel = matches!(self.sel, Some((s, e)) if c.frame >= s && c.frame < e);
            let col = if in_sel { Color32::from_rgb(255, 210, 0) } else { Color32::from_rgb(0, 90, 200) };
            painter.add(egui::Shape::dashed_line(&[Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, col), 4.0, 3.0));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x, ruler.top() + 2.0), Pos2::new(x + 7.0, ruler.top() + 6.0), Pos2::new(x, ruler.top() + 10.0)],
                Color32::from_rgb(0, 90, 200),
                Stroke::NONE,
            ));
            painter.line_segment([Pos2::new(x, ruler.top() + 2.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, Color32::from_rgb(0, 90, 200)));
            let label = painter.layout_no_wrap(c.name.clone(), FontId::proportional(10.0), Color32::from_rgb(0, 60, 160));
            let bg = Rect::from_min_size(Pos2::new(x + 8.0, ruler.top() + 1.0), label.size() + Vec2::new(4.0, 0.0));
            painter.rect_filled(bg, 0.0, Color32::from_gray(230));
            painter.galley(bg.min + Vec2::new(2.0, 0.0), label, Color32::from_rgb(0, 60, 160));
        }

        // Insertion point (blinks like a text caret) and playhead.
        let cx = frame_to_x(self.cursor as f64);
        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        if self.sel.is_none() && cx >= wave.left() && cx <= wave.right() {
            if blink || playing {
                painter.line_segment([Pos2::new(cx, wave.top()), Pos2::new(cx, wave.bottom())], Stroke::new(1.0, black));
            }
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(cx - 4.0, ruler.bottom() - 6.0), Pos2::new(cx + 4.0, ruler.bottom() - 6.0), Pos2::new(cx, ruler.bottom())],
                black,
                Stroke::NONE,
            ));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        if playing {
            let px = frame_to_x(play_pos as f64);
            if px >= wave.left() && px <= wave.right() {
                painter.line_segment([Pos2::new(px, ruler.top()), Pos2::new(px, wave.bottom())], Stroke::new(1.5, Color32::from_rgb(220, 0, 0)));
            }
        }

        // Scroll bar: platinum track with a lavender thumb.
        painter.rect_filled(scroll, 0.0, Color32::from_gray(200));
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, black));
        let thumb_col = if self.scrollbar_drag { Color32::from_rgb(150, 150, 220) } else { Color32::from_rgb(204, 204, 255) };
        painter.rect_filled(thumb.shrink2(Vec2::new(0.0, 1.0)), 0.0, thumb_col);
        painter.rect_stroke(thumb.shrink2(Vec2::new(0.0, 1.0)), 0.0, Stroke::new(1.0, black), egui::StrokeKind::Inside);
        for dx in [-3.0f32, 0.0, 3.0] {
            let x = thumb.center().x + dx;
            painter.line_segment([Pos2::new(x, thumb.top() + 4.0), Pos2::new(x, thumb.bottom() - 4.0)], Stroke::new(1.0, Color32::from_rgb(90, 90, 160)));
        }

        // Hover time readout.
        if let Some(p) = resp.hover_pos() {
            let f = x_to_frame(p.x, vs, vl);
            painter.text(Pos2::new(wave.right() - 6.0, ruler.top() + 1.0), Align2::RIGHT_TOP, fmt_time(f as f64 / rate), FontId::monospace(11.0), black);
        }
    }

    /// Draw a pencil line from the previous point to (`f`, `v`) in the stroke's channel.
    fn pencil_to(&mut self, f: usize, v: f32) {
        let Some(stroke) = self.pencil.as_mut() else { return };
        let (lf, lv) = stroke.last.unwrap_or((f, v));
        stroke.last = Some((f, v));
        let channel = stroke.channel;
        let Some(arc) = self.audio.as_mut() else { return };
        // Copies the audio once at the start of a stroke (the undo step keeps the original).
        let ch = &mut Arc::make_mut(arc).channels[channel];
        let ((f0, v0), (f1, v1)) = if lf <= f { ((lf, lv), (f, v)) } else { ((f, v), (lf, lv)) };
        for i in f0..=f1.min(ch.len().saturating_sub(1)) {
            let t = if f1 > f0 { (i - f0) as f32 / (f1 - f0) as f32 } else { 0.0 };
            ch[i] = v0 + (v1 - v0) * t;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else { return };
        let what = match pending {
            Pending::New => "start a new sound",
            Pending::Open(_) => "open another file",
            Pending::Close => "quit",
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading("Unsaved changes");
            ui.label(format!("You have edits that haven't been exported. Discard them and {what}?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Export…").clicked() {
                    choice = Some(0);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(2);
                }
            });
        });
        match choice {
            Some(0) => {
                self.export(false);
                if !self.dirty {
                    self.resolve_pending(ctx);
                }
            }
            Some(1) => self.resolve_pending(ctx),
            Some(2) => self.pending = None,
            _ => {}
        }
    }

    fn resolve_pending(&mut self, ctx: &egui::Context) {
        match self.pending.take() {
            Some(Pending::New) => self.new_sound(),
            Some(Pending::Open(p)) => {
                self.dirty = false;
                self.start_load(p, ctx);
            }
            Some(Pending::Close) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loading();

        // Drag and drop.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.into_iter().next() {
            self.request_open(p, &ctx);
        }

        // Ask before quitting with unsaved edits.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        let name = self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into());
        let title = if self.audio.is_some() { format!("{name}{} — Sound Edit", if self.dirty { " (edited)" } else { "" }) } else { "Sound Edit".into() };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let modal_open = self.pending.is_some() || self.dialog.is_some() || self.curve.is_some();
        for a in self.menu.poll() {
            if !modal_open {
                self.run(a, &ctx);
            } else if a == Action::Quit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !modal_open {
            self.handle_shortcuts(&ctx);
        }
        self.menu.sync(|a| self.enabled(a), |a| self.checked(a), |a| self.menu_label(a));

        // The "desktop", with the sound shown in a classic document window.
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let screen = ui.max_rect();
            ui.painter().rect_filled(screen, 0.0, DESKTOP);
            self.desk = screen;
            let left = if self.show_tools { 112.0 } else { 12.0 };
            let bottom = if self.show_controls || self.show_info { 150.0 } else { 12.0 };
            let doc = Rect::from_min_max(screen.min + Vec2::new(left, 12.0), screen.max - Vec2::new(12.0, bottom));
            if doc.width() < 100.0 || doc.height() < 80.0 {
                return;
            }
            let p = ui.painter();
            p.rect_filled(doc.translate(Vec2::new(2.0, 2.0)), 0.0, Color32::from_black_alpha(110));
            p.rect_filled(doc, 0.0, Color32::from_gray(221));
            let title_bar = Rect::from_min_size(doc.min, Vec2::new(doc.width(), 20.0));
            paint_title_bar(p, title_bar, &format!("{name}{}", if self.dirty { " •" } else { "" }));
            let status = Rect::from_min_max(Pos2::new(doc.left(), doc.bottom() - 20.0), doc.max);
            let body = Rect::from_min_max(Pos2::new(doc.left() + 1.0, title_bar.bottom()), Pos2::new(doc.right() - 1.0, status.top()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.waveform(ui));
            ui.painter().line_segment([status.left_top(), status.right_top()], Stroke::new(1.0, Color32::BLACK));
            ui.scope_builder(egui::UiBuilder::new().max_rect(status.shrink2(Vec2::new(6.0, 1.0))), |ui| self.status_bar(ui));
            ui.painter().rect_stroke(doc, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
        });

        if self.show_tools {
            self.tools_window(&ctx);
        }
        if self.show_controls {
            self.controls_window(&ctx);
        }
        if self.show_cues {
            self.cues_window(&ctx);
        }
        if self.show_record || self.recorder.is_some() {
            self.show_record = true;
            self.record_window(&ctx);
        }
        if self.show_info {
            self.info_window(&ctx);
        }

        // Highlight when a file is dragged over the window.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
            painter.text(screen.center(), Align2::CENTER_CENTER, "Drop to open", FontId::proportional(28.0), Color32::WHITE);
        }

        self.dialog_window(&ctx);
        self.curve_window(&ctx);
        self.confirm_dialog(&ctx);
        self.run_automation(&ctx);
        self.frame_count += 1;
    }
}

const DESKTOP: Color32 = Color32::from_rgb(160, 160, 184);

/// Classic Mac title bar: pinstripes with the title on a plain patch in the middle.
fn paint_title_bar(p: &egui::Painter, r: Rect, title: &str) {
    p.rect_filled(r, 0.0, Color32::from_gray(221));
    let mut y = r.top() + 4.0;
    while y < r.bottom() - 3.0 {
        p.line_segment([Pos2::new(r.left() + 4.0, y), Pos2::new(r.right() - 4.0, y)], Stroke::new(1.0, Color32::from_gray(120)));
        p.line_segment([Pos2::new(r.left() + 4.0, y + 1.0), Pos2::new(r.right() - 4.0, y + 1.0)], Stroke::new(1.0, Color32::WHITE));
        y += 2.0;
    }
    let galley = p.layout_no_wrap(title.to_string(), FontId::proportional(13.0), Color32::BLACK);
    let patch = Rect::from_center_size(r.center(), galley.size() + Vec2::new(16.0, 0.0)).intersect(r);
    p.rect_filled(Rect::from_x_y_ranges(patch.x_range(), r.y_range()), 0.0, Color32::from_gray(221));
    p.galley(patch.center() - galley.size() / 2.0, galley, Color32::BLACK);
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, Color32::BLACK));
}

/// Platinum-era Mac look: grey surfaces, square corners, 1 px black outlines, inverted
/// highlights, and Geneva/Monaco when the system has them.
fn classic_style(ctx: &egui::Context) {
    use egui::{CornerRadius, Shadow};
    ctx.set_theme(egui::Theme::Light);
    let black = Color32::BLACK;
    let mut v = egui::Visuals::light();
    let platinum = Color32::from_gray(221);
    v.panel_fill = platinum;
    v.window_fill = platinum;
    v.window_stroke = Stroke::new(1.0, black);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    let shadow = Shadow { offset: [2, 2], blur: 0, spread: 0, color: Color32::from_black_alpha(110) };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.extreme_bg_color = Color32::WHITE;
    v.override_text_color = Some(black);
    v.selection.bg_fill = black;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.window_highlight_topmost = false;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.0, black);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(150));
    v.widgets.inactive.bg_fill = Color32::from_gray(238);
    v.widgets.inactive.weak_bg_fill = Color32::from_gray(238);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, black);
    v.widgets.hovered.bg_fill = Color32::from_gray(250);
    v.widgets.hovered.weak_bg_fill = Color32::from_gray(250);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, black);
    v.widgets.active.bg_fill = Color32::from_gray(120);
    v.widgets.active.weak_bg_fill = Color32::from_gray(120);
    v.widgets.active.bg_stroke = Stroke::new(1.0, black);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.open = v.widgets.hovered;
    ctx.set_visuals_of(egui::Theme::Light, v);

    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        ("Geneva", "/System/Library/Fonts/Geneva.ttf", egui::FontFamily::Proportional),
        ("Monaco", "/System/Library/Fonts/Monaco.ttf", egui::FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
}

/// Scripted self-test, driven by environment variables (used to verify the app headlessly):
/// `SOUND_EDIT_ACTIONS="sel:1.0-2.5,fadeout,normalize"` and `SOUND_EDIT_SNAPSHOT=out.png`.
/// Once the file has loaded, runs the actions, saves a screenshot and quits.
struct Automation {
    actions: Vec<String>,
    snapshot: Option<PathBuf>,
    frames_after_load: u32,
    requested: bool,
    /// Print the native menu bar a few frames later, once its state has synced.
    dump_menu: bool,
    done: Arc<std::sync::atomic::AtomicBool>,
}

impl Automation {
    fn from_env() -> Option<Automation> {
        let snapshot = std::env::var_os("SOUND_EDIT_SNAPSHOT").map(PathBuf::from);
        let actions: Vec<String> = std::env::var("SOUND_EDIT_ACTIONS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        (snapshot.is_some() || !actions.is_empty()).then(|| Automation {
            actions,
            snapshot,
            frames_after_load: 0,
            requested: false,
            dump_menu: false,
            done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

impl App {
    fn run_automation(&mut self, ctx: &egui::Context) {
        let Some(mut auto) = self.automation.take() else { return };
        ctx.request_repaint();
        if self.audio.is_none() || self.loading.is_some() {
            self.automation = Some(auto);
            return;
        }
        auto.frames_after_load += 1;
        if auto.frames_after_load == 3 {
            let rate = self.audio.as_ref().unwrap().rate as f64;
            for a in std::mem::take(&mut auto.actions) {
                let (cmd, arg) = a.split_once(':').unwrap_or((a.as_str(), ""));
                match cmd {
                    "sel" => {
                        let (s, e) = arg.split_once('-').unwrap_or(("0", "0"));
                        let f = |t: &str| ((t.parse::<f64>().unwrap_or(0.0) * rate) as usize).min(self.frames());
                        self.sel = Some((f(s), f(e))).filter(|(s, e)| e > s);
                    }
                    "cursor" => self.cursor = (arg.parse::<f64>().unwrap_or(0.0) * rate) as usize,
                    "trim" => self.trim(),
                    "delete" => self.delete(),
                    "fadein" => self.fade(true),
                    "fadeout" => self.fade(false),
                    "normalize" => self.normalize(),
                    "gain" => self.gain(arg.parse().unwrap_or(3.0)),
                    "silence" => self.silence(),
                    "reverse" => self.reverse(),
                    "split" => self.split_to(Path::new(arg)),
                    "dumpmenu" => auto.dump_menu = true,
                    "apply" => {
                        if let Some(d) = self.dialog.take() {
                            self.apply_dialog(d);
                        }
                        if let Some(k) = self.curve.take() {
                            self.apply_curve(k);
                        }
                    }
                    "set" => {
                        if let Some((k, v)) = arg.split_once('=') {
                            if let Some(slot) = self.params.iter_mut().find(|(key, _)| **key == k) {
                                *slot.1 = v.parse().unwrap_or(*slot.1);
                            }
                        }
                    }
                    "run" => match Action::from_id(arg) {
                        Some(act) => self.run(act, ctx),
                        None => eprintln!("unknown action id {arg}"),
                    },
                    "copy" => { self.copy(ctx); }
                    "cut" => self.cut(ctx),
                    "paste" => self.paste(),
                    "undo" => self.undo(),
                    "redo" => self.redo(),
                    "play" => self.toggle_play(),
                    "zoomsel" => self.zoom_to_selection(),
                    "export" => {
                        let a = self.audio.clone().unwrap();
                        let r = audio::save_wav(&a, Path::new(arg), self.wav_format);
                        self.status = format!("export {arg}: {r:?}");
                    }
                    other => eprintln!("unknown action {other}"),
                }
                eprintln!("action {a}: {}", self.status);
            }
        }
        #[cfg(target_os = "macos")]
        if auto.dump_menu && auto.frames_after_load == 10 {
            crate::menu::dump_main_menu();
        }
        if auto.frames_after_load >= 30 && ctx.input(|i| i.time) > 2.0 && !auto.requested {
            auto.requested = true;
            match auto.snapshot.clone() {
                Some(path) => {
                    let done = auto.done.clone();
                    ctx.request_screenshot(move |img| {
                        if let Err(e) = save_png(&img, &path) {
                            eprintln!("snapshot failed: {e}");
                        }
                        done.store(true, std::sync::atomic::Ordering::SeqCst);
                    });
                }
                None => auto.done.store(true, std::sync::atomic::Ordering::SeqCst),
            }
        }
        if auto.done.load(std::sync::atomic::Ordering::SeqCst) {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.automation = Some(auto);
    }
}

fn save_png(img: &egui::ColorImage, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(|e| e.to_string())
}

/// Monochrome vector icons for the Tools and Controls palettes.
fn paint_icon(p: &egui::Painter, rect: Rect, a: Action, color: Color32, playing: bool) {
    let c = rect.center();
    let st = Stroke::new(1.5, color);
    let pt = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        p.line_segment([pt(x0, y0), pt(x1, y1)], st);
    };
    let thick = |x0: f32, y0: f32, x1: f32, y1: f32, w: f32| {
        p.line_segment([pt(x0, y0), pt(x1, y1)], Stroke::new(w, color));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| pt(x, y)).collect(), color, Stroke::NONE));
    };
    // Square brackets at ±x, 16 px tall.
    let brackets = |x: f32| {
        for s in [-1.0f32, 1.0] {
            line(s * x, -8.0, s * x, 8.0);
            line(s * x, -8.0, s * (x - 3.0), -8.0);
            line(s * x, 8.0, s * (x - 3.0), 8.0);
        }
    };
    let arrow_h = |x0: f32, x1: f32| {
        line(x0, 0.0, x1, 0.0);
        line(x0, 0.0, x0 + 3.0, -3.0);
        line(x0, 0.0, x0 + 3.0, 3.0);
        line(x1, 0.0, x1 - 3.0, -3.0);
        line(x1, 0.0, x1 - 3.0, 3.0);
    };
    let bars = |bars: &[(f32, f32)]| {
        for &(x, h) in bars {
            line(x, -h, x, h);
        }
    };
    match a {
        Action::ToolSelect => {
            line(0.0, -8.0, 0.0, 8.0);
            line(-3.0, -8.0, 3.0, -8.0);
            line(-3.0, 8.0, 3.0, 8.0);
        }
        Action::ToolHand => {
            let palm = Rect::from_min_max(pt(-5.0, -1.0), pt(5.0, 8.0));
            p.rect_stroke(palm, 3.0, st, egui::StrokeKind::Middle);
            for (x, top) in [(-3.5f32, -6.0f32), (-1.0, -8.0), (1.5, -8.0), (4.0, -6.0)] {
                line(x, top, x, -1.0);
            }
            line(-5.0, 4.0, -8.0, 0.0);
        }
        Action::ToolZoom => {
            p.circle_stroke(pt(-2.0, -2.0), 5.5, st);
            thick(2.0, 2.0, 7.5, 7.5, 3.0);
        }
        Action::ToolPencil => {
            thick(-3.5, 3.5, 6.0, -6.0, 4.5);
            fill(&[(-8.0, 8.0), (-6.5, 2.5), (-2.5, 6.5)]);
        }
        Action::Cut => {
            p.circle_stroke(pt(-4.0, 5.0), 2.6, st);
            p.circle_stroke(pt(4.0, 5.0), 2.6, st);
            line(-2.5, 3.0, 4.0, -8.0);
            line(2.5, 3.0, -4.0, -8.0);
        }
        Action::Copy => {
            let r1 = Rect::from_center_size(pt(-2.5, -2.5), Vec2::new(10.0, 12.0));
            let r2 = r1.translate(Vec2::new(5.0, 5.0));
            p.rect_stroke(r1, 0.0, st, egui::StrokeKind::Middle);
            p.rect_filled(r2, 0.0, Color32::from_gray(238));
            p.rect_stroke(r2, 0.0, st, egui::StrokeKind::Middle);
        }
        Action::Paste => {
            p.rect_stroke(Rect::from_min_max(pt(-6.0, -6.0), pt(6.0, 8.0)), 0.0, st, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(pt(-3.0, -8.5), pt(3.0, -4.5)), 0.0, color);
            line(-3.0, 0.0, 3.0, 0.0);
            line(-3.0, 3.5, 3.0, 3.5);
        }
        Action::Delete => {
            line(-7.0, -5.0, 7.0, -5.0);
            line(-2.0, -7.5, 2.0, -7.5);
            line(-5.5, -5.0, -4.0, 8.0);
            line(5.5, -5.0, 4.0, 8.0);
            line(-4.0, 8.0, 4.0, 8.0);
            line(-1.5, -2.0, -1.0, 5.5);
            line(1.5, -2.0, 1.0, 5.5);
        }
        Action::Trim => {
            brackets(8.0);
            bars(&[(-3.0, 3.0), (0.0, 6.0), (3.0, 4.0)]);
            line(-12.0, 0.0, -10.0, 0.0);
            line(10.0, 0.0, 12.0, 0.0);
        }
        Action::Silence => {
            brackets(8.0);
            line(-6.0, 0.0, 6.0, 0.0);
        }
        Action::FadeIn => fill(&[(-8.0, 7.0), (8.0, 7.0), (8.0, -7.0)]),
        Action::FadeOut => fill(&[(-8.0, 7.0), (-8.0, -7.0), (8.0, 7.0)]),
        Action::Normalize => {
            line(-9.0, -8.0, 9.0, -8.0);
            line(-9.0, 8.0, 9.0, 8.0);
            bars(&[(-6.0, 3.0), (-2.0, 6.0), (2.0, 4.0), (6.0, 2.0)]);
        }
        Action::Gain => {
            bars(&[(-7.0, 1.5), (-3.5, 3.5), (0.0, 5.5), (3.5, 7.0), (7.0, 8.5)]);
        }
        Action::Reverse => {
            bars(&[(-1.0, 2.0), (2.0, 4.0), (5.0, 6.0), (8.0, 8.0)]);
            line(-8.0, 0.0, -3.0, 0.0);
            line(-8.0, 0.0, -5.0, -3.0);
            line(-8.0, 0.0, -5.0, 3.0);
        }
        Action::Echo => {
            bars(&[(-8.0, 8.0), (-3.0, 5.0), (2.0, 3.0), (6.5, 1.5)]);
        }
        Action::Split => {
            for x in [-5.0f32, 5.0] {
                for y in [-8.0f32, -3.0, 2.0, 7.0] {
                    line(x, y, x, y + 2.5);
                }
            }
            bars(&[(-10.0, 5.0), (-8.0, 3.0), (-1.0, 6.0), (1.0, 4.0), (8.0, 4.0), (10.0, 6.0)]);
        }
        Action::ZoomFit => {
            line(-10.0, -7.0, -10.0, 7.0);
            line(10.0, -7.0, 10.0, 7.0);
            arrow_h(-8.0, 8.0);
        }
        Action::ZoomSel => {
            brackets(10.0);
            arrow_h(-6.0, 6.0);
        }
        Action::GoStart => {
            thick(-6.0, -6.0, -6.0, 6.0, 2.5);
            fill(&[(-4.5, 0.0), (5.5, -6.0), (5.5, 6.0)]);
        }
        Action::GoEnd => {
            thick(6.0, -6.0, 6.0, 6.0, 2.5);
            fill(&[(4.5, 0.0), (-5.5, -6.0), (-5.5, 6.0)]);
        }
        Action::PlayPause if playing => {
            p.rect_filled(Rect::from_min_max(pt(-5.0, -6.0), pt(-1.5, 6.0)), 0.0, color);
            p.rect_filled(Rect::from_min_max(pt(1.5, -6.0), pt(5.0, 6.0)), 0.0, color);
        }
        Action::PlayPause => fill(&[(-5.0, -7.0), (7.0, 0.0), (-5.0, 7.0)]),
        Action::Stop => {
            p.rect_filled(Rect::from_center_size(c, Vec2::splat(11.0)), 0.0, color);
        }
        Action::Record => {
            let red = if color == Color32::WHITE { Color32::WHITE } else if color == Color32::BLACK { Color32::from_rgb(210, 0, 0) } else { color };
            p.circle_filled(c, 6.5, red);
        }
        Action::Loop => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = 0.4 + i as f32 / 24.0 * 5.2;
                    pt(7.0 * t.cos(), 5.5 * t.sin())
                })
                .collect();
            p.add(egui::Shape::line(pts, st));
            let t = 0.4f32;
            let tip = pt(7.0 * t.cos(), 5.5 * t.sin());
            p.add(egui::Shape::convex_polygon(
                vec![tip + Vec2::new(-3.5, -1.0), tip + Vec2::new(2.5, -2.5), tip + Vec2::new(0.5, 3.0)],
                color,
                Stroke::NONE,
            ));
        }
        _ => {}
    }
}

/// sound01.wav, sound02.wav… (more digits once there are 100+ sounds).
fn split_names(count: usize) -> Vec<String> {
    let width = count.to_string().len().max(2);
    (1..=count).map(|i| format!("sound{i:0width$}.wav")).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn fmt_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    let m = (secs / 60.0).floor() as u64;
    let s = secs - m as f64 * 60.0;
    format!("{m}:{s:06.3}")
}

fn fmt_tick(t: f64, step: f64) -> String {
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    if step >= 1.0 {
        format!("{m}:{:02}", s.round() as u64)
    } else if step >= 0.1 {
        format!("{m}:{s:04.1}")
    } else if step >= 0.01 {
        format!("{m}:{s:05.2}")
    } else {
        format!("{m}:{s:06.3}")
    }
}
