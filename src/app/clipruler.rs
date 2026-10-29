//! The clip editor's ruler, as in the song view: a loop strip, drag-to-zoom bar numbers, and
//! the start flag (Live's start marker) with its 1.1.1 point.

use super::*;
use super::editor::WaveArea;

impl App {
    /// Spacing of the finest bar-ruler ticks at least 8 px apart (a sixteenth, beat, bar…).
    pub(super) fn tick_frames(&self, spp: f64) -> f64 {
        let mut g = self.beat_frames() / 4.0;
        while g / spp < 8.0 && g > 0.0 {
            g *= 2.0;
        }
        g
    }

    /// Drags in the clip editor's ruler, as in the song view: the strip on top draws, moves and
    /// resizes the loop (always whole bars); the bar numbers zoom (drag down to zoom in, sideways
    /// to scroll); the start flag sets where 1.1.1 is. Returns whether the ruler has the pointer,
    /// so the tool leaves it alone.
    pub(super) fn clip_ruler(&mut self, ui: &egui::Ui, resp: &egui::Response, a: &WaveArea) -> bool {
        let WaveArea { strip, ruler, wave, spp, n, press, command, .. } = *a;
        let (vs, vl) = (self.view_start, self.view_len);
        let x_of = |f: f64| wave.left() + ((f - vs) / vl) as f32 * wave.width();
        let f_of = |x: f32| (vs + ((x - wave.left()) / wave.width()) as f64 * vl).max(0.0);
        let bar = self.bar_frames();
        let (start, origin) = self.ed_markers();
        // Bar lines count from 1.1.1, wherever that is.
        let to_bar = |f: f64| (((f - origin) / bar).round() * bar + origin).max(0.0) as usize;
        let bar_n = bar.round() as usize;
        let start_x = x_of(match self.ed_ruler {
            Some(EdRuler::Start(f)) => f as f64,
            _ => start as f64,
        });
        // The flag can be grabbed anywhere in the ruler's height, from just left of its line to
        // past the "1" beside it (it often sits right at the left edge).
        let on_start = |q: Pos2| q.y >= ruler.top() && q.y < ruler.bottom() && q.x >= start_x - 4.0 && q.x <= start_x + 16.0;
        let on_cue = |q: Pos2| q.y <= ruler.top() + 14.0 && self.cues.iter().any(|c| (x_of(c.frame as f64) - q.x).abs() < 6.0);
        let brace_at = |q: Pos2| -> Option<LoopGrab> {
            let (ls, le) = self.loop_points?;
            let (x0, x1) = (x_of(ls as f64), x_of(le as f64));
            if !strip.contains(q) || q.x < x0 - 4.0 || q.x > x1 + 4.0 {
                return None;
            }
            let edge = 6.0f32.min((x1 - x0) / 3.0);
            Some(if q.x < x0 + edge { LoopGrab::Start } else if q.x > x1 - edge { LoopGrab::End } else { LoopGrab::Move })
        };
        // Cursor: what a drag here would do.
        if let Some(q) = resp.hover_pos().filter(|q| q.y < ruler.bottom()) {
            let icon = match (self.ed_ruler, brace_at(q)) {
                (Some(EdRuler::Zoom(..)), _) => Some(egui::CursorIcon::ZoomIn),
                (Some(EdRuler::Start(_)) | Some(EdRuler::Loop(_, _, LoopGrab::Start | LoopGrab::End)), _) => Some(egui::CursorIcon::ResizeHorizontal),
                (Some(_), _) | (None, Some(LoopGrab::Move)) => Some(egui::CursorIcon::Grab),
                (None, Some(_)) => Some(egui::CursorIcon::ResizeHorizontal),
                (None, None) if on_start(q) => Some(egui::CursorIcon::ResizeHorizontal),
                (None, None) if strip.contains(q) || on_cue(q) => None,
                (None, None) => Some(egui::CursorIcon::ZoomIn),
            };
            if let Some(i) = icon {
                ui.ctx().set_cursor_icon(i);
            }
        }
        if resp.drag_started() {
            if let Some(q) = press.filter(|q| q.y < ruler.bottom() && !on_cue(*q)) {
                let f = f_of(q.x);
                self.ed_ruler = Some(if on_start(q) {
                    EdRuler::Start(start)
                } else if strip.contains(q) {
                    match (brace_at(q), self.loop_points) {
                        (Some(g), Some(range)) => EdRuler::Loop(f, range, g),
                        _ => EdRuler::Draw(to_bar(f)),
                    }
                } else {
                    EdRuler::Zoom(f, q.y, vl)
                });
            }
        }
        // Right-click the flag for Set 1.1.1 Here.
        if resp.secondary_clicked() {
            self.flag_menu = resp.interact_pointer_pos().is_some_and(on_start);
        }
        if self.flag_menu {
            let mut set = false;
            resp.context_menu(|ui| {
                if ui.button("Set 1.1.1 Here").clicked() {
                    set = true;
                    ui.close();
                }
            });
            if set {
                self.flag_menu = false;
                self.set_origin_here();
            }
        }
        let Some(drag) = self.ed_ruler else { return false };
        if let (true, Some(q)) = (resp.dragged(), resp.interact_pointer_pos()) {
            let f = f_of(q.x);
            match drag {
                EdRuler::Zoom(anchor, y0, len0) => {
                    // The spot grabbed stays under the pointer while the zoom changes around it.
                    self.view_len = len0 * ((y0 - q.y) as f64 / 80.0).exp();
                    self.clamp_view();
                    self.view_start = anchor - ((q.x - wave.left()) / wave.width()) as f64 * self.view_len;
                    self.clamp_view();
                }
                EdRuler::Draw(a0) => {
                    let b = to_bar(f);
                    let b = if b != a0 { b } else if f < a0 as f64 { a0.saturating_sub(bar_n) } else { a0 + bar_n };
                    if b != a0 {
                        self.loop_points = Some((a0.min(b), a0.max(b)));
                        self.looping = true;
                    }
                }
                EdRuler::Loop(at, (ls, le), g) => {
                    let moved = |p: usize| to_bar(p as f64 + f - at);
                    self.loop_points = Some(match g {
                        LoopGrab::Move => {
                            let s = moved(ls);
                            (s, s + (le - ls))
                        }
                        LoopGrab::Start => (moved(ls).min(le.saturating_sub(bar_n)), le),
                        LoopGrab::End => (ls, moved(le).max(ls + bar_n)),
                    });
                }
                EdRuler::Start(_) => {
                    // Snaps to the ruler's ticks (⌘ for anywhere).
                    let g = self.tick_frames(spp);
                    let p = if command || g <= 0.0 { f } else { ((f - origin) / g).round() * g + origin };
                    self.ed_ruler = Some(EdRuler::Start((p.max(0.0) as usize).min(n.saturating_sub(1))));
                }
            }
        }
        if resp.drag_stopped() {
            self.ed_ruler = None;
            match drag {
                EdRuler::Start(m) if m != start => self.set_clip_start(m),
                EdRuler::Draw(_) | EdRuler::Loop(..) => {
                    self.dirty = true;
                    if self.player.is_playing() {
                        self.toggle_play();
                        self.toggle_play();
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// The clip being edited, and where the editor's frame 0 sits in its audio (a trimmed clip
    /// is edited as just its visible part).
    pub(super) fn ed_clip(&self) -> Option<((usize, usize), usize)> {
        let View::Editor { track, clip } = self.view else { return None };
        let c = self.clip((track, clip))?;
        let full = c.looping || c.warp.is_some() || (c.trim == 0 && c.len >= c.audio.frames());
        Some(((track, clip), if full { 0 } else { c.trim }))
    }

    /// The clip's start marker and its 1.1.1 point, in editor frames (1.1.1 can lie before the
    /// part shown).
    pub(super) fn ed_markers(&self) -> (usize, f64) {
        match self.ed_clip().and_then(|(p, off)| Some((self.clip(p)?, off))) {
            Some((c, off)) => (c.trim.saturating_sub(off), c.origin as f64 - off as f64),
            None => (0, 0.0),
        }
    }

    /// Start the clip at editor frame `m`, as Live's start marker does: the waveform stays put and
    /// the clip plays from there, the audio before it coming round at the end (the clip loops).
    pub(super) fn set_clip_start(&mut self, m: usize) {
        let Some(((t, ci), off)) = self.ed_clip() else { return };
        self.push_undo("Move Start");
        let c = &mut self.tracks[t].clips[ci];
        c.trim = (m + off).min(c.audio.frames().saturating_sub(1));
        c.looping = true;
        self.cursor = m;
        self.dirty = true;
        if off != 0 {
            // It was shown trimmed; show the whole audio now that it loops.
            self.edit_clip((t, ci));
        }
        self.status = "Moved the start — the clip plays from the flag and loops round (⌘Z to undo)".into();
    }

    /// Right-click on the start flag ▸ Set 1.1.1 Here: the ruler counts bars from the flag, with
    /// -1, -2… before it.
    pub(super) fn set_origin_here(&mut self) {
        let Some(((t, ci), _)) = self.ed_clip() else { return };
        self.push_undo("Set 1.1.1");
        let c = &mut self.tracks[t].clips[ci];
        c.origin = c.trim;
        self.dirty = true;
        self.status = "1.1.1 is at the start flag now".into();
    }
}
