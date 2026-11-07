//! Things the app does to your files. Every one is reversible and logged.

use crate::config::Config;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Moves a folder to the Recycle Bin (never a permanent delete) and logs it.
pub fn recycle(path: &Path) -> anyhow::Result<()> {
    trash::delete(path)?;
    log_action("recycled", path);
    Ok(())
}

/// Puts a folder previously moved by [`recycle`] back where it was.
/// Picks the most recently deleted item with that original path.
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
pub fn restore(path: &Path) -> anyhow::Result<()> {
    use crate::config::same_path;
    let item = trash::os_limited::list()?
        .into_iter()
        .filter(|i| same_path(&i.original_path(), path))
        .max_by_key(|i| i.time_deleted)
        .ok_or_else(|| anyhow::anyhow!("it's no longer in the Recycle Bin"))?;
    trash::os_limited::restore_all([item])?;
    log_action("restored", path);
    Ok(())
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
pub fn restore(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("restoring isn't supported on this platform; use the system trash")
}

/// Opens a terminal in the folder (Windows Terminal if installed, else cmd).
pub fn open_terminal(path: &Path) {
    #[cfg(windows)]
    {
        use std::process::Command;
        if Command::new("wt").arg("-d").arg(path).spawn().is_err() {
            let _ = Command::new("cmd")
                .args(["/C", "start", "", "cmd", "/K", "cd", "/d"])
                .arg(path)
                .spawn();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("x-terminal-emulator")
            .current_dir(path)
            .spawn();
    }
}

/// Opens the folder in File Explorer (or the platform file manager).
pub fn open_folder(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

/// Appends a line to `actions.log` next to the config file, so there's always
/// a record of what was moved where.
pub fn log_action(action: &str, path: &Path) {
    let Some(dir) = Config::data_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("actions.log"))
    {
        let _ = writeln!(
            f,
            "{}\t{action}\t{}",
            utc_timestamp(SystemTime::now()),
            path.display()
        );
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` without pulling in a date library.
pub fn utc_timestamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    fn recycle_then_restore() {
        let tmp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let dir = tmp
            .path()
            .join(format!("restore-me-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        recycle(&dir).unwrap();
        assert!(!dir.exists());
        restore(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "hi");
    }

    #[test]
    fn timestamps() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_794_070_800)),
            "2026-11-07T17:00:00Z"
        );
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
    }
}
