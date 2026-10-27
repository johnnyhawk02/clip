//! The editor window: tool palette, waveform view with selection, status bar, menus and shortcuts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self, Align2, Button, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2,
};

use crate::actions::{self, Action};
use crate::audio::{self, Audio, Peaks, WavFormat};
use crate::menu::NativeMenu;
use crate::player::Player;

type Sel = (usize, usize);

/// Undo history is capped by memory rather than step count, since each step is a full copy.
const UNDO_BUDGET_BYTES: usize = 2 << 30;

struct Snapshot {
    audio: Arc<Audio>,
    sel: Option<Sel>,
    cursor: usize,
    label: &'static str,
}

/// A small dialog asking for an effect's setting before applying it.
#[derive(Clone, Copy, PartialEq)]
enum ParamDialog {
    Normalize,
    Gain,
    Split,
}

enum Pending {
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
    normalize_db: f32,
    gain_db: f32,
    split_gap: usize,

    status: String,
    loading: Option<(PathBuf, Receiver<Result<Audio, String>>)>,
    pending: Option<Pending>,
    allow_close: bool,
    automation: Option<Automation>,
    menu: NativeMenu,
    param: Option<ParamDialog>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
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
            normalize_db: -0.1,
            gain_db: 3.0,
            split_gap: 10,
            status,
            loading: None,
            pending: None,
            allow_close: false,
            automation: Automation::from_env(),
            menu: NativeMenu::install(&cc.egui_ctx),
            param: None,
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
        match audio::save_wav(&a, &out, self.wav_format) {
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
        self.undo.push(Snapshot { audio: a, sel: self.sel, cursor: self.cursor, label });
        self.redo.clear();
        // Drop oldest steps beyond the memory budget (always keep at least one).
        let mut total: usize = self.undo.iter().map(|s| s.audio.bytes()).sum();
        while total > UNDO_BUDGET_BYTES && self.undo.len() > 1 {
            total -= self.undo.remove(0).audio.bytes();
        }
    }

    fn apply(&mut self, label: &'static str, new: Audio, sel: Option<Sel>) {
        self.push_undo(label);
        self.sel = sel;
        let was_playing = self.player.is_playing();
        self.set_audio(Arc::new(new));
        if was_playing && self.player.position() >= self.frames() {
            self.player.stop();
        }
        self.dirty = true;
        self.status = format!("{label} — ⌘Z to undo");
    }

    fn undo(&mut self) {
        let Some(snap) = self.undo.pop() else { return };
        let cur = self.audio.clone().unwrap();
        self.redo.push(Snapshot { audio: cur, sel: self.sel, cursor: self.cursor, label: snap.label });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.set_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Undid {}", snap.label);
    }

    fn redo(&mut self) {
        let Some(snap) = self.redo.pop() else { return };
        let cur = self.audio.clone().unwrap();
        self.undo.push(Snapshot { audio: cur, sel: self.sel, cursor: self.cursor, label: snap.label });
        self.sel = snap.sel;
        self.cursor = snap.cursor;
        self.set_audio(snap.audio);
        self.dirty = true;
        self.status = format!("Redid {}", snap.label);
    }

    fn trim(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.cursor = 0;
        self.apply("Trim to selection", a.crop(s, e), None);
        self.zoom_fit();
    }

    fn delete(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        if e - s >= a.frames() {
            self.status = "Can't delete everything — use Trim instead".into();
            return;
        }
        self.cursor = s;
        self.apply("Delete selection", a.delete(s, e), None);
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
                self.apply("Cut", a.empty_like(), None);
            } else {
                self.apply("Cut", a.delete(s, e), None);
            }
        }
    }

    fn paste(&mut self) {
        let (Some(a), Some(clip)) = (self.audio.clone(), self.clipboard.clone()) else { return };
        let (at, end) = self.sel.unwrap_or((self.cursor, self.cursor));
        let new = a.insert(at, end, &clip);
        let pasted_len = new.frames() + (end - at) - a.frames();
        self.cursor = at;
        self.apply("Paste", new, Some((at, at + pasted_len)));
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
        match a.normalize(s, e, self.normalize_db) {
            Some(n) => self.apply("Normalize", n, self.sel),
            None => self.status = "Nothing to normalize — range is silent".into(),
        }
    }

    fn gain(&mut self, db: f32) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Gain", a.gain(s, e, 10f32.powf(db / 20.0)), self.sel);
    }

    fn silence(&mut self) {
        let (Some(a), Some((s, e))) = (self.audio.clone(), self.sel) else { return };
        self.apply("Silence", a.silence(s, e), self.sel);
    }

    fn reverse(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        self.apply("Reverse", a.reverse(s, e), self.sel);
    }

    fn to_mono(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        if a.num_channels() > 1 {
            self.apply("Convert to mono", a.to_mono(), self.sel);
        }
    }

    /// Cut the recording (or the selection) into separate sounds and save each one as
    /// soundNN.wav in a folder the user picks.
    fn split_dialog(&mut self) {
        let Some(a) = self.audio.clone() else { return };
        let (s, e) = self.range_or_all();
        let count = a.slice(s, e).find_sounds(self.split_gap).len();
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
        let sounds = src.find_sounds(self.split_gap);
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
        match a {
            Action::Open | Action::Quit => true,
            Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Loop => true,
            Action::Undo => !self.undo.is_empty(),
            Action::Redo => !self.redo.is_empty(),
            Action::Paste => has && self.clipboard.is_some(),
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
            _ => has,
        }
    }

    fn checked(&self, a: Action) -> bool {
        match a {
            Action::Loop => self.looping,
            Action::FormatPcm16 => self.wav_format == WavFormat::Pcm16,
            Action::FormatPcm24 => self.wav_format == WavFormat::Pcm24,
            Action::FormatFloat32 => self.wav_format == WavFormat::Float32,
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
            Action::Open => self.open_dialog(ctx),
            Action::Export => self.export(false),
            Action::ExportSelection => self.export(true),
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
            Action::Delete => self.delete(),
            Action::SelectAll => self.sel = Some((0, self.frames())),
            Action::SelectNone => self.sel = None,
            Action::Trim => self.trim(),
            Action::Silence => self.silence(),
            Action::FadeIn => self.fade(true),
            Action::FadeOut => self.fade(false),
            Action::Normalize => self.param = Some(ParamDialog::Normalize),
            Action::Gain => self.param = Some(ParamDialog::Gain),
            Action::Reverse => self.reverse(),
            Action::ToMono => self.to_mono(),
            Action::Split => self.param = Some(ParamDialog::Split),
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
            (none, Key::Z, Action::ZoomSel),
            (none, Key::L, Action::Loop),
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

    /// The row of icon buttons under the menu bar.
    fn palette(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for slot in actions::PALETTE {
                let Some(a) = *slot else {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    continue;
                };
                let playing = a == Action::PlayPause && self.player.is_playing();
                let icon = if playing { "⏸" } else { a.icon() };
                let mut tip = if playing { "Pause".to_string() } else { self.menu_label(a).trim_end_matches('…').to_string() };
                if let Some(k) = a.shortcut() {
                    tip += &format!("  ({k})");
                }
                if !a.help().is_empty() {
                    tip += &format!("\n{}", a.help());
                }
                let drawn = has_drawn_icon(a);
                let btn = Button::new(egui::RichText::new(if drawn { "" } else { icon }).size(17.0))
                    .min_size(Vec2::new(34.0, 30.0))
                    .selected(self.checked(a));
                let enabled = self.enabled(a);
                let resp = ui.add_enabled(enabled, btn);
                if drawn {
                    let color = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
                    paint_icon(ui.painter(), resp.rect, a, color);
                }
                if resp.on_hover_text(tip).on_disabled_hover_text(a.label()).clicked() {
                    self.run(a, &ctx);
                }
            }
        });
    }

    fn param_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.param else { return };
        let scope = if self.sel.is_some() { "the selection" } else { "the whole file" };
        let mut apply = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::unique("param")).show(ctx, |ui| {
            ui.set_width(340.0);
            match kind {
                ParamDialog::Normalize => {
                    ui.heading("Normalize");
                    ui.label(format!("Scale {scope} so its loudest peak reaches:"));
                    ui.add(egui::DragValue::new(&mut self.normalize_db).range(-24.0..=0.0).speed(0.1).suffix(" dB"));
                }
                ParamDialog::Gain => {
                    ui.heading("Gain");
                    ui.label(format!("Make {scope} louder or quieter by:"));
                    ui.add(egui::DragValue::new(&mut self.gain_db).range(-48.0..=24.0).speed(0.1).suffix(" dB"));
                }
                ParamDialog::Split => {
                    ui.heading("Split into sounds");
                    ui.label(format!(
                        "Save every separate sound in {scope} as sound01.wav, sound02.wav… \
                         ({}). A sound starts at the first non-zero sample and ends after this many zero samples in a row:",
                        self.wav_format.label()
                    ));
                    ui.add(egui::DragValue::new(&mut self.split_gap).range(1..=100_000).suffix(" zeros"));
                    if let Some(a) = &self.audio {
                        let (s, e) = self.sel.unwrap_or((0, a.frames()));
                        let n = a.slice(s, e).find_sounds(self.split_gap).len();
                        ui.label(format!("{n} sound{} found.", if n == 1 { "" } else { "s" }));
                    }
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = if kind == ParamDialog::Split { "Choose Folder…" } else { "Apply" };
                apply = ui.button(ok).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        apply |= ctx.input(|i| i.key_pressed(Key::Enter));
        if cancel || resp.should_close() {
            self.param = None;
        } else if apply {
            self.param = None;
            match kind {
                ParamDialog::Normalize => self.normalize(),
                ParamDialog::Gain => self.gain(self.gain_db),
                ParamDialog::Split => self.split_dialog(),
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(a) = &self.audio {
                let name = self.path.as_deref().map(file_name).unwrap_or_default();
                let dirty = if self.dirty { " •" } else { "" };
                ui.strong(format!("{name}{dirty}"));
                ui.label(format!("{} Hz · {} ch · {}", a.rate, a.num_channels(), fmt_time(a.duration_secs())));
                ui.separator();
                let r = a.rate as f64;
                match self.sel {
                    Some((s, e)) => ui.label(format!(
                        "Selection {} – {} ({})",
                        fmt_time(s as f64 / r),
                        fmt_time(e as f64 / r),
                        fmt_time((e - s) as f64 / r)
                    )),
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
        let Some(audio) = self.audio.clone() else {
            let rect = ui.available_rect_before_wrap();
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                if self.loading.is_some() { "Loading…" } else { "Drop an audio file here\n(WAV, MP3, FLAC, OGG, M4A, AIFF…)" },
                FontId::proportional(22.0),
                Color32::GRAY,
            );
            return;
        };
        let n = audio.frames();
        let rate = audio.rate as f64;

        let full = ui.available_rect_before_wrap();
        let ruler = Rect::from_min_size(full.min, Vec2::new(full.width(), 20.0));
        let scroll = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - 14.0), full.max);
        let wave = Rect::from_min_max(Pos2::new(full.left(), ruler.bottom()), Pos2::new(full.right(), scroll.top() - 2.0));

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

        let shift = ui.input(|i| i.modifiers.shift);
        if resp.drag_started() {
            // Anchor at where the button went down, not where the drag was recognised
            // (egui only reports a drag after the pointer has moved a few pixels).
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(p) = origin.or(resp.interact_pointer_pos()) {
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
                let min_frames = (self.view_len / wave.width() as f64 * 2.0) as usize;
                if e - s > min_frames {
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
        if resp.double_clicked() {
            self.sel = Some((0, n));
        }

        // --- scrollbar ---
        let sresp = ui.allocate_rect(scroll, Sense::click_and_drag());
        let thumb_w = (self.view_len / n.max(1) as f64 * scroll.width() as f64).max(12.0) as f32;
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
            if self.drag_anchor.is_none() && !self.scrollbar_drag {
                let p = play_pos as f64;
                if p > self.view_start + self.view_len || p < self.view_start {
                    self.view_start = p - self.view_len * 0.05;
                    self.clamp_view();
                }
            }
        }

        // --- painting ---
        let painter = ui.painter_at(full);
        let vs = self.view_start;
        let vl = self.view_len;
        let frame_to_x = |f: f64| wave.left() + ((f - vs) / vl) as f32 * wave.width();

        painter.rect_filled(wave, 0.0, Color32::from_gray(18));
        painter.rect_filled(ruler, 0.0, Color32::from_gray(30));
        painter.rect_filled(scroll, 3.0, Color32::from_gray(35));
        painter.rect_filled(thumb, 3.0, if self.scrollbar_drag { Color32::from_gray(140) } else { Color32::from_gray(95) });

        // Time ruler with "nice" tick spacing (~90 px apart).
        let secs_per_px = vl / rate / wave.width() as f64;
        let nice = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = *nice.iter().find(|&&s| s / secs_per_px >= 90.0).unwrap_or(&1200.0);
        let t0 = (vs / rate / step).floor() * step;
        let mut t = t0;
        while t <= (vs + vl) / rate {
            let x = frame_to_x(t * rate);
            if x >= wave.left() {
                painter.line_segment([Pos2::new(x, ruler.bottom() - 6.0), Pos2::new(x, ruler.bottom())], Stroke::new(1.0, Color32::GRAY));
                painter.line_segment([Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, Color32::from_gray(28)));
                painter.text(Pos2::new(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, fmt_tick(t, step), FontId::monospace(11.0), Color32::LIGHT_GRAY);
            }
            t += step;
        }

        // Selection background.
        if let Some((s, e)) = self.sel {
            let x0 = frame_to_x(s as f64).max(wave.left());
            let x1 = frame_to_x(e as f64).min(wave.right());
            if x1 > x0 {
                let r = Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, wave.bottom()));
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(70, 130, 220, 70));
            }
        }

        // Waveforms, one lane per channel.
        let peaks = self.peaks.as_ref().unwrap();
        let n_ch = audio.num_channels();
        let lane_h = wave.height() / n_ch as f32;
        let spp = vl / wave.width() as f64; // frames per pixel
        let wave_col = Color32::from_rgb(90, 200, 170);
        let sel_col = Color32::from_rgb(170, 230, 255);
        for (ci, ch) in audio.channels.iter().enumerate() {
            let top = wave.top() + ci as f32 * lane_h;
            let mid = top + lane_h / 2.0;
            let amp = lane_h / 2.0 * 0.92;
            painter.line_segment([Pos2::new(wave.left(), mid), Pos2::new(wave.right(), mid)], Stroke::new(1.0, Color32::from_gray(50)));
            if ci > 0 {
                painter.line_segment([Pos2::new(wave.left(), top), Pos2::new(wave.right(), top)], Stroke::new(1.0, Color32::from_gray(60)));
            }
            let color_at = |f: usize| match self.sel {
                Some((s, e)) if f >= s && f < e => sel_col,
                _ => wave_col,
            };
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
                        painter.circle_filled(p, 2.0, color_at(i));
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

        // Selection edges.
        if let Some((s, e)) = self.sel {
            for f in [s, e] {
                let x = frame_to_x(f as f64);
                if x >= wave.left() && x <= wave.right() {
                    painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, wave.bottom())], Stroke::new(1.0, Color32::from_rgb(110, 170, 255)));
                }
            }
        }

        // Edit cursor and playhead.
        let cx = frame_to_x(self.cursor as f64);
        if self.sel.is_none() && cx >= wave.left() && cx <= wave.right() {
            painter.line_segment([Pos2::new(cx, ruler.top()), Pos2::new(cx, wave.bottom())], Stroke::new(1.0, Color32::WHITE));
        }
        if playing {
            let px = frame_to_x(play_pos as f64);
            if px >= wave.left() && px <= wave.right() {
                painter.line_segment([Pos2::new(px, ruler.top()), Pos2::new(px, wave.bottom())], Stroke::new(2.0, Color32::from_rgb(255, 140, 60)));
            }
        }

        // Hover time readout.
        if let Some(p) = resp.hover_pos() {
            let f = x_to_frame(p.x, vs, vl);
            painter.text(
                Pos2::new(wave.right() - 6.0, wave.top() + 4.0),
                Align2::RIGHT_TOP,
                fmt_time(f as f64 / rate),
                FontId::monospace(12.0),
                Color32::LIGHT_GRAY,
            );
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else { return };
        let what = match pending {
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

        // Window title.
        let title = match &self.path {
            Some(p) => format!("{}{} — Sound Edit", file_name(p), if self.dirty { " (edited)" } else { "" }),
            None => "Sound Edit".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        for a in self.menu.poll() {
            if self.pending.is_none() && self.param.is_none() {
                self.run(a, &ctx);
            } else if a == Action::Quit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self.pending.is_none() && self.param.is_none() {
            self.handle_shortcuts(&ctx);
        }
        self.menu.sync(|a| self.enabled(a), |a| self.checked(a), |a| self.menu_label(a));

        egui::Panel::top("palette").show(ui, |ui| {
            ui.add_space(4.0);
            self.palette(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(2.0);
            self.status_bar(ui);
            ui.add_space(2.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.waveform(ui);
        });

        // Highlight when a file is dragged over the window.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(160));
            painter.text(screen.center(), Align2::CENTER_CENTER, "Drop to open", FontId::proportional(28.0), Color32::WHITE);
        }

        self.param_dialog(&ctx);
        self.confirm_dialog(&ctx);
        self.run_automation(&ctx);
    }
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
        if auto.frames_after_load >= 30 && !auto.requested {
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

fn has_drawn_icon(a: Action) -> bool {
    matches!(a, Action::Trim | Action::Copy | Action::Normalize | Action::ZoomSel | Action::ZoomFit | Action::Split)
}

/// Vector icons for palette tools that have no suitable glyph in the bundled fonts.
fn paint_icon(p: &egui::Painter, rect: Rect, a: Action, color: Color32) {
    let c = rect.center();
    let st = Stroke::new(1.6, color);
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        p.line_segment([c + Vec2::new(x0, y0), c + Vec2::new(x1, y1)], st);
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
    match a {
        Action::Trim => {
            brackets(8.0);
            for (x, h) in [(-3.0, 3.0), (0.0, 6.0), (3.0, 4.0)] {
                line(x, -h, x, h);
            }
            for x in [-12.0f32, 12.0] {
                line(x, -1.0, x, 1.0);
            }
        }
        Action::Copy => {
            let r1 = Rect::from_center_size(c + Vec2::new(-2.5, -2.5), Vec2::new(10.0, 12.0));
            let r2 = r1.translate(Vec2::new(5.0, 5.0));
            p.rect_stroke(r1, 1.5, st, egui::StrokeKind::Middle);
            p.rect_filled(r2, 1.5, p.ctx().global_style().visuals.widgets.inactive.weak_bg_fill);
            p.rect_stroke(r2, 1.5, st, egui::StrokeKind::Middle);
        }
        Action::Normalize => {
            line(-9.0, -8.0, 9.0, -8.0);
            line(-9.0, 8.0, 9.0, 8.0);
            for (x, h) in [(-6.0, 3.0), (-2.0, 6.0), (2.0, 4.0), (6.0, 2.0)] {
                line(x, -h, x, h);
            }
        }
        Action::ZoomSel => {
            brackets(10.0);
            arrow_h(-6.0, 6.0);
        }
        Action::ZoomFit => {
            line(-10.0, -7.0, -10.0, 7.0);
            line(10.0, -7.0, 10.0, 7.0);
            arrow_h(-8.0, 8.0);
        }
        Action::Split => {
            for x in [-5.0f32, 5.0] {
                for y in [-8.0f32, -3.0, 2.0, 7.0] {
                    line(x, y, x, y + 2.5);
                }
            }
            for (x, h) in [(-10.0, 5.0), (-8.0, 3.0), (-1.0, 6.0), (1.0, 4.0), (8.0, 4.0), (10.0, 6.0)] {
                line(x, -h, x, h);
            }
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
