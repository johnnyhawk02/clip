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
mod layers;
mod commands;
mod undo;
mod clipruler;
mod tempo;
mod timesel;
mod clipview;
mod locators;

use self::clip::*;
use self::chop::*;
use self::dialogs::*;
use self::files::*;
use self::look::*;
use self::automation::Automation;
use self::autoarrange::{PoolDrag, PoolLoop};
use self::arrange::{LoopGrab, LOOP_H, MAX_TICKS};
use self::editor::EdRuler;

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
    tempo: f64,
    beats_per_bar: u32,
    arr_loop: Option<Sel>,
    locators: Vec<(usize, String)>,
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
    /// A selection box being dragged from empty space: where it started, and the clips that
    /// were already selected (kept when Shift is held).
    /// A time selection in the song view, as in Live: a stretch of time across a run of layers
    /// (first, last). Delete, Split and Loop work on it.
    time_sel: Option<(Sel, (usize, usize))>,
    /// Dragging out a time selection: the frame and layer it started at.
    time_drag: Option<(usize, usize)>,
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
    /// A tempo or time-signature edit in progress (one undo step however long it's dragged).
    song_edit: bool,
    /// Named markers in the song (frame, name), in time order, as Live's locators.
    locators: Vec<(usize, String)>,
    /// A right-click in the ruler: the locator under it (if any) and the frame there.
    loc_menu: Option<(Option<usize>, usize)>,
    /// Dragging a clip's fade handle: (layer, clip, fade in?).
    fade_drag: Option<(usize, usize, bool)>,
    /// The view scrolls along with the playhead (Control ▸ Follow, ⌘F).
    follow: bool,
    /// Where song playback last stopped: ⇧Space carries on from here.
    stopped_at: usize,
    /// A Clip View edit in progress (one undo step per drag or typed value).
    clip_view_edit: bool,
    /// The speed last handed to the player for a tempo still being dragged in.
    live_speed: f64,
    /// Warped clips being re-stretched for a new tempo on another thread.
    stretching: Option<Receiver<(usize, f64, Arc<Audio>, Arc<Peaks>)>>,
    /// Dragging the tempo box: the tempo so far and pixels not yet turned into a step.
    tempo_drag: Option<(f64, f32)>,
    /// The tempo the clips are laid out for: when the tempo changes, every clip moves to keep
    /// its bar position (warped clips also stretch).
    laid_out_tempo: f64,
    /// A fine line across the arrangement every this many bars (0 = none), from Preferences.
    phrase_bars: u32,
    /// Preferences window open, holding the settings to go back to on Cancel.
    prefs_open: Option<(u32, bool)>,
    /// Dragging the loop brace: where it was grabbed (frames), the range then, and which part
    /// (the whole brace moves, an end resizes).
    arr_loop_grab: Option<(f64, Sel, LoopGrab)>,
    /// Dragging the arrangement ruler to zoom: the frame grabbed, the pointer's height then and
    /// the view length then.
    ruler_zoom: Option<(f64, f32, f64)>,
    /// A drag in the clip editor's ruler: zooming, drawing or moving the loop, or the start flag.
    ed_ruler: Option<EdRuler>,
    /// The last right-click in the clip editor was on the start flag (its menu is showing).
    flag_menu: bool,
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
    /// The view before Zoom to Selection, for Zoom Back (in the arrangement or the editor).
    zoom_back: Option<(bool, f64, f64)>,
    /// Headphone crossfeed on playback, remembered between runs.
    crossfeed: bool,
    /// The master channel's volume, applied to the whole mix (and saved in the project).
    master_gain: f32,
    ruler: RulerUnits,
    show_spectrum: bool,
    spectrum_3d: bool,
    /// Cached 3D spectrum slices, keyed by (audio pointer, range).
    spectrum_cache: Option<((usize, Sel), Vec<Vec<f32>>)>,
    show_batch: bool,
    batch: Batch,
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

/// A file whose presence means Headphone Crossfeed was left on.
fn crossfeed_pref() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/Sound Edit/crossfeed-on")
}

/// Clip processes that, with a time selection, work on just the part inside it.
fn on_time_sel(a: Action) -> bool {
    use Action::*;
    matches!(a, ReverseClip | SwapSection | SliceClip | CollapseClip | WarpClip | LoopClip | Copy | Cut | AddToPool | GroupClips)
}

/// A file holding the Preferences setting for the phrase lines (bars between them).
fn phrase_pref() -> PathBuf {
    crossfeed_pref().with_file_name("phrase-bars")
}

/// A file whose presence means warped clips keep their pitch when the tempo changes (stretching)
/// instead of re-pitching.
fn keep_pitch_pref() -> PathBuf {
    crossfeed_pref().with_file_name("keep-pitch")
}

/// A file holding the path of the project last opened or saved, reopened at launch.
fn last_project_pref() -> PathBuf {
    crossfeed_pref().with_file_name("last-project")
}

/// Remember `path` as the project to reopen next launch (scripted test runs leave it alone).
fn remember_project(path: &Path) {
    if std::env::var_os("SOUND_EDIT_ACTIONS").is_some() || std::env::var_os("SOUND_EDIT_SNAPSHOT").is_some() {
        return;
    }
    let pref = last_project_pref();
    if let Some(dir) = pref.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(pref, path.to_string_lossy().as_bytes());
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        clean_style(&cc.egui_ctx);
        let player = Player::new();
        player.set_crossfeed(crossfeed_pref().exists());
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
            time_sel: None,
            time_drag: None,
            sel_extra: Vec::new(),
            nest: Vec::new(),
            import_as_clip: false,
            drop_target: None,
            tempo: 120.0,
            beats_per_bar: 4,
            arr_loop: None,
            arr_loop_drag: None,
            song_edit: false,
            locators: Vec::new(),
            loc_menu: None,
            fade_drag: None,
            follow: true,
            stopped_at: 0,
            clip_view_edit: false,
            live_speed: 1.0,
            stretching: None,
            tempo_drag: None,
            laid_out_tempo: 120.0,
            phrase_bars: std::fs::read_to_string(phrase_pref()).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(4),
            prefs_open: {
                REPITCH.store(!keep_pitch_pref().exists(), std::sync::atomic::Ordering::Relaxed);
                None
            },
            arr_loop_grab: None,
            ruler_zoom: None,
            ed_ruler: None,
            flag_menu: false,
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
            zoom_back: None,
            crossfeed: crossfeed_pref().exists(),
            master_gain: 1.0,
            ruler: RulerUnits::Time,
            show_spectrum: false,
            spectrum_3d: false,
            spectrum_cache: None,
            show_batch: false,
            batch: Batch::default(),
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
        // With nothing asked for, pick up the last project (not in scripted test runs).
        let initial = initial.or_else(|| {
            let p = PathBuf::from(std::fs::read_to_string(last_project_pref()).ok()?.trim());
            ((app.automation.is_none() || std::env::var_os("SOUND_EDIT_REOPEN").is_some()) && p.is_file()).then_some(p)
        });
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
                c.peaks = peaks.clone();
                // An edited warped clip stays warped: the edited audio becomes its new original,
                // made at the tempo it's fitted to now, so it keeps following tempo changes.
                if let Some(w) = c.warp.as_mut() {
                    (w.bpm, w.source, w.source_peaks) = (w.tempo, a.clone(), peaks.clone());
                }
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
            View::Arrangement => {
                // The song is mixed live as it plays: just hand over what to play.
                let song = layers::song(&self.tracks, self.rate(), self.arrangement_frames(), self.master_gain);
                self.player.set_song(Arc::new(song));
                self.play_audio = None;
                return;
            }
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

    /// One track rendered to audio: its clips placed at their positions, crossfading where
    /// they overlap, with a short fade on any other edge that would click.
    fn render_track(&self, t: &Track) -> Audio {
        let rate = self.rate();
        let channels = t.clips.iter().map(|c| c.audio.num_channels()).max().unwrap_or(1);
        let mut out = Audio { rate, channels: vec![Vec::new(); channels] };
        for (c, fades) in t.clips.iter().zip(crossfades(&t.clips)) {
            let mut v = c.visible();
            let fades = with_clip_fades(c, edge_fades(&v, fades));
            apply_crossfade(&mut v, fades);
            out = effects::mix_into(&out, c.start, &v, c.gain);
        }
        out
    }

    /// Mix of all audible tracks at the level being shown (solo wins over mute).
    fn mixdown(&self) -> Audio {
        let mix = self.master(self.render_tracks(&self.tracks));
        if self.auto_gain { auto_gain(mix) } else { mix }
    }

    /// The whole song's mix, whichever group is open.
    fn song_mix(&self) -> Audio {
        let mix = self.master(self.render_tracks(&self.song_tracks()));
        if self.auto_gain { auto_gain(mix) } else { mix }
    }

    /// The master channel's volume on a mix.
    fn master(&self, mut mix: Audio) -> Audio {
        if self.master_gain != 1.0 {
            for ch in &mut mix.channels {
                ch.iter_mut().for_each(|v| *v *= self.master_gain);
            }
        }
        mix
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

    /// What a set of layers sounds like: at each moment only the topmost visible layer with a
    /// clip there is heard.
    fn render_tracks(&self, tracks: &[Track]) -> Audio {
        let layers: Vec<layers::Layer> = tracks
            .iter()
            .filter(|t| !t.mute)
            .map(|t| layers::Layer::of(t, self.render_track(t)))
            .collect();
        layers::flatten(&layers, self.rate())
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










    // ---------- clips ----------

    /// Timeline span covered by the selected clips.
    fn selected_span(&self) -> Option<Sel> {
        if let (View::Arrangement, Some((range, _))) = (self.view, self.time_sel) {
            return Some(range);
        }
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
            // Recordings land on a new top layer, so they are heard over everything else.
            let (t, c) = self.add_clip("Recording", Arc::new(clip), "Record");
            if self.tracks.len() > 1 || self.tracks[t].clips.len() > 1 {
                let c = self.tracks[t].clips.remove(c);
                let mut layer = Track::new("Recording");
                layer.color = self.tracks.len() % TRACK_COLORS.len();
                layer.clips.push(c);
                self.tracks.insert(0, layer);
                self.active = 0;
                self.sel_clip = Some((0, 0));
                self.refresh_playback();
            }
            self.status = format!("Recorded {} on a new top layer", fmt_time(secs));
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
            // As in Live, play starts from the insert marker (where you last clicked) and
            // stopping leaves it there; ⇧Space carries on from where playback stopped.
            if self.player.is_playing() {
                self.stopped_at = self.player.position();
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

    /// Control ▸ Continue Playing (⇧Space): play on from where song playback stopped (or stop).
    fn continue_play(&mut self) {
        if self.view != View::Arrangement {
            return self.toggle_play();
        }
        if self.player.is_playing() {
            self.stopped_at = self.player.position();
            self.player.stop();
            return;
        }
        let end = self.arrangement_frames();
        if self.stopped_at >= end {
            self.stopped_at = 0;
        }
        self.play_origin = self.stopped_at;
        self.player.play(self.stopped_at, end, false);
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
        // A view that isn't a number (a zoom gone wrong) shows the whole sound instead.
        if !self.view_len.is_finite() || !self.view_start.is_finite() {
            (self.view_start, self.view_len) = (0.0, n);
        }
        self.view_len = self.view_len.clamp(32f64.min(n), n);
        self.view_start = self.view_start.clamp(0.0, (n - self.view_len).max(0.0));
    }

    // ---------- UI ----------






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
        self.poll_stretch();
        // A tempo being dragged in is heard straight away: the song re-pitches live until it's
        // laid out for the new tempo.
        let speed = if self.view == View::Arrangement && repitch_mode() { self.tempo / self.laid_out_tempo } else { 1.0 };
        if speed != self.live_speed {
            self.live_speed = speed;
            self.player.set_speed(speed);
        }
        // Warped clips follow the tempo once it's been set (not on every step of a drag).
        if self.view == View::Arrangement && !ui.ctx().input(|i| i.pointer.any_down()) {
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
        let title = if self.tracks.is_empty() { "PhotoSound".into() } else { format!("{name}{} — PhotoSound", if self.dirty { " (edited)" } else { "" }) };
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
        self.prefs_dialog(&ctx);

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
    fn overlapping_clips_crossfade() {
        let a = Arc::new(Audio { rate: 10, channels: vec![vec![1.0; 100]] });
        let (x, y, inner) = (Clip::new("x", a.clone(), 0), Clip::new("y", a.clone(), 80), Clip::new("z", a, 10));
        let mut inner = inner;
        inner.len = 20;
        // x fades out over the 20 frames y overlaps it, y fades in over them; z just sits inside x.
        assert_eq!(crossfades(&[x.clone(), y.clone(), inner]), vec![(0, 20), (20, 0), (0, 0)]);
        let mut v = x.visible();
        apply_crossfade(&mut v, (0, 20));
        let mut w = y.visible();
        apply_crossfade(&mut w, (20, 0));
        // Equal power: the two gains' squares add up to one across the overlap.
        for k in 0..20 {
            let (g, h) = (v.channels[0][80 + k], w.channels[0][k]);
            assert!((g * g + h * h - 1.0).abs() < 0.02, "frame {k}: {g} {h}");
        }
        assert_eq!(v.channels[0][79], 1.0);
    }

    #[test]
    fn edges_fade_only_where_they_would_click() {
        // At 1 kHz the longest edge fade is 4 frames.
        let quiet_start = Audio { rate: 1000, channels: vec![vec![0.0, 0.5, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.5]] };
        assert_eq!(edge_fades(&quiet_start, (0, 0)), (0, 4), "zero crossing at the start, loud cut at the end");
        assert_eq!(edge_fades(&quiet_start, (0, 2)), (0, 2), "a crossfade is left alone");
        let soft = Audio { rate: 1000, channels: vec![vec![0.025; 10]] };
        assert_eq!(edge_fades(&soft, (0, 0)), (2, 2), "half the click level, half the fade");
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
