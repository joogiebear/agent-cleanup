//! The weekly background check: scans quietly, without a window, and sums
//! up what the app would offer to clean. Nothing is moved; the user decides
//! in the app. The scheduled task that runs it lives in [`crate::schedule`].

use crate::cache::ScanCache;
use crate::config::Config;
use crate::library::Library;
use crate::model::human_size;
use crate::{junk, scan, trail};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, SystemTime};

/// Below this, a check stays quiet unless it found a whole repo.
pub const QUIET_BELOW: u64 = 100 * 1024 * 1024;

/// What the app would offer to clean, counting only things it marks safe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Repos outside your projects, unused and fully pushed.
    pub repos: usize,
    pub repo_bytes: u64,
    /// Agent trail items with no warning.
    pub trail: usize,
    pub trail_bytes: u64,
    /// Build folders in projects you haven't touched for a while.
    pub junk: usize,
    pub junk_bytes: u64,
    /// Agent session data for deleted projects, not used in the last week.
    pub agent_data: usize,
    pub agent_data_bytes: u64,
}

impl Summary {
    pub fn total_bytes(&self) -> u64 {
        self.repo_bytes + self.trail_bytes + self.junk_bytes + self.agent_data_bytes
    }

    /// Whether this is worth a notification.
    pub fn worth_telling(&self) -> bool {
        self.repos > 0 || self.total_bytes() >= QUIET_BELOW
    }

    /// Notification title and body, or `None` to stay quiet.
    pub fn message(&self) -> Option<(String, String)> {
        if !self.worth_telling() {
            return None;
        }
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.repos > 0 {
            parts.push(format!(
                "{} safe to remove ({})",
                plural(self.repos, "old project copy", "old project copies"),
                human_size(self.repo_bytes)
            ));
        }
        if self.trail > 0 {
            parts.push(format!(
                "{} left by agents ({})",
                plural(self.trail, "file", "files"),
                human_size(self.trail_bytes)
            ));
        }
        if self.junk > 0 {
            parts.push(format!(
                "{} of build folders in idle projects",
                human_size(self.junk_bytes)
            ));
        }
        if self.agent_data > 0 {
            parts.push(format!(
                "{} of agent history for deleted projects",
                human_size(self.agent_data_bytes)
            ));
        }
        let title = format!("{} to clean up", human_size(self.total_bytes()));
        Some((title, format!("{}. Click to review.", parts.join(", "))))
    }
}

/// Scans like the app does, saves the results so the app opens with them,
/// and sums up what's safe. Takes a while on a big disk; that's fine here.
pub fn run(cfg: &Config, home: Option<&Path>) -> Summary {
    let cancel = AtomicBool::new(false);
    let now = SystemTime::now();

    let mut paths = Vec::new();
    scan::find_repos(cfg, &cancel, |p| paths.push(p), |_, _| {});
    let repos: Vec<_> = paths
        .iter()
        .filter(|p| !cfg.is_ignored(p))
        .map(|p| scan::inspect_repo(p, cfg, &cancel))
        .collect();
    let _ = ScanCache {
        format: crate::cache::FORMAT,
        scanned_at: now,
        repos: repos.clone(),
    }
    .save();
    let lib = Library::new(repos);
    let stats = lib.stats(cfg, now);

    let mut s = Summary {
        repos: stats.safe,
        repo_bytes: stats.safe_bytes,
        ..Summary::default()
    };

    if let Some(home) = home {
        let since = now - Duration::from_secs(trail::DAYS * 86_400);
        for it in trail::collect(cfg, home, since, &cancel) {
            if !it.needs_care() {
                s.trail += 1;
                s.trail_bytes += it.size_bytes;
            }
        }
        let known = crate::agentdata::known_projects(&paths, cfg);
        for o in crate::agentdata::find(home, cfg, &known, &cancel) {
            if o.warning(now).is_none() {
                s.agent_data += 1;
                s.agent_data_bytes += o.size_bytes;
            }
        }
    }

    let with_activity: Vec<_> = lib
        .repos()
        .iter()
        .map(|r| (r.path.clone(), r.last_activity))
        .collect();
    let items = junk::find(&with_activity, cfg, &cancel, &AtomicUsize::new(0));
    let js = junk::stats(&items, cfg.stale_after_days, now);
    s.junk = js.idle_count;
    s.junk_bytes = js.idle_bytes;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn quiet_when_small() {
        let s = Summary {
            trail: 3,
            trail_bytes: 2 * MB,
            ..Summary::default()
        };
        assert_eq!(s.message(), None);
        assert_eq!(Summary::default().message(), None);
    }

    #[test]
    fn says_what_was_found() {
        let s = Summary {
            repos: 2,
            repo_bytes: 300 * MB,
            trail: 1,
            trail_bytes: 5 * MB,
            junk: 4,
            junk_bytes: 1200 * MB,
            ..Summary::default()
        };
        let (title, body) = s.message().unwrap();
        assert_eq!(title, format!("{} to clean up", human_size(1505 * MB)));
        assert_eq!(
            body,
            format!(
                "2 old project copies safe to remove ({}), 1 file left by agents ({}), {} of build folders in idle projects. Click to review.",
                human_size(300 * MB),
                human_size(5 * MB),
                human_size(1200 * MB)
            )
        );
        // One small repo is still worth a mention.
        let one = Summary {
            repos: 1,
            repo_bytes: MB,
            ..Summary::default()
        };
        assert!(
            one.message()
                .unwrap()
                .1
                .starts_with("1 old project copy safe")
        );
    }
}
