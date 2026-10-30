//! The browser, as Live's: a panel on the left with folders of samples. Click a file to hear it
//! (over whatever plays), drag it onto a track or a Session slot, and star files or folders to
//! keep them under Favourites. Places, favourites and open folders are kept between runs.

use super::*;
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use super::session::play_triangle;

/// Files the browser lists (the ones Clip can open).
const AUDIO_EXT: [&str; 11] = ["wav", "mp3", "flac", "ogg", "oga", "m4a", "aac", "aif", "aiff", "caf", "mp4"];
/// Indent per folder level.
const INDENT: f32 = 14.0;
const ROW_H: f32 = 20.0;

/// What a browser row carries while it is dragged onto the tracks.
#[derive(Clone)]
pub(super) struct BrowserDrag(pub(super) PathBuf);

/// What is kept between runs.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    places: Vec<PathBuf>,
    favourites: Vec<PathBuf>,
    open: Vec<PathBuf>,
}

#[derive(Default)]
pub(super) struct Browser {
    places: Vec<PathBuf>,
    favourites: Vec<PathBuf>,
    /// Folders shown open.
    open: HashSet<PathBuf>,
    /// Each open folder's subfolders and audio files, read when it opens.
    listing: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
    pub(super) selected: Option<PathBuf>,
    /// Where the panel was last drawn (a drag released over it goes nowhere).
    pub(super) rect: Option<Rect>,
    /// A preview being decoded, and the one heard.
    loading: Option<(PathBuf, Receiver<Result<Audio, String>>)>,
    playing: Option<PathBuf>,
}

/// A file holding the browser's places and favourites (JSON).
/// (Test runs point SOUND_EDIT_BROWSER at a file of their own.)
fn browser_pref() -> PathBuf {
    std::env::var_os("SOUND_EDIT_BROWSER").map_or_else(|| crossfeed_pref().with_file_name("browser.json"), PathBuf::from)
}

fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str()))
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// A folder's subfolders, then its audio files, each by name (hidden ones left out).
fn read_folder(dir: &Path) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter_map(|e| {
                    let p = e.path();
                    let dir = p.is_dir();
                    (dir || is_audio(&p)).then_some((p, dir))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_cached_key(|(p, dir)| (!dir, name_of(p).to_lowercase()));
    v
}

impl Browser {
    pub(super) fn load() -> Browser {
        let saved: Saved = std::fs::read(browser_pref()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| {
            // The first time: the Mac's Music folder, as a place to start.
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            Saved { places: [home.join("Music")].into_iter().filter(|p| p.is_dir()).collect(), ..Default::default() }
        });
        Browser { places: saved.places, favourites: saved.favourites, open: saved.open.into_iter().collect(), ..Default::default() }
    }

    fn save(&self) {
        let mut open: Vec<PathBuf> = self.open.iter().cloned().collect();
        open.sort();
        let saved = Saved { places: self.places.clone(), favourites: self.favourites.clone(), open };
        if let Ok(json) = serde_json::to_vec_pretty(&saved) {
            let p = browser_pref();
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(p, json);
        }
    }

    pub(super) fn add_place(&mut self, dir: PathBuf) {
        if !self.places.contains(&dir) {
            self.places.push(dir.clone());
        }
        self.open.insert(dir);
        self.save();
    }

    pub(super) fn is_favourite(&self, p: &Path) -> bool {
        self.favourites.iter().any(|f| f == p)
    }

    pub(super) fn toggle_favourite(&mut self, p: &Path) {
        match self.favourites.iter().position(|f| f == p) {
            Some(i) => {
                self.favourites.remove(i);
            }
            None => self.favourites.push(p.to_path_buf()),
        }
        self.save();
    }

    fn toggle_open(&mut self, dir: &Path) {
        if !self.open.remove(dir) {
            self.open.insert(dir.to_path_buf());
        }
        self.listing.remove(dir);
        self.save();
    }
}

/// What a click in the browser asks for, done once the rows are drawn.
enum Do {
    Open(PathBuf),
    Preview(PathBuf),
    Load(PathBuf),
    Favourite(PathBuf),
    RemovePlace(PathBuf),
    Reveal(PathBuf),
}

impl App {
    /// Audition `path` (decoded on a thread; it starts once it's read).
    pub(super) fn preview_file(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (p, ctx) = (path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(audio::load(&p));
            ctx.request_repaint();
        });
        self.browser.loading = Some((path, rx));
    }

    pub(super) fn stop_preview(&mut self) {
        self.browser.loading = None;
        self.browser.playing = None;
        self.player.preview(None);
    }

    /// A decoded preview starts; a finished one is forgotten.
    fn poll_preview(&mut self, ctx: &egui::Context) {
        if let Some((path, rx)) = &self.browser.loading {
            if let Ok(result) = rx.try_recv() {
                let path = path.clone();
                self.browser.loading = None;
                match result {
                    Ok(a) if a.frames() > 1 => {
                        self.player.preview(Some(Arc::new(a)));
                        self.browser.playing = Some(path);
                    }
                    Ok(_) => self.status = format!("{} contains no audio", file_name(&path)),
                    Err(e) => self.status = format!("Couldn't open {}: {e}", file_name(&path)),
                }
            }
        }
        if self.browser.playing.is_some() {
            if self.player.previewing() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            } else {
                self.browser.playing = None;
            }
        }
    }

    /// The browser panel, on the window's left edge.
    pub(super) fn browser_panel(&mut self, ui: &mut egui::Ui) {
        self.poll_preview(ui.ctx());
        let mut todo = None;
        ui.add_space(6.0);
        ui.label(small_heading("FAVOURITES"));
        ui.add_space(2.0);
        if self.browser.favourites.is_empty() {
            ui.label(egui::RichText::new("Star a sample or folder (☆, or right-click) to keep it here.").color(muted()));
        }
        for f in self.browser.favourites.clone() {
            self.browser_row(ui, &f, f.is_dir(), 0, false, &mut todo);
        }
        ui.add_space(10.0);
        ui.label(small_heading("PLACES"));
        ui.add_space(2.0);
        for p in self.browser.places.clone() {
            self.browser_row(ui, &p, true, 0, true, &mut todo);
        }
        ui.add_space(6.0);
        if ui.button("Add Folder…").on_hover_text("Add a folder of samples to Places").clicked() {
            if let Some(dir) = rfd::FileDialog::new().set_title("Add a folder to the browser").pick_folder() {
                self.browser.add_place(dir);
            }
        }
        match todo {
            Some(Do::Open(d)) => self.browser.toggle_open(&d),
            Some(Do::Preview(p)) => {
                self.browser.selected = Some(p.clone());
                // A click on the file being heard stops it, as in Live.
                if self.browser.playing.as_ref() == Some(&p) || self.browser.loading.as_ref().is_some_and(|(l, _)| *l == p) {
                    self.stop_preview();
                } else {
                    self.preview_file(p, ui.ctx());
                }
            }
            Some(Do::Load(p)) => {
                self.stop_preview();
                self.handle_drop(vec![p], None, ui.ctx());
            }
            Some(Do::Favourite(p)) => self.browser.toggle_favourite(&p),
            Some(Do::RemovePlace(p)) => {
                self.browser.places.retain(|x| *x != p);
                self.browser.save();
            }
            Some(Do::Reveal(p)) => {
                let _ = std::process::Command::new("open").arg("-R").arg(&p).spawn();
            }
            None => {}
        }
    }

    /// One file or folder, and (for an open folder) everything in it.
    fn browser_row(&mut self, ui: &mut egui::Ui, path: &Path, dir: bool, depth: usize, place: bool, todo: &mut Option<Do>) {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
        let open = dir && self.browser.open.contains(path);
        let fav = self.browser.is_favourite(path);
        let selected = self.browser.selected.as_deref() == Some(path);
        let heard = self.browser.playing.as_deref() == Some(path) || self.browser.loading.as_ref().is_some_and(|(l, _)| l == path);
        let p = ui.painter();
        if selected {
            p.rect_filled(r, 3.0, gray(212));
        } else if resp.hovered() {
            p.rect_filled(r, 3.0, gray(228));
        }
        let x = r.left() + 4.0 + depth as f32 * INDENT;
        let font = FontId::proportional(12.0);
        if dir {
            p.text(Pos2::new(x + 4.0, r.center().y), Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, font.clone(), muted());
        } else if heard {
            play_triangle(p, Pos2::new(x + 4.0, r.center().y), Color32::from_rgb(40, 200, 60));
        }
        // The star: always on a favourite, else on hover.
        let star = Rect::from_center_size(Pos2::new(r.right() - 10.0, r.center().y), Vec2::splat(16.0));
        let on_star = resp.hover_pos().is_some_and(|q| star.contains(q));
        if fav || resp.hovered() {
            let c = if fav { Color32::from_rgb(230, 160, 30) } else if on_star { ink() } else { gray(170) };
            p.text(star.center(), Align2::CENTER_CENTER, if fav { "★" } else { "☆" }, font.clone(), c);
        }
        let mut job = egui::text::LayoutJob::simple_singleline(name_of(path), font, if dir { ink() } else { gray(40) });
        job.wrap = egui::text::TextWrapping { max_width: (star.left() - x - 14.0).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let g = ui.ctx().fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(Pos2::new(x + 12.0, r.center().y - g.size().y / 2.0), g, ink());
        let resp = resp.on_hover_text(path.to_string_lossy());
        if resp.clicked() && on_star {
            *todo = Some(Do::Favourite(path.to_path_buf()));
        } else if resp.double_clicked() && !dir {
            *todo = Some(Do::Load(path.to_path_buf()));
        } else if resp.clicked() {
            *todo = Some(if dir { Do::Open(path.to_path_buf()) } else { Do::Preview(path.to_path_buf()) });
        }
        // Dragging a file (not a folder) takes it to the tracks.
        if !dir && resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), BrowserDrag(path.to_path_buf()));
        }
        resp.context_menu(|ui| {
            if ui.button(if fav { "Remove from Favourites" } else { "Add to Favourites" }).clicked() {
                *todo = Some(Do::Favourite(path.to_path_buf()));
            }
            if ui.button("Show in Finder").clicked() {
                *todo = Some(Do::Reveal(path.to_path_buf()));
            }
            if place && ui.button("Remove from Places").clicked() {
                *todo = Some(Do::RemovePlace(path.to_path_buf()));
            }
        });
        if open {
            let items = self.browser.listing.entry(path.to_path_buf()).or_insert_with(|| read_folder(path)).clone();
            if items.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(4.0 + (depth + 1) as f32 * INDENT + 12.0);
                    ui.label(egui::RichText::new("No audio files").color(muted()).size(11.0));
                });
            }
            for (p, d) in items {
                self.browser_row(ui, &p, d, depth + 1, false, todo);
            }
        }
    }

    /// A sample dragged out of the browser: its name follows the pointer, the tracks show
    /// where it will land, and on release it goes there (as a file dropped from Finder). Runs
    /// before anything else is drawn: a drop target asking for its own kind of payload on
    /// release would take this one too.
    pub(super) fn browser_drag(&mut self, ctx: &egui::Context) {
        let browser = self.browser.rect.filter(|_| self.show_browser);
        let Some(drag) = egui::DragAndDrop::payload::<BrowserDrag>(ctx) else { return };
        let Some(q) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        let over_browser = browser.is_some_and(|r| r.contains(q));
        if ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ctx);
            let target = self.tracks.is_empty() || self.drop_target_at(q).is_some() || (self.show_session && self.slot_rects.iter().any(|(_, r)| r.contains(q)));
            if !over_browser && target {
                self.handle_drop(vec![drag.0.clone()], Some(q), ctx);
            }
            return;
        }
        if !over_browser && !self.show_session {
            self.drop_highlight(ctx, q);
        }
        if self.show_session {
            if let Some((_, r)) = self.slot_rects.iter().find(|(_, r)| r.contains(q)) {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("browser-slot")));
                painter.rect_stroke(*r, 0.0, Stroke::new(2.0, ink().gamma_multiply(0.5)), egui::StrokeKind::Inside);
            }
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("browser-drag")));
        let g = painter.layout_no_wrap(name_of(&drag.0), FontId::proportional(12.0), ink());
        let r = Rect::from_min_size(q + Vec2::new(12.0, 6.0), g.size() + Vec2::new(12.0, 6.0));
        painter.rect(r, 4.0, gray(250), Stroke::new(1.0, rule()), egui::StrokeKind::Inside);
        painter.galley(r.min + Vec2::new(6.0, 3.0), g, ink());
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}
