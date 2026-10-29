//! Loading, saving and exporting: opening files, project files, export (mix, stems, Ableton,
//! regions, split) and Batch Convert.

use super::*;

/// Output formats for batch conversion: (label, file extension, sample format).
pub(super) const BATCH_FORMATS: [(&str, &str, WavFormat); 6] = [
    ("WAV 16-bit", "wav", WavFormat::Pcm16),
    ("WAV 24-bit", "wav", WavFormat::Pcm24),
    ("WAV 32-bit float", "wav", WavFormat::Float32),
    ("WAV 8-bit", "wav", WavFormat::Pcm8),
    ("AIFF 16-bit", "aiff", WavFormat::Pcm16),
    ("AIFF 24-bit", "aiff", WavFormat::Pcm24),
];

#[derive(Default)]
pub(super) struct BatchProgress {
    pub(super) done: usize,
    pub(super) total: usize,
    pub(super) errors: Vec<String>,
    pub(super) finished: bool,
}

/// Settings and progress for File ▸ Batch Convert.
#[derive(Default)]
pub(super) struct Batch {
    pub(super) files: Vec<PathBuf>,
    pub(super) out_dir: Option<PathBuf>,
    pub(super) format: usize,
    /// 0 keeps each file's own rate.
    pub(super) rate: u32,
    /// 0 keeps each file's own channels.
    pub(super) channels: usize,
    pub(super) normalize: bool,
    pub(super) progress: Option<Arc<std::sync::Mutex<BatchProgress>>>,
}

impl App {
    pub(super) fn request_open(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.dirty {
            self.pending = Some(Pending::Open(path));
        } else {
            self.start_load(path, ctx);
        }
    }

    pub(super) fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "PhotoSound projects and audio",
                &["sndproj", "wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf", "mkv", "webm"],
            )
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(p) = picked {
            self.request_open(p, ctx);
        }
    }

    pub(super) fn start_load(&mut self, path: PathBuf, ctx: &egui::Context) {
        if project::is_project(&path) {
            return self.open_project(path);
        }
        self.start_load_as(path, ctx, false);
    }

    pub(super) fn import_track_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Import a file as a new track")
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf"])
            .pick_file();
        if let Some(p) = picked {
            self.start_load_as(p, ctx, true);
        }
    }

    pub(super) fn import_clip_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Import a file as a clip at the playhead")
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "caf"])
            .pick_file();
        if let Some(p) = picked {
            self.import_as_clip = true;
            self.start_load_as(p, ctx, true);
        }
    }

    pub(super) fn start_load_as(&mut self, path: PathBuf, ctx: &egui::Context, as_track: bool) {
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

    pub(super) fn poll_loading(&mut self) {
        let Some((path, rx, as_track)) = &self.loading else { return };
        let Ok(result) = rx.try_recv() else { return };
        let (path, as_track) = (path.clone(), *as_track);
        self.loading = None;
        let as_clip = std::mem::take(&mut self.import_as_clip);
        if as_track && as_clip {
            match result {
                Ok(a) if a.frames() > 0 => {
                    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Clip".into());
                    let pos = match self.drop_target.take() {
                        Some((Some(t), frame)) if t < self.tracks.len() => {
                            self.set_active(t);
                            self.arr_cursor = frame;
                            self.add_clip(&name, Arc::new(a), "Import Clip")
                        }
                        // Dropped below the tracks (or past the last one): a new track named after the file.
                        Some((_, frame)) => {
                            self.push_undo("Add Track");
                            self.tracks.push(Track::new(name.clone()));
                            self.active = self.tracks.len() - 1;
                            self.arr_cursor = frame;
                            let pos = self.add_clip(&name, Arc::new(a), "Import Clip");
                            // One undo step for both.
                            self.undo.pop();
                            pos
                        }
                        None => self.add_clip(&name, Arc::new(a), "Import Clip"),
                    };
                    let m = audio::read_wav_markers(&path);
                    let c = &mut self.tracks[pos.0].clips[pos.1];
                    (c.cues, c.regions, c.loop_points) = (m.cues, m.regions, m.loop_points);
                    self.status = format!("Added {} as a clip", file_name(&path));
                    self.auto_warp(pos);
                    self.place_over(pos);
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
                    self.auto_warp((self.active, 0));
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
                // A file with its tempo in the name sets the song tempo, and is warped so it
                // follows any later tempo change.
                if let Some(bpm) = clip_tempo_from_name(&name, a.frames(), a.rate, self.beats_per_bar).filter(|_| self.auto_warp) {
                    self.tempo = (bpm * 100.0).round() / 100.0;
                    self.laid_out_tempo = self.tempo;
                    clip.warp = Some(Warp { bpm, source: a.clone(), source_peaks: clip.peaks.clone(), tempo: bpm, repitch: repitch_mode() });
                    clip.fit_to_tempo(self.tempo);
                }
                let mut t = Track::new(name);
                t.clips.push(clip);
                self.tracks = vec![t];
                self.nest.clear();
                self.active = 0;
                self.sel_clip = Some((0, 0));
                self.view = View::Arrangement;
                self.arr_cursor = 0;
                self.project_path = None;
                self.path = Some(path);
                self.audio = Some(self.tracks[0].clips[0].audio.clone());
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

    /// Save the project (⌘S), asking where the first time or for Save As.
    pub(super) fn save_project(&mut self, save_as: bool) -> bool {
        if self.tracks.is_empty() {
            return false;
        }
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let stem = self
                    .project_path
                    .as_ref()
                    .or(self.path.as_ref())
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into());
                let mut d = rfd::FileDialog::new().add_filter("PhotoSound project", &["sndproj"]).set_file_name(format!("{stem}.sndproj"));
                if let Some(dir) = self.project_path.as_ref().or(self.path.as_ref()).and_then(|p| p.parent()) {
                    d = d.set_directory(dir);
                }
                let Some(mut p) = d.save_file() else { return false };
                if !project::is_project(&p) {
                    p.set_extension("sndproj");
                }
                p
            }
        };
        match self.write_project(&path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", path.display());
                remember_project(&path);
                self.project_path = Some(path.clone());
                self.path = Some(path);
                true
            }
            Err(e) => {
                self.status = format!("Couldn't save the project: {e}");
                false
            }
        }
    }

    pub(super) fn write_project(&mut self, path: &Path) -> Result<(), String> {
        self.sync_clip_markers();
        let tracks = self.song_tracks();
        // Each distinct piece of audio is stored once, however many clips use it.
        let mut pool: Vec<Arc<Audio>> = Vec::new();
        fn tracks_data(tracks: &[Track], pool: &mut Vec<Arc<Audio>>) -> Vec<TrackData> {
            tracks
                .iter()
                .map(|t| TrackData {
                    name: t.name.clone(),
                    gain: t.gain,
                    pan: 0.0,
                    color: t.color,
                    mute: t.mute,
                    solo: false,
                    clips: t
                        .clips
                        .iter()
                        .map(|c| ClipData {
                            name: c.name.clone(),
                            audio: if c.group.is_some() {
                                None
                            } else {
                                Some(pool.iter().position(|a| Arc::ptr_eq(a, &c.audio)).unwrap_or_else(|| {
                                    pool.push(c.audio.clone());
                                    pool.len() - 1
                                }))
                            },
                            start: c.start,
                            trim: c.trim,
                            len: c.len,
                            cues: c.cues.iter().map(|q| (q.frame, q.name.clone())).collect(),
                            regions: c.regions.iter().map(|r| (r.start, r.end, r.name.clone())).collect(),
                            loop_points: c.loop_points,
                            looping: c.looping,
                            origin: c.origin,
                            gain: c.gain,
                            fades: (c.fade_in, c.fade_out),
                            group: c.group.as_ref().map(|g| tracks_data(g, pool)),
                            warp: c.warp.as_ref().map(|w| {
                                let i = pool.iter().position(|a| Arc::ptr_eq(a, &w.source)).unwrap_or_else(|| {
                                    pool.push(w.source.clone());
                                    pool.len() - 1
                                });
                                (w.bpm, w.tempo, i)
                            }),
                        })
                        .collect(),
                })
                .collect()
        }
        let data = tracks_data(&tracks, &mut pool);
        let file = ProjectFile {
            version: 1,
            tempo: self.tempo,
            beats_per_bar: self.beats_per_bar,
            grid: self.grid,
            snap: self.snap,
            arr_loop: self.arr_loop,
            locators: self.locators.clone(),
            looping: self.looping,
            master_gain: self.master_gain,
            rate: self.rate(),
            tracks: data,
            audio: pool.iter().map(|a| project::AudioData { rate: a.rate, channels: a.num_channels(), frames: a.frames() }).collect(),
        };
        let refs: Vec<&Audio> = pool.iter().map(|a| a.as_ref()).collect();
        project::write(path, &file, &refs)
    }

    pub(super) fn open_project(&mut self, path: PathBuf) {
        let (file, audio) = match project::read(&path) {
            Ok(x) => x,
            Err(e) => {
                self.status = format!("Couldn't open {}: {e}", file_name(&path));
                return;
            }
        };
        let audio: Vec<Arc<Audio>> = audio.into_iter().map(Arc::new).collect();
        let rate = file.rate;
        (self.tempo, self.beats_per_bar, self.grid, self.snap) = (file.tempo, file.beats_per_bar.max(1), file.grid.min(GRID.len() - 1), file.snap);
        self.laid_out_tempo = self.tempo;
        self.master_gain = file.master_gain;
        self.locators = file.locators.clone();
        (self.arr_loop, self.looping) = (file.arr_loop, file.looping);
        // Older projects could hold a loop that ends part-way through a bar.
        self.arr_loop = self.arr_loop.map(|l| self.whole_bars(l));
        fn build(app: &App, data: &[TrackData], audio: &[Arc<Audio>], rate: u32) -> Vec<Track> {
            data.iter()
                .map(|t| {
                    let mut track = Track::new(t.name.clone());
                    (track.gain, track.color, track.mute) = (t.gain, t.color, t.mute);
                    for c in &t.clips {
                        let a = c.audio.and_then(|i| audio.get(i).cloned()).unwrap_or_else(|| Arc::new(Audio { rate, channels: vec![Vec::new()] }));
                        let mut clip = Clip::new(c.name.clone(), a, c.start);
                        if let Some(g) = &c.group {
                            let inner = build(app, g, audio, rate);
                            app.set_group_content(&mut clip, inner);
                        }
                        clip.trim = c.trim.min(clip.audio.frames());
                        clip.len = c.len.min(clip.audio.frames() - clip.trim);
                        clip.cues = c.cues.iter().map(|(f, n)| Cue { frame: *f, name: n.clone() }).collect();
                        clip.regions = c.regions.iter().map(|(s, e, n)| Region { start: *s, end: *e, name: n.clone() }).collect();
                        clip.loop_points = c.loop_points;
                        clip.looping = c.looping;
                        clip.origin = c.origin;
                        clip.gain = c.gain;
                        (clip.fade_in, clip.fade_out) = c.fades;
                        clip.len = if c.looping { c.len } else { clip.len };
                        if let Some((bpm, tempo, i)) = c.warp {
                            if let Some(source) = audio.get(i) {
                                clip.warp = Some(Warp { bpm, source: source.clone(), source_peaks: Arc::new(Peaks::build(source)), tempo, repitch: false });
                            }
                        }
                        track.clips.push(clip);
                    }
                    track
                })
                .collect()
        }
        self.player.stop();
        // Group mixes are rendered at the project's rate.
        self.audio = Some(Arc::new(Audio { rate, channels: vec![Vec::new(); 2] }));
        self.tracks = Vec::new();
        let tracks = build(self, &file.tracks, &audio, rate);
        self.tracks = tracks;
        self.nest.clear();
        self.undo.clear();
        self.redo.clear();
        self.active = 0;
        self.sel_clip = None;
        self.sel_extra.clear();
        self.view = View::Arrangement;
        self.arr_cursor = 0;
        self.sel = None;
        self.cues.clear();
        self.regions.clear();
        self.loop_points = None;
        self.dirty = false;
        remember_project(&path);
        self.project_path = Some(path.clone());
        self.path = Some(path.clone());
        self.refresh_playback();
        self.arr_fit();
        self.status = format!("Opened project {}", file_name(&path));
        // Warped clips saved in the other warp mode are redone in this one.
        self.stretch_in_background();
    }

    pub(super) fn export(&mut self, selection_only: bool) {
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
                let _ = suffix;
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    // ---------- editing ----------

    /// File ▸ Export Ableton Live Set: a "<name> Project" folder holding a .als set (one audio
    /// track per track, clips in place, loops, tempo) and the samples it plays.
    pub(super) fn export_ableton(&mut self) {
        let tracks = self.song_tracks();
        if tracks.is_empty() {
            return;
        }
        let stem = self
            .project_path
            .as_ref()
            .or(self.path.as_ref())
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let mut d = rfd::FileDialog::new().set_title("Export Ableton Live Set").add_filter("Ableton Live Set", &["als"]).set_file_name(format!("{stem}.als"));
        if let Some(dir) = self.project_path.as_ref().or(self.path.as_ref()).and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        let Some(p) = d.save_file() else { return };
        let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(stem);
        let dir = p.parent().unwrap_or(Path::new(".")).join(format!("{name} Project"));
        match self.write_ableton(&tracks, &dir, &name) {
            Ok(als) => self.status = format!("Exported Ableton Live Set {}", als.display()),
            Err(e) => self.status = format!("Couldn't export the Live Set: {e}"),
        }
    }

    pub(super) fn write_ableton(&self, tracks: &[Track], dir: &Path, name: &str) -> Result<PathBuf, String> {
        // Live's palette indices closest to each track colour (grey, blue, green, amber, red, purple).
        const LIVE_COLORS: [u32; 6] = [27, 22, 19, 15, 14, 24];
        // Live gets what is heard: the layers flattened onto one track.
        let flat = [layers::flatten_clips(tracks, 0, 0)];
        let tracks = &flat[..];
        let samples = dir.join("Samples").join("Imported");
        std::fs::create_dir_all(&samples).map_err(|e| e.to_string())?;
        // This (empty) folder is what makes Live treat the folder as a project, so it finds
        // the samples by their project-relative paths.
        std::fs::create_dir_all(dir.join("Ableton Project Info")).map_err(|e| e.to_string())?;
        let bf = self.beat_frames();
        let beats = |f: usize| f as f64 / bf;
        let mut written: Vec<(*const Audio, String, u64)> = Vec::new();
        let mut live = Vec::new();
        for t in tracks {
            let color = LIVE_COLORS[t.color % LIVE_COLORS.len()];
            let mut clips = Vec::new();
            for c in &t.clips {
                // A warped clip exports its original audio, and Live does the stretching.
                let src = c.warp.as_ref().map_or(&c.audio, |w| &w.source);
                // Each piece of audio is saved once, however many clips play it.
                let key = Arc::as_ptr(src);
                let (file, size) = match written.iter().find(|w| w.0 == key) {
                    Some(w) => (w.1.clone(), w.2),
                    None => {
                        let clean: String = c.name.chars().map(|ch| if ch.is_alphanumeric() || " -_".contains(ch) { ch } else { '_' }).collect();
                        let base = if clean.trim().is_empty() { "Clip".to_string() } else { clean.trim().to_string() };
                        let mut file = format!("{base}.wav");
                        let mut n = 2;
                        while written.iter().any(|w| w.1 == file) {
                            file = format!("{base} {n}.wav");
                            n += 1;
                        }
                        let path = samples.join(&file);
                        audio::save_wav(src, &path, WavFormat::Float32)?;
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        written.push((key, file.clone(), size));
                        (file, size)
                    }
                };
                let rate = c.audio.rate.max(1);
                // Sample positions in beats at the song tempo, the sample playing at its own speed.
                let fit_tempo = c.warp.as_ref().map_or(self.tempo, |w| w.tempo);
                let sb = |f: usize| f as f64 / rate as f64 * fit_tempo / 60.0;
                let (looping, ls, le, trim) = if c.looping {
                    let (ls, le) = c.loop_range();
                    let trim = if c.trim >= le && le > ls { ls + (c.trim - le) % (le - ls) } else { c.trim };
                    (true, ls, le, trim)
                } else {
                    (false, c.trim, c.trim + c.len, c.trim)
                };
                clips.push(ableton::LiveClip {
                    name: c.name.clone(),
                    color,
                    rel_path: format!("Samples/Imported/{file}"),
                    abs_path: samples.join(&file).to_string_lossy().into_owned(),
                    file_size: size,
                    frames: src.frames(),
                    rate,
                    start: beats(c.start),
                    end: beats(c.end()),
                    looping,
                    loop_start: sb(ls),
                    loop_end: sb(le),
                    start_rel: if looping { sb(trim) - sb(ls) } else { 0.0 },
                    sample_beats: src.frames() as f64 / rate as f64 * c.warp.as_ref().map_or(self.tempo, |w| w.bpm) / 60.0,
                });
            }
            live.push(ableton::LiveTrack { name: t.name.clone(), color, gain: t.gain, pan: 0.0, mute: t.mute, solo: false, clips });
        }
        let als = dir.join(format!("{name}.als"));
        ableton::write(&als, self.tempo, self.beats_per_bar, &live)?;
        Ok(als)
    }

    pub(super) fn export_regions(&mut self) {
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

    /// Cut the recording (or the selection) into separate sounds and save each one as
    /// soundNN.wav in a folder the user picks.
    pub(super) fn split_dialog(&mut self) {
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

    pub(super) fn split_to(&mut self, dir: &Path) {
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

    /// File ▸ Batch Convert: convert many files to WAV or AIFF in one go.
    pub(super) fn batch_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_batch;
        let b = &mut self.batch;
        let running = b.progress.as_ref().is_some_and(|p| !p.lock().unwrap().finished);
        let center = self.desk.center();
        egui::Window::new("Batch Convert")
            .title_bar(false)
            .frame(card_frame())
            .default_pos(center - Vec2::new(210.0, 180.0))
            .default_width(420.0)
            .resizable(false)
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if window_header(ui, "Batch Convert") {
                    open = false;
                }
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
                    egui::ComboBox::from_id_salt("bfmt").width(FIELD_W).selected_text(BATCH_FORMATS[b.format].0).show_ui(ui, |ui| {
                        for (i, f) in BATCH_FORMATS.iter().enumerate() {
                            ui.selectable_value(&mut b.format, i, f.0);
                        }
                    });
                    ui.end_row();
                    ui.label("Sample rate");
                    let rates = [0u32, 8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000];
                    let label = |r: u32| if r == 0 { "Keep".to_string() } else { format!("{} Hz", r) };
                    egui::ComboBox::from_id_salt("brate").width(FIELD_W).selected_text(label(b.rate)).show_ui(ui, |ui| {
                        for r in rates {
                            ui.selectable_value(&mut b.rate, r, label(r));
                        }
                    });
                    ui.end_row();
                    ui.label("Channels");
                    let chl = |c: usize| ["Keep", "Mono", "Stereo"][c.min(2)];
                    egui::ComboBox::from_id_salt("bch").width(FIELD_W).selected_text(chl(b.channels)).show_ui(ui, |ui| {
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
}
