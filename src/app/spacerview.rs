//! The Spacer window: the selected track's Spacer (it ducks where another track plays, as
//! Trackspacer does), with a live view of the cut in each band.

use super::*;

impl App {
    pub(super) fn spacer_window(&mut self, ctx: &egui::Context) {
        let t = self.active.min(self.tracks.len().saturating_sub(1));
        let Some(track) = self.tracks.get(t) else { return };
        let name = track.name.clone();
        let before = track.spacer;
        let mut set = before;
        let others: Vec<(usize, String)> = self.tracks.iter().enumerate().filter(|(i, _)| *i != t).map(|(i, tr)| (i, tr.name.clone())).collect();
        let cuts = self.player.spacer_cuts(t).filter(|_| self.player.is_playing());
        let mut open = true;
        let mut active = false;
        self.place("Spacer", Pos2::new(self.desk.center().x - 180.0, self.desk.top() + 60.0))
            .title_bar(false)
            .frame(card_frame())
            .auto_sized()
            .fade_in(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(360.0);
                if window_header(ui, &format!("Spacer · {name}")) {
                    open = false;
                }
                ui.label(egui::RichText::new("Turns this track down only at the frequencies another track is playing, so they sit together.").size(11.0).color(muted()));
                ui.add_space(6.0);
                dialog_grid("spacergrid").show(ui, |ui| {
                    ui.label("Make room for");
                    let current = set.and_then(|s| others.iter().find(|o| o.0 == s.source)).map_or("Nothing", |o| o.1.as_str());
                    egui::ComboBox::from_id_salt("spacer_src").width(200.0).selected_text(current).show_ui(ui, |ui| {
                        if ui.selectable_label(set.is_none(), "Nothing").clicked() {
                            set = None;
                        }
                        for (i, n) in &others {
                            if ui.selectable_label(set.is_some_and(|s| s.source == *i), n).clicked() {
                                set = Some(set.map_or(spacer::Settings::new(*i), |s| spacer::Settings { source: *i, ..s }));
                            }
                        }
                    });
                    ui.end_row();
                    let Some(s) = set.as_mut() else { return };
                    let mut row = |ui: &mut egui::Ui, label: &str, w: egui::Response| {
                        let _ = label;
                        active |= w.dragged() || w.has_focus();
                        if w.hovered() || w.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        ui.end_row();
                    };
                    ui.label("On");
                    ui.checkbox(&mut s.on, "");
                    ui.end_row();
                    ui.label("Amount");
                    let mut pct = s.amount * 100.0;
                    let w = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").fixed_decimals(0));
                    s.amount = pct / 100.0;
                    row(ui, "Amount", w);
                    ui.label("Low Cut");
                    let w = ui.add(egui::Slider::new(&mut s.low_cut, 20.0..=2000.0).logarithmic(true).suffix(" Hz").fixed_decimals(0));
                    row(ui, "Low Cut", w);
                    ui.label("High Cut");
                    let w = ui.add(egui::Slider::new(&mut s.high_cut, 500.0..=20000.0).logarithmic(true).suffix(" Hz").fixed_decimals(0));
                    row(ui, "High Cut", w);
                    ui.label("Attack");
                    let w = ui.add(egui::Slider::new(&mut s.attack, 0.5..=100.0).logarithmic(true).suffix(" ms").fixed_decimals(1));
                    row(ui, "Attack", w);
                    ui.label("Release");
                    let w = ui.add(egui::Slider::new(&mut s.release, 10.0..=1000.0).logarithmic(true).suffix(" ms").fixed_decimals(0));
                    row(ui, "Release", w);
                    ui.label("Stereo");
                    ui.horizontal(|ui| {
                        for (i, n) in ["Linked", "L / R", "Mid / Side"].iter().enumerate() {
                            ui.radio_value(&mut s.stereo, i as u8, *n);
                        }
                    });
                    ui.end_row();
                    ui.label("Output");
                    let w = ui.add(egui::Slider::new(&mut s.output, -12.0..=12.0).suffix(" dB").fixed_decimals(1));
                    row(ui, "Output", w);
                });
                // The cut in each band, live while playing: 30 Hz to 18 kHz, 0 to -30 dB.
                if set.is_some() {
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(Vec2::new(360.0, 70.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, 3.0, gray(242));
                    let bw = r.width() / spacer::BANDS as f32;
                    let rows = if set.is_some_and(|s| s.stereo != 0) { 2 } else { 1 };
                    for b in 0..spacer::BANDS {
                        for row in 0..rows {
                            let cut = cuts.map_or(0.0, |c| c[row][b]).clamp(0.0, 30.0);
                            let h = cut / 30.0 * (r.height() - 4.0);
                            let x0 = r.left() + b as f32 * bw + 1.0 + row as f32 * (bw - 2.0) / rows as f32;
                            let bar = Rect::from_min_size(Pos2::new(x0, r.top() + 2.0), Vec2::new((bw - 2.0) / rows as f32, h));
                            p.rect_filled(bar, 1.0, if row == 0 { ink() } else { muted() });
                        }
                    }
                    p.text(r.left_bottom() + Vec2::new(4.0, -3.0), Align2::LEFT_BOTTOM, "30 Hz", FontId::proportional(9.5), muted());
                    p.text(r.right_bottom() + Vec2::new(-4.0, -3.0), Align2::RIGHT_BOTTOM, "18 kHz", FontId::proportional(9.5), muted());
                    let deepest = cuts.map_or(0.0, |c| c.iter().flatten().fold(0.0f32, |a, &v| a.max(v)));
                    p.text(r.right_top() + Vec2::new(-4.0, 3.0), Align2::RIGHT_TOP, if cuts.is_some() { format!("-{deepest:.1} dB") } else { "play to see the cuts".into() }, FontId::proportional(9.5), muted());
                    if cuts.is_some() {
                        ui.ctx().request_repaint();
                    }
                }
            });
        self.show_spacer = open;
        if set != before {
            if !self.spacer_edit {
                self.push_undo("Spacer");
                self.spacer_edit = true;
            }
            self.tracks[t].spacer = set;
            self.dirty = true;
            self.refresh_playback();
        }
        if !active {
            self.spacer_edit = false;
        }
    }
}
