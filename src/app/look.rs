//! The app's look: colours, egui style, shared dialog widgets, the level meter and the drawn icons.

use super::*;

/// A level meter bar: dark fill up to the level on a -48..0 dB scale, black near clipping.
pub(super) fn draw_meter(p: &egui::Painter, r: Rect, level: f32) {
    let db = 20.0 * level.max(1e-6).log10();
    let t = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
    p.rect_filled(r, 2.0, Color32::from_gray(228));
    if t > 0.0 {
        let fill = Rect::from_min_size(r.min, Vec2::new(r.width() * t, r.height()));
        p.rect_filled(fill, 2.0, if db > -3.0 { Color32::BLACK } else { Color32::from_gray(90) });
    }
}

/// Text, secondary text, panel and line colours for the clean monochrome look.
pub(super) const INK: Color32 = Color32::from_gray(20);
pub(super) const MUTED: Color32 = Color32::from_gray(120);
pub(super) const PANEL: Color32 = Color32::from_gray(246);
pub(super) const LINE: Color32 = Color32::from_gray(218);

/// Clean monochrome look: white and light-grey surfaces, thin grey lines, black accents, and
/// the system font when available.
pub(super) fn clean_style(ctx: &egui::Context) {
    use egui::{CornerRadius, Shadow};
    ctx.set_theme(egui::Theme::Light);
    let mut v = egui::Visuals::light();
    v.panel_fill = PANEL;
    v.window_fill = Color32::WHITE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow = Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(40) };
    v.popup_shadow = Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(35) };
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_gray(250);
    v.override_text_color = None;
    v.selection.bg_fill = Color32::from_gray(30);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.window_highlight_topmost = false;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(5);
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.0, INK);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    // bg_fill draws slider rails and checkbox boxes; buttons use weak_bg_fill.
    v.widgets.inactive.bg_fill = Color32::from_gray(222);
    v.widgets.inactive.weak_bg_fill = Color32::WHITE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(205));
    v.widgets.hovered.bg_fill = Color32::from_gray(238);
    v.widgets.hovered.weak_bg_fill = Color32::from_gray(238);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(160));
    v.widgets.active.bg_fill = Color32::from_gray(215);
    v.widgets.active.weak_bg_fill = Color32::from_gray(215);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_gray(120));
    v.widgets.open = v.widgets.hovered;
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|style| {
        use egui::{FontFamily, TextStyle};
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 4.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.window_margin = egui::Margin::same(14);
        style.spacing.combo_width = 140.0;
        style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    });

    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        ("SF", "/System/Library/Fonts/SFNS.ttf", egui::FontFamily::Proportional),
        ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf", egui::FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
}

/// Monochrome vector icons for the Tools and Controls palettes.
pub(super) fn paint_icon(p: &egui::Painter, rect: Rect, a: Action, color: Color32, playing: bool) {
    let c = rect.center();
    let st = Stroke::new(1.5, color);
    let pt = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        p.line_segment([pt(x0, y0), pt(x1, y1)], st);
    };
    let thick = |x0: f32, y0: f32, x1: f32, y1: f32, w: f32| {
        p.line_segment([pt(x0, y0), pt(x1, y1)], Stroke::new(w, color));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| pt(x, y)).collect(), color, Stroke::NONE));
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
    let bars = |bars: &[(f32, f32)]| {
        for &(x, h) in bars {
            line(x, -h, x, h);
        }
    };
    match a {
        Action::ToolSelect => {
            line(0.0, -8.0, 0.0, 8.0);
            line(-3.0, -8.0, 3.0, -8.0);
            line(-3.0, 8.0, 3.0, 8.0);
        }
        Action::ToolHand => {
            let palm = Rect::from_min_max(pt(-5.0, -1.0), pt(5.0, 8.0));
            p.rect_stroke(palm, 3.0, st, egui::StrokeKind::Middle);
            for (x, top) in [(-3.5f32, -6.0f32), (-1.0, -8.0), (1.5, -8.0), (4.0, -6.0)] {
                line(x, top, x, -1.0);
            }
            line(-5.0, 4.0, -8.0, 0.0);
        }
        Action::ToolZoom => {
            p.circle_stroke(pt(-2.0, -2.0), 5.5, st);
            thick(2.0, 2.0, 7.5, 7.5, 3.0);
        }
        Action::ToolSplit => {
            p.rect_stroke(Rect::from_min_max(pt(-9.0, -5.0), pt(-2.0, 5.0)), 2.0, st, egui::StrokeKind::Middle);
            p.rect_stroke(Rect::from_min_max(pt(2.0, -5.0), pt(9.0, 5.0)), 2.0, st, egui::StrokeKind::Middle);
            line(0.0, -9.0, 0.0, 9.0);
        }
        Action::SliceClip => {
            p.rect_stroke(Rect::from_min_max(pt(-10.0, -5.0), pt(10.0, 5.0)), 2.0, st, egui::StrokeKind::Middle);
            for x in [-5.0f32, 0.0, 5.0] {
                line(x, -8.0, x, 8.0);
            }
        }
        Action::DuplicateClip => {
            p.rect_stroke(Rect::from_min_max(pt(-10.0, -4.0), pt(-1.0, 4.0)), 2.0, st, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(pt(1.0, -4.0), pt(10.0, 4.0)), 2.0, color);
        }
        Action::LoopClip => {
            p.rect_stroke(Rect::from_min_max(pt(-10.0, -4.0), pt(-2.0, 4.0)), 2.0, st, egui::StrokeKind::Middle);
            for x in [1.0f32, 5.0, 9.0] {
                line(x, -4.0, x, 4.0);
            }
            line(-2.0, 0.0, 10.0, 0.0);
        }
        Action::WarpClip => {
            // A clip stretched between two arrows.
            arrow_h(-10.0, 10.0);
            bars(&[(-5.0, 4.0), (-2.0, 7.0), (2.0, 5.0), (5.0, 3.0)]);
        }
        Action::ToolPencil => {
            thick(-3.5, 3.5, 6.0, -6.0, 4.5);
            fill(&[(-8.0, 8.0), (-6.5, 2.5), (-2.5, 6.5)]);
        }
        Action::Cut => {
            p.circle_stroke(pt(-4.0, 5.0), 2.6, st);
            p.circle_stroke(pt(4.0, 5.0), 2.6, st);
            line(-2.5, 3.0, 4.0, -8.0);
            line(2.5, 3.0, -4.0, -8.0);
        }
        Action::Copy => {
            let r1 = Rect::from_center_size(pt(-2.5, -2.5), Vec2::new(10.0, 12.0));
            let r2 = r1.translate(Vec2::new(5.0, 5.0));
            p.rect_stroke(r1, 0.0, st, egui::StrokeKind::Middle);
            p.rect_filled(r2, 0.0, Color32::WHITE);
            p.rect_stroke(r2, 0.0, st, egui::StrokeKind::Middle);
        }
        Action::Paste => {
            p.rect_stroke(Rect::from_min_max(pt(-6.0, -6.0), pt(6.0, 8.0)), 0.0, st, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(pt(-3.0, -8.5), pt(3.0, -4.5)), 0.0, color);
            line(-3.0, 0.0, 3.0, 0.0);
            line(-3.0, 3.5, 3.0, 3.5);
        }
        Action::Delete => {
            line(-7.0, -5.0, 7.0, -5.0);
            line(-2.0, -7.5, 2.0, -7.5);
            line(-5.5, -5.0, -4.0, 8.0);
            line(5.5, -5.0, 4.0, 8.0);
            line(-4.0, 8.0, 4.0, 8.0);
            line(-1.5, -2.0, -1.0, 5.5);
            line(1.5, -2.0, 1.0, 5.5);
        }
        Action::Trim => {
            brackets(8.0);
            bars(&[(-3.0, 3.0), (0.0, 6.0), (3.0, 4.0)]);
            line(-12.0, 0.0, -10.0, 0.0);
            line(10.0, 0.0, 12.0, 0.0);
        }
        Action::Silence => {
            brackets(8.0);
            line(-6.0, 0.0, 6.0, 0.0);
        }
        Action::FadeIn => fill(&[(-8.0, 7.0), (8.0, 7.0), (8.0, -7.0)]),
        Action::FadeOut => fill(&[(-8.0, 7.0), (-8.0, -7.0), (8.0, 7.0)]),
        Action::Normalize => {
            line(-9.0, -8.0, 9.0, -8.0);
            line(-9.0, 8.0, 9.0, 8.0);
            bars(&[(-6.0, 3.0), (-2.0, 6.0), (2.0, 4.0), (6.0, 2.0)]);
        }
        Action::Gain => {
            bars(&[(-7.0, 1.5), (-3.5, 3.5), (0.0, 5.5), (3.5, 7.0), (7.0, 8.5)]);
        }
        Action::Reverse => {
            bars(&[(-1.0, 2.0), (2.0, 4.0), (5.0, 6.0), (8.0, 8.0)]);
            line(-8.0, 0.0, -3.0, 0.0);
            line(-8.0, 0.0, -5.0, -3.0);
            line(-8.0, 0.0, -5.0, 3.0);
        }
        Action::Echo => {
            bars(&[(-8.0, 8.0), (-3.0, 5.0), (2.0, 3.0), (6.5, 1.5)]);
        }
        Action::Split => {
            for x in [-5.0f32, 5.0] {
                for y in [-8.0f32, -3.0, 2.0, 7.0] {
                    line(x, y, x, y + 2.5);
                }
            }
            bars(&[(-10.0, 5.0), (-8.0, 3.0), (-1.0, 6.0), (1.0, 4.0), (8.0, 4.0), (10.0, 6.0)]);
        }
        Action::ZoomFit => {
            line(-10.0, -7.0, -10.0, 7.0);
            line(10.0, -7.0, 10.0, 7.0);
            arrow_h(-8.0, 8.0);
        }
        Action::ZoomSel => {
            brackets(10.0);
            arrow_h(-6.0, 6.0);
        }
        Action::GoStart => {
            thick(-6.0, -6.0, -6.0, 6.0, 2.5);
            fill(&[(-4.5, 0.0), (5.5, -6.0), (5.5, 6.0)]);
        }
        Action::GoEnd => {
            thick(6.0, -6.0, 6.0, 6.0, 2.5);
            fill(&[(4.5, 0.0), (-5.5, -6.0), (-5.5, 6.0)]);
        }
        Action::PlayPause if playing => {
            p.rect_filled(Rect::from_min_max(pt(-5.0, -6.0), pt(-1.5, 6.0)), 0.0, color);
            p.rect_filled(Rect::from_min_max(pt(1.5, -6.0), pt(5.0, 6.0)), 0.0, color);
        }
        Action::PlayPause => fill(&[(-5.0, -7.0), (7.0, 0.0), (-5.0, 7.0)]),
        Action::Stop => {
            p.rect_filled(Rect::from_center_size(c, Vec2::splat(11.0)), 0.0, color);
        }
        Action::Record => {
            p.circle_filled(c, 6.0, color);
        }
        Action::Loop => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = 0.4 + i as f32 / 24.0 * 5.2;
                    pt(7.0 * t.cos(), 5.5 * t.sin())
                })
                .collect();
            p.add(egui::Shape::line(pts, st));
            let t = 0.4f32;
            let tip = pt(7.0 * t.cos(), 5.5 * t.sin());
            p.add(egui::Shape::convex_polygon(
                vec![tip + Vec2::new(-3.5, -1.0), tip + Vec2::new(2.5, -2.5), tip + Vec2::new(0.5, 3.0)],
                color,
                Stroke::NONE,
            ));
        }
        Action::Crossfeed => {
            // Headphones: a band over the top and an ear cup either side.
            let pts: Vec<Pos2> = (0..=20).map(|i| {
                let t = std::f32::consts::PI * (1.0 + i as f32 / 20.0);
                pt(7.0 * t.cos(), 2.0 + 8.0 * t.sin())
            }).collect();
            p.add(egui::Shape::line(pts, st));
            for s in [-1.0f32, 1.0] {
                p.rect_filled(Rect::from_min_max(pt(s * 7.5 - 2.5, 1.0), pt(s * 7.5 + 2.5, 8.0)), 1.5, color);
            }
        }
        _ => {}
    }
}

/// Dialog width, label column width and spacing shared by every dialog.
pub(super) const DIALOG_W: f32 = 400.0;
/// Width of every number field, pop-up menu and text field in a dialog.
pub(super) const FIELD_W: f32 = 140.0;
pub(super) const LABEL_W: f32 = 120.0;

/// A modal dialog with the app's frame: white, rounded, generous margins.
pub(super) fn dialog_modal(id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(id)).frame(
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(22))
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(45) }),
    )
}

/// The look every floating window shares with the dialogs: a white card, no title bar.
pub(super) fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(18))
        .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(45) })
}

/// A floating window's title (as a dialog's) with a close button on the right; true when the
/// close button was clicked. Drag the window by any empty part of it.
pub(super) fn window_header(ui: &mut egui::Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(17.0).strong().color(INK));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = close_button(ui).on_hover_text("Close").clicked();
        });
    });
    ui.add_space(10.0);
    close
}

/// A small drawn × for closing a floating window.
pub(super) fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    let color = if resp.hovered() { INK } else { MUTED };
    let r = rect.shrink(6.0);
    let st = Stroke::new(1.5, color);
    ui.painter().line_segment([r.left_top(), r.right_bottom()], st);
    ui.painter().line_segment([r.right_top(), r.left_bottom()], st);
    resp
}

/// A small heading in capitals, as the side panel's sections and the palette's rows use.
pub(super) fn small_heading(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(11.0).color(MUTED).strong()
}

/// Dialog title and a muted explanation under it.
pub(super) fn dialog_header(ui: &mut egui::Ui, title: &str, blurb: &str) {
    ui.label(egui::RichText::new(title).size(17.0).strong().color(INK));
    if !blurb.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(blurb).color(MUTED));
    }
    ui.add_space(14.0);
}

/// The grid used for a dialog's labelled settings.
pub(super) fn dialog_grid(id: &str) -> egui::Grid {
    egui::Grid::new(id).num_columns(2).min_col_width(LABEL_W).spacing(Vec2::new(12.0, 10.0))
}

/// A filled black button for a dialog's main action.
pub(super) fn primary_button(label: &str) -> Button<'static> {
    Button::new(egui::RichText::new(label.to_string()).color(Color32::WHITE)).fill(INK).min_size(Vec2::new(84.0, 26.0))
}

/// Right-aligned Cancel and main-action buttons at the bottom of a dialog, with optional extra
/// buttons on the left (`extra`). Returns (main clicked, cancel clicked).
pub(super) fn dialog_buttons(ui: &mut egui::Ui, ok_label: &str, ok_enabled: bool, extra: impl FnOnce(&mut egui::Ui)) -> (bool, bool) {
    ui.add_space(18.0);
    let mut out = (false, false);
    ui.horizontal(|ui| {
        // Extra buttons (Flat, Reset…) are as tall as Cancel and OK, and line up with them.
        ui.spacing_mut().interact_size.y = 26.0;
        extra(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            out.0 = ui.add_enabled(ok_enabled, primary_button(ok_label)).clicked();
            out.1 = ui.add(Button::new("Cancel").min_size(Vec2::new(84.0, 26.0))).clicked();
        });
    });
    out
}
