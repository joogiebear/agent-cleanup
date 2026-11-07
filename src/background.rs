//! `agent-cleanup.exe --check`: what the weekly scheduled task runs. No
//! window; it scans, and if there's enough to clean it shows a notification
//! that opens the app when clicked. It never moves anything itself.

use agent_cleanup::check;
use agent_cleanup::config::Config;

pub fn run() {
    let cfg = Config::load();
    // Before setup every repo looks like a leftover, so say nothing.
    if cfg.project_roots.is_empty() {
        return;
    }
    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    let summary = check::run(&cfg, home.as_deref());
    if let Some((title, body)) = summary.message() {
        notify(&title, &body);
    }
}

#[cfg(windows)]
fn notify(title: &str, body: &str) {
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri_winrt_notification::{Duration as Shown, Toast, ToastDismissalReason};

    let (tx, rx) = mpsc::channel::<bool>();
    let on_close = tx.clone();
    let shown = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(&format!("Agent Cleanup: {title}"))
        .text1(body)
        .duration(Shown::Long)
        .on_activated(move |_| {
            let _ = tx.send(true);
            Ok(())
        })
        .on_dismissed(move |reason| {
            // Timing out only moves it to the notification center, where it
            // can still be clicked, so keep waiting for that.
            if matches!(reason, Some(ToastDismissalReason::UserCanceled)) {
                let _ = on_close.send(false);
            }
            Ok(())
        })
        .show();
    if shown.is_err() {
        return;
    }
    // A click only works while this process is alive; give it an hour.
    if let Ok(true) = rx.recv_timeout(Duration::from_secs(3600))
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::process::Command::new(exe).spawn();
    }
}

#[cfg(not(windows))]
fn notify(title: &str, body: &str) {
    println!("{title}\n{body}");
}
