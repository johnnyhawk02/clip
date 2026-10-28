//! The panels around the main view: the Tools palette, transport bar, Info/Cues/Regions side
//! panel and status bar.

use super::*;

impl App {
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
    pub(super) fn place(&mut self, name: &'static str, pos: Pos2) -> egui::Window<'static> {
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

    /// The floating palette: a strip of the tools and commands, then the mangler's buttons.
    pub(super) fn floating_palette(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for row in actions::PALETTE {
                match row {
                    Some(pair) => {
                        for a in pair {
                            self.palette_button(ui, *a, Vec2::new(30.0, 26.0));
                        }
                    }
                    None => {
                        ui.add_space(8.0);
                    }
                }
            }
        });
        ui.add_space(4.0);
        let small = |t: &str| egui::RichText::new(t).size(12.0);

        let buttons: [(&str, Action); 7] = [
            ("Shuffle", Action::MangleShuffle),
            ("Reverse", Action::MangleReverse),
            ("Swap", Action::MangleSwap),
            ("Stutter", Action::MangleStutter),
            ("Flip", Action::MangleRandomReverse),
            ("Drop", Action::MangleDropOut),
            ("Again", Action::MangleAgain),
        ];
        let styles: [(&str, Action); 8] = [
            ("Jungle", Action::MangleBBCut),
            ("House", Action::MangleCutHouse),
            ("Hip-Hop", Action::MangleCutHipHop),
            ("Dembow", Action::MangleCutDembow),
            ("Footwork", Action::MangleCutFootwork),
            ("Trap", Action::MangleCutTrap),
            ("Breakcore", Action::MangleCutBreakcore),
            ("Halftime", Action::MangleCutHalftime),
        ];
        let ctx = ui.ctx().clone();
        let mut run = None;
        let mut row = |ui: &mut egui::Ui, title: &str, items: &[(&str, Action)], this: &Self| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_sized(Vec2::new(54.0, 22.0), egui::Label::new(small_heading(title)));
                for (label, a) in items {
                    let b = ui.add_enabled(this.enabled(*a), Button::new(small(label)).min_size(Vec2::new(0.0, 22.0)));
                    if b.on_hover_text(this.menu_label(*a)).clicked() {
                        run = Some(*a);
                    }
                }
            });
        };
        row(ui, "MANGLE", &buttons, self);
        row(ui, "BBCUT", &styles, self);
        if let Some(a) = run {
            self.run(a, &ctx);
        }
        }

    /// Tools column on the left: editing tools on top, common commands below.
    pub(super) fn tools_panel(&mut self, ui: &mut egui::Ui) {
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
    pub(super) fn update_meters(&mut self, ctx: &egui::Context) {
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
    pub(super) fn transport_panel(&mut self, ui: &mut egui::Ui) {
        let playing = self.player.is_playing();
        let pos = self.player.position();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for a in [Action::GoStart, Action::PlayPause, Action::Stop, Action::Loop, Action::GoEnd] {
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

    /// Right-hand side panel with the Info, Cues, Regions and Loop Pool sections.
    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui) {
        let section = |ui: &mut egui::Ui, title: &str, first: bool| {
            if !first {
                ui.add_space(12.0);
                ui.separator();
            }
            ui.add_space(8.0);
            ui.label(small_heading(title));
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
            first = false;
            self.regions_section(ui);
        }
        if self.show_pool {
            section(ui, "LOOP POOL", first);
            self.pool_section(ui);
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
                key(ui, "Tempo");
                ui.label(format!("{:.2} BPM, {}/4", self.tempo, self.beats_per_bar));
                ui.end_row();
                key(ui, "Playhead");
                ui.label(format!("{}  ({})", self.fmt_pos(self.arr_cursor), self.fmt_bars(self.arr_cursor)));
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

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if self.view == View::Arrangement {
                if !self.tracks.is_empty() {
                    let clips: usize = self.tracks.iter().map(|t| t.clips.len()).sum();
                    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
                    ui.label(format!("{} Hz · {} · {} · {}", self.rate(), plural(self.tracks.len(), "layer"), plural(clips, "clip"), self.fmt_pos(self.arrangement_frames())));
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
}
