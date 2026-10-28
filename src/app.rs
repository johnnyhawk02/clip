//! The editor window: tools, waveform view, transport bar and Info/Cues panel, effect dialogs,
//! menus and shortcuts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self, Align2, Button, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2,
};

use crate::actions::{self, Action};
use crate::audio::{self, Audio, Cue, Markers, Peaks, Region, WavFormat};
use crate::effects::{self, Wave};
use crate::menu::NativeMenu;
use crate::player::Player;
use crate::recorder::Recorder;

type Sel = (usize, usize);

/// Undo history is capped by memory rather than step count, since each step is a full copy.
const UNDO_BUDGET_BYTES: usize = 2 << 30;

/// A piece of audio placed on a track's timeline. Only `len` frames from `trim` onward are
/// heard, so its edges can be trimmed without losing the audio underneath (as in Ableton).
#[derive(Clone)]
struct Clip {
    name: String,
    audio: Arc<Audio>,
    peaks: Arc<Peaks>,
    /// Timeline position of the clip's first frame.
    start: usize,
    trim: usize,
    len: usize,
    cues: Vec<Cue>,
    regions: Vec<Region>,
    loop_points: Option<Sel>,
    /// A group clip holds its own tracks of clips; `audio` is then their rendered mix. Copies
    /// share this until one is opened and changed, so they stay independent.
    group: Option<Arc<Vec<Track>>>,
}

/// One level up from the arrangement being shown: the parent's tracks, and which of its clips
/// is the open group.
#[derive(Clone)]
struct NestFrame {
    tracks: Vec<Track>,
    track: usize,
    clip: usize,
    active: usize,
    view_start: f64,
    view_len: f64,
}

impl Clip {
    fn new(name: impl Into<String>, audio: Arc<Audio>, start: usize) -> Clip {
        let peaks = Arc::new(Peaks::build(&audio));
        let len = audio.frames();
        Clip { name: name.into(), audio, peaks, start, trim: 0, len, cues: Vec::new(), regions: Vec::new(), loop_points: None, group: None }
    }

    fn end(&self) -> usize {
        self.start + self.len
    }

    /// The part of the audio the clip plays.
    fn visible(&self) -> Audio {
        let end = (self.trim + self.len).min(self.audio.frames());
        self.audio.slice(self.trim.min(end), end)
    }
}

/// Which view is showing: the arrangement of clips, or one clip open in the waveform editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Arrangement,
    Editor { track: usize, clip: usize },
}

/// A clip being moved or trimmed in the arrangement.
struct ClipDrag {
    track: usize,
    clip: usize,
    mode: DragMode,
    /// Pointer position minus clip start when the drag began (for moves).
    grab: i64,
    orig: Clip,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}

/// One track of the arrangement: its clips and mixer settings. All clips share a sample rate.
#[derive(Clone)]
struct Track {
    name: String,
    clips: Vec<Clip>,
    /// Linear volume (1 = unchanged).
    gain: f32,
    /// -1 (left) .. 1 (right).
    pan: f32,
    /// Index into `TRACK_COLORS` (0 is the default grey).
    color: usize,
    mute: bool,
    solo: bool,
}

/// Waveform colours a track can be given; muted tones so they sit with the grey look.
const TRACK_COLORS: [Color32; 6] = [
    Color32::from_gray(55),
    Color32::from_rgb(70, 95, 145),
    Color32::from_rgb(55, 120, 85),
    Color32::from_rgb(170, 115, 35),
    Color32::from_rgb(160, 60, 60),
    Color32::from_rgb(115, 75, 145),
];

/// Height of the time ruler: tick labels and cues, the regions band, then loop and tick marks.
const RULER_H: f32 = 30.0;

impl Track {
    fn new(name: impl Into<String>) -> Track {
        Track { name: name.into(), clips: Vec::new(), gain: 1.0, pan: 0.0, color: 0, mute: false, solo: false }
    }
}

struct Snapshot {
    audio: Option<Arc<Audio>>,
    tracks: Vec<Track>,
    nest: Vec<NestFrame>,
    active: usize,
    sel_clip: Option<(usize, usize)>,
    cues: Vec<Cue>,
    regions: Vec<Region>,
    loop_points: Option<Sel>,
    sel: Option<Sel>,
    cursor: usize,
    label: &'static str,
}

/// Output formats for batch conversion: (label, file extension, sample format).
const BATCH_FORMATS: [(&str, &str, WavFormat); 6] = [
    ("WAV 16-bit", "wav", WavFormat::Pcm16),
    ("WAV 24-bit", "wav", WavFormat::Pcm24),
    ("WAV 32-bit float", "wav", WavFormat::Float32),
    ("WAV 8-bit", "wav", WavFormat::Pcm8),
    ("AIFF 16-bit", "aiff", WavFormat::Pcm16),
    ("AIFF 24-bit", "aiff", WavFormat::Pcm24),
];

#[derive(Default)]
struct BatchProgress {
    done: usize,
    total: usize,
    errors: Vec<String>,
    finished: bool,
}

/// Settings and progress for File ▸ Batch Convert.
#[derive(Default)]
struct Batch {
    files: Vec<PathBuf>,
    out_dir: Option<PathBuf>,
    format: usize,
    /// 0 keeps each file's own rate.
    rate: u32,
    /// 0 keeps each file's own channels.
    channels: usize,
    normalize: bool,
    progress: Option<Arc<std::sync::Mutex<BatchProgress>>>,
}

/// How positions are shown in the ruler, counter and readouts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RulerUnits {
    Time,
    Samples,
    Timecode,
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
    const ALL: [Dialog; 25] = [
        Dialog::Normalize, Dialog::Amplify, Dialog::Split, Dialog::Echo, Dialog::Reverb, Dialog::Flanger,
        Dialog::Smooth, Dialog::NoiseGate, Dialog::Pitch, Dialog::Tempo, Dialog::Speed,
        Dialog::InsertSilence, Dialog::InsertTone, Dialog::InsertNoise, Dialog::MixPaste,
        Dialog::Convert, Dialog::Equalizer, Dialog::InsertDtmf, Dialog::Delay, Dialog::Emphasize,
        Dialog::Compressor, Dialog::Fade, Dialog::FmSynth, Dialog::OffsetTrack, Dialog::CutGaps,
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

enum Pending {
    New,
    Open(PathBuf),
    Close,
}

pub struct App {
    audio: Option<Arc<Audio>>,
    peaks: Option<Arc<Peaks>>,
    /// All tracks; `audio`/`peaks` mirror the active one, which is the one being edited.
    tracks: Vec<Track>,
    active: usize,
    view: View,
    /// Arrangement view: visible range, playhead, selected clip and any clip being dragged.
    arr_view_start: f64,
    arr_view_len: f64,
    arr_cursor: usize,
    sel_clip: Option<(usize, usize)>,
    clip_drag: Option<ClipDrag>,
    /// Clips selected besides `sel_clip` (shift-click), for grouping.
    sel_extra: Vec<(usize, usize)>,
    /// Open groups, outermost first; empty at the top level (the song).
    nest: Vec<NestFrame>,
    /// The file being loaded becomes a clip at the playhead (not a new track).
    import_as_clip: bool,
    /// What the player is playing: the active track alone, or the mix of all tracks.
    play_audio: Option<Arc<Audio>>,
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
    dtmf_digits: String,

    status: String,
    /// File being loaded in the background, and whether it becomes a new track.
    loading: Option<(PathBuf, Receiver<Result<Audio, String>>, bool)>,
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
    regions: Vec<Region>,
    loop_points: Option<Sel>,
    show_regions: bool,
    ruler: RulerUnits,
    show_spectrum: bool,
    spectrum_3d: bool,
    /// Cached 3D spectrum slices, keyed by (audio pointer, range).
    spectrum_cache: Option<((usize, Sel), Vec<Vec<f32>>)>,
    show_batch: bool,
    batch: Batch,
    /// Mix dialog: which tracks to include (open while Some), and whether to add a new track.
    mix_include: Option<Vec<bool>>,
    mix_to_new: bool,
    /// Turn mixes down when they would clip.
    auto_gain: bool,
    cue_drag: Option<usize>,
    /// ⌥-drag fine adjustment: (moving the end edge?, exact position in frames).
    fine_edge: Option<(bool, f64)>,
    /// The region the last double-click selected, so a second double-click selects everything.
    last_double: Option<Sel>,
    /// Space is held (temporary Hand tool), and whether it was used to scroll.
    space_down: bool,
    space_used: bool,
    /// End of the last pencil stroke: (channel, frame, value), for Shift-click straight lines.
    pencil_last: Option<(usize, usize, f32)>,
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
        clean_style(&cc.egui_ctx);
        let player = Player::new();
        let status = match &player.error {
            Some(e) => format!("Audio output unavailable: {e}"),
            None => "Drop an audio file here, or press ⌘O to open one.".into(),
        };
        let mut app = App {
            audio: None,
            peaks: None,
            tracks: Vec::new(),
            active: 0,
            view: View::Arrangement,
            arr_view_start: 0.0,
            arr_view_len: 44100.0 * 10.0,
            arr_cursor: 0,
            sel_clip: None,
            clip_drag: None,
            sel_extra: Vec::new(),
            nest: Vec::new(),
            import_as_clip: false,
            play_audio: None,
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
            dtmf_digits: "555-0123".into(),
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
            regions: Vec::new(),
            loop_points: None,
            show_regions: false,
            ruler: RulerUnits::Time,
            show_spectrum: false,
            spectrum_3d: false,
            spectrum_cache: None,
            show_batch: false,
            batch: Batch::default(),
            mix_include: None,
            mix_to_new: true,
            auto_gain: true,
            cue_drag: None,
            fine_edge: None,
            last_double: None,
            space_down: false,
            space_used: false,
            pencil_last: None,
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
        self.start_load_as(path, ctx, false);
    }

    fn import_track_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Import a file as a new track")
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf"])
            .pick_file();
        if let Some(p) = picked {
            self.start_load_as(p, ctx, true);
        }
    }

    fn import_clip_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Import a file as a clip at the playhead")
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf"])
            .pick_file();
        if let Some(p) = picked {
            self.import_as_clip = true;
            self.start_load_as(p, ctx, true);
        }
    }

    fn start_load_as(&mut self, path: PathBuf, ctx: &egui::Context, as_track: bool) {
        let (tx, rx) = channel();
        let p = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(audio::load(&p));
            ctx.request_repaint();
        });
        self.status = format!("Loading {}…", file_name(&path));
        self.loading = Some((path, rx, as_track));
    }

    fn poll_loading(&mut self) {
        let Some((path, rx, as_track)) = &self.loading else { return };
        let Ok(result) = rx.try_recv() else { return };
        let (path, as_track) = (path.clone(), *as_track);
        self.loading = None;
        let as_clip = std::mem::take(&mut self.import_as_clip);
        if as_track && as_clip {
            match result {
                Ok(a) if a.frames() > 0 => {
                    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Clip".into());
                    let pos = self.add_clip(&name, Arc::new(a), "Import Clip");
                    let m = audio::read_wav_markers(&path);
                    let c = &mut self.tracks[pos.0].clips[pos.1];
                    (c.cues, c.regions, c.loop_points) = (m.cues, m.regions, m.loop_points);
                    self.status = format!("Added {} as a clip at the playhead", file_name(&path));
                }
                Ok(_) => self.status = format!("{} contains no audio", file_name(&path)),
                Err(e) => self.status = format!("Couldn't open {}: {e}", file_name(&path)),
            }
            return;
        }
        if as_track && !self.tracks.is_empty() {
            match result {
                Ok(a) if a.frames() > 0 => {
                    let a = effects::convert_rate(&a, self.rate());
                    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Track".into());
                    let mut clip = Clip::new(name.clone(), Arc::new(a), 0);
                    let m = audio::read_wav_markers(&path);
                    (clip.cues, clip.regions, clip.loop_points) = (m.cues, m.regions, m.loop_points);
                    let mut t = Track::new(name);
                    t.clips.push(clip);
                    self.add_track(t, "Import Track");
                    self.sel_clip = Some((self.active, 0));
                    self.status = format!("Imported {} as a new track", file_name(&path));
                }
                Ok(_) => self.status = format!("{} contains no audio", file_name(&path)),
                Err(e) => self.status = format!("Couldn't open {}: {e}", file_name(&path)),
            }
            return;
        }
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
                let markers = audio::read_wav_markers(&path);
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Track 1".into());
                let a = Arc::new(a);
                let mut clip = Clip::new(name.clone(), a.clone(), 0);
                (clip.cues, clip.regions, clip.loop_points) = (markers.cues, markers.regions, markers.loop_points);
                let mut t = Track::new(name);
                t.clips.push(clip);
                self.tracks = vec![t];
                self.active = 0;
                self.sel_clip = Some((0, 0));
                self.view = View::Arrangement;
                self.arr_cursor = 0;
                self.path = Some(path);
                self.audio = Some(a);
                self.peaks = Some(self.tracks[0].clips[0].peaks.clone());
                self.cues.clear();
                self.regions.clear();
                self.loop_points = None;
                self.refresh_playback();
                self.arr_fit();
            }
            Ok(_) => self.status = format!("{} contains no audio", file_name(&path)),
            Err(e) => self.status = format!("Couldn't open {}: {e}", file_name(&path)),
        }
    }

    fn set_audio(&mut self, a: Arc<Audio>) {
        // If the whole file was in view, keep it that way after the length changes.
        let was_full = self.view_start <= 0.0 && self.view_len >= self.frames() as f64;
        let peaks = Arc::new(Peaks::build(&a));
        self.peaks = Some(peaks.clone());
        // The editor works on one clip: keep that clip in step with every edit.
        if let View::Editor { track, clip } = self.view {
            if let Some(c) = self.tracks.get_mut(track).and_then(|t| t.clips.get_mut(clip)) {
                c.audio = a.clone();
                c.peaks = peaks;
                c.trim = 0;
                c.len = a.frames();
            }
        }
        let n = a.frames();
        self.cursor = self.cursor.min(n);
        self.sel = self.sel.map(|(s, e)| (s.min(n), e.min(n))).filter(|(s, e)| e > s);
        self.audio = Some(a);
        self.refresh_playback();
        if was_full {
            self.zoom_fit();
        }
        self.clamp_view();
    }

    /// The editor plays the clip on its own; the arrangement plays the mix of all tracks.
    fn refresh_playback(&mut self) {
        let play = match self.view {
            View::Editor { .. } => match self.audio.clone() {
                Some(a) => a,
                None => return,
            },
            View::Arrangement => Arc::new(self.mixdown()),
        };
        self.player.set_audio(play.clone());
        self.play_audio = Some(play);
    }

    /// The sound's sample rate (all clips share it).
    fn rate(&self) -> u32 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.first())
            .map(|c| c.audio.rate)
            .next()
            .or(self.audio.as_ref().map(|a| a.rate))
            .unwrap_or(44100)
    }

    /// One track rendered to audio: its clips placed at their positions.
    fn render_track(&self, t: &Track) -> Audio {
        let rate = self.rate();
        let channels = t.clips.iter().map(|c| c.audio.num_channels()).max().unwrap_or(1);
        let mut out = Audio { rate, channels: vec![Vec::new(); channels] };
        for c in &t.clips {
            out = effects::mix_into(&out, c.start, &c.visible(), 1.0);
        }
        out
    }

    /// Mix of all audible tracks at the level being shown (solo wins over mute).
    fn mixdown(&self) -> Audio {
        let mix = self.render_tracks(&self.tracks);
        if self.auto_gain { auto_gain(mix) } else { mix }
    }

    /// The whole song's mix, whichever group is open.
    fn song_mix(&self) -> Audio {
        let mix = self.render_tracks(&self.song_tracks());
        if self.auto_gain { auto_gain(mix) } else { mix }
    }

    /// The top-level tracks, with the open group(s) folded back in.
    fn song_tracks(&self) -> Vec<Track> {
        let mut cur = self.tracks.clone();
        for f in self.nest.iter().rev() {
            let mut parent = f.tracks.clone();
            if let Some(c) = parent.get_mut(f.track).and_then(|t| t.clips.get_mut(f.clip)) {
                self.set_group_content(c, cur);
            }
            cur = parent;
        }
        cur
    }

    /// Put new content into a group clip and re-render it, keeping its trim where possible.
    fn set_group_content(&self, c: &mut Clip, tracks: Vec<Track>) {
        let showed_all = c.trim == 0 && c.len >= c.audio.frames();
        let audio = Arc::new(self.render_tracks(&tracks));
        c.peaks = Arc::new(Peaks::build(&audio));
        c.len = if showed_all { audio.frames() } else { c.len.min(audio.frames().saturating_sub(c.trim)) };
        c.audio = audio;
        c.group = Some(Arc::new(tracks));
    }

    /// Mix of a set of tracks (solo wins over mute), with their volumes and pan.
    fn render_tracks(&self, tracks: &[Track]) -> Audio {
        let any_solo = tracks.iter().any(|t| t.solo);
        let rendered: Vec<(Audio, f32, f32)> = tracks
            .iter()
            .filter(|t| if any_solo { t.solo } else { !t.mute })
            .map(|t| (self.render_track(t), t.gain, t.pan))
            .collect();
        let inputs: Vec<(&Audio, f32, f32)> = rendered.iter().map(|(a, g, p)| (a, *g, *p)).collect();
        effects::mix_panned(&inputs, self.rate())
    }

    /// Extent of the timeline: the clip's length in the editor, or the end of the last clip
    /// in the arrangement.
    fn timeline_frames(&self) -> usize {
        match self.view {
            View::Editor { .. } => self.frames(),
            View::Arrangement => self.arrangement_frames(),
        }
    }

    fn arrangement_frames(&self) -> usize {
        self.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.end())).max().unwrap_or(0)
    }

    fn set_active(&mut self, i: usize) {
        if i < self.tracks.len() {
            self.active = i;
        }
    }

    fn add_track(&mut self, track: Track, label: &'static str) {
        self.push_undo(label);
        let at = (self.active + 1).min(self.tracks.len());
        self.tracks.insert(at, track);
        self.active = at;
        self.dirty = true;
        self.refresh_playback();
    }

    fn new_track(&mut self) {
        let n = (1..).find(|i| !self.tracks.iter().any(|t| t.name == format!("Track {i}"))).unwrap();
        self.add_track(Track::new(format!("Track {n}")), "New Track");
        self.status = "New empty track — paste, record or import clips into it".into();
    }

    fn duplicate_track(&mut self) {
        let Some(t) = self.tracks.get(self.active) else { return };
        let mut t = t.clone();
        t.name = format!("{} copy", t.name);
        t.solo = false;
        self.add_track(t, "Duplicate Track");
    }

    fn delete_track(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        self.push_undo("Delete Track");
        self.tracks.remove(self.active);
        self.active = self.active.min(self.tracks.len() - 1);
        self.sel_clip = None;
        self.dirty = true;
        self.refresh_playback();
    }

    fn mix_down(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        let mix = Arc::new(self.mixdown());
        self.push_undo("Mix Down");
        let mut t = Track::new("Mix");
        t.clips.push(Clip::new("Mix", mix, 0));
        self.tracks = vec![t];
        self.active = 0;
        self.sel_clip = Some((0, 0));
        self.dirty = true;
        self.refresh_playback();
        self.status = "Mixed all tracks into one — ⌘Z to undo".into();
    }

    // ---------- clips ----------

    fn clip(&self, (t, c): (usize, usize)) -> Option<&Clip> {
        self.tracks.get(t).and_then(|t| t.clips.get(c))
    }

    /// Open a clip in the waveform editor.
    fn edit_clip(&mut self, (t, c): (usize, usize)) {
        let Some(clip) = self.clip((t, c)).cloned() else { return };
        if clip.group.is_some() {
            return self.enter_group((t, c));
        }
        self.player.stop();
        // Editing works on what the clip shows: bake its trim in first.
        let visible = if clip.trim > 0 || clip.len < clip.audio.frames() { Arc::new(clip.visible()) } else { clip.audio.clone() };
        self.view = View::Editor { track: t, clip: c };
        self.active = t;
        self.sel_clip = Some((t, c));
        self.cues = clip.cues;
        self.regions = clip.regions;
        self.loop_points = clip.loop_points;
        self.sel = None;
        self.cursor = 0;
        self.view_start = 0.0;
        self.view_len = visible.frames().max(1) as f64;
        self.set_audio(visible);
        self.zoom_fit();
        self.status = format!("Editing {} — ⌘Return or ◀ Arrangement to go back", clip.name);
    }

    /// Store the editor's cues, regions and loop with the clip being edited.
    fn sync_clip_markers(&mut self) {
        if let View::Editor { track, clip } = self.view {
            if let Some(c) = self.tracks.get_mut(track).and_then(|t| t.clips.get_mut(clip)) {
                c.cues = self.cues.clone();
                c.regions = self.regions.clone();
                c.loop_points = self.loop_points;
            }
        }
    }

    fn back_to_arrangement(&mut self) {
        if self.view == View::Arrangement {
            return self.leave_group();
        }
        self.player.stop();
        self.sync_clip_markers();
        self.view = View::Arrangement;
        self.sel = None;
        self.refresh_playback();
        self.status = "Arrangement — double-click a clip to edit it".into();
    }

    /// Add a clip to the active track at the playhead, select it, and return its position.
    fn add_clip(&mut self, name: &str, audio: Arc<Audio>, label: &'static str) -> (usize, usize) {
        self.push_undo(label);
        if self.tracks.is_empty() {
            self.tracks.push(Track::new("Track 1"));
            self.active = 0;
        }
        let rate = self.rate();
        let audio = if audio.rate != rate { Arc::new(audio.resampled(rate)) } else { audio };
        let t = self.active.min(self.tracks.len() - 1);
        self.tracks[t].clips.push(Clip::new(name, audio, self.arr_cursor));
        let c = self.tracks[t].clips.len() - 1;
        self.sel_clip = Some((t, c));
        self.dirty = true;
        self.refresh_playback();
        (t, c)
    }

    fn new_clip(&mut self) {
        let rate = self.rate();
        let ch = self.tracks.iter().flat_map(|t| t.clips.first()).map(|c| c.audio.num_channels()).next().unwrap_or(2);
        let pos = self.add_clip("New clip", Arc::new(effects::silence(rate, ch, 2.0)), "New Clip");
        self.edit_clip(pos);
    }

    fn delete_clip(&mut self) {
        self.sel_extra.clear();
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some()) else { return };
        self.push_undo("Delete Clip");
        self.tracks[t].clips.remove(c);
        self.sel_clip = None;
        self.dirty = true;
        self.refresh_playback();
    }

    /// All selected clips: the main one plus any shift-clicked ones.
    fn selected_clips(&self) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = self.sel_clip.into_iter().chain(self.sel_extra.iter().copied()).filter(|&p| self.clip(p).is_some()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Open a group clip as its own arrangement.
    fn enter_group(&mut self, (t, c): (usize, usize)) {
        let Some(clip) = self.clip((t, c)).cloned() else { return };
        let Some(inner) = clip.group.clone() else { return };
        self.player.stop();
        self.nest.push(NestFrame {
            tracks: std::mem::replace(&mut self.tracks, (*inner).clone()),
            track: t,
            clip: c,
            active: self.active,
            view_start: self.arr_view_start,
            view_len: self.arr_view_len,
        });
        self.active = 0;
        self.sel_clip = None;
        self.sel_extra.clear();
        self.arr_cursor = 0;
        self.arr_fit();
        self.refresh_playback();
        self.status = format!("Inside {} — ⌘Return or the path above goes back up", clip.name);
    }

    /// Close the open group, folding its arrangement back into its clip.
    fn leave_group(&mut self) {
        let Some(f) = self.nest.pop() else { return };
        self.player.stop();
        let inner = std::mem::replace(&mut self.tracks, f.tracks);
        if let Some(mut c) = self.tracks.get(f.track).and_then(|t| t.clips.get(f.clip)).cloned() {
            self.set_group_content(&mut c, inner);
            self.tracks[f.track].clips[f.clip] = c;
        }
        self.active = f.active;
        self.sel_clip = Some((f.track, f.clip));
        self.sel_extra.clear();
        self.arr_view_start = f.view_start;
        self.arr_view_len = f.view_len;
        self.refresh_playback();
    }

    /// Group the selected clips into one clip that holds them, in the same places (⌘G).
    fn group_clips(&mut self) {
        let sel = self.selected_clips();
        if sel.is_empty() {
            return;
        }
        self.push_undo("Group Clips");
        let start = sel.iter().map(|&p| self.clip(p).unwrap().start).min().unwrap();
        let first_track = sel[0].0;
        // One inner track per source track, keeping their mixer settings.
        let mut inner: Vec<Track> = Vec::new();
        let mut track_of: Vec<(usize, usize)> = Vec::new();
        for &(t, c) in &sel {
            let i = match track_of.iter().find(|(src, _)| *src == t) {
                Some(&(_, i)) => i,
                None => {
                    let mut nt = self.tracks[t].clone();
                    nt.clips.clear();
                    nt.solo = false;
                    inner.push(nt);
                    track_of.push((t, inner.len() - 1));
                    inner.len() - 1
                }
            };
            let mut clip = self.tracks[t].clips[c].clone();
            clip.start -= start;
            inner[i].clips.push(clip);
        }
        // Remove the originals (highest index first so the others stay valid).
        let mut gone = sel.clone();
        gone.sort_by(|a, b| b.cmp(a));
        for (t, c) in gone {
            self.tracks[t].clips.remove(c);
        }
        let n = (1..).find(|i| !self.tracks.iter().flat_map(|t| &t.clips).any(|c| c.name == format!("Group {i}"))).unwrap();
        let mut g = Clip::new(format!("Group {n}"), Arc::new(Audio { rate: self.rate(), channels: vec![Vec::new()] }), start);
        self.set_group_content(&mut g, inner);
        g.trim = 0;
        g.len = g.audio.frames();
        self.tracks[first_track].clips.push(g);
        self.sel_clip = Some((first_track, self.tracks[first_track].clips.len() - 1));
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Grouped {} clip{} into Group {n} — double-click it to open it", sel.len(), if sel.len() == 1 { "" } else { "s" });
    }

    /// Put a group's clips back onto the tracks where it sits (⇧⌘G).
    fn ungroup_clip(&mut self) {
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some_and(|c| c.group.is_some())) else { return };
        self.push_undo("Ungroup");
        let g = self.tracks[t].clips.remove(c);
        let inner = g.group.clone().unwrap();
        for (i, it) in inner.iter().enumerate() {
            let dest = t + i;
            if dest >= self.tracks.len() {
                let mut nt = it.clone();
                nt.clips.clear();
                self.tracks.push(nt);
            }
            for clip in &it.clips {
                // Only what the group showed (its trim) comes back out.
                if clip.end() <= g.trim || clip.start >= g.trim + g.len {
                    continue;
                }
                let mut clip = clip.clone();
                if clip.start < g.trim {
                    let cut = g.trim - clip.start;
                    clip.trim += cut;
                    clip.len -= cut;
                    clip.start = g.trim;
                }
                clip.len = clip.len.min(g.trim + g.len - clip.start);
                clip.start = g.start + clip.start - g.trim;
                self.tracks[dest].clips.push(clip);
            }
        }
        self.sel_clip = None;
        self.sel_extra.clear();
        self.dirty = true;
        self.refresh_playback();
        self.status = format!("Ungrouped {}", g.name);
    }

    /// Duplicate the selected clip, placing the copy right after it (as in Ableton).
    fn duplicate_clip(&mut self) {
        self.sel_extra.clear();
        let Some((t, c)) = self.sel_clip.filter(|&p| self.clip(p).is_some()) else { return };
        self.push_undo("Duplicate Clip");
        let mut copy = self.tracks[t].clips[c].clone();
        copy.start = copy.end();
        self.tracks[t].clips.push(copy);
        self.sel_clip = Some((t, self.tracks[t].clips.len() - 1));
        self.dirty = true;
        self.refresh_playback();
    }

    /// Split the selected clip (or the clip under the playhead on the active track) at the playhead.
    fn split_clip(&mut self) {
        self.sel_extra.clear();
        let at = self.arr_cursor;
        let pos = self.sel_clip.filter(|&p| self.clip(p).is_some_and(|c| at > c.start && at < c.end())).or_else(|| {
            let t = self.active;
            self.tracks.get(t)?.clips.iter().position(|c| at > c.start && at < c.end()).map(|c| (t, c))
        });
        let Some((t, c)) = pos else {
            self.status = "Put the playhead inside a clip to split it".into();
            return;
        };
        self.push_undo("Split Clip");
        let left = &mut self.tracks[t].clips[c];
        let cut = at - left.start;
        let mut right = left.clone();
        left.len = cut;
        right.start = at;
        right.trim += cut;
        right.len -= cut;
        self.tracks[t].clips.push(right);
        self.sel_clip = Some((t, self.tracks[t].clips.len() - 1));
        self.dirty = true;
        self.refresh_playback();
    }

    fn export(&mut self, selection_only: bool) {
        if self.tracks.is_empty() {
            return;
        }
        self.sync_clip_markers();
        let in_editor = matches!(self.view, View::Editor { .. });
        let (a, suffix) = match (selection_only, in_editor, self.sel, self.sel_clip.and_then(|p| self.clip(p))) {
            // In the editor: the selection of the clip being edited.
            (true, true, Some((s, e)), _) => (Arc::new(self.audio.as_ref().unwrap().slice(s, e)), "-selection"),
            // In the arrangement: the selected clip.
            (true, false, _, Some(c)) => (Arc::new(c.visible()), "-clip"),
            _ => (Arc::new(self.song_mix()), "-edit"),
        };
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        let mut dialog = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .add_filter("AIFF audio", &["aif", "aiff"])
            .set_file_name(format!("{stem}{suffix}.wav"));
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut out) = dialog.save_file() else { return };
        if out.extension().is_none() {
            out.set_extension("wav");
        }
        // Markers live with each clip; place them on the timeline (or keep the clip's own).
        let (all_cues, all_regions, all_loop) = if suffix == "-edit" {
            let mut cues = Vec::new();
            let mut regions = Vec::new();
            let mut lp = None;
            for t in &self.song_tracks() {
                for c in &t.clips {
                    let place = |f: usize| (f >= c.trim && f < c.trim + c.len).then(|| c.start + f - c.trim);
                    cues.extend(c.cues.iter().filter_map(|q| place(q.frame).map(|frame| Cue { frame, name: q.name.clone() })));
                    regions.extend(c.regions.iter().filter_map(|r| {
                        Some(Region { start: place(r.start)?, end: place(r.end.saturating_sub(1))? + 1, name: r.name.clone() })
                    }));
                    lp = lp.or(c.loop_points.and_then(|(s, e)| Some((place(s)?, place(e.saturating_sub(1))? + 1))));
                }
            }
            (cues, regions, lp)
        } else if let (true, Some(c)) = (suffix == "-clip", self.sel_clip.and_then(|p| self.clip(p))) {
            (c.cues.clone(), c.regions.clone(), c.loop_points)
        } else {
            (self.cues.clone(), self.regions.clone(), self.loop_points)
        };
        let (lo, hi) = match (suffix, self.sel) {
            ("-selection", Some(r)) => r,
            _ => (0, usize::MAX),
        };
        let markers = Markers {
            cues: all_cues
                .iter()
                .filter(|c| c.frame >= lo && c.frame < hi)
                .map(|c| Cue { frame: c.frame - lo, name: c.name.clone() })
                .collect(),
            regions: all_regions
                .iter()
                .filter(|r| r.start >= lo && r.end <= hi)
                .map(|r| Region { start: r.start - lo, end: r.end - lo, name: r.name.clone() })
                .collect(),
            loop_points: all_loop.filter(|(s, e)| *s >= lo && *e <= hi).map(|(s, e)| (s - lo, e - lo)),
        };
        let is_wav = !matches!(out.extension().and_then(|e| e.to_str()), Some("aif" | "aiff" | "AIF" | "AIFF"));
        let saved = audio::save_any(&a, &out, self.wav_format).and_then(|_| if is_wav { audio::write_wav_markers(&out, &markers, a.rate) } else { Ok(()) });
        match saved {
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
        if self.tracks.is_empty() && self.audio.is_none() {
            return;
        }
        self.undo.push(Snapshot {
            audio: self.audio.clone(),
            tracks: self.tracks.clone(),
            nest: self.nest.clone(),
            active: self.active,
            sel_clip: self.sel_clip,
            cues: self.cues.clone(),
            regions: self.regions.clone(),
            loop_points: self.loop_points,
            sel: self.sel,
            cursor: self.cursor,
            label,
        });
        self.redo.clear();
        // Drop oldest steps beyond the memory budget (always keep at least one).
        let bytes = |s: &Snapshot| s.audio.as_ref().map_or(0, |a| a.bytes());
        let mut total: usize = self.undo.iter().map(bytes).sum();
        while total > UNDO_BUDGET_BYTES && self.undo.len() > 1 {
            total -= bytes(&self.undo.remove(0));
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
        let moved = |f: usize| -> Option<usize> {
            if f < at {
                Some(f)
            } else if f < at + removed {
                (inserted > 0).then(|| at + ((f - at) as f64 * inserted as f64 / removed as f64) as usize)
            } else {
                Some(f + inserted - removed)
            }
        };
        self.regions.retain_mut(|r| {
            let s = moved(r.start).unwrap_or(at);
            let e = moved(r.end).unwrap_or(at);
            r.start = s;
            r.end = e;
            e > s
        });
        self.loop_points = self.loop_points.and_then(|(s, e)| {
            let (s, e) = (moved(s).unwrap_or(at), moved(e).unwrap_or(at));
            (e > s).then_some((s, e))
        });
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

    /// After undo/redo (which stay in the current view): reload the clip being edited from the
    /// restored tracks, or refresh the arrangement.
    fn restore_audio(&mut self, a: Option<Arc<Audio>>) {
        match self.view {
            View::Editor { track, clip } => match self.clip((track, clip)).cloned() {
                Some(c) => {
                    let audio = if c.trim > 0 || c.len < c.audio.frames() { Arc::new(c.visible()) } else { c.audio };
                    self.set_audio(audio);
                }
                // The clip being edited no longer exists in the restored state.
                None => {
                    self.view = View::Arrangement;
                    self.refresh_playback();
                }
            },
            View::Arrangement => {
                if let Some(a) = a {
                    self.audio = Some(a);
                }
                self.refresh_playback();
            }
        }
    }

    fn undo(&mut self) {
        let Some(snap) = self.undo.pop() else { return };
        let cur = self.audio.clone();
        self.redo.push(Snapshot {
            audio: cur,
            tracks: std::mem::replace(&mut self.tracks, snap.tracks),
            nest: std::mem::replace(&mut self.nest, snap.nest),
            active: std::mem::replace(&mut self.active, snap.active),
            sel_clip: std::mem::replace(&mut self.sel_clip, snap.sel_clip),
            cues: std::mem::replace(&mut self.cues, snap.cues),
            regions: std::mem::replace(&mut self.regions, snap.regions),
            loop_points: std::mem::replace(&mut self.loop_points, snap.loop_points),
            sel: self.sel,
            cursor: self.cursor,
            label: snap.label,
        });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.restore_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Undid {}", snap.label);
    }

    fn redo(&mut self) {
        let Some(snap) = self.redo.pop() else { return };
        let cur = self.audio.clone();
        self.undo.push(Snapshot {
            audio: cur,
            tracks: std::mem::replace(&mut self.tracks, snap.tracks),
            nest: std::mem::replace(&mut self.nest, snap.nest),
            active: std::mem::replace(&mut self.active, snap.active),
            sel_clip: std::mem::replace(&mut self.sel_clip, snap.sel_clip),
            cues: std::mem::replace(&mut self.cues, snap.cues),
            regions: std::mem::replace(&mut self.regions, snap.regions),
            loop_points: std::mem::replace(&mut self.loop_points, snap.loop_points),
            sel: self.sel,
            cursor: self.cursor,
            label: snap.label,
        });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.restore_audio(snap.audio);
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
        self.status = format!("Added Cue {n} — rename it in the Cues panel (⌘3)");
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

    /// ⌘-drag snapping: to a cue point within 8 pixels, otherwise to the nearest zero crossing.
    fn snap(&self, f: usize, spp: f64) -> usize {
        let radius = (8.0 * spp).max(1.0) as usize;
        if let Some(c) = self.cues.iter().map(|c| c.frame).filter(|c| c.abs_diff(f) <= radius).min_by_key(|c| c.abs_diff(f)) {
            return c;
        }
        self.snap_zero(f, spp)
    }

    /// Nearest zero crossing (of all channels summed) within 8 pixels, or `f` if there is none.
    fn snap_zero(&self, f: usize, spp: f64) -> usize {
        let Some(a) = &self.audio else { return f };
        a.nearest_zero_crossing(f, ((8.0 * spp) as usize).clamp(32, 50_000)).unwrap_or(f)
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
        if self.view == View::Arrangement {
            if self.tracks.is_empty() {
                self.new_sound();
            }
            self.add_clip("Recording", Arc::new(clip), "Record");
            self.status = format!("Recorded {} as a new clip at the playhead", fmt_time(secs));
        } else {
            self.insert_clip("Record", clip);
            self.status = format!("Recorded {} — ⌘Z to undo", fmt_time(secs));
        }
    }

    /// Pad the start of the active track with silence (positive) or trim it (negative).
    fn offset_track(&mut self, secs: f32) {
        let Some(a) = self.audio.clone() else { return };
        let frames = (secs.abs() as f64 * a.rate as f64) as usize;
        if frames == 0 {
            return;
        }
        let new = if secs > 0.0 {
            a.insert(0, 0, &effects::silence(a.rate, a.num_channels(), secs))
        } else {
            a.delete(0, frames.min(a.frames()))
        };
        self.apply("Offset Track", new, None);
    }

    fn set_loop(&mut self) {
        if let Some(sel) = self.sel {
            self.push_undo("Set Loop");
            self.loop_points = Some(sel);
            self.dirty = true;
            self.status = "Loop set — turn on Loop (⌘L) and play to hear it repeat".into();
        }
    }

    fn add_region(&mut self) {
        let Some((s, e)) = self.sel else { return };
        let n = (1..).find(|i| !self.regions.iter().any(|r| r.name == format!("Region {i}"))).unwrap();
        self.push_undo("Region");
        self.regions.push(Region { start: s, end: e, name: format!("Region {n}") });
        self.regions.sort_by_key(|r| r.start);
        self.show_regions = true;
        self.dirty = true;
        self.status = format!("Added Region {n} — rename it in the Regions panel");
    }

    /// Save each region as its own file, named after the region.
    fn export_regions(&mut self) {
        if self.regions.is_empty() {
            return;
        }
        let Some(a) = self.audio.clone() else { return };
        let src = if self.tracks.len() > 1 { Arc::new(self.mixdown()) } else { a };
        let Some(dir) = rfd::FileDialog::new().set_title(format!("Choose a folder for {} regions", self.regions.len())).pick_folder() else { return };
        let mut saved = 0;
        for r in &self.regions {
            let clean: String = r.name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
            let path = dir.join(format!("{}.wav", clean.trim()));
            let e = r.end.min(src.frames());
            if r.start >= e {
                continue;
            }
            if let Err(err) = audio::save_wav(&src.slice(r.start, e), &path, self.wav_format) {
                self.status = format!("Couldn't save {}: {err}", path.display());
                return;
            }
            saved += 1;
        }
        self.status = format!("Saved {saved} regions to {}", dir.display());
    }

    /// A position (in frames) in the current ruler units.
    fn fmt_pos(&self, f: usize) -> String {
        fmt_pos_in(self.ruler, self.audio.as_ref().map_or(44100, |a| a.rate), f)
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
        self.regions.clear();
        self.loop_points = None;
        self.tracks = vec![Track::new("Track 1")];
        self.active = 0;
        self.sel_clip = None;
        self.view = View::Arrangement;
        self.arr_cursor = 0;
        self.audio = Some(Arc::new(Audio { rate: 44100, channels: vec![Vec::new(); 2] }));
        self.refresh_playback();
        self.arr_fit();
        self.zoom_fit();
        self.status = "New sound (44.1 kHz). Add a clip with Clip ▸ New Clip, Record, Paste or Import.".into();
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
        if self.view == View::Arrangement {
            if self.player.is_playing() {
                self.arr_cursor = self.player.position();
                self.player.stop();
                return;
            }
            let end = self.arrangement_frames();
            if self.arr_cursor >= end {
                self.arr_cursor = 0;
            }
            self.play_origin = self.arr_cursor;
            self.player.play(self.arr_cursor, end, false);
            return;
        }
        if self.audio.is_none() {
            return;
        }
        if self.player.is_playing() {
            // Pause: leave the cursor where playback got to.
            self.cursor = self.player.position().min(self.frames());
            self.player.stop();
            return;
        }
        let n = self.timeline_frames();
        if let (true, None, Some((ls, le))) = (self.looping, self.sel, self.loop_points) {
            // Loop points: play them round and round.
            self.play_origin = ls;
            self.player.play(ls, le.min(n), true);
            return;
        }
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
        if self.view == View::Arrangement {
            self.arr_cursor = self.play_origin;
            return;
        }
        self.cursor = self.sel.map_or(self.play_origin, |(s, _)| s);
    }

    // ---------- view ----------

    fn zoom_fit(&mut self) {
        self.view_start = 0.0;
        self.view_len = self.timeline_frames().max(1) as f64;
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
        let new_len = (self.view_len * factor).clamp(32.0, self.timeline_frames().max(32) as f64);
        let t = (anchor - self.view_start) / self.view_len;
        self.view_start = anchor - t * new_len;
        self.view_len = new_len;
        self.clamp_view();
    }

    fn clamp_view(&mut self) {
        let n = self.timeline_frames().max(1) as f64;
        self.view_len = self.view_len.clamp(32f64.min(n), n);
        self.view_start = self.view_start.clamp(0.0, (n - self.view_len).max(0.0));
    }

    // ---------- UI ----------

    fn enabled(&self, a: Action) -> bool {
        let doc = !self.tracks.is_empty();
        if self.view == View::Arrangement {
            let has_clip = self.sel_clip.and_then(|p| self.clip(p)).is_some();
            return match a {
                Action::New | Action::Open | Action::Quit | Action::BatchConvert | Action::Record | Action::NewClip | Action::ImportClip => true,
                Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Dither | Action::AutoGain => true,
                Action::Loop | Action::ShowTools | Action::ShowControls | Action::ShowInfo | Action::ShowCues | Action::ShowRegions => true,
                Action::ShowSpectrum | Action::RulerTime | Action::RulerSamples | Action::RulerTimecode => true,
                Action::Export => doc && self.arrangement_frames() > 0,
                Action::ExportSelection | Action::EditClip | Action::DuplicateClip | Action::Cut | Action::Copy | Action::Delete => has_clip,
                Action::SplitClip => doc,
                Action::GroupClips => has_clip,
                Action::Ungroup => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.group.is_some()),
                Action::SelectAll => doc,
                Action::BackToArrangement => !self.nest.is_empty(),
                Action::Paste => self.clipboard.is_some(),
                Action::Undo => !self.undo.is_empty(),
                Action::Redo => !self.redo.is_empty(),
                Action::PlayPause | Action::Stop | Action::GoStart | Action::GoEnd | Action::ZoomIn | Action::ZoomOut | Action::ZoomFit => doc,
                Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::MuteTrack | Action::SoloTrack => doc,
                Action::TrackColor | Action::OffsetTrack => doc,
                Action::DeleteTrack | Action::MixDown | Action::NextTrack | Action::PrevTrack | Action::MixDialog => self.tracks.len() > 1,
                _ => false,
            };
        }
        match a {
            Action::BackToArrangement => return true,
            Action::EditClip | Action::SplitClip | Action::DuplicateClip | Action::NewClip | Action::ImportClip => return false,
            Action::GroupClips | Action::Ungroup => return false,
            Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::DeleteTrack | Action::MixDown => return false,
            Action::MixDialog | Action::NextTrack | Action::PrevTrack | Action::OffsetTrack => return false,
            _ => {}
        }
        let has = self.audio.is_some();
        let sel = has && self.sel.is_some();
        let not_empty = self.frames() > 0;
        match a {
            Action::New | Action::Open | Action::Quit => true,
            Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Loop => true,
            Action::ShowCues | Action::Record => true,
            Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::MuteTrack | Action::SoloTrack => has,
            Action::DeleteTrack | Action::MixDown | Action::NextTrack | Action::PrevTrack | Action::MixDialog => has && self.tracks.len() > 1,
            Action::SetLoop | Action::AddRegion => sel,
            Action::ClearLoop => has && self.loop_points.is_some(),
            Action::ExportRegions => has && !self.regions.is_empty(),
            Action::RulerTime | Action::RulerSamples | Action::RulerTimecode => true,
            Action::ShowSpectrum | Action::BatchConvert | Action::ShowRegions => true,
            Action::FmSynth | Action::OffsetTrack | Action::TrackColor => has,
            Action::Dither | Action::AutoGain => true,
            Action::AddCue => has,
            Action::NextCue | Action::PrevCue => has && !self.cues.is_empty(),
            Action::ShowTools | Action::ShowControls | Action::ShowInfo => true,
            Action::ToolSelect | Action::ToolHand | Action::ToolZoom | Action::ToolPencil => true,
            Action::Undo => !self.undo.is_empty(),
            Action::Redo => !self.redo.is_empty(),
            Action::Paste | Action::MixPaste => has && self.clipboard.is_some(),
            Action::InsertSilence | Action::InsertTone | Action::InsertNoise | Action::InsertDtmf | Action::SelectAll => has,
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
            Action::MuteTrack => self.tracks.get(self.active).is_some_and(|t| t.mute),
            Action::RulerTime => self.ruler == RulerUnits::Time,
            Action::RulerSamples => self.ruler == RulerUnits::Samples,
            Action::RulerTimecode => self.ruler == RulerUnits::Timecode,
            Action::ShowSpectrum => self.show_spectrum,
            Action::ShowRegions => self.show_regions,
            Action::Dither => audio::DITHER.load(std::sync::atomic::Ordering::Relaxed),
            Action::AutoGain => self.auto_gain,
            Action::SoloTrack => self.tracks.get(self.active).is_some_and(|t| t.solo),
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
        if self.view == View::Arrangement {
            let mid = self.arr_view_start + self.arr_view_len / 2.0;
            match a {
                Action::Delete => return self.delete_clip(),
                Action::Copy | Action::Cut => {
                    if let Some(c) = self.sel_clip.and_then(|p| self.clip(p)) {
                        let clip = c.visible();
                        ctx.copy_text(format!("[Sound Edit clip, {}]", fmt_time(clip.duration_secs())));
                        self.status = format!("Copied clip {}", c.name);
                        self.clipboard = Some(clip);
                        if a == Action::Cut {
                            self.delete_clip();
                        }
                    }
                    return;
                }
                Action::Paste => {
                    if let Some(c) = self.clipboard.clone() {
                        self.add_clip("Pasted", Arc::new(c), "Paste Clip");
                    }
                    return;
                }
                Action::ZoomIn => return self.arr_zoom(0.5, mid),
                Action::ZoomOut => return self.arr_zoom(2.0, mid),
                Action::ZoomFit => return self.arr_fit(),
                Action::GoStart => {
                    self.arr_cursor = 0;
                    self.arr_view_start = 0.0;
                    return self.arr_clamp();
                }
                Action::GoEnd => {
                    self.arr_cursor = self.arrangement_frames();
                    return;
                }
                Action::SelectAll => {
                    let all: Vec<(usize, usize)> =
                        self.tracks.iter().enumerate().flat_map(|(t, tr)| (0..tr.clips.len()).map(move |c| (t, c))).collect();
                    self.sel_clip = all.first().copied();
                    self.sel_extra = all.into_iter().skip(1).collect();
                    return;
                }
                _ => {}
            }
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
            Action::InsertDtmf => self.dialog = Some(Dialog::InsertDtmf),
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
                self.player.set_looping(self.looping && (self.sel.is_some() || self.loop_points.is_some()));
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
            Action::EditClip => {
                if let Some(p) = self.sel_clip {
                    self.edit_clip(p);
                }
            }
            Action::BackToArrangement => self.back_to_arrangement(),
            Action::SplitClip => self.split_clip(),
            Action::DuplicateClip => self.duplicate_clip(),
            Action::NewClip => self.new_clip(),
            Action::GroupClips => self.group_clips(),
            Action::Ungroup => self.ungroup_clip(),
            Action::ImportClip => self.import_clip_dialog(ctx),
            Action::NewTrack => self.new_track(),
            Action::ImportTrack => self.import_track_dialog(ctx),
            Action::DuplicateTrack => self.duplicate_track(),
            Action::DeleteTrack => self.delete_track(),
            Action::MixDown => self.mix_down(),
            Action::MuteTrack => {
                let t = &mut self.tracks[self.active];
                t.mute = !t.mute;
                self.dirty = true;
                self.refresh_playback();
            }
            Action::SoloTrack => {
                let t = &mut self.tracks[self.active];
                t.solo = !t.solo;
                self.dirty = true;
                self.refresh_playback();
            }
            Action::NextTrack => self.set_active((self.active + 1) % self.tracks.len()),
            Action::Delay => self.dialog = Some(Dialog::Delay),
            Action::Emphasize => self.dialog = Some(Dialog::Emphasize),
            Action::Compressor => self.dialog = Some(Dialog::Compressor),
            Action::FadeDialog => self.dialog = Some(Dialog::Fade),
            Action::FmSynth => self.dialog = Some(Dialog::FmSynth),
            Action::OffsetTrack => self.dialog = Some(Dialog::OffsetTrack),
            Action::SetLoop => self.set_loop(),
            Action::ClearLoop => {
                self.push_undo("Clear Loop");
                self.loop_points = None;
                self.dirty = true;
            }
            Action::AddRegion => self.add_region(),
            Action::ExportRegions => self.export_regions(),
            Action::RulerTime => self.ruler = RulerUnits::Time,
            Action::RulerSamples => self.ruler = RulerUnits::Samples,
            Action::RulerTimecode => self.ruler = RulerUnits::Timecode,
            Action::ShowSpectrum => self.show_spectrum = !self.show_spectrum,
            Action::ShowRegions => self.show_regions = !self.show_regions,
            Action::Dither => {
                let on = !audio::DITHER.load(std::sync::atomic::Ordering::Relaxed);
                audio::DITHER.store(on, std::sync::atomic::Ordering::Relaxed);
            }
            Action::CutGaps => self.dialog = Some(Dialog::CutGaps),
            Action::AutoGain => {
                self.auto_gain = !self.auto_gain;
                self.refresh_playback();
            }
            Action::BatchConvert => self.show_batch = true,
            Action::MixDialog => self.mix_include = Some(vec![true; self.tracks.len()]),
            Action::TrackColor => {
                let t = &mut self.tracks[self.active];
                t.color = (t.color + 1) % TRACK_COLORS.len();
            }
            Action::PrevTrack => self.set_active((self.active + self.tracks.len() - 1) % self.tracks.len()),
            Action::NextCue => self.jump_cue(true),
            Action::PrevCue => self.jump_cue(false),
        }
    }

    /// Plain-key shortcuts. ⌘ shortcuts come through the native menu bar instead.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Space: a tap plays/pauses; holding it gives the Hand tool. Released before the text
        // field check so a held Space can never get stuck.
        let space_released = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Space, pressed: false, .. })));
        if space_released && self.space_down {
            self.space_down = false;
            if !self.space_used {
                self.run(Action::PlayPause, ctx);
            }
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) && !self.space_down {
            self.space_down = true;
            self.space_used = false;
        }
        let none = Modifiers::NONE;
        let keys: &[(Modifiers, Key, Action)] = &[
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
            (none, Key::Enter, Action::EditClip),
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
            INK
        } else {
            Color32::from_gray(185)
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

    /// Tools column on the left: editing tools on top, common commands below.
    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        egui::Grid::new("palette").spacing(Vec2::new(4.0, 4.0)).show(ui, |ui| {
            for row in actions::PALETTE {
                match row {
                    Some(pair) => {
                        for a in pair {
                            self.palette_button(ui, *a, Vec2::new(30.0, 28.0));
                        }
                    }
                    None => {
                        ui.add_space(4.0);
                    }
                }
                ui.end_row();
            }
        });
    }

    /// Update the playback level meters (they follow the playhead and fall back smoothly).
    fn update_meters(&mut self, ctx: &egui::Context) {
        let n_ch = self.play_audio.as_ref().map_or(2, |a| a.num_channels().max(1));
        self.meter.resize(n_ch, 0.0);
        let playing = self.player.is_playing();
        let pos = self.player.position();
        for (c, m) in self.meter.iter_mut().enumerate() {
            let now = match (&self.play_audio, playing) {
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
    }

    /// Transport bar along the bottom: buttons, time counter and level meters.
    fn transport_panel(&mut self, ui: &mut egui::Ui) {
        let playing = self.player.is_playing();
        let pos = self.player.position();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for a in [Action::GoStart, Action::PlayPause, Action::Stop, Action::Record, Action::Loop, Action::GoEnd] {
                self.palette_button(ui, a, Vec2::new(34.0, 28.0));
            }
            ui.add_space(12.0);
            let at = match (playing, self.view) {
                (true, _) => pos,
                (false, View::Arrangement) => self.arr_cursor,
                _ => self.cursor,
            };
            ui.label(egui::RichText::new(self.fmt_pos(at)).monospace().size(20.0).color(INK));
            if let (Some((s, e)), View::Editor { .. }) = (self.sel, self.view) {
                ui.label(egui::RichText::new(format!("sel {}", self.fmt_pos(e - s))).monospace().size(12.0).color(MUTED));
            }
            ui.add_space(12.0);
            let meters = self.meter.clone();
            let (r, _) = ui.allocate_exact_size(Vec2::new(180.0, 22.0), Sense::hover());
            let h = (r.height() - 2.0 * (meters.len().saturating_sub(1)) as f32) / meters.len().max(1) as f32;
            for (c, level) in meters.iter().enumerate() {
                let row = Rect::from_min_size(Pos2::new(r.left(), r.top() + c as f32 * (h + 2.0)), Vec2::new(r.width(), h));
                draw_meter(ui.painter(), row, *level);
            }
        });
    }

    /// Right-hand side panel with the Info, Cues and Regions sections.
    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let section = |ui: &mut egui::Ui, title: &str, first: bool| {
            if !first {
                ui.add_space(12.0);
                ui.separator();
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(title).size(11.0).color(MUTED).strong());
            ui.add_space(6.0);
        };
        let mut first = true;
        if self.show_info {
            section(ui, "INFO", first);
            first = false;
            self.info_section(ui);
        }
        if self.show_cues {
            section(ui, "CUES", first);
            first = false;
            self.cues_section(ui);
        }
        if self.show_regions {
            section(ui, "REGIONS", first);
            self.regions_section(ui);
        }
    }

    /// List of labelled regions: rename, select, delete; plus the loop points.
    fn regions_section(&mut self, ui: &mut egui::Ui) {
        if self.view == View::Arrangement {
            ui.label(egui::RichText::new("Regions and loops belong to clips. Double-click a clip to see and edit them.").color(MUTED));
            return;
        }
        let mut select: Option<Sel> = None;
        let mut remove: Option<usize> = None;
        let mut renamed = false;
        if self.regions.is_empty() {
            ui.label(egui::RichText::new("No regions yet. Select something and choose Insert ▸ Region from Selection (⇧⌘R).").color(MUTED));
        }
        let units = self.ruler;
        let rate = self.audio.as_ref().map_or(44100, |a| a.rate);
        egui::ScrollArea::vertical().id_salt("regions").max_height(200.0).show(ui, |ui| {
            egui::Grid::new("regions").num_columns(3).spacing(Vec2::new(6.0, 6.0)).show(ui, |ui| {
                for (i, r) in self.regions.iter_mut().enumerate() {
                    if ui.add_sized([150.0, 22.0], egui::TextEdit::singleline(&mut r.name)).changed() {
                        renamed = true;
                    }
                    if ui.small_button("Select").on_hover_text(format!("{} – {}", fmt_pos_in(units, rate, r.start), fmt_pos_in(units, rate, r.end))).clicked() {
                        select = Some((r.start, r.end));
                    }
                    if ui.small_button("×").on_hover_text("Remove this region").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.sel.is_some(), Button::new("Add from Selection")).clicked() {
                self.add_region();
            }
            if ui.add_enabled(!self.regions.is_empty(), Button::new("Export…")).on_hover_text("Save each region as its own file").clicked() {
                self.export_regions();
            }
        });
        ui.add_space(10.0);
        match self.loop_points {
            Some((s, e)) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Loop {} – {}", fmt_pos_in(units, rate, s), fmt_pos_in(units, rate, e))).color(MUTED));
                    if ui.small_button("Select").clicked() {
                        select = Some((s, e));
                    }
                    if ui.small_button("×").on_hover_text("Clear the loop").clicked() {
                        self.push_undo("Clear Loop");
                        self.loop_points = None;
                        self.dirty = true;
                    }
                });
            }
            None => {
                if ui.add_enabled(self.sel.is_some(), Button::new("Set Loop to Selection")).clicked() {
                    self.set_loop();
                }
            }
        }
        if renamed {
            self.dirty = true;
        }
        if let Some((s, e)) = select {
            self.sel = Some((s.min(self.frames()), e.min(self.frames()))).filter(|(s, e)| e > s);
            self.cursor = s;
        }
        if let Some(i) = remove {
            self.push_undo("Remove Region");
            self.regions.remove(i);
            self.dirty = true;
        }
    }

    fn info_section(&mut self, ui: &mut egui::Ui) {
        if self.view == View::Arrangement {
            let key = |ui: &mut egui::Ui, t: &str| {
                ui.label(egui::RichText::new(t).color(MUTED));
            };
            let clips: usize = self.tracks.iter().map(|t| t.clips.len()).sum();
            let length = self.arrangement_frames();
            let sel = self.sel_clip.and_then(|p| self.clip(p).map(|c| (p, c.clone())));
            egui::Grid::new("arrinfo").num_columns(2).min_col_width(78.0).spacing(Vec2::new(12.0, 6.0)).show(ui, |ui| {
                key(ui, "Sound");
                ui.label(self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into()));
                ui.end_row();
                key(ui, "Tracks");
                ui.label(format!("{} · {} clip{}", self.tracks.len(), clips, if clips == 1 { "" } else { "s" }));
                ui.end_row();
                key(ui, "Length");
                ui.label(self.fmt_pos(length));
                ui.end_row();
                key(ui, "Playhead");
                ui.label(self.fmt_pos(self.arr_cursor));
                ui.end_row();
                if let Some(((t, _), c)) = &sel {
                    key(ui, "Clip");
                    ui.label(format!("{} on {}", c.name, self.tracks[*t].name));
                    ui.end_row();
                    key(ui, "Starts");
                    ui.label(self.fmt_pos(c.start));
                    ui.end_row();
                    key(ui, "Clip length");
                    ui.label(self.fmt_pos(c.len));
                    ui.end_row();
                }
            });
            return;
        }
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
        let Some(a) = &self.audio else {
            ui.label("No sound open.");
            return;
        };
        let r = a.rate as f64;
        let key = |ui: &mut egui::Ui, t: &str| {
            ui.label(egui::RichText::new(t).color(MUTED));
        };
        egui::Grid::new("info").num_columns(2).min_col_width(78.0).spacing(Vec2::new(12.0, 6.0)).show(ui, |ui| {
            key(ui, "Sound");
            ui.label(self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into()));
            ui.end_row();
            if self.tracks.len() > 1 {
                key(ui, "Tracks");
                ui.label(format!("{} — editing {}", self.tracks.len(), self.tracks[self.active].name));
                ui.end_row();
            }
            key(ui, "Format");
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
            key(ui, "Length");
            ui.label(format!("{}  ({} samples)", fmt_time(a.duration_secs()), a.frames()));
            ui.end_row();
            // Selection times can be typed in (in the ruler's units) or dragged.
            let (units, rate, n) = (self.ruler, a.rate, a.frames() as f64);
            let time_field = |ui: &mut egui::Ui, v: &mut f64| {
                ui.add(
                    egui::DragValue::new(v)
                        .range(0.0..=n)
                        .speed(if units == RulerUnits::Samples { 10.0 } else { r * 0.002 })
                        .custom_formatter(move |x, _| fmt_pos_in(units, rate, x.max(0.0) as usize))
                        .custom_parser(move |t| parse_pos(units, rate, t)),
                )
                .on_hover_text("Type a time, or drag")
                .changed()
            };
            let (mut s0, mut e0) = self.sel.map_or((self.cursor as f64, self.cursor as f64), |(s, e)| (s as f64, e as f64));
            let mut len = e0 - s0;
            key(ui, "Start");
            let cs = time_field(ui, &mut s0);
            ui.end_row();
            key(ui, "End");
            let ce = time_field(ui, &mut e0);
            ui.end_row();
            key(ui, "Length");
            let cl = time_field(ui, &mut len);
            ui.end_row();
            if cs || ce || cl {
                let (s, e) = if cl { (s0, (s0 + len).min(n)) } else if ce { (s0.min(e0), e0.max(s0)) } else { (s0, e0.max(s0)) };
                let (s, e) = (s as usize, e as usize);
                if e > s {
                    self.sel = Some((s, e));
                } else {
                    self.sel = None;
                }
                self.cursor = s;
            }
            if let Some(p) = peak {
                key(ui, "Peak");
                ui.label(if p > 0.0 { format!("{:.1} dB", 20.0 * p.log10()) } else { "silent".into() });
                ui.end_row();
            }
            key(ui, "Export as");
            ui.label(self.wav_format.label());
            ui.end_row();
        });
    }

    /// List of cue points: rename, jump to, delete.
    fn cues_section(&mut self, ui: &mut egui::Ui) {
        if self.view == View::Arrangement {
            ui.label(egui::RichText::new("Cues belong to clips. Double-click a clip to see and edit its cues.").color(MUTED));
            return;
        }
        let rate = self.audio.as_ref().map_or(44100, |a| a.rate) as f64;
        let mut go: Option<usize> = None;
        let mut remove: Option<usize> = None;
        let mut renamed = false;
        if self.cues.is_empty() {
            ui.label(egui::RichText::new("No cue points yet. Insert ▸ Cue Point (⌘M) adds one at the cursor.").color(MUTED));
        }
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            egui::Grid::new("cues").num_columns(4).spacing(Vec2::new(6.0, 6.0)).show(ui, |ui| {
                for (i, c) in self.cues.iter_mut().enumerate() {
                    if ui.add_sized([118.0, 22.0], egui::TextEdit::singleline(&mut c.name)).changed() {
                        renamed = true;
                    }
                    ui.label(egui::RichText::new(fmt_time(c.frame as f64 / rate)).monospace().color(MUTED));
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
        ui.add_space(8.0);
        let add = ui.add_enabled(self.audio.is_some(), Button::new("Add at Cursor")).clicked();
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
    fn spectrum_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_spectrum;
        let src = self.play_audio.clone().or_else(|| self.audio.clone());
        let playing = self.player.is_playing();
        let pos = if playing { self.player.position() } else { self.cursor };
        let mut three_d = self.spectrum_3d;
        let sel = self.sel;
        let cache = &mut self.spectrum_cache;
        let center = self.desk.center();
        egui::Window::new("Spectrum")
            .open(&mut open)
            .default_pos(center - Vec2::new(280.0, 150.0))
            .default_size(Vec2::new(560.0, 300.0))
            .resizable(true)
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
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

    /// File ▸ Batch Convert: convert many files to WAV or AIFF in one go.
    fn batch_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_batch;
        let b = &mut self.batch;
        let running = b.progress.as_ref().is_some_and(|p| !p.lock().unwrap().finished);
        let center = self.desk.center();
        egui::Window::new("Batch Convert")
            .open(&mut open)
            .default_pos(center - Vec2::new(210.0, 180.0))
            .default_width(420.0)
            .resizable(false)
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(!running, Button::new("Add Files…")).clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf"])
                            .pick_files()
                        {
                            for f in files {
                                if !b.files.contains(&f) {
                                    b.files.push(f);
                                }
                            }
                        }
                    }
                    if ui.add_enabled(!running && !b.files.is_empty(), Button::new("Clear")).clicked() {
                        b.files.clear();
                    }
                    ui.label(egui::RichText::new(format!("{} file{}", b.files.len(), if b.files.len() == 1 { "" } else { "s" })).color(MUTED));
                });
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for f in &b.files {
                        ui.label(egui::RichText::new(file_name(f)).size(12.0));
                    }
                });
                ui.add_space(6.0);
                dialog_grid("batch").show(ui, |ui| {
                    ui.label("Save to");
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!running, Button::new("Choose…")).clicked() {
                            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                                b.out_dir = Some(d);
                            }
                        }
                        ui.label(egui::RichText::new(b.out_dir.as_ref().map_or("(pick a folder)".into(), |d| d.display().to_string())).color(MUTED));
                    });
                    ui.end_row();
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("bfmt").selected_text(BATCH_FORMATS[b.format].0).show_ui(ui, |ui| {
                        for (i, f) in BATCH_FORMATS.iter().enumerate() {
                            ui.selectable_value(&mut b.format, i, f.0);
                        }
                    });
                    ui.end_row();
                    ui.label("Sample rate");
                    let rates = [0u32, 8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000];
                    let label = |r: u32| if r == 0 { "Keep".to_string() } else { format!("{} Hz", r) };
                    egui::ComboBox::from_id_salt("brate").selected_text(label(b.rate)).show_ui(ui, |ui| {
                        for r in rates {
                            ui.selectable_value(&mut b.rate, r, label(r));
                        }
                    });
                    ui.end_row();
                    ui.label("Channels");
                    let chl = |c: usize| ["Keep", "Mono", "Stereo"][c.min(2)];
                    egui::ComboBox::from_id_salt("bch").selected_text(chl(b.channels)).show_ui(ui, |ui| {
                        for c in 0..3 {
                            ui.selectable_value(&mut b.channels, c, chl(c));
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut b.normalize, "Normalize to -1 dB");
                    ui.end_row();
                });
                ui.add_space(14.0);
                let can_go = !running && !b.files.is_empty() && b.out_dir.is_some();
                let mut go = false;
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        go = ui.add_enabled(can_go, primary_button(&format!("Convert {} files", b.files.len()))).clicked();
                    });
                });
                if go {
                    let progress = Arc::new(std::sync::Mutex::new(BatchProgress { total: b.files.len(), ..Default::default() }));
                    b.progress = Some(progress.clone());
                    let (files, dir, (_, ext, fmt), rate, ch, norm) =
                        (b.files.clone(), b.out_dir.clone().unwrap(), BATCH_FORMATS[b.format], b.rate, b.channels, b.normalize);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        for f in files {
                            let result = audio::load(&f).and_then(|mut a| {
                                if ch > 0 {
                                    a = effects::convert_channels(&a, ch);
                                }
                                if rate > 0 {
                                    a = effects::convert_rate(&a, rate);
                                }
                                if norm {
                                    if let Some(x) = a.normalize(0, a.frames(), -1.0) {
                                        a = x;
                                    }
                                }
                                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "sound".into());
                                audio::save_any(&a, &dir.join(format!("{stem}.{ext}")), fmt)
                            });
                            let mut p = progress.lock().unwrap();
                            p.done += 1;
                            if let Err(e) = result {
                                p.errors.push(format!("{}: {e}", file_name(&f)));
                            }
                            ctx.request_repaint();
                        }
                        progress.lock().unwrap().finished = true;
                        ctx.request_repaint();
                    });
                }
                if let Some(p) = &b.progress {
                    let p = p.lock().unwrap();
                    ui.add(egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32).text(format!("{} of {}", p.done, p.total)));
                    if p.finished {
                        ui.label(if p.errors.is_empty() { "Done.".to_string() } else { format!("Done, with {} problem(s):", p.errors.len()) });
                    }
                    for e in &p.errors {
                        ui.label(egui::RichText::new(e).size(11.0).color(MUTED));
                    }
                }
            });
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        self.show_batch = open;
    }

    /// Tracks ▸ Mix: choose tracks and their levels and pan, then mix into a new track or
    /// replace all tracks with the mix.
    fn mix_dialog(&mut self, ctx: &egui::Context) {
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
        self.place("Record", Pos2::new(self.desk.center().x - 135.0, self.desk.top() + 40.0))
            .open(&mut open)
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(270.0);
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

    fn dialog_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.dialog else { return };
        let scope = if self.sel.is_some() { "the selection" } else { "the whole sound" };
        let mut apply = false;
        let mut cancel = false;
        let resp = dialog_modal("dialog").show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            ui.spacing_mut().interact_size.x = 96.0;
            dialog_header(ui, kind.title(), &kind.blurb(scope));
            dialog_grid("fields").show(ui, |ui| {
                for f in kind.fields().iter().filter(|_| kind != Dialog::Equalizer) {
                    ui.label(f.label);
                    let v = self.params.entry(f.key).or_insert(f.default);
                    if !f.choices.is_empty() {
                        let current = f.choices.iter().find(|c| c.1 == *v).map_or("?", |c| c.0);
                        egui::ComboBox::from_id_salt(f.key).width(140.0).selected_text(current).show_ui(ui, |ui| {
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
                    ui.add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut self.dtmf_digits).hint_text("e.g. 555-0123"))
                        .on_hover_text("0–9, *, #, A–D. A comma adds a pause; other characters are ignored.");
                    ui.end_row();
                }
                if kind == Dialog::InsertTone {
                    ui.label("Waveform");
                    egui::ComboBox::from_id_salt("wave").width(140.0).selected_text(self.tone_wave.label()).show_ui(ui, |ui| {
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
                            ui.label(egui::RichText::new(format!("{:+.1} dB", *v)).monospace().size(11.0).color(INK));
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if self.view == View::Arrangement {
                if !self.tracks.is_empty() {
                    let clips: usize = self.tracks.iter().map(|t| t.clips.len()).sum();
                    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
                    ui.label(format!("{} Hz · {} · {} · {}", self.rate(), plural(self.tracks.len(), "track"), plural(clips, "clip"), self.fmt_pos(self.arrangement_frames())));
                    ui.separator();
                    ui.label(format!("Playhead {}", self.fmt_pos(self.arr_cursor)));
                    ui.separator();
                }
            } else if let Some(a) = &self.audio {
                if self.tracks.len() > 1 {
                    ui.label(format!("{} ·", self.tracks[self.active].name));
                }
                ui.label(format!("{} Hz · {} ch · {}", a.rate, a.num_channels(), fmt_time(a.duration_secs())));
                ui.separator();
                match self.sel {
                    Some((s, e)) => ui.label(format!("Sel {} – {} ({})", self.fmt_pos(s), self.fmt_pos(e), self.fmt_pos(e - s))),
                    None => ui.label(format!("Cursor {}", self.fmt_pos(self.cursor))),
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
        let Some(_) = self.audio.as_ref() else {
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

        // With several tracks, a header column on the left holds each track's controls.
        // The editor shows the one clip being edited.
        let n_tracks = 1;
        let header_w = if n_tracks > 1 { 150.0 } else { 0.0 };
        let total = self.timeline_frames();
        let ruler = Rect::from_min_max(Pos2::new(full.left() + header_w, full.top()), Pos2::new(full.right(), full.top() + RULER_H));
        let scroll = Rect::from_min_max(Pos2::new(full.left() + header_w, full.bottom() - 15.0), full.max);
        let wave = Rect::from_min_max(Pos2::new(full.left() + header_w, ruler.bottom()), Pos2::new(full.right(), scroll.top()));
        let header = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.left() + header_w, scroll.top()));
        let track_h = wave.height() / n_tracks as f32;
        let track_lane = move |i: usize| Rect::from_min_size(Pos2::new(wave.left(), wave.top() + i as f32 * track_h), Vec2::new(wave.width(), track_h));
        let alane = track_lane(0);
        let lane_h = alane.height() / n_ch as f32;
        let spp = self.view_len / wave.width() as f64; // frames per pixel

        // --- interaction on the waveform ---
        let resp = ui.allocate_rect(Rect::from_min_max(ruler.min, wave.max), Sense::click_and_drag());
        // Pressing in another track's lane makes it the track being edited.
        if n_tracks > 1 && (resp.drag_started() || resp.clicked()) {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                if p.y > wave.top() {
                    let ti = (((p.y - wave.top()) / track_h) as usize).min(n_tracks - 1);
                    if ti != self.active {
                        self.set_active(ti);
                    }
                }
            }
        }
        // The active track may have changed above.
        let n = self.frames();
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
            let alt_scroll = ui.input(|i| i.modifiers.alt) && scroll_delta.y != 0.0;
            if alt_scroll {
                // ⌥-scroll zooms around the mouse.
                let anchor = hover.map_or(self.view_start + self.view_len / 2.0, |p| {
                    self.view_start + ((p.x - wave.left()) / wave.width()) as f64 * self.view_len
                });
                self.zoom((-scroll_delta.y as f64 / 150.0).exp(), anchor);
            }
            let d = if scroll_delta.x.abs() > scroll_delta.y.abs() { scroll_delta.x } else { scroll_delta.y };
            if d != 0.0 && !alt_scroll {
                self.view_start -= d as f64 / wave.width() as f64 * self.view_len;
                self.clamp_view();
            }
        }

        let (shift, alt, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        // Where the button went down: egui only reports a drag after the pointer has moved a
        // few pixels, and anchoring there would shift the start of the drag.
        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        // Holding Space switches to the Hand tool for as long as it is held.
        let tool = if self.space_down { Tool::Hand } else { self.tool };
        match tool {
            Tool::Select => {
                // Cue flags in the ruler can be dragged along (⌥-drag duplicates), or clicked to jump there.
                let flag_at = |q: Pos2, cues: &[Cue], vs: f64, vl: f64| {
                    (q.y <= ruler.top() + 14.0).then(|| {
                        cues.iter().position(|c| (wave.left() + ((c.frame as f64 - vs) / vl) as f32 * wave.width() - q.x).abs() < 6.0)
                    })?
                };
                let hover_flag = resp.hover_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len));
                if hover_flag.is_some() || self.cue_drag.is_some() {
                    ui.ctx().set_cursor_icon(if alt && self.cue_drag.is_none() { egui::CursorIcon::Copy } else { egui::CursorIcon::ResizeHorizontal });
                } else if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
                let clicked_flag = if resp.clicked() { resp.interact_pointer_pos().and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)) } else { None };
                // A click on a region's bar in the ruler selects that region.
                let clicked_region = if resp.clicked() && clicked_flag.is_none() {
                    resp.interact_pointer_pos().filter(|q| q.y >= ruler.top() + 15.0 && q.y <= ruler.top() + 23.0).and_then(|q| {
                        let f = x_to_frame(q.x, self.view_start, self.view_len);
                        self.regions.iter().find(|r| f >= r.start && f < r.end).map(|r| (r.start, r.end))
                    })
                } else {
                    None
                };
                if resp.drag_started() {
                    if let Some(i) = press.and_then(|q| flag_at(q, &self.cues, self.view_start, self.view_len)) {
                        if alt {
                            self.push_undo("Duplicate Cue");
                            let copy = Cue { frame: self.cues[i].frame, name: format!("{} copy", self.cues[i].name) };
                            self.cues.push(copy);
                            self.cue_drag = Some(self.cues.len() - 1);
                        } else {
                            self.push_undo("Move Cue");
                            self.cue_drag = Some(i);
                        }
                    }
                }
                if let Some(i) = self.cue_drag {
                    if let Some(q) = resp.interact_pointer_pos() {
                        let mut f = x_to_frame(q.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                        if command {
                            f = self.snap_zero(f, spp);
                        }
                        if let Some(c) = self.cues.get_mut(i) {
                            c.frame = f;
                        }
                    }
                    if resp.drag_stopped() {
                        self.cue_drag = None;
                        self.cues.sort_by_key(|c| c.frame);
                        self.dirty = true;
                    }
                } else if let Some(i) = clicked_flag {
                    let f = self.cues[i].frame;
                    self.go_to(f);
                } else if let Some((rs, re)) = clicked_region {
                    self.sel = Some((rs.min(n), re.min(n))).filter(|(s, e)| e > s);
                    self.cursor = rs.min(n);
                } else {
                    if resp.drag_started() {
                        if let Some(p) = press {
                            let f = x_to_frame(p.x, self.view_start, self.view_len);
                            match (alt && !shift, self.sel) {
                                // ⌥-drag moves the nearest selection edge at a tenth of the speed.
                                (true, Some((s, e))) => {
                                    let is_end = f.abs_diff(e) < f.abs_diff(s);
                                    self.fine_edge = Some((is_end, if is_end { e } else { s } as f64));
                                    self.status = "Fine adjust: the edge moves at 1/10 speed".into();
                                }
                                _ => {
                                    let f = if command { self.snap(f, spp) } else { f };
                                    self.drag_anchor = Some(match (shift, self.sel) {
                                        // Shift-drag extends the existing selection from its far end.
                                        (true, Some((s, e))) => {
                                            if f.abs_diff(s) < f.abs_diff(e) { e } else { s }
                                        }
                                        (true, None) => self.cursor,
                                        _ => f,
                                    });
                                }
                            }
                        }
                    }
                    if resp.dragged() {
                        if let (Some((is_end, pos)), Some((s, e))) = (self.fine_edge, self.sel) {
                            let pos = (pos + resp.drag_delta().x as f64 * spp * 0.1).clamp(0.0, n as f64);
                            self.fine_edge = Some((is_end, pos));
                            let other = if is_end { s } else { e };
                            let f = pos.round() as usize;
                            if f != other {
                                self.sel = Some((f.min(other), f.max(other)));
                                self.cursor = f.min(other);
                            }
                        } else if let (Some(anchor), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos()) {
                            // Auto-scroll when dragging past the edges.
                            if p.x < wave.left() {
                                self.view_start -= self.view_len * 0.02;
                                self.clamp_view();
                            } else if p.x > wave.right() {
                                self.view_start += self.view_len * 0.02;
                                self.clamp_view();
                            }
                            let mut f = x_to_frame(p.x.clamp(wave.left(), wave.right()), self.view_start, self.view_len);
                            if command {
                                f = self.snap(f, spp);
                            }
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
                        self.fine_edge = None;
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
                            let f = if command { self.snap(f, spp) } else { f };
                            if shift {
                                let (a, b) = match self.sel {
                                    Some((s, e)) => {
                                        if f.abs_diff(s) < f.abs_diff(e) { (f, e) } else { (s, f) }
                                    }
                                    None => (self.cursor, f),
                                };
                                if a.max(b) > a.min(b) {
                                    self.sel = Some((a.min(b), a.max(b)));
                                }
                            } else {
                                self.sel = None;
                                self.cursor = f;
                                self.play_origin = f;
                                // ⌥-click plays from the mouse; while playing, any click jumps there.
                                if alt || self.player.is_playing() {
                                    self.player.play(f, n, false);
                                }
                            }
                        }
                    }
                    if resp.double_clicked() && n > 0 {
                        // Double-click selects between the surrounding cues; again selects everything.
                        let f = resp.interact_pointer_pos().map_or(0, |p| x_to_frame(p.x, self.view_start, self.view_len));
                        let a = self.cues.iter().map(|c| c.frame).filter(|&c| c <= f).max().unwrap_or(0);
                        let b = self.cues.iter().map(|c| c.frame).filter(|&c| c > f).min().unwrap_or(n);
                        let region = (a, b);
                        self.sel = Some(if self.last_double == Some(region) && (a, b) != (0, n) { (0, n) } else { region });
                        self.last_double = self.sel.filter(|r| *r == region);
                        self.cursor = self.sel.map_or(f, |(s, _)| s);
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
                    if self.space_down {
                        self.space_used = true;
                    }
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
                    let mid = alane.top() + ch as f32 * lane_h + lane_h / 2.0;
                    ((mid - y) / (lane_h / 2.0 * 0.92)).clamp(-1.0, 1.0)
                };
                if (resp.drag_started() || resp.clicked()) && self.pencil.is_none() {
                    if spp > PENCIL_MAX_SPP || n == 0 {
                        self.status = "Zoom in closer to draw with the pencil".into();
                    } else if let Some(p) = press {
                        let mut channel = (((p.y - alane.top()).max(0.0) / lane_h) as usize).min(n_ch - 1);
                        // Shift-click draws a straight line on from the end of the last stroke.
                        let mut last = None;
                        if let (true, Some((ch, f, v))) = (shift, self.pencil_last) {
                            channel = ch.min(n_ch - 1);
                            last = Some((f.min(n - 1), v));
                        }
                        self.push_undo("Pencil");
                        self.pencil = Some(PencilStroke { channel, last });
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
                if (resp.drag_stopped() || resp.clicked()) && self.pencil.is_some() {
                    let stroke = self.pencil.take().unwrap();
                    self.pencil_last = stroke.last.map(|(f, v)| (stroke.channel, f, v));
                    let a = self.audio.clone().unwrap();
                    self.set_audio(a);
                    self.dirty = true;
                    self.status = "Pencil — ⌘Z to undo".into();
                }
            }
        }

        // --- scrollbar ---
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let thumb_w = (self.view_len / total.max(1) as f64 * scroll.width() as f64).max(16.0).min(scroll.width() as f64) as f32;
        let track = scroll.width() - thumb_w;
        let max_start = (total as f64 - self.view_len).max(0.0);
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

        // --- painting: dark waveform on white, selection as a grey band ---
        let audio = self.audio.clone().unwrap();
        let painter = ui.painter_at(full);
        let vs = self.view_start;
        let vl = self.view_len;
        let frame_to_x = |f: f64| wave.left() + ((f - vs) / vl) as f32 * wave.width();
        let black = Color32::BLACK;

        painter.rect_filled(wave, 0.0, Color32::WHITE);
        painter.rect_filled(ruler, 0.0, PANEL);
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, LINE));
        if let Some((s, e)) = self.sel {
            let (x0, x1) = (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right()));
            if x1 > x0 {
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, ruler.bottom() - 1.0)), 0.0, Color32::from_gray(222));
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
                painter.line_segment([Pos2::new(x, ruler.bottom() - h), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, MUTED));
                if major {
                    painter.line_segment([Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, Color32::from_gray(242)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + 1.0), Align2::LEFT_TOP, ruler_label(self.ruler, rate as u32, t, step), FontId::proportional(11.0), MUTED);
                }
            }
            k += 1;
            t = k as f64 * minor;
        }

        // Selection: a grey band behind the active track's waveform.
        let sel_px = self.sel.map(|(s, e)| (frame_to_x(s as f64).max(wave.left()), frame_to_x(e as f64).min(wave.right())));

        // Waveform, split into one strip per channel.
        for ti in 0..n_tracks {
            let is_active = true;
            let (audio, peaks) = (audio.clone(), self.peaks.clone().unwrap());
            let lane = track_lane(ti);
            let tn = audio.frames();
            let muted = false;
            if n_tracks > 1 {
                if !is_active {
                    painter.rect_filled(lane, 0.0, Color32::from_gray(250));
                }
                if ti > 0 {
                    painter.line_segment([lane.left_top(), lane.right_top()], Stroke::new(1.0, Color32::from_gray(190)));
                }
            }
            // Selection band (only on the track being edited).
            if let (true, Some((x0, x1))) = (is_active, sel_px) {
                if x1 > x0 {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x0, lane.top()), Pos2::new(x1, lane.bottom())), 0.0, Color32::from_gray(222));
                }
            }
            let tc = TRACK_COLORS[self.tracks.get(self.active).map_or(0, |t| t.color) % TRACK_COLORS.len()];
            let base = if muted {
                Color32::from_gray(190)
            } else if is_active || n_tracks == 1 {
                tc
            } else {
                tc.lerp_to_gamma(Color32::WHITE, 0.35)
            };
            let tch = audio.num_channels().max(1);
            let lane_h = lane.height() / tch as f32;
            for (ci, ch) in audio.channels.iter().enumerate() {
                let top = lane.top() + ci as f32 * lane_h;
                let mid = top + lane_h / 2.0;
                let amp = lane_h / 2.0 * 0.92;
                let in_sel = |f: usize| is_active && matches!(self.sel, Some((s, e)) if f >= s && f < e);
                let color_at = |f: usize| if in_sel(f) && !muted { base.lerp_to_gamma(black, 0.6) } else { base };
                painter.line_segment([Pos2::new(wave.left(), mid), Pos2::new(wave.right(), mid)], Stroke::new(1.0, Color32::from_gray(212)));
                if ci > 0 {
                    painter.line_segment([Pos2::new(wave.left(), top), Pos2::new(wave.right(), top)], Stroke::new(1.0, Color32::from_gray(232)));
                }
                if spp <= 1.5 {
                    // Zoomed in: connect individual samples.
                    let first = vs.floor().max(0.0) as usize;
                    let last = ((vs + vl).ceil() as usize + 1).min(tn);
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
                        let f1 = ((vs + (px + 1) as f64 * spp) as usize).min(tn);
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
        }
        let _ = header;

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
            let col = if in_sel { black } else { Color32::from_gray(110) };
            painter.add(egui::Shape::dashed_line(&[Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, col), 4.0, 3.0));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x, ruler.top() + 2.0), Pos2::new(x + 7.0, ruler.top() + 6.0), Pos2::new(x, ruler.top() + 10.0)],
                Color32::from_gray(60),
                Stroke::NONE,
            ));
            painter.line_segment([Pos2::new(x, ruler.top() + 2.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, Color32::from_gray(60)));
            let label = painter.layout_no_wrap(c.name.clone(), FontId::proportional(10.0), INK);
            let bg = Rect::from_min_size(Pos2::new(x + 8.0, ruler.top() + 1.0), label.size() + Vec2::new(4.0, 0.0));
            painter.rect_filled(bg, 2.0, Color32::WHITE);
            painter.galley(bg.min + Vec2::new(2.0, 0.0), label, INK);
        }

        // Regions: a band across the middle of the ruler with their names.
        let band = Rect::from_min_max(Pos2::new(ruler.left(), ruler.top() + 15.0), Pos2::new(ruler.right(), ruler.top() + 23.0));
        for r in &self.regions {
            let (x0, x1) = (frame_to_x(r.start as f64).max(wave.left()), frame_to_x(r.end as f64).min(wave.right()));
            if x1 <= x0 {
                continue;
            }
            let rr = Rect::from_min_max(Pos2::new(x0, band.top()), Pos2::new(x1, band.bottom()));
            painter.rect_filled(rr, 2.0, Color32::from_gray(150));
            let label = painter.layout_no_wrap(r.name.clone(), FontId::proportional(9.0), Color32::WHITE);
            if label.size().x + 6.0 < rr.width() {
                painter.galley(Pos2::new(rr.left() + 3.0, rr.center().y - label.size().y / 2.0), label, Color32::WHITE);
            }
        }
        // Loop points: a black bar with end ticks along the bottom of the ruler.
        if let Some((ls, le)) = self.loop_points {
            let (x0, x1) = (frame_to_x(ls as f64), frame_to_x(le as f64));
            let y = ruler.bottom() - 4.0;
            let (c0, c1) = (x0.max(wave.left()), x1.min(wave.right()));
            if c1 > c0 {
                painter.line_segment([Pos2::new(c0, y), Pos2::new(c1, y)], Stroke::new(2.5, black));
            }
            for x in [x0, x1] {
                if x >= wave.left() && x <= wave.right() {
                    painter.line_segment([Pos2::new(x, y - 5.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.5, black));
                }
            }
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
                painter.line_segment([Pos2::new(px, ruler.top()), Pos2::new(px, wave.bottom())], Stroke::new(1.5, black));
            }
        }

        // Scroll bar: light track with a grey thumb.
        painter.rect_filled(scroll, 0.0, PANEL);
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, LINE));
        let thumb_col = if self.scrollbar_drag { Color32::from_gray(120) } else { Color32::from_gray(185) };
        painter.rect_filled(thumb.shrink2(Vec2::new(1.0, 4.0)), 4.0, thumb_col);

        // Hover time readout.
        if let Some(p) = resp.hover_pos() {
            let f = x_to_frame(p.x, vs, vl);
            painter.text(Pos2::new(wave.right() - 6.0, ruler.top() + 1.0), Align2::RIGHT_TOP, self.fmt_pos(f), FontId::monospace(11.0), MUTED);
        }
    }

    /// Per-track controls beside each lane: name, mute, solo and volume.
    fn track_headers(&mut self, ui: &mut egui::Ui, header: Rect, n_tracks: usize, lane: &dyn Fn(usize) -> Rect) {
        // The whole column, including the corners beside the ruler and scroll bar.
        let column = Rect::from_min_max(Pos2::new(header.left(), header.top() - RULER_H), Pos2::new(header.right(), header.bottom() + 15.0));
        ui.painter().rect_filled(column, 0.0, PANEL);
        ui.painter().line_segment([header.right_top(), header.right_bottom()], Stroke::new(1.0, LINE));
        let mut activate = None;
        let mut changed = false;
        for ti in 0..n_tracks {
            let l = lane(ti);
            let r = Rect::from_min_max(Pos2::new(header.left(), l.top()), Pos2::new(header.right() - 1.0, l.bottom()));
            let is_active = ti == self.active;
            if is_active {
                ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, INK);
            }
            if ti > 0 {
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, Color32::from_gray(190)));
            }
            let bg = ui.interact(r, ui.scope_id().with(("track_bg", ti)), Sense::click());
            if bg.clicked() {
                activate = Some(ti);
            }
            let inner = r.shrink2(Vec2::new(10.0, 6.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                let t = &mut self.tracks[ti];
                let name = ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(inner.width()).font(egui::TextStyle::Body));
                if name.gained_focus() {
                    activate = Some(ti);
                }
                if inner.height() > 44.0 {
                    ui.horizontal(|ui| {
                        if ui.add(Button::new("M").selected(t.mute).min_size(Vec2::new(24.0, 20.0))).on_hover_text("Mute").clicked() {
                            t.mute = !t.mute;
                            changed = true;
                        }
                        if ui.add(Button::new("S").selected(t.solo).min_size(Vec2::new(24.0, 20.0))).on_hover_text("Solo").clicked() {
                            t.solo = !t.solo;
                            changed = true;
                        }
                        ui.spacing_mut().slider_width = (inner.width() - 60.0).max(30.0);
                        let vol = ui.add(egui::Slider::new(&mut t.gain, 0.0..=2.0).show_value(false)).on_hover_text(format!("Volume {:.0}%", t.gain * 100.0));
                        if vol.drag_stopped() || (vol.changed() && !vol.dragged()) {
                            changed = true;
                        }
                    });
                }
                if inner.height() > 70.0 {
                    ui.horizontal(|ui| {
                        let (sw, swr) = ui.allocate_exact_size(Vec2::new(24.0, 20.0), Sense::click());
                        ui.painter().rect_filled(sw.shrink(3.0), 3.0, TRACK_COLORS[t.color % TRACK_COLORS.len()]);
                        if swr.on_hover_text("Track colour (click to change)").clicked() {
                            t.color = (t.color + 1) % TRACK_COLORS.len();
                        }
                        ui.label(egui::RichText::new("Pan").size(11.0).color(MUTED));
                        ui.spacing_mut().slider_width = (inner.width() - 64.0).max(30.0);
                        let pan_tip = match t.pan {
                            p if p.abs() < 0.01 => "Pan centre".to_string(),
                            p if p < 0.0 => format!("Pan {:.0}% left", -p * 100.0),
                            p => format!("Pan {:.0}% right", p * 100.0),
                        };
                        let pan = ui.add(egui::Slider::new(&mut t.pan, -1.0..=1.0).show_value(false)).on_hover_text(pan_tip);
                        if pan.double_clicked() {
                            t.pan = 0.0;
                            changed = true;
                        }
                        if pan.drag_stopped() || (pan.changed() && !pan.dragged()) {
                            changed = true;
                        }
                    });
                }
            });
        }
        if let Some(ti) = activate {
            self.set_active(ti);
        }
        if changed {
            self.dirty = true;
            self.refresh_playback();
        }
    }

    // ---------- arrangement view ----------

    fn arr_fit(&mut self) {
        self.arr_view_start = 0.0;
        self.arr_view_len = (self.arrangement_frames().max(self.rate() as usize * 4) as f64 * 1.1).max(32.0);
    }

    fn arr_clamp(&mut self) {
        let n = (self.arrangement_frames() as f64 * 1.5).max(self.rate() as f64 * 30.0);
        self.arr_view_len = self.arr_view_len.clamp(32.0, n);
        self.arr_view_start = self.arr_view_start.clamp(0.0, (n - self.arr_view_len).max(0.0));
    }

    fn arr_zoom(&mut self, factor: f64, anchor: f64) {
        let new_len = (self.arr_view_len * factor).max(32.0);
        let t = (anchor - self.arr_view_start) / self.arr_view_len;
        self.arr_view_start = anchor - t * new_len;
        self.arr_view_len = new_len;
        self.arr_clamp();
    }

    /// Strip above the main view: where you are, and the way back from the editor.
    fn view_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            match self.view {
                View::Arrangement => {
                    // Path: Song ▸ Group ▸ … ; click a level to go back up to it.
                    let mut names: Vec<String> = vec!["Song".into()];
                    for f in &self.nest {
                        names.push(f.tracks.get(f.track).and_then(|t| t.clips.get(f.clip)).map_or("Group".into(), |c| c.name.clone()));
                    }
                    let depth = names.len() - 1;
                    let mut go_up = None;
                    for (i, n) in names.iter().enumerate() {
                        if i > 0 {
                            ui.label(egui::RichText::new("▸").color(MUTED));
                        }
                        if i == depth {
                            ui.label(egui::RichText::new(n).strong().color(INK));
                        } else if ui.link(egui::RichText::new(n).color(MUTED)).clicked() {
                            go_up = Some(depth - i);
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Double-click a clip to open it · drag to move · drag edges to trim · shift-click to select several").color(MUTED));
                    if let Some(levels) = go_up {
                        for _ in 0..levels {
                            self.leave_group();
                        }
                    }
                }
                View::Editor { track, clip } => {
                    if ui.button("◀ Arrangement").on_hover_text("Back to the arrangement (⌘Return)").clicked() {
                        self.back_to_arrangement();
                        return;
                    }
                    let (tname, cname) = self
                        .tracks
                        .get(track)
                        .map(|t| (t.name.clone(), t.clips.get(clip).map(|c| c.name.clone()).unwrap_or_default()))
                        .unwrap_or_default();
                    ui.label(egui::RichText::new("Editing").color(MUTED));
                    if let Some(c) = self.tracks.get_mut(track).and_then(|t| t.clips.get_mut(clip)) {
                        ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).on_hover_text("Clip name");
                    } else {
                        ui.label(cname);
                    }
                    ui.label(egui::RichText::new(format!("on {tname}")).color(MUTED));
                }
            }
        });
    }

    /// The arrangement: track headers on the left, clips on track lanes along a timeline.
    fn arrangement(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let header_w = 150.0;
        let ruler_h = 22.0;
        let ruler = Rect::from_min_max(Pos2::new(full.left() + header_w, full.top()), Pos2::new(full.right(), full.top() + ruler_h));
        let scroll = Rect::from_min_max(Pos2::new(full.left() + header_w, full.bottom() - 15.0), full.max);
        let lanes = Rect::from_min_max(Pos2::new(full.left() + header_w, ruler.bottom()), Pos2::new(full.right(), scroll.top()));
        let header = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.left() + header_w, scroll.top()));
        let n_tracks = self.tracks.len().max(1);
        let lane_h = (lanes.height() / n_tracks as f32).min(120.0);
        let lane = move |i: usize| Rect::from_min_size(Pos2::new(lanes.left(), lanes.top() + i as f32 * lane_h), Vec2::new(lanes.width(), lane_h));
        let rate = self.rate() as f64;
        let spp = self.arr_view_len / lanes.width() as f64;

        let resp = ui.allocate_rect(Rect::from_min_max(ruler.min, lanes.max), Sense::click_and_drag());
        let x_to_f = |x: f32, vs: f64, vl: f64| (vs + ((x - lanes.left()) / lanes.width()) as f64 * vl).max(0.0);
        let f_to_x = |f: f64, vs: f64, vl: f64| lanes.left() + ((f - vs) / vl) as f32 * lanes.width();

        // Zoom and scroll, as in the editor.
        if resp.hovered() {
            let (zoom, delta, hover, alt) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos(), i.modifiers.alt));
            let anchor = hover.map_or(self.arr_view_start + self.arr_view_len / 2.0, |p| x_to_f(p.x, self.arr_view_start, self.arr_view_len));
            if zoom != 1.0 {
                self.arr_zoom(1.0 / zoom as f64, anchor);
            } else if alt && delta.y != 0.0 {
                self.arr_zoom((-delta.y as f64 / 150.0).exp(), anchor);
            } else {
                let d = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
                if d != 0.0 {
                    self.arr_view_start -= d as f64 / lanes.width() as f64 * self.arr_view_len;
                    self.arr_clamp();
                }
            }
        }

        // Which clip is at a point, and whether the point is on one of its edges.
        let hit = |tracks: &[Track], q: Pos2, vs: f64, vl: f64| -> Option<(usize, usize, DragMode)> {
            if q.y < lanes.top() {
                return None;
            }
            let t = ((q.y - lanes.top()) / lane_h) as usize;
            let clips = &tracks.get(t)?.clips;
            // Later clips are drawn on top, so test them first.
            clips.iter().enumerate().rev().find_map(|(i, c)| {
                let (x0, x1) = (f_to_x(c.start as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
                if q.x < x0 - 1.0 || q.x > x1 + 1.0 {
                    return None;
                }
                let edge = 6.0f32.min((x1 - x0) / 3.0);
                let mode = if q.x - x0 < edge {
                    DragMode::TrimStart
                } else if x1 - q.x < edge {
                    DragMode::TrimEnd
                } else {
                    DragMode::Move
                };
                Some((t, i, mode))
            })
        };
        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        let hover_hit = resp.hover_pos().and_then(|q| hit(&self.tracks, q, vs, vl));
        match (self.clip_drag.as_ref().map(|d| d.mode), hover_hit.map(|h| h.2)) {
            (Some(DragMode::Move), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            (Some(_), _) | (None, Some(DragMode::TrimStart | DragMode::TrimEnd)) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            (None, Some(DragMode::Move)) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            _ => {}
        }

        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        if resp.drag_started() {
            if let Some(q) = press {
                if let Some((t, c, mode)) = hit(&self.tracks, q, vs, vl) {
                    self.push_undo(if mode == DragMode::Move { "Move Clip" } else { "Trim Clip" });
                    let orig = self.tracks[t].clips[c].clone();
                    let grab = x_to_f(q.x, vs, vl) as i64 - orig.start as i64;
                    self.clip_drag = Some(ClipDrag { track: t, clip: c, mode, grab, orig });
                    self.sel_extra.clear();
                    self.sel_clip = Some((t, c));
                    self.active = t;
                }
            }
        }
        if let (true, Some(q)) = (resp.dragged(), resp.interact_pointer_pos()) {
            if let Some(d) = self.clip_drag.as_mut() {
                let f = x_to_f(q.x, vs, vl);
                // Snap to other clips' edges and the playhead within 8 pixels (⌘ turns it off).
                let snap_px = 8.0 * spp;
                let no_snap = ui.input(|i| i.modifiers.command);
                let (dt, dc) = (d.track, d.clip);
                let edges: Vec<f64> = std::iter::once(self.arr_cursor as f64)
                    .chain(self.tracks.iter().enumerate().flat_map(|(ti, t)| {
                        t.clips
                            .iter()
                            .enumerate()
                            .filter(move |(ci, _)| !(ti == dt && *ci == dc))
                            .flat_map(|(_, c)| [c.start as f64, c.end() as f64])
                    }))
                    .chain(std::iter::once(0.0))
                    .collect();
                let snap = |x: f64| -> f64 {
                    if no_snap {
                        return x;
                    }
                    edges.iter().copied().filter(|e| (e - x).abs() < snap_px).min_by(|a, b| (a - x).abs().total_cmp(&(b - x).abs())).unwrap_or(x)
                };
                let o = &d.orig;
                match d.mode {
                    DragMode::Move => {
                        let raw = (f - d.grab as f64).max(0.0);
                        // Snap whichever edge (start or end) is closer to something.
                        let s1 = snap(raw);
                        let s2 = snap(raw + o.len as f64) - o.len as f64;
                        let start = if (s1 - raw).abs() <= (s2 - raw).abs() && s1 != raw { s1 } else if s2 != raw { s2 } else { raw };
                        let target = (((q.y - lanes.top()) / lane_h).max(0.0) as usize).min(self.tracks.len() - 1);
                        let mut clip = self.tracks[d.track].clips.remove(d.clip);
                        clip.start = start.max(0.0) as usize;
                        self.tracks[target].clips.push(clip);
                        d.track = target;
                        d.clip = self.tracks[target].clips.len() - 1;
                        self.sel_clip = Some((d.track, d.clip));
                        self.active = target;
                    }
                    DragMode::TrimStart => {
                        // The left edge can move back as far as the audio goes, and up to near the end.
                        let lo = o.start as f64 - o.trim as f64;
                        let hi = (o.end() - 32) as f64;
                        let start = snap(f).clamp(lo.max(0.0), hi);
                        let c = &mut self.tracks[d.track].clips[d.clip];
                        let delta = start as i64 - o.start as i64;
                        c.start = start as usize;
                        c.trim = (o.trim as i64 + delta) as usize;
                        c.len = (o.len as i64 - delta) as usize;
                    }
                    DragMode::TrimEnd => {
                        let max_end = (o.start + o.audio.frames() - o.trim) as f64;
                        let end = snap(f).clamp((o.start + 32) as f64, max_end);
                        self.tracks[d.track].clips[d.clip].len = end as usize - o.start;
                    }
                }
            }
        }
        if resp.drag_stopped() && self.clip_drag.take().is_some() {
            self.dirty = true;
            self.refresh_playback();
        }
        if resp.double_clicked() {
            if let Some((t, c, _)) = resp.interact_pointer_pos().and_then(|q| hit(&self.tracks, q, vs, vl)) {
                self.edit_clip((t, c));
                return;
            }
        } else if resp.clicked() {
            if let Some(q) = resp.interact_pointer_pos() {
                match hit(&self.tracks, q, vs, vl) {
                    Some((t, c, _)) => {
                        if ui.input(|i| i.modifiers.shift) {
                            // Shift-click adds or removes clips from the selection.
                            if let Some(prev) = self.sel_clip.filter(|&p| p != (t, c)) {
                                if !self.sel_extra.contains(&prev) {
                                    self.sel_extra.push(prev);
                                }
                            }
                            if let Some(i) = self.sel_extra.iter().position(|&p| p == (t, c)) {
                                self.sel_extra.remove(i);
                            }
                        } else {
                            self.sel_extra.clear();
                        }
                        self.sel_clip = Some((t, c));
                        self.active = t;
                    }
                    None => {
                        // Clicking empty space moves the playhead (and picks that track).
                        self.sel_clip = None;
                        self.sel_extra.clear();
                        self.arr_cursor = x_to_f(q.x, vs, vl) as usize;
                        self.play_origin = self.arr_cursor;
                        if q.y > lanes.top() {
                            self.set_active((((q.y - lanes.top()) / lane_h) as usize).min(n_tracks - 1));
                        }
                        if self.player.is_playing() {
                            let end = self.arrangement_frames();
                            self.player.play(self.arr_cursor, end, false);
                        }
                    }
                }
            }
        }

        // Scroll bar.
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let total = (self.arrangement_frames() as f64 * 1.5).max(rate * 30.0);
        let thumb_w = ((self.arr_view_len / total) * scroll.width() as f64).clamp(16.0, scroll.width() as f64) as f32;
        let track_w = scroll.width() - thumb_w;
        let max_start = (total - self.arr_view_len).max(0.0);
        if (sresp.dragged() || sresp.clicked()) && track_w > 0.0 {
            if let Some(p) = sresp.interact_pointer_pos() {
                self.arr_view_start = (((p.x - scroll.left() - thumb_w / 2.0) / track_w).clamp(0.0, 1.0) as f64) * max_start;
                self.arr_clamp();
            }
        }

        // Follow the playhead.
        let playing = self.player.is_playing();
        let play_pos = self.player.position();
        if playing {
            ui.ctx().request_repaint();
            let p = play_pos as f64;
            if self.clip_drag.is_none() && (p > self.arr_view_start + self.arr_view_len || p < self.arr_view_start) {
                self.arr_view_start = p - self.arr_view_len * 0.05;
                self.arr_clamp();
            }
        }

        // --- painting ---
        let (vs, vl) = (self.arr_view_start, self.arr_view_len);
        let painter = ui.painter_at(full);
        painter.rect_filled(lanes, 0.0, Color32::WHITE);
        painter.rect_filled(ruler, 0.0, PANEL);
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, LINE));
        // Ruler ticks.
        let secs_per_px = vl / rate / lanes.width() as f64;
        let nice = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = *nice.iter().find(|&&s| s / secs_per_px >= 90.0).unwrap_or(&1200.0);
        let minor = step / 5.0;
        let mut k = (vs / rate / minor).floor() as i64;
        loop {
            let t = k as f64 * minor;
            if t > (vs + vl) / rate {
                break;
            }
            let x = f_to_x(t * rate, vs, vl);
            if x >= lanes.left() {
                let major = k % 5 == 0;
                painter.line_segment([Pos2::new(x, ruler.bottom() - if major { 7.0 } else { 3.0 }), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, MUTED));
                if major {
                    painter.line_segment([Pos2::new(x, lanes.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.0, Color32::from_gray(242)));
                    painter.text(Pos2::new(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, ruler_label(self.ruler, rate as u32, t, step), FontId::proportional(11.0), MUTED);
                }
            }
            k += 1;
        }
        // Lanes and clips.
        let any_solo = self.tracks.iter().any(|t| t.solo);
        for (ti, t) in self.tracks.iter().enumerate() {
            let l = lane(ti);
            if ti == self.active {
                painter.rect_filled(l, 0.0, Color32::from_gray(250));
            }
            painter.line_segment([l.left_bottom(), l.right_bottom()], Stroke::new(1.0, Color32::from_gray(225)));
            let muted = if any_solo { !t.solo } else { t.mute };
            let color = TRACK_COLORS[t.color % TRACK_COLORS.len()];
            for (ci, c) in t.clips.iter().enumerate() {
                let (x0, x1) = (f_to_x(c.start as f64, vs, vl), f_to_x(c.end() as f64, vs, vl));
                if x1 < lanes.left() || x0 > lanes.right() {
                    continue;
                }
                let r = Rect::from_min_max(Pos2::new(x0, l.top() + 3.0), Pos2::new(x1.max(x0 + 2.0), l.bottom() - 3.0));
                let selected = self.sel_clip == Some((ti, ci)) || self.sel_extra.contains(&(ti, ci));
                draw_clip(&painter, r, c, color, selected, muted, lanes);
            }
        }
        if self.tracks.is_empty() {
            painter.text(lanes.center(), Align2::CENTER_CENTER, "Drop a sound here, or choose File ▸ Open", FontId::proportional(16.0), Color32::from_gray(120));
        }
        // Playhead (or the playback position).
        let ph = if playing { play_pos } else { self.arr_cursor };
        let x = f_to_x(ph as f64, vs, vl);
        if x >= lanes.left() && x <= lanes.right() {
            painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, lanes.bottom())], Stroke::new(1.5, INK));
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(x - 5.0, ruler.bottom() - 7.0), Pos2::new(x + 5.0, ruler.bottom() - 7.0), Pos2::new(x, ruler.bottom())],
                INK,
                Stroke::NONE,
            ));
        }
        // Scroll bar.
        painter.rect_filled(scroll, 0.0, PANEL);
        painter.line_segment([scroll.left_top(), scroll.right_top()], Stroke::new(1.0, LINE));
        let thumb_x = if max_start > 0.0 { (self.arr_view_start / max_start) as f32 * track_w } else { 0.0 };
        let thumb = Rect::from_min_size(Pos2::new(scroll.left() + thumb_x, scroll.top()), Vec2::new(thumb_w, scroll.height()));
        painter.rect_filled(thumb.shrink2(Vec2::new(1.0, 4.0)), 4.0, Color32::from_gray(185));
        if let Some(q) = resp.hover_pos() {
            painter.text(Pos2::new(lanes.right() - 6.0, ruler.top() + 2.0), Align2::RIGHT_TOP, self.fmt_pos(x_to_f(q.x, vs, vl) as usize), FontId::monospace(11.0), MUTED);
        }
        if !self.tracks.is_empty() {
            self.track_headers(ui, header, self.tracks.len(), &lane);
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
        dialog_modal("unsaved").show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            dialog_header(ui, "Unsaved changes", &format!("You have edits that haven't been exported. Discard them and {what}?"));
            let (export, cancel) = dialog_buttons(ui, "Export…", true, |ui| {
                if ui.add(Button::new("Discard").min_size(Vec2::new(84.0, 26.0))).clicked() {
                    choice = Some(1);
                }
            });
            if export {
                choice = Some(0);
            }
            if cancel {
                choice = Some(2);
            }
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
        for p in dropped {
            if self.view == View::Arrangement && !self.tracks.is_empty() && self.loading.is_none() {
                // Dropping onto an open arrangement adds clips at the playhead.
                self.import_as_clip = true;
                self.start_load_as(p, &ctx, true);
            } else {
                self.request_open(p, &ctx);
            }
            break;
        }

        // Ask before quitting with unsaved edits.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        let name = self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into());
        let title = if self.audio.is_some() { format!("{name}{} — Sound Edit", if self.dirty { " (edited)" } else { "" }) } else { "Sound Edit".into() };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let modal_open = self.pending.is_some() || self.dialog.is_some() || self.curve.is_some() || self.mix_include.is_some();
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

        // Docked layout: tools on the left, transport and status at the bottom, Info/Cues on
        // the right, and the waveform filling the middle.
        self.update_meters(&ctx);
        let bar = egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(10, 6));
        egui::Panel::bottom("status").frame(bar).show(ui, |ui| self.status_bar(ui));
        if self.show_controls {
            egui::Panel::bottom("transport").frame(bar).show(ui, |ui| self.transport_panel(ui));
        }
        if self.show_tools {
            egui::Panel::left("tools")
                .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(8, 4)))
                .resizable(false)
                .show(ui, |ui| self.tools_panel(ui));
        }
        if self.show_info || self.show_cues || self.show_regions {
            egui::Panel::right("side")
                .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(12, 4)))
                .resizable(false)
                .exact_size(320.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side_panel(ui));
                });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.desk = ui.max_rect();
            if !self.tracks.is_empty() {
                let full = ui.max_rect();
                let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), 34.0));
                ui.painter().rect_filled(bar, 0.0, PANEL);
                ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, LINE));
                ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(Vec2::new(12.0, 4.0))), |ui| self.view_bar(ui));
                let body = Rect::from_min_max(Pos2::new(full.left(), bar.bottom()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| match self.view {
                    View::Arrangement => self.arrangement(ui),
                    View::Editor { .. } => self.waveform(ui),
                });
            } else {
                self.waveform(ui);
            }
        });

        if self.show_record || self.recorder.is_some() {
            self.show_record = true;
            self.record_window(&ctx);
        }
        if self.show_spectrum {
            self.spectrum_window(&ctx);
        }
        if self.show_batch {
            self.batch_window(&ctx);
        }
        self.mix_dialog(&ctx);

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

/// A level meter bar: dark fill up to the level on a -48..0 dB scale, black near clipping.
fn draw_meter(p: &egui::Painter, r: Rect, level: f32) {
    let db = 20.0 * level.max(1e-6).log10();
    let t = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
    p.rect_filled(r, 2.0, Color32::from_gray(228));
    if t > 0.0 {
        let fill = Rect::from_min_size(r.min, Vec2::new(r.width() * t, r.height()));
        p.rect_filled(fill, 2.0, if db > -3.0 { Color32::BLACK } else { Color32::from_gray(90) });
    }
}

/// Text, secondary text, panel and line colours for the clean monochrome look.
const INK: Color32 = Color32::from_gray(20);
const MUTED: Color32 = Color32::from_gray(120);
const PANEL: Color32 = Color32::from_gray(246);
const LINE: Color32 = Color32::from_gray(218);

/// Clean monochrome look: white and light-grey surfaces, thin grey lines, black accents, and
/// the system font when available.
fn clean_style(ctx: &egui::Context) {
    use egui::{CornerRadius, Shadow};
    ctx.set_theme(egui::Theme::Light);
    let mut v = egui::Visuals::light();
    v.panel_fill = PANEL;
    v.window_fill = Color32::WHITE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow = Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(40) };
    v.popup_shadow = Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(35) };
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_gray(250);
    v.override_text_color = None;
    v.selection.bg_fill = Color32::from_gray(30);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.window_highlight_topmost = false;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(5);
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.0, INK);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    // bg_fill draws slider rails and checkbox boxes; buttons use weak_bg_fill.
    v.widgets.inactive.bg_fill = Color32::from_gray(222);
    v.widgets.inactive.weak_bg_fill = Color32::WHITE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(205));
    v.widgets.hovered.bg_fill = Color32::from_gray(238);
    v.widgets.hovered.weak_bg_fill = Color32::from_gray(238);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(160));
    v.widgets.active.bg_fill = Color32::from_gray(215);
    v.widgets.active.weak_bg_fill = Color32::from_gray(215);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_gray(120));
    v.widgets.open = v.widgets.hovered;
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|style| {
        use egui::{FontFamily, TextStyle};
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 4.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.window_margin = egui::Margin::same(14);
        style.spacing.combo_width = 140.0;
        style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    });

    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        ("SF", "/System/Library/Fonts/SFNS.ttf", egui::FontFamily::Proportional),
        ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf", egui::FontFamily::Monospace),
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
                    "spectrum3d" => self.spectrum_3d = true,
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
                    "playhead" => self.arr_cursor = (arg.parse::<f64>().unwrap_or(0.0) * rate) as usize,
                    "export" => {
                        let a = if self.view == View::Arrangement { Arc::new(self.mixdown()) } else { self.audio.clone().unwrap() };
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

/// A clip block in the arrangement: a name strip on top and its waveform below.
fn draw_clip(p: &egui::Painter, r: Rect, c: &Clip, color: Color32, selected: bool, muted: bool, clip_to: Rect) {
    let p = p.with_clip_rect(clip_to);
    let wave_col = if muted { Color32::from_gray(190) } else { color };
    let fill = if muted { Color32::from_gray(248) } else { color.lerp_to_gamma(Color32::WHITE, 0.88) };
    p.rect_filled(r, 4.0, fill);
    let title = Rect::from_min_max(r.min, Pos2::new(r.right(), (r.top() + 16.0).min(r.bottom())));
    p.rect_filled(title, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, if muted { Color32::from_gray(225) } else { color.lerp_to_gamma(Color32::WHITE, 0.55) });
    let mut name_x = r.left().max(clip_to.left()) + 5.0;
    if c.group.is_some() && r.right() - name_x > 30.0 {
        // Group clips get a small stacked-boxes mark before the name.
        let m = Rect::from_min_size(Pos2::new(name_x, title.center().y - 4.0), Vec2::new(8.0, 6.0));
        p.rect_stroke(m.translate(Vec2::new(2.0, 2.0)), 1.0, Stroke::new(1.0, INK), egui::StrokeKind::Inside);
        p.rect_filled(m, 1.0, INK);
        name_x += 15.0;
    }
    if r.right() - name_x > 20.0 {
        let galley = p.layout(c.name.clone(), FontId::proportional(11.0), INK, r.right() - name_x - 4.0);
        p.galley(Pos2::new(name_x, title.center().y - galley.size().y / 2.0), galley, INK);
    }
    // Waveform of the visible part, one min/max line per pixel (all channels together).
    let body = Rect::from_min_max(Pos2::new(r.left(), title.bottom() + 2.0), Pos2::new(r.right(), r.bottom() - 2.0));
    if body.height() > 4.0 && c.len > 0 {
        let mid = body.center().y;
        let amp = body.height() / 2.0 * 0.9;
        let x0 = body.left().max(clip_to.left());
        let x1 = body.right().min(clip_to.right());
        let fpp = c.len as f64 / body.width().max(1.0) as f64;
        let mut x = x0;
        while x < x1 {
            let f0 = c.trim + ((x - body.left()) as f64 * fpp) as usize;
            let f1 = (c.trim + ((x + 1.0 - body.left()) as f64 * fpp) as usize).max(f0 + 1).min(c.audio.frames());
            if f0 >= f1 {
                break;
            }
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            if fpp >= c.peaks.block as f64 * 2.0 {
                for ch in &c.peaks.data {
                    for &(l, h) in &ch[f0 / c.peaks.block..(f1 / c.peaks.block).max(f0 / c.peaks.block + 1).min(ch.len())] {
                        lo = lo.min(l);
                        hi = hi.max(h);
                    }
                }
            } else {
                for ch in &c.audio.channels {
                    for &v in &ch[f0..f1] {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            let y0 = mid - hi.clamp(-1.0, 1.0) * amp;
            let y1 = (mid - lo.clamp(-1.0, 1.0) * amp).max(y0 + 1.0);
            p.line_segment([Pos2::new(x + 0.5, y0), Pos2::new(x + 0.5, y1)], Stroke::new(1.0, wave_col));
            x += 1.0;
        }
    }
    let stroke = if selected { Stroke::new(2.0, INK) } else { Stroke::new(1.0, color.lerp_to_gamma(Color32::WHITE, 0.3)) };
    p.rect_stroke(r, 4.0, stroke, egui::StrokeKind::Inside);
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
            p.rect_filled(r2, 0.0, Color32::WHITE);
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
            p.circle_filled(c, 6.0, color);
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

/// Dialog width, label column width and spacing shared by every dialog.
const DIALOG_W: f32 = 400.0;
const LABEL_W: f32 = 120.0;

/// A modal dialog with the app's frame: white, rounded, generous margins.
fn dialog_modal(id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(id)).frame(
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(22))
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(45) }),
    )
}

/// Dialog title and a muted explanation under it.
fn dialog_header(ui: &mut egui::Ui, title: &str, blurb: &str) {
    ui.label(egui::RichText::new(title).size(17.0).strong().color(INK));
    if !blurb.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(blurb).color(MUTED));
    }
    ui.add_space(14.0);
}

/// The grid used for a dialog's labelled settings.
fn dialog_grid(id: &str) -> egui::Grid {
    egui::Grid::new(id).num_columns(2).min_col_width(LABEL_W).spacing(Vec2::new(12.0, 10.0))
}

/// A filled black button for a dialog's main action.
fn primary_button(label: &str) -> Button<'static> {
    Button::new(egui::RichText::new(label.to_string()).color(Color32::WHITE)).fill(INK).min_size(Vec2::new(84.0, 26.0))
}

/// Right-aligned Cancel and main-action buttons at the bottom of a dialog, with optional extra
/// buttons on the left (`extra`). Returns (main clicked, cancel clicked).
fn dialog_buttons(ui: &mut egui::Ui, ok_label: &str, ok_enabled: bool, extra: impl FnOnce(&mut egui::Ui)) -> (bool, bool) {
    ui.add_space(18.0);
    let mut out = (false, false);
    ui.horizontal(|ui| {
        extra(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            out.0 = ui.add_enabled(ok_enabled, primary_button(ok_label)).clicked();
            out.1 = ui.add(Button::new("Cancel").min_size(Vec2::new(84.0, 26.0))).clicked();
        });
    });
    out
}

/// Mixer auto gain: if a mix would clip, scale it down so its peak sits just below full scale.
fn auto_gain(mix: Audio) -> Audio {
    let peak = mix.peak(0, mix.frames());
    if peak > 0.99 { mix.gain(0, mix.frames(), 0.99 / peak) } else { mix }
}

fn fmt_pos_in(units: RulerUnits, rate: u32, f: usize) -> String {
    match units {
        RulerUnits::Time => fmt_time(f as f64 / rate as f64),
        RulerUnits::Samples => f.to_string(),
        RulerUnits::Timecode => {
            let secs = f as f64 / rate as f64;
            let total = secs.floor() as u64;
            let frames = ((secs - total as f64) * 30.0).floor() as u64;
            format!("{:02}:{:02}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60, frames)
        }
    }
}

/// Parse a typed position: samples ("44100"), seconds ("1.5"), m:ss.mmm ("0:01.500") or
/// timecode ("00:00:01:15"). Returns frames.
fn parse_pos(units: RulerUnits, rate: u32, text: &str) -> Option<f64> {
    let t = text.trim();
    let parts: Vec<&str> = t.split(':').collect();
    let secs = match parts.len() {
        1 if units == RulerUnits::Samples => return t.parse::<f64>().ok(),
        1 => t.parse::<f64>().ok()?,
        2 => parts[0].parse::<f64>().ok()? * 60.0 + parts[1].parse::<f64>().ok()?,
        3 => parts[0].parse::<f64>().ok()? * 3600.0 + parts[1].parse::<f64>().ok()? * 60.0 + parts[2].parse::<f64>().ok()?,
        4 => {
            parts[0].parse::<f64>().ok()? * 3600.0
                + parts[1].parse::<f64>().ok()? * 60.0
                + parts[2].parse::<f64>().ok()?
                + parts[3].parse::<f64>().ok()? / 30.0
        }
        _ => return None,
    };
    Some(secs * rate as f64)
}

/// Ruler tick label for time `t` (seconds), given the tick spacing.
fn ruler_label(units: RulerUnits, rate: u32, t: f64, step: f64) -> String {
    match units {
        RulerUnits::Time => fmt_tick(t, step),
        RulerUnits::Samples => {
            let f = (t * rate as f64).round() as u64;
            if f >= 1_000_000 { format!("{:.2}M", f as f64 / 1e6) } else if f >= 10_000 { format!("{}k", f / 1000) } else { f.to_string() }
        }
        RulerUnits::Timecode => fmt_pos_in(units, rate, (t * rate as f64).round() as usize),
    }
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
