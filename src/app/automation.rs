//! Scripted runs for headless testing (SOUND_EDIT_ACTIONS / SOUND_EDIT_SNAPSHOT).

use super::*;

/// Scripted self-test, driven by environment variables (used to verify the app headlessly):
/// `SOUND_EDIT_ACTIONS="sel:1.0-2.5,fadeout,normalize"` and `SOUND_EDIT_SNAPSHOT=out.png`.
/// Once the file has loaded, runs the actions, saves a screenshot and quits. A run that hasn't
/// finished after `TIMEOUT` (waiting for a load that never comes, say) gives up and exits with
/// status 2, so a stuck test never keeps running in the background.
pub(super) struct Automation {
    pub(super) actions: Vec<String>,
    pub(super) snapshot: Option<PathBuf>,
    pub(super) frames_after_load: u32,
    pub(super) requested: bool,
    /// Print the native menu bar a few frames later, once its state has synced.
    pub(super) dump_menu: bool,
    pub(super) done: Arc<std::sync::atomic::AtomicBool>,
    pub(super) started: std::time::Instant,
}

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl Automation {
    pub(super) fn from_env() -> Option<Automation> {
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
            started: std::time::Instant::now(),
        })
    }
}

impl App {
    pub(super) fn run_automation(&mut self, ctx: &egui::Context) {
        let Some(mut auto) = self.automation.take() else { return };
        if auto.started.elapsed() > TIMEOUT {
            eprintln!("automation: gave up after {}s ({} actions left)", TIMEOUT.as_secs(), auto.actions.len());
            std::process::exit(2);
        }
        ctx.request_repaint();
        // Wait for loading, background stretching and scripted pointer input to finish before
        // the next step.
        if self.audio.is_none() || self.loading.is_some() || self.stretching.is_some() || !self.synth.is_empty() {
            self.automation = Some(auto);
            return;
        }
        auto.frames_after_load += 1;
        if auto.frames_after_load == 3 {
            let rate = self.audio.as_ref().unwrap().rate as f64;
            let mut queue = std::mem::take(&mut auto.actions).into_iter();
            while let Some(a) = queue.next() {
                let (cmd, arg) = a.split_once(':').unwrap_or((a.as_str(), ""));
                match cmd {
                    // wait: carry on with the rest once loading (a drop, an import) has finished.
                    "wait" => {
                        auto.actions = queue.collect();
                        auto.frames_after_load = 0;
                        break;
                    }
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
                    "tempo" => self.tempo = arg.parse().unwrap_or(self.tempo),
                    // loop:A-B sets the arrangement loop from bar A to bar B (counting from 0) and turns Loop on.
                    "loop" => {
                        let (a, b) = arg.split_once('-').unwrap_or(("0", "1"));
                        let bar = |t: &str| (t.parse::<f64>().unwrap_or(0.0) * self.bar_frames()) as usize;
                        self.arr_loop = Some((bar(a), bar(b)));
                        self.looping = true;
                    }
                    "deselect" => {
                        self.sel_clip = None;
                        self.sel_extra.clear();
                    }
                    // seed:N makes Mangle and BBCut repeatable.
                    "seed" => self.mangle_seed = arg.parse::<u64>().unwrap_or(1) | 1,
                    // rclick:x|y and hover:x|y — pointer events at window point (x, y), a few frames apart.
                    "rclick" | "click" | "hover" => {
                        let (x, y) = arg.split_once('|').unwrap_or(("0", "0"));
                        let pos = Pos2::new(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                        let at = self.synth.last().map_or(self.synth_frame, |l| l.0) + 1;
                        self.synth.push((at, egui::Event::PointerMoved(pos)));
                        if cmd != "hover" {
                            let button = if cmd == "rclick" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                            for (i, pressed) in [(1, true), (2, false)] {
                                self.synth.push((at + i, egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }));
                            }
                        }
                        // Give menus time to open before the next step.
                        let last = self.synth.last().unwrap().0;
                        self.synth.push((last + 8, egui::Event::PointerMoved(pos)));
                    }
                    // drag:x1|y1|x2|y2 presses at one point, moves to the other and lets go there;
                    // draghold:… stops before letting go (to snapshot a drag in progress).
                    "drag" | "draghold" => {
                        let v: Vec<f32> = arg.split('|').map(|n| n.parse().unwrap_or(0.0)).collect();
                        let (from, to) = (Pos2::new(v[0], v[1]), Pos2::new(v[2], v[3]));
                        let button = egui::PointerButton::Primary;
                        let mut at = self.synth.last().map_or(self.synth_frame, |l| l.0) + 1;
                        self.synth.push((at, egui::Event::PointerMoved(from)));
                        self.synth.push((at + 1, egui::Event::PointerButton { pos: from, button, pressed: true, modifiers: Modifiers::NONE }));
                        at += 2;
                        for i in 1..=12 {
                            self.synth.push((at + i, egui::Event::PointerMoved(from + (to - from) * (i as f32 / 12.0))));
                        }
                        at += 14;
                        if cmd == "drag" {
                            self.synth.push((at, egui::Event::PointerButton { pos: to, button, pressed: false, modifiers: Modifiers::NONE }));
                        }
                        self.synth.push((at + 8, egui::Event::PointerMoved(to)));
                    }
                    "clips" => {
                        for (ti, t) in self.tracks.iter().enumerate() {
                            for c in &t.clips {
                                let w = c.warp.as_ref().map(|w| (w.bpm, w.tempo));
                                eprintln!("clip t{ti} {:?} start {} trim {} len {} frames {} warp {:?} tempo {}", c.name, c.start, c.trim, c.len, c.audio.frames(), w, self.tempo);
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
                    "drop" => {
                        // drop:x|y|path|path… — what dropping the files at window point (x, y) does.
                        let parts: Vec<&str> = arg.split('|').collect();
                        let q = Pos2::new(parts[0].parse().unwrap_or(0.0), parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0));
                        let files = parts.iter().skip(2).map(PathBuf::from).collect();
                        self.handle_drop(files, Some(q), ctx);
                    }
                    "als" => {
                        let p = Path::new(arg);
                        let r = self.write_ableton(&self.song_tracks(), p, "Test");
                        self.status = format!("als {arg}: {r:?}");
                    }
                    "stems" => {
                        let r = self.write_stems(&self.song_tracks(), Path::new(arg));
                        self.status = format!("stems {arg}: {r:?}");
                    }
                    "saveproj" => {
                        let r = self.write_project(Path::new(arg));
                        self.status = format!("saveproj {arg}: {r:?}");
                    }
                    "openproj" => self.open_project(PathBuf::from(arg)),
                    "playhead" => self.arr_cursor = (arg.parse::<f64>().unwrap_or(0.0) * rate) as usize,
                    "liveexport" => {
                        // What the live engine plays, rendered start to end (to compare with export).
                        let song = layers::song(&self.tracks, self.rate(), self.arrangement_frames(), self.master_gain);
                        let mut out = vec![[0.0f32; 2]; song.end];
                        song.render(0.0, 1.0, &mut out);
                        let a = Audio { rate: song.rate, channels: vec![out.iter().map(|f| f[0]).collect(), out.iter().map(|f| f[1]).collect()] };
                        let r = audio::save_wav(&a, Path::new(arg), WavFormat::Float32);
                        self.status = format!("liveexport {arg}: {r:?}");
                    }
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
        if auto.frames_after_load >= 30 && ctx.input(|i| i.time) > 2.0 && !auto.requested && self.synth.is_empty() {
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
