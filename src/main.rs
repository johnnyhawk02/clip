#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ableton;
mod actions;
mod app;
mod audio;
mod crossfeed;
mod effects;
mod engine;
mod menu;
mod player;
mod project;
mod recorder;

fn main() -> eframe::Result {
    // Catch files Finder asks us to open at launch (double-clicking a project).
    #[cfg(target_os = "macos")]
    menu::install_open_event_handler();

    // Optional: open a file passed on the command line.
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PhotoSound")
            .with_inner_size([1100.0, 560.0])
            .with_min_inner_size([640.0, 320.0])
            .with_drag_and_drop(true)
            // Our own icon, or eframe puts its default one in the Dock while the app runs.
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "Sound Edit",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))),
    )
}
