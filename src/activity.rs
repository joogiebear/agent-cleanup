//! Reads `actions.log` back for the Activity page.

use crate::config::Config;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `YYYY-MM-DDTHH:MM:SSZ`, as written by `actions::utc_timestamp`.
    pub when: String,
    pub action: String,
    pub path: PathBuf,
}

impl Entry {
    /// Plain-language verb for the action column.
    pub fn verb(&self) -> String {
        match self.action.as_str() {
            "recycled" => "Moved to Recycle Bin".to_string(),
            "restored" => "Put back".to_string(),
            other => {
                if let Some(v) = other.strip_prefix("updated to ") {
                    format!("Updated the app to {v}")
                } else if let Some(cmd) = other.strip_prefix("ran ") {
                    format!("Opened a terminal to run {cmd}")
                } else if let Some(to) = other.strip_prefix("moved to ") {
                    format!("Moved into {to}")
                } else if let Some(from) = other.strip_prefix("moved back from ") {
                    format!("Moved back from {from}")
                } else {
                    other.to_string()
                }
            }
        }
    }

    /// `2026-11-07 17:23` (UTC) for display.
    pub fn when_short(&self) -> String {
        let s = self.when.trim_end_matches('Z').replacen('T', " ", 1);
        s.get(..16).unwrap_or(&s).to_string()
    }
}

pub fn log_path() -> Option<PathBuf> {
    Config::data_dir().map(|d| d.join("actions.log"))
}

/// Parses log text, newest first. Malformed lines are skipped.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let when = parts.next()?.trim();
            let action = parts.next()?.trim();
            let path = parts.next()?.trim();
            (!when.is_empty() && !action.is_empty() && !path.is_empty()).then(|| Entry {
                when: when.to_string(),
                action: action.to_string(),
                path: PathBuf::from(path),
            })
        })
        .collect();
    out.reverse();
    out
}

/// The newest `limit` entries from the log, or nothing if there's no log yet.
pub fn load(limit: usize) -> Vec<Entry> {
    load_from(log_path().as_deref(), limit)
}

pub fn load_from(path: Option<&Path>, limit: usize) -> Vec<Entry> {
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let mut entries = parse(&text);
    entries.truncate(limit);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_newest_first_and_skips_junk() {
        let text = "2026-11-07T10:00:00Z\trecycled\tC:\\Users\\me\\Downloads\\clone\n\
                    garbage line\n\
                    \n\
                    2026-11-07T10:05:00Z\trestored\tC:\\Users\\me\\Downloads\\clone\n";
        let e = parse(text);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].action, "restored");
        assert_eq!(e[0].verb(), "Put back");
        assert_eq!(e[1].verb(), "Moved to Recycle Bin");
        let u = parse("2026-01-01T00:00:00Z\tupdated to v0.3.0\tC:\\a.exe");
        assert_eq!(u[0].verb(), "Updated the app to v0.3.0");
        assert_eq!(e[1].when_short(), "2026-11-07 10:00");
        assert_eq!(e[1].path, PathBuf::from("C:\\Users\\me\\Downloads\\clone"));
    }

    #[test]
    fn paths_with_tabs_survive() {
        let e = parse("2026-01-01T00:00:00Z\trecycled\t/tmp/a\tb\n");
        assert_eq!(e[0].path, PathBuf::from("/tmp/a\tb"));
    }

    #[test]
    fn missing_log_is_empty() {
        assert!(load_from(Some(Path::new("/definitely/not/here.log")), 10).is_empty());
        assert!(load_from(None, 10).is_empty());
    }
}
