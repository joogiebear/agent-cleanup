// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod background;
mod ui;

fn main() {
    if std::env::args().any(|a| a == agent_cleanup::schedule::CHECK_ARG) {
        background::run();
    } else {
        ui::run();
    }
}
