//! Everything Agent Cleanup itself leaves on the PC, and removing it.
//!
//! The app is a portable exe. Besides that it only ever writes:
//! - `%APPDATA%\agent-cleanup\config\`: `config.toml`, `last_scan.toml`, `actions.log`;
//! - the "Agent Cleanup weekly check" scheduled task, if the weekly check is on;
//! - short-lived files while updating or turning on the weekly check, which
//!   are removed straight away. If the app is killed in the middle, the next
//!   start sweeps them up ([`sweep_leftovers`]).

use crate::config::Config;
use crate::schedule;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Where the update is downloaded to, next to the exe.
pub const UPDATE_DOWNLOAD: &str = "agent-cleanup.update.exe";
/// The weekly check's task definition, written to `%TEMP%` for `schtasks`.
pub const TASK_XML: &str = "agent-cleanup-task.xml";
/// Names the self-replace step of an update gives the old exe while it's
/// swapped out and deleted (`.agent-cleanup.<random>.__relocated__.exe`).
const SWAP_SUFFIXES: [&str; 3] = [".__relocated__.exe", ".__selfdelete__.exe", ".__temp__.exe"];
/// Files younger than this may still be in use by an update that's running.
const MIN_AGE: Duration = Duration::from_secs(10 * 60);

/// The folder holding settings, the last scan and the action log.
pub fn data_dir() -> Option<PathBuf> {
    Config::data_dir()
}

/// The files in the data folder with their sizes, for showing what would go.
pub fn data_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            (e.path(), size)
        })
        .collect();
    files.sort();
    files
}

/// Whether `name` is a temporary file from the app's updater or weekly check
/// setup. `exe_stem` is the running exe's name without `.exe`, which the
/// self-replace step puts into its file names.
pub fn is_leftover(name: &str, exe_stem: &str) -> bool {
    if name.eq_ignore_ascii_case(UPDATE_DOWNLOAD) || name.eq_ignore_ascii_case(TASK_XML) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    let ours = lower.contains("agent-cleanup")
        || (!exe_stem.is_empty()
            && lower.contains(&format!(".{}.", exe_stem.to_ascii_lowercase())));
    lower.starts_with('.') && ours && SWAP_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// Deletes the app's own temporary files in `dir` that are older than
/// `min_age`. Returns the paths it removed. Files still in use stay put.
pub fn sweep_dir(dir: &Path, exe_stem: &str, min_age: Duration) -> Vec<PathBuf> {
    let now = SystemTime::now();
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_leftover(&name, exe_stem) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        if age < min_age {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed.push(entry.path());
        }
    }
    removed
}

/// The folders the app's temporary files can end up in: next to the exe
/// and `%TEMP%`.
fn temp_places() -> (Vec<PathBuf>, String) {
    let exe = std::env::current_exe().ok();
    let stem = exe
        .as_deref()
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = exe
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    let tmp = std::env::temp_dir();
    if !dirs.iter().any(|d| crate::config::same_path(d, &tmp)) {
        dirs.push(tmp);
    }
    (dirs, stem)
}

/// Removes temporary files an interrupted update or weekly check setup left
/// behind. Safe to run at every start; recent files are left alone.
pub fn sweep_leftovers() -> Vec<PathBuf> {
    let (dirs, stem) = temp_places();
    dirs.iter()
        .flat_map(|d| sweep_dir(d, &stem, MIN_AGE))
        .collect()
}

/// Deletes the data folder, and its parent `agent-cleanup` folder if that's
/// left empty. Nothing else in the parent is touched.
pub fn remove_data_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    if let Some(parent) = dir.parent()
        && parent
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("agent-cleanup"))
    {
        // Fails (and is left alone) if anything else is in there.
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

/// Removes everything the app keeps on this PC except the exe itself: the
/// weekly check task, settings, last scan, action log and leftover temp
/// files. Nothing is logged afterwards, since that would recreate the log.
/// The app should exit right after, before anything saves again.
pub fn remove_all() -> anyhow::Result<()> {
    let mut problems = Vec::new();
    if cfg!(windows)
        && schedule::status() != schedule::Status::Off
        && let Err(e) = schedule::delete_task()
    {
        problems.push(format!("the weekly check task: {e}"));
    }
    if let Some(dir) = data_dir()
        && let Err(e) = remove_data_dir(&dir)
    {
        problems.push(format!("{}: {e}", dir.display()));
    }
    let (dirs, stem) = temp_places();
    for d in &dirs {
        sweep_dir(d, &stem, Duration::ZERO);
    }
    if problems.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("Couldn't remove {}", problems.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_our_temp_files() {
        let stem = "agent-cleanup-v0.10.1-windows-x64";
        assert!(is_leftover("agent-cleanup.update.exe", stem));
        assert!(is_leftover("agent-cleanup-task.xml", stem));
        assert!(is_leftover(
            ".agent-cleanup-v0.10.1-windows-x64.abcdefgh.__relocated__.exe",
            stem
        ));
        assert!(is_leftover(
            ".agent-cleanup.xyz.__selfdelete__.exe",
            "renamed"
        ));
        assert!(is_leftover(".cleaner.xyz.__temp__.exe", "cleaner"));
        // Another app's self-replace files, and look-alikes, are left alone.
        assert!(!is_leftover(".rustup.xyz.__selfdelete__.exe", stem));
        assert!(!is_leftover("agent-cleanup.exe", stem));
        assert!(!is_leftover("agent-cleanup-v0.10.1-windows-x64.exe", stem));
        assert!(!is_leftover(".agent-cleanup.xyz.exe", stem));
        assert!(!is_leftover(".x.__temp__.exe", ""));
    }

    #[test]
    fn sweep_removes_old_leftovers_only() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for n in [
            "agent-cleanup.update.exe",
            ".agent-cleanup.abc.__relocated__.exe",
            "agent-cleanup.exe",
            "notes.txt",
        ] {
            std::fs::write(d.join(n), "x").unwrap();
        }
        std::fs::create_dir(d.join("agent-cleanup-task.xml")).unwrap();
        // Brand new files may belong to an update in progress.
        assert!(sweep_dir(d, "agent-cleanup", MIN_AGE).is_empty());
        let mut gone = sweep_dir(d, "agent-cleanup", Duration::ZERO);
        gone.sort();
        assert_eq!(
            gone,
            vec![
                d.join(".agent-cleanup.abc.__relocated__.exe"),
                d.join("agent-cleanup.update.exe"),
            ]
        );
        assert!(d.join("agent-cleanup.exe").exists());
        assert!(d.join("notes.txt").exists());
        assert!(d.join("agent-cleanup-task.xml").is_dir());
    }

    #[test]
    fn removes_data_folder_and_empty_parent_only() {
        let root = tempfile::tempdir().unwrap();
        let cfg = root.path().join("agent-cleanup").join("config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.toml"), "a").unwrap();
        std::fs::write(cfg.join("actions.log"), "bb").unwrap();
        let files = data_files(&cfg);
        assert_eq!(files.len(), 2);
        assert_eq!(files.iter().map(|f| f.1).sum::<u64>(), 3);
        remove_data_dir(&cfg).unwrap();
        assert!(!root.path().join("agent-cleanup").exists());
        assert!(root.path().exists());
        // Already gone is fine.
        remove_data_dir(&cfg).unwrap();

        // Something else in the parent keeps the parent.
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(root.path().join("agent-cleanup").join("other"), "").unwrap();
        remove_data_dir(&cfg).unwrap();
        assert!(!cfg.exists());
        assert!(root.path().join("agent-cleanup").join("other").exists());
    }
}
