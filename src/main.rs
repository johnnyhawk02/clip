#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ableton;
mod actions;
mod app;
mod audio;
mod crossfeed;
mod devices;
mod effects;
mod fx;
mod engine;
mod menu;
mod player;
mod project;
mod spacer;
mod tape;
mod phaser;
mod recorder;

fn main() -> eframe::Result {
    log_panics();
    // Catch files Finder asks us to open at launch (double-clicking a project).
    #[cfg(target_os = "macos")]
    menu::install_open_event_handler();

    // Optional: open a file passed on the command line.
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Clip")
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

/// Write any crash (a panic, on any thread) to ~/Library/Logs/Clip/crash-<time>.log with a
/// backtrace, so it can be looked into: an app opened from Finder has nowhere else to say.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(home) = std::env::var_os("HOME") {
            let dir = std::path::Path::new(&home).join("Library/Logs/Clip");
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
            let text = format!("Clip {} crashed on thread {thread}: {info}\n\n{}\n", env!("CARGO_PKG_VERSION"), std::backtrace::Backtrace::force_capture());
            if std::fs::create_dir_all(&dir).is_ok() {
                let _ = std::fs::write(dir.join(format!("crash-{secs}.log")), text);
            }
        }
        default(info);
    }));
}

