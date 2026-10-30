//! Locators, as in Live: named markers in the song's ruler to jump between and loop.

use super::*;

impl App {
    /// Control ▸ Add Locator: a new marker at `at` (the insert marker, from the menu).
    pub(super) fn add_locator(&mut self, at: usize) {
        if self.locators.iter().any(|l| l.0 == at) {
            return;
        }
        self.push_undo("Add Locator");
        let n = (1..).find(|i| !self.locators.iter().any(|l| l.1 == format!("Locator {i}"))).unwrap();
        self.locators.push((at, format!("Locator {n}")));
        self.locators.sort_by_key(|l| l.0);
        self.dirty = true;
        self.status = format!("Added Locator {n} — right-click it to rename");
    }

    /// Control ▸ Previous / Next Locator ([ and ]): move the insert marker (and playback) there.
    pub(super) fn jump_locator(&mut self, forward: bool) {
        let here = if self.player.is_playing() { self.player.position() } else { self.arr_cursor };
        let to = if forward {
            self.locators.iter().map(|l| l.0).find(|&f| f > here)
        } else {
            // A little grace so a second press while playing steps back past the one just left.
            self.locators.iter().map(|l| l.0).filter(|&f| f + self.rate() as usize / 4 < here).last()
        };
        if let Some(f) = to {
            self.go_to_frame(f);
        }
    }

    /// Put the insert marker at `f`, and carry playback on from there if it's playing.
    pub(super) fn go_to_frame(&mut self, f: usize) {
        self.arr_cursor = f;
        self.play_origin = f;
        if self.player.is_playing() {
            let end = self.arrangement_frames();
            self.player.play(f, end, false);
        }
    }

    /// The locator whose flag or name is at window x `x`, if any.
    pub(super) fn locator_at(&self, x: f32, f_to_x: &dyn Fn(f64) -> f32) -> Option<usize> {
        self.locators.iter().rposition(|l| {
            let lx = f_to_x(l.0 as f64);
            x >= lx - 5.0 && x <= lx + 12.0 + 6.0 * l.1.chars().count() as f32
        })
    }

    /// The ruler's right-click menu: rename, loop or delete the locator under the pointer, or
    /// add one there.
    pub(super) fn locator_menu(&mut self, ui: &mut egui::Ui, (hit, at): (Option<usize>, usize)) {
        match hit.filter(|&i| i < self.locators.len()) {
            Some(i) => {
                let mut name = self.locators[i].1.clone();
                let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
                if edit.changed() {
                    if !self.song_edit {
                        self.push_undo("Rename Locator");
                        self.song_edit = true;
                    }
                    self.locators[i].1 = name;
                    self.dirty = true;
                }
                if edit.lost_focus() {
                    self.song_edit = false;
                }
                ui.separator();
                if ui.button("Loop to Next Locator").clicked() {
                    let end = self.locators.get(i + 1).map_or(self.arrangement_frames(), |l| l.0);
                    if end > self.locators[i].0 {
                        self.push_undo("Set Loop");
                        self.arr_loop = Some(self.whole_bars((self.locators[i].0, end)));
                        self.looping = true;
                        self.dirty = true;
                    }
                    ui.close();
                }
                if ui.button("Delete Locator").clicked() {
                    self.push_undo("Delete Locator");
                    self.locators.remove(i);
                    self.dirty = true;
                    ui.close();
                }
            }
            None => {
                if ui.button("Add Locator Here").clicked() {
                    self.add_locator(at);
                    ui.close();
                }
            }
        }
    }

    /// Delete the selected locator (Delete key).
    pub(super) fn delete_locator(&mut self) {
        let Some(i) = self.selected_locator.take().filter(|&i| i < self.locators.len()) else { return };
        self.push_undo("Delete Locator");
        self.locators.remove(i);
        self.dirty = true;
    }

    /// What Edit ▸ Rename (⌘R) would rename now: the selected clip (a Session clip in the Session
    /// view), else the selected locator, as in Live.
    pub(super) fn rename_target(&self) -> Option<Renaming> {
        let clip = if self.show_session && self.view == View::Arrangement && self.nest.is_empty() {
            self.sel_slot.map(|(t, s)| (t, SLOT + s))
        } else {
            self.sel_clip
        };
        clip.filter(|&p| self.clip(p).is_some())
            .map(Renaming::Clip)
            .or(self.selected_locator.filter(|&i| i < self.locators.len()).map(Renaming::Locator))
    }

    /// Edit ▸ Rename (⌘R): a small box to type the selected clip's or locator's new name.
    pub(super) fn rename_locator_dialog(&mut self, ctx: &egui::Context) {
        let Some((what, mut name)) = self.loc_rename.take() else { return };
        let mut ok = false;
        let mut cancel = false;
        let resp = dialog_modal("rename-locator").show(ctx, |ui| {
            ui.set_width(300.0);
            dialog_header(ui, if matches!(what, Renaming::Clip(_)) { "Rename Clip" } else { "Rename Locator" }, "");
            let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
            edit.request_focus();
            ok = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let (b_ok, b_cancel) = dialog_buttons(ui, "Rename", !name.trim().is_empty(), |_| {});
            ok |= b_ok;
            cancel = b_cancel;
        });
        if cancel || resp.should_close() {
            return;
        }
        if ok {
            let name = name.trim().to_string();
            match what {
                _ if name.is_empty() => {}
                Renaming::Clip(p) if self.clip(p).is_some() => {
                    self.push_undo("Rename Clip");
                    if let Some(c) = self.clip_mut(p) {
                        c.name = name;
                    }
                    self.dirty = true;
                }
                Renaming::Locator(i) if i < self.locators.len() => {
                    self.push_undo("Rename Locator");
                    self.locators[i].1 = name;
                    self.dirty = true;
                }
                _ => {}
            }
            return;
        }
        self.loc_rename = Some((what, name));
    }
}
