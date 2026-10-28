//! The app: its state, undo, playback, the action dispatcher, shortcuts and the frame loop.
//! The views, dialogs and file handling live in the submodules under `app/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self, Align2, Button, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2,
};

use crate::ableton;
use crate::actions::{self, Action};
use crate::audio::{self, Audio, Cue, Markers, Peaks, Region, WavFormat};
use crate::effects::{self, Wave};
use crate::menu::NativeMenu;
use crate::player::Player;
use crate::project::{self, ClipData, ProjectFile, TrackData};
use crate::recorder::Recorder;

mod clip;
mod chop;
mod arrange;
mod editor;
mod dialogs;
mod files;
mod panels;
mod look;
mod automation;
mod autoarrange;

use self::clip::*;
use self::chop::*;
use self::dialogs::*;
use self::files::*;
use self::look::*;
use self::automation::Automation;
use self::autoarrange::{PoolDrag, PoolLoop};

type Sel = (usize, usize);

/// Undo history is capped by memory rather than step count, since each step is a full copy.
const UNDO_BUDGET_BYTES: usize = 2 << 30;

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
    /// A track made by dragging the clip below the others (removed again if the clip leaves it).
    new_track: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
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

/// Grid divisions for the arrangement: label and length in beats (0 = off, -1 = automatic).
const GRID: [(&str, f64); 8] =
    [("Off", 0.0), ("Auto", -1.0), ("1 Bar", -2.0), ("1/2", 2.0), ("1/4", 1.0), ("1/8", 0.5), ("1/16", 0.25), ("1/32", 0.125)];

/// Height of the time ruler: tick labels and cues, the regions band, then loop and tick marks.
const RULER_H: f32 = 30.0;

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
    /// Arrangement only: click a clip to cut it in two.
    Split,
    Hand,
    Zoom,
    Pencil,
}

struct PencilStroke {
    channel: usize,
    last: Option<(usize, f32)>,
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
    /// Where a dropped file goes: (track, or None for a new track; timeline frame).
    drop_target: Option<(Option<usize>, usize)>,
    /// Song tempo and beats per bar (x/4), the grid division (index into GRID) and snapping.
    tempo: f64,
    beats_per_bar: u32,
    /// The arrangement's loop range (played round when Loop is on), and a ruler drag setting it.
    arr_loop: Option<Sel>,
    arr_loop_drag: Option<usize>,
    /// Which track a right-click on empty lane space is for (None: the clip menu).
    ctx_menu_track: Option<usize>,
    grid: usize,
    snap: bool,
    /// Further dropped files waiting to load, with where each goes.
    drop_queue: std::collections::VecDeque<(PathBuf, (Option<usize>, usize))>,
    /// The arrangement's lane area and lane height, from the last frame (for drops).
    arr_lanes: Option<(Rect, f32)>,
    /// The clip under the pointer in the arrangement, and the (snapped) frame there.
    arr_hover: Option<(usize, usize, usize)>,
    /// The mangler's random state, and what it did last (for Mangle Again).
    mangle_seed: u64,
    /// Auto Arrange: the loops it draws from, which variation is showing, and the section length.
    pool: Vec<PoolLoop>,
    variation: u32,
    section_bars: u32,
    last_mangle: Option<Mangle>,
    /// Automation: pointer events to feed in, each on its frame (counted in `synth_frame`).
    synth: Vec<(u32, egui::Event)>,
    synth_frame: u32,
    /// What the player is playing: the active track alone, or the mix of all tracks.
    play_audio: Option<Arc<Audio>>,
    path: Option<PathBuf>,
    /// The .sndproj file this sound was saved to or opened from.
    project_path: Option<PathBuf>,
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
    /// The tool palette sits in the left column instead of floating.
    palette_docked: bool,
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
    show_pool: bool,
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
    /// Warp clips whose names give their tempo when they're imported.
    auto_warp: bool,
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
            drop_target: None,
            tempo: 120.0,
            beats_per_bar: 4,
            arr_loop: None,
            arr_loop_drag: None,
            ctx_menu_track: None,
            grid: 1,
            snap: true,
            drop_queue: Default::default(),
            arr_lanes: None,
            arr_hover: None,
            pool: Vec::new(),
            variation: 1,
            section_bars: 8,
            mangle_seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1,
            last_mangle: None,
            synth: Vec::new(),
            synth_frame: 0,
            play_audio: None,
            path: None,
            project_path: None,
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
            menu: {
                #[cfg(target_os = "macos")]
                crate::menu::install_open_handler(&cc.egui_ctx);
                NativeMenu::install(&cc.egui_ctx)
            },
            dialog: None,
            tool: Tool::Select,
            pencil: None,
            zoom_band: None,
            show_tools: true,
            palette_docked: false,
            show_controls: true,
            show_info: false,
            meter: Vec::new(),
            info_peak: None,
            desk: Rect::NOTHING,
            cues: Vec::new(),
            regions: Vec::new(),
            loop_points: None,
            show_regions: false,
            show_pool: false,
            ruler: RulerUnits::Time,
            show_spectrum: false,
            spectrum_3d: false,
            spectrum_cache: None,
            show_batch: false,
            batch: Batch::default(),
            mix_include: None,
            mix_to_new: true,
            auto_gain: true,
            auto_warp: true,
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

    fn set_audio(&mut self, a: Arc<Audio>) {
        // If the whole file was in view, keep it that way after the length changes.
        let was_full = self.view_start <= 0.0 && self.view_len >= self.frames() as f64;
        let peaks = Arc::new(Peaks::build(&a));
        self.peaks = Some(peaks.clone());
        // The editor works on one clip: keep that clip in step with every edit.
        if let View::Editor { track, clip } = self.view {
            if let Some(c) = self.tracks.get_mut(track).and_then(|t| t.clips.get_mut(clip)).filter(|c| !Arc::ptr_eq(&c.audio, &a)) {
                c.audio = a.clone();
                c.peaks = peaks;
                // An edited clip is plain audio at its current speed.
                c.warp = None;
                if c.looping {
                    // A looping clip keeps its length on the timeline; it repeats the new audio.
                    c.trim = c.trim.min(a.frames().saturating_sub(1));
                } else {
                    c.trim = 0;
                    c.len = a.frames();
                }
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

    /// Timeline span covered by the selected clips.
    fn selected_span(&self) -> Option<Sel> {
        let sel = self.selected_clips();
        let s = sel.iter().filter_map(|&p| self.clip(p)).map(|c| c.start).min()?;
        let e = sel.iter().filter_map(|&p| self.clip(p)).map(|c| c.end()).max()?;
        (e > s).then_some((s, e))
    }

    /// All selected clips: the main one plus any shift-clicked ones.
    fn selected_clips(&self) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = self.sel_clip.into_iter().chain(self.sel_extra.iter().copied()).filter(|&p| self.clip(p).is_some()).collect();
        v.sort();
        v.dedup();
        v
    }

    // ---------- projects ----------

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

    /// After undo/redo (which stay in the current view): reload the clip being edited from the
    /// restored tracks, or refresh the arrangement.
    fn restore_audio(&mut self, a: Option<Arc<Audio>>) {
        match self.view {
            View::Editor { track, clip } => match self.clip((track, clip)).cloned() {
                Some(c) => {
                    let audio = if c.looping || (c.trim == 0 && c.len >= c.audio.frames()) { c.audio.clone() } else { Arc::new(c.visible()) };
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
        self.nest.clear();
        self.project_path = None;
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
            if let (true, Some((ls, le))) = (self.looping, self.arr_loop) {
                // Loop on: play the loop range round and round.
                self.play_origin = ls;
                self.player.play(ls, le, true);
                return;
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
                Action::SaveProject | Action::SaveProjectAs => doc,
                Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Dither | Action::AutoGain => true,
                Action::Loop | Action::ShowTools | Action::ShowControls | Action::ShowInfo | Action::ShowCues | Action::ShowRegions | Action::ShowPool => true,
                Action::ShowSpectrum | Action::RulerTime | Action::RulerSamples | Action::RulerTimecode => true,
                Action::Export | Action::ExportStems | Action::ExportAbleton => doc && self.arrangement_frames() > 0,
                Action::ExportSelection | Action::EditClip | Action::DuplicateClip | Action::Cut | Action::Copy | Action::Delete => has_clip,
                Action::SplitClip => doc,
                Action::ToolSelect | Action::ToolSplit => true,
                Action::GroupClips | Action::TempoFromClip | Action::LoopClip | Action::SetLoop | Action::CollapseClip | Action::WarpClip | Action::SliceClip | Action::ReverseClip => has_clip,
                Action::MangleShuffle | Action::MangleReverse | Action::MangleSwap | Action::MangleStutter | Action::MangleRandomReverse | Action::MangleDropOut | Action::MangleBBCut => has_clip || self.loop_span().is_some(),
                Action::MangleCutHouse | Action::MangleCutHipHop | Action::MangleCutDembow | Action::MangleCutFootwork | Action::MangleCutTrap | Action::MangleCutBreakcore | Action::MangleCutHalftime => has_clip || self.loop_span().is_some(),
                Action::MangleAgain => (has_clip || self.loop_span().is_some()) && self.last_mangle.is_some(),
                Action::AutoWarp => true,
                Action::AutoArrange => self.nest.is_empty() && self.tracks.iter().any(|t| !t.clips.is_empty()),
                Action::PrevVariation => self.nest.is_empty() && !self.pool.is_empty() && self.variation > 1,
                Action::NextVariation => self.nest.is_empty() && !self.pool.is_empty(),
                Action::SwapSection => has_clip,
                Action::CollapseTrack => self.tracks.get(self.active).is_some_and(|t| !t.clips.is_empty()),
                Action::ClearLoop => self.arr_loop.is_some(),
                Action::ToggleSnap => doc,
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
            Action::SaveProject | Action::SaveProjectAs => return doc,
            Action::EditClip | Action::SplitClip | Action::DuplicateClip | Action::NewClip | Action::ImportClip => return false,
            Action::GroupClips | Action::Ungroup | Action::TempoFromClip | Action::ToggleSnap | Action::LoopClip => return false,
            Action::CollapseClip | Action::CollapseTrack | Action::WarpClip | Action::SliceClip | Action::ReverseClip => return false,
            Action::MangleShuffle | Action::MangleReverse | Action::MangleSwap | Action::MangleStutter | Action::MangleRandomReverse | Action::MangleDropOut | Action::MangleBBCut | Action::MangleAgain => return false,
            Action::MangleCutHouse | Action::MangleCutHipHop | Action::MangleCutDembow | Action::MangleCutFootwork | Action::MangleCutTrap | Action::MangleCutBreakcore | Action::MangleCutHalftime => return false,
            Action::AutoWarp => return true,
            Action::AutoArrange | Action::PrevVariation | Action::NextVariation | Action::SwapSection => return false,
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
            Action::ShowSpectrum | Action::BatchConvert | Action::ShowRegions | Action::ShowPool => true,
            Action::FmSynth | Action::OffsetTrack | Action::TrackColor => has,
            Action::Dither | Action::AutoGain => true,
            Action::AddCue => has,
            Action::NextCue | Action::PrevCue => has && !self.cues.is_empty(),
            Action::ShowTools | Action::ShowControls | Action::ShowInfo => true,
            Action::ToolSelect | Action::ToolHand | Action::ToolZoom | Action::ToolPencil => true,
            Action::ToolSplit => false,
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
            Action::ToolSplit => self.tool == Tool::Split,
            Action::ShowTools => self.show_tools,
            Action::DockPalette => self.palette_docked,
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
            Action::ShowPool => self.show_pool,
            Action::ToggleSnap => self.snap,
            Action::AutoWarp => self.auto_warp,
            Action::WarpClip => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.warp.is_some()),
            Action::LoopClip => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.looping),
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
                Action::Loop => {
                    if self.arr_loop.is_none() {
                        self.arr_loop = self.selected_span().or(Some((self.arr_cursor, self.arr_cursor + (self.bar_frames() * 4.0) as usize)));
                    }
                    self.looping = !self.looping;
                    if self.player.is_playing() {
                        self.toggle_play();
                        self.toggle_play();
                    }
                    return;
                }
                Action::SetLoop => {
                    if let Some(span) = self.selected_span() {
                        self.push_undo("Set Loop");
                        self.arr_loop = Some(span);
                        self.looping = true;
                        self.dirty = true;
                    }
                    return;
                }
                Action::ClearLoop => {
                    self.push_undo("Clear Loop");
                    self.arr_loop = None;
                    self.looping = false;
                    self.dirty = true;
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
            Action::ToolSplit => self.tool = Tool::Split,
            Action::ShowTools => self.show_tools = !self.show_tools,
            Action::DockPalette => self.palette_docked = !self.palette_docked,
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
            Action::TempoFromClip => self.tempo_from_clip(),
            Action::LoopClip => self.toggle_clip_loop(),
            Action::CollapseClip => self.collapse_clip(),
            Action::CollapseTrack => self.collapse_track(self.active),
            Action::ToggleSnap => self.snap = !self.snap,
            Action::AutoWarp => self.auto_warp = !self.auto_warp,
            Action::WarpClip => self.toggle_warp(),
            Action::SliceClip => self.slice_clip(),
            Action::AutoArrange => self.auto_arrange(),
            Action::PrevVariation => self.step_variation(false),
            Action::NextVariation => self.step_variation(true),
            Action::SwapSection => self.swap_sections(),
            Action::ReverseClip => self.reverse_clips(),
            Action::MangleShuffle => self.mangle(Mangle::Shuffle),
            Action::MangleReverse => self.mangle(Mangle::ReverseOrder),
            Action::MangleSwap => self.mangle(Mangle::SwapPairs),
            Action::MangleStutter => self.mangle(Mangle::Stutter),
            Action::MangleRandomReverse => self.mangle(Mangle::RandomReverse),
            Action::MangleDropOut => self.mangle(Mangle::DropOut),
            Action::MangleBBCut => self.mangle(Mangle::Cut(CutStyle::Jungle)),
            Action::MangleCutHouse => self.mangle(Mangle::Cut(CutStyle::House)),
            Action::MangleCutHipHop => self.mangle(Mangle::Cut(CutStyle::HipHop)),
            Action::MangleCutDembow => self.mangle(Mangle::Cut(CutStyle::Dembow)),
            Action::MangleCutFootwork => self.mangle(Mangle::Cut(CutStyle::Footwork)),
            Action::MangleCutTrap => self.mangle(Mangle::Cut(CutStyle::Trap)),
            Action::MangleCutBreakcore => self.mangle(Mangle::Cut(CutStyle::Breakcore)),
            Action::MangleCutHalftime => self.mangle(Mangle::Cut(CutStyle::Halftime)),
            Action::MangleAgain => {
                if let Some(m) = self.last_mangle {
                    self.mangle(m);
                }
            }
            Action::SaveProject => {
                self.save_project(false);
            }
            Action::SaveProjectAs => {
                self.save_project(true);
            }
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
            Action::ExportStems => self.export_stems(),
            Action::ExportAbleton => self.export_ableton(),
            Action::RulerTime => self.ruler = RulerUnits::Time,
            Action::RulerSamples => self.ruler = RulerUnits::Samples,
            Action::RulerTimecode => self.ruler = RulerUnits::Timecode,
            Action::ShowSpectrum => self.show_spectrum = !self.show_spectrum,
            Action::ShowRegions => self.show_regions = !self.show_regions,
            Action::ShowPool => {
                self.show_pool = !self.show_pool;
                if self.show_pool && self.pool.is_empty() {
                    self.gather_pool();
                }
            }
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
            (none, Key::B, Action::ToolSplit),
            (none, Key::R, Action::SwapSection),
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

    /// Files dropped on the window. Onto an open arrangement they become clips at the drop
    /// point: the first on the track under the mouse and each further one on the next track
    /// down (new tracks are made as needed); below the tracks, each gets a new track.
    fn handle_drop(&mut self, files: Vec<PathBuf>, drop_pos: Option<Pos2>, ctx: &egui::Context) {
        // A project file opens (replacing what's open) rather than becoming a clip.
        if let Some(proj) = files.iter().find(|p| project::is_project(p)) {
            return self.request_open(proj.clone(), ctx);
        }
        let mut files = files;
        let arrangement_open = self.view == View::Arrangement && !self.tracks.is_empty();
        if !arrangement_open {
            // Nothing open yet: the first file opens, the rest go onto tracks of their own.
            if files.is_empty() {
                return;
            }
            let first = files.remove(0);
            self.request_open(first, ctx);
            self.drop_queue.extend(files.into_iter().map(|p| (p, (None, 0))));
            return;
        }
        let (track, frame) = drop_pos.and_then(|q| self.drop_target_at(q)).unwrap_or((Some(self.active), self.arr_cursor));
        for (k, p) in files.into_iter().enumerate() {
            self.drop_queue.push_back((p, (track.map(|t| t + k), frame)));
        }
    }

    /// Load the next queued dropped file once the previous one has finished.
    fn next_queued_drop(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() || self.tracks.is_empty() {
            return;
        }
        if let Some((p, target)) = self.drop_queue.pop_front() {
            self.drop_target = Some(target);
            self.import_as_clip = true;
            self.start_load_as(p, ctx, true);
        }
    }

    // ---------- tempo grid ----------

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
            dialog_header(ui, "Unsaved changes", &format!("Save the project before you {what}?"));
            let (export, cancel) = dialog_buttons(ui, "Save…", true, |ui| {
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
                if self.save_project(false) {
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
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.synth.is_empty() {
            return;
        }
        self.synth_frame += 1;
        let now = self.synth_frame;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.synth).into_iter().partition(|(f, _)| *f <= now);
        self.synth = later;
        raw_input.events.extend(due.into_iter().map(|(_, e)| e));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.synth.is_empty() {
            ctx.request_repaint();
        }
        self.poll_loading();
        // Warped clips follow the tempo once it's been set (not on every step of a drag).
        if self.view == View::Arrangement && !ui.ctx().input(|i| i.pointer.any_down()) && ui.ctx().memory(|m| m.focused().is_none()) {
            self.refit_warped();
        }

        // Drag and drop.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        // Files Finder asks us to open (double-click, Open With, the Dock icon).
        #[cfg(target_os = "macos")]
        for p in crate::menu::take_open_requests() {
            self.request_open(p, &ctx);
        }
        if !dropped.is_empty() {
            #[cfg(target_os = "macos")]
            let drop_pos = ctx.input(|i| i.viewport().inner_rect).and_then(crate::menu::mouse_in_window);
            #[cfg(not(target_os = "macos"))]
            let drop_pos = ctx.input(|i| i.pointer.latest_pos());
            self.handle_drop(dropped, drop_pos, &ctx);
        }
        self.next_queued_drop(&ctx);

        // Ask before quitting with unsaved edits.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        let name = self.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".into());
        let title = if self.tracks.is_empty() { "Sound Edit".into() } else { format!("{name}{} — Sound Edit", if self.dirty { " (edited)" } else { "" }) };
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
        // (Wait for the first layout so the palette has somewhere to go.)
        if self.show_tools && !self.palette_docked && self.desk.is_positive() {
            // Floating palette: drag it by its title bar; the cross hides it (Window ▸ Tools).
            let mut open = true;
            let pos = Pos2::new(self.desk.center().x - 280.0, self.desk.bottom() - 150.0);
            self.place("Tools", pos).title_bar(false).frame(card_frame().inner_margin(egui::Margin::same(12))).resizable(false).show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(small_heading("TOOLS"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if close_button(ui).on_hover_text("Close (Window ▸ Tools brings it back)").clicked() {
                            open = false;
                        }
                    });
                });
                ui.add_space(4.0);
                self.floating_palette(ui);
            });
            self.show_tools &= open;
        }
        if self.show_tools && self.palette_docked {
            egui::Panel::left("tools")
                .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(8, 4)))
                .resizable(false)
                .show(ui, |ui| self.tools_panel(ui));
        }
        if self.show_info || self.show_cues || self.show_regions || self.show_pool {
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
            #[cfg(target_os = "macos")]
            let hover = ctx.input(|i| i.viewport().inner_rect).and_then(crate::menu::mouse_in_window);
            #[cfg(not(target_os = "macos"))]
            let hover = ctx.input(|i| i.pointer.latest_pos());
            if let (true, Some(q), Some((lanes, lane_h))) = (self.view == View::Arrangement && !self.tracks.is_empty(), hover, self.arr_lanes) {
                // Show where the drop will land: the track under the mouse, or a new track.
                ctx.request_repaint();
                match self.drop_target_at(q) {
                    Some((Some(t), _)) => {
                        let r = Rect::from_min_size(Pos2::new(lanes.left(), lanes.top() + t as f32 * lane_h), Vec2::new(lanes.width(), lane_h));
                        painter.rect_filled(r, 0.0, Color32::from_black_alpha(25));
                        painter.rect_stroke(r.shrink(2.0), 4.0, Stroke::new(2.0, INK), egui::StrokeKind::Inside);
                    }
                    Some((None, _)) => {
                        let top = lanes.top() + self.tracks.len() as f32 * lane_h;
                        let r = Rect::from_min_size(Pos2::new(lanes.left(), top), Vec2::new(lanes.width(), lane_h.min(lanes.bottom() - top)));
                        painter.rect_stroke(r.shrink(4.0), 4.0, Stroke::new(2.0, INK), egui::StrokeKind::Inside);
                        painter.text(r.center(), Align2::CENTER_CENTER, "New track", FontId::proportional(16.0), INK);
                    }
                    None => {}
                }
                if lanes.contains(q) {
                    painter.line_segment([Pos2::new(q.x, lanes.top()), Pos2::new(q.x, lanes.bottom())], Stroke::new(1.5, INK));
                }
                self.status = "Drop on a track to add a clip there, or below the tracks for a new track".into();
            } else {
                painter.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
                painter.text(screen.center(), Align2::CENTER_CENTER, "Drop to open", FontId::proportional(24.0), Color32::WHITE);
            }
        }

        self.dialog_window(&ctx);
        self.curve_window(&ctx);
        self.confirm_dialog(&ctx);
        self.run_automation(&ctx);
        self.frame_count += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looping_clip_repeats_its_loop_section() {
        let a = Arc::new(Audio { rate: 10, channels: vec![(0..10).map(|i| i as f32).collect()] });
        let mut c = Clip::new("c", a, 0);
        c.looping = true;
        c.loop_points = Some((4, 8));
        c.len = 16;
        let v = c.visible();
        // Plays 0..8, then repeats 4..8.
        assert_eq!(v.channels[0], vec![0., 1., 2., 3., 4., 5., 6., 7., 4., 5., 6., 7., 4., 5., 6., 7.]);
        // A split-off right half keeps looping from where it is.
        c.trim = 10;
        c.len = 3;
        assert_eq!(c.visible().channels[0], vec![6., 7., 4.]);
    }

    #[test]
    fn reads_tempo_and_bars_from_names() {
        assert_eq!(tempo_in_name("elixir Quekrixn Grukon 118bpm 16bar 2"), (Some(118.0), Some(16.0)));
        assert_eq!(tempo_in_name("BVKER - 909 Kit - Kick Loop 02 - 120 BPM"), (Some(120.0), None));
        assert_eq!(tempo_in_name("Soundiron_VORT_Phrase_Latin_80BPM_D_25"), (Some(80.0), None));
        assert_eq!(tempo_in_name("Bpm130_Am_BrainFreeze_Drums"), (Some(130.0), None));
        assert_eq!(tempo_in_name("Pad 4 bars"), (None, Some(4.0)));
        assert_eq!(tempo_in_name("barrel 2"), (None, None));
        // 16 bars at 118 BPM is 32.5424 s: the bar count gives the exact tempo.
        let t = clip_tempo_from_name("x 118bpm 16bar", (32.542381 * 44100.0) as usize, 44100, 4).unwrap();
        assert!((t - 118.0).abs() < 0.001, "{t}");
    }

    #[test]
    fn warped_clip_keeps_its_beats() {
        let a = Arc::new(Audio { rate: 1000, channels: vec![(0..4000).map(|i| ((i as f32) * 0.05).sin()).collect()] });
        let mut c = Clip::new("loop 120bpm", a, 2000);
        c.warp_from(120.0, 60.0);
        // Half the tempo: twice as long, and it starts twice as late (same beat).
        assert_eq!(c.start, 4000);
        assert!((c.audio.frames() as i64 - 8000).abs() < 50);
        assert!((c.len as i64 - 8000).abs() < 50);
        c.unwarp();
        assert_eq!((c.start, c.len, c.audio.frames()), (2000, 4000, 4000));
    }
}
