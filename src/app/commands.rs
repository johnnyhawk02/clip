//! Commands: which actions are available and ticked, running them, and the keyboard shortcuts.

use super::*;

impl App {
    pub(super) fn enabled(&self, a: Action) -> bool {
        let doc = !self.tracks.is_empty();
        if self.view == View::Arrangement {
            // A time selection stands in for selected clips: processes work on what's inside it.
            let has_clip = self.sel_clip.and_then(|p| self.clip(p)).is_some() || self.time_sel.is_some();
            return match a {
                Action::New | Action::Open | Action::Quit | Action::Preferences | Action::BatchConvert | Action::Record | Action::NewClip | Action::ImportClip => true,
                Action::SaveProject | Action::SaveProjectAs => doc,
                Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Dither | Action::AutoGain => true,
                Action::Loop | Action::ShowTools | Action::ShowControls | Action::ShowInfo | Action::ShowCues | Action::ShowRegions | Action::ShowPool | Action::Crossfeed => true,
                Action::ShowSpectrum | Action::RulerTime | Action::RulerSamples | Action::RulerTimecode => true,
                Action::Export | Action::ExportAbleton => doc && self.arrangement_frames() > 0,
                Action::Delete => has_clip || self.time_sel.is_some(),
                Action::ExportSelection | Action::EditClip | Action::DuplicateClip | Action::Cut | Action::Copy => has_clip,
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
                Action::SwapSection | Action::AddToPool => has_clip,
                Action::CollapseTrack => self.tracks.get(self.active).is_some_and(|t| !t.clips.is_empty()),
                Action::ClearLoop => self.arr_loop.is_some(),
                Action::ToggleSnap | Action::NarrowGrid | Action::WidenGrid => doc,
                Action::ZoomSel => has_clip || self.arr_loop.is_some(),
                Action::ZoomBack => self.zoom_back.is_some_and(|z| z.0),
                Action::Ungroup => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.group.is_some()),
                Action::SelectAll => doc,
                Action::BackToArrangement => !self.nest.is_empty(),
                Action::Paste => self.clipboard.is_some(),
                Action::Undo => !self.undo.is_empty(),
                Action::Redo => !self.redo.is_empty(),
                Action::PlayPause | Action::Stop | Action::GoStart | Action::GoEnd | Action::ZoomIn | Action::ZoomOut | Action::ZoomFit => doc,
                Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::MuteTrack => doc,
                Action::TrackColor | Action::OffsetTrack => doc,
                Action::DeleteTrack | Action::MixDown | Action::NextTrack | Action::PrevTrack => self.tracks.len() > 1,
                Action::LayerUp => self.active > 0,
                Action::LayerDown => self.active + 1 < self.tracks.len(),
                Action::MergeDown => self.can_merge_down(),
                _ => false,
            };
        }
        match a {
            Action::BackToArrangement => return true,
            Action::SaveProject | Action::SaveProjectAs => return doc,
            Action::EditClip | Action::SplitClip | Action::DuplicateClip | Action::NewClip | Action::ImportClip => return false,
            Action::GroupClips | Action::Ungroup | Action::TempoFromClip | Action::ToggleSnap | Action::LoopClip | Action::NarrowGrid | Action::WidenGrid => return false,
            Action::ZoomBack => return self.zoom_back.is_some_and(|z| !z.0),
            Action::CollapseClip | Action::CollapseTrack | Action::WarpClip | Action::SliceClip | Action::ReverseClip => return false,
            Action::MangleShuffle | Action::MangleReverse | Action::MangleSwap | Action::MangleStutter | Action::MangleRandomReverse | Action::MangleDropOut | Action::MangleBBCut | Action::MangleAgain => return false,
            Action::MangleCutHouse | Action::MangleCutHipHop | Action::MangleCutDembow | Action::MangleCutFootwork | Action::MangleCutTrap | Action::MangleCutBreakcore | Action::MangleCutHalftime => return false,
            Action::AutoWarp => return true,
            Action::AutoArrange | Action::PrevVariation | Action::NextVariation | Action::SwapSection | Action::AddToPool => return false,
            Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::DeleteTrack | Action::MixDown => return false,
            Action::NextTrack | Action::PrevTrack | Action::OffsetTrack | Action::LayerUp | Action::LayerDown | Action::MergeDown => return false,
            _ => {}
        }
        let has = self.audio.is_some();
        let sel = has && self.sel.is_some();
        let not_empty = self.frames() > 0;
        match a {
            Action::New | Action::Open | Action::Quit | Action::Preferences => true,
            Action::FormatPcm8 | Action::FormatPcm16 | Action::FormatPcm24 | Action::FormatFloat32 | Action::Loop | Action::Crossfeed => true,
            Action::ShowCues | Action::Record => true,
            Action::NewTrack | Action::ImportTrack | Action::DuplicateTrack | Action::MuteTrack => has,
            Action::DeleteTrack | Action::MixDown | Action::NextTrack | Action::PrevTrack => has && self.tracks.len() > 1,
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

    pub(super) fn checked(&self, a: Action) -> bool {
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
            Action::Crossfeed => self.crossfeed,
            Action::ToggleSnap => self.snap,
            Action::AutoWarp => self.auto_warp,
            Action::WarpClip => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.warp.is_some()),
            Action::LoopClip => self.sel_clip.and_then(|p| self.clip(p)).is_some_and(|c| c.looping),
            Action::Dither => audio::DITHER.load(std::sync::atomic::Ordering::Relaxed),
            Action::AutoGain => self.auto_gain,
            _ => false,
        }
    }

    pub(super) fn menu_label(&self, a: Action) -> String {
        match a {
            Action::Undo => self.undo.last().map_or("Undo".into(), |s| format!("Undo {}", s.label)),
            Action::Redo => self.redo.last().map_or("Redo".into(), |s| format!("Redo {}", s.label)),
            _ => a.label().to_string(),
        }
    }

    pub(super) fn run(&mut self, a: Action, ctx: &egui::Context) {
        if !self.enabled(a) {
            return;
        }
        if self.view == View::Arrangement && self.time_sel.is_some() && on_time_sel(a) {
            return self.run_on_time_sel(a, ctx);
        }
        if self.view == View::Arrangement {
            let mid = self.arr_view_start + self.arr_view_len / 2.0;
            match a {
                Action::Delete if self.time_sel.is_some() => return self.delete_time_sel(),
                Action::Delete => return self.delete_clip(),
                Action::Copy | Action::Cut => {
                    if let Some(c) = self.sel_clip.and_then(|p| self.clip(p)) {
                        let clip = c.visible();
                        ctx.copy_text(format!("[PhotoSound clip, {}]", fmt_time(clip.duration_secs())));
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
                Action::ZoomSel => {
                    // The selected clips, or the loop range.
                    let span = self.selected_clips().iter().filter_map(|&p| self.clip(p)).map(|c| (c.start, c.end())).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).or(self.arr_loop);
                    if let Some((s, e)) = span {
                        self.zoom_back = Some((true, self.arr_view_start, self.arr_view_len));
                        let pad = (e - s) as f64 * 0.05;
                        self.arr_view_start = s as f64 - pad;
                        self.arr_view_len = (e - s) as f64 + 2.0 * pad;
                        self.arr_clamp();
                    }
                    return;
                }
                Action::ZoomBack => {
                    if let Some((true, s, l)) = self.zoom_back.take() {
                        (self.arr_view_start, self.arr_view_len) = (s, l);
                        self.arr_clamp();
                    }
                    return;
                }
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
                        self.arr_loop = Some(self.whole_bars(self.selected_span().unwrap_or((self.arr_cursor, self.arr_cursor + (self.bar_frames() * 4.0) as usize))));
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
                        self.arr_loop = Some(self.whole_bars(span));
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
            Action::ZoomSel => {
                self.zoom_back = Some((false, self.view_start, self.view_len));
                self.zoom_to_selection();
            }
            Action::ZoomBack => {
                if let Some((false, s, l)) = self.zoom_back.take() {
                    (self.view_start, self.view_len) = (s, l);
                    self.clamp_view();
                }
            }
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
            Action::SplitClip if self.view == View::Arrangement && self.time_sel.is_some() => self.split_time_sel(),
            Action::SplitClip => self.split_clip(),
            Action::DuplicateClip => self.duplicate_clip(),
            Action::NewClip => self.new_clip(),
            Action::GroupClips => self.group_clips(),
            Action::TempoFromClip => self.tempo_from_clip(),
            Action::LoopClip => self.toggle_clip_loop(),
            Action::CollapseClip => self.collapse_clip(),
            Action::CollapseTrack => self.collapse_track(self.active),
            Action::ToggleSnap => self.snap = !self.snap,
            Action::Crossfeed => {
                self.crossfeed = !self.crossfeed;
                self.player.set_crossfeed(self.crossfeed);
                let pref = crossfeed_pref();
                let _ = if self.crossfeed {
                    std::fs::create_dir_all(pref.parent().unwrap()).and_then(|_| std::fs::write(&pref, b""))
                } else {
                    std::fs::remove_file(&pref)
                };
                self.status = if self.crossfeed { "Headphone crossfeed on (playback only)".into() } else { "Headphone crossfeed off".into() };
            }
            Action::NarrowGrid | Action::WidenGrid => {
                // Steps through 1 Bar … 1/32; from Off or Auto it starts at 1/4.
                self.grid = match (self.grid, a == Action::NarrowGrid) {
                    (0 | 1, _) => 4,
                    (g, true) => (g + 1).min(GRID.len() - 1),
                    (g, false) => (g - 1).max(2),
                };
                self.status = format!("Grid {}", GRID[self.grid].0);
            }
            Action::AutoWarp => self.auto_warp = !self.auto_warp,
            Action::WarpClip => self.toggle_warp(),
            Action::SliceClip => self.slice_clip(),
            Action::AutoArrange => self.auto_arrange(),
            Action::PrevVariation => self.step_variation(false),
            Action::NextVariation => self.step_variation(true),
            Action::SwapSection => self.swap_sections(),
            Action::AddToPool => self.add_to_pool(),
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
            Action::LayerUp => self.move_layer(true),
            Action::LayerDown => self.move_layer(false),
            Action::Preferences => self.prefs_open = Some((self.phrase_bars, repitch_mode())),
            Action::MergeDown => self.merge_down(),
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
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
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
            (none, Key::W, Action::ZoomFit),
            (none, Key::Z, Action::ZoomSel),
            (none, Key::X, Action::ZoomBack),
            (none, Key::L, Action::Loop),
            (none, Key::S, Action::ToolSelect),
            (none, Key::H, Action::ToolHand),
            (none, Key::B, Action::ToolPencil),
            (none, Key::C, Action::ToolSplit),
            // ⇧R before R: a plain-key binding also catches the shifted key.
            (Modifiers::SHIFT, Key::R, Action::SwapSection),
            (none, Key::R, Action::ReverseClip),
            (none, Key::Enter, Action::EditClip),
            (none, Key::OpenBracket, Action::PrevCue),
            (none, Key::CloseBracket, Action::NextCue),
        ];
        for &(m, k, a) in keys {
            if ctx.input_mut(|i| i.consume_key(m, k)) {
                self.run(a, ctx);
            }
        }
        // ⇧Tab switches between the arrangement and the clip editor, as in Live.
        if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab)) {
            let a = if self.view == View::Arrangement { Action::EditClip } else { Action::BackToArrangement };
            self.run(a, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(none, Key::Escape)) {
            if self.player.is_playing() {
                self.stop();
            } else {
                self.sel = None;
            }
        }
    }
}
