// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Agent Cleanup")
            .with_inner_size([1280.0, 760.0])
            .with_min_inner_size([760.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Agent Cleanup",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
