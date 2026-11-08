//! Remembers the last scan's results so the app opens instantly, without
//! walking the whole disk again every launch.

use crate::config::Config;
use crate::model::Repo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Bumped when what a scan records changes, so older results get rescanned
/// instead of shown. 1: git-ignored private files are checked. 2: private
/// files are looked for in repos git can't read.
pub const FORMAT: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct ScanCache {
    /// [`FORMAT`] when it was written; 0 for caches from before it existed.
    #[serde(default)]
    pub format: u32,
    pub scanned_at: SystemTime,
    pub repos: Vec<Repo>,
}

impl ScanCache {
    pub fn file_path() -> Option<PathBuf> {
        Config::data_dir().map(|d| d.join("last_scan.toml"))
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string(self)?)?;
        Ok(())
    }

    /// Loads a cache, dropping repos that no longer exist on disk and
    /// re-classifying the rest against the current project folders.
    pub fn load_from(path: &Path, cfg: &Config) -> Option<Self> {
        let mut cache: Self = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        if cache.format != FORMAT {
            return None;
        }
        cache
            .repos
            .retain(|r| r.inspected && r.path.join(".git").exists() && !cfg.is_ignored(&r.path));
        for r in &mut cache.repos {
            r.project_root = cfg.project_root_for(&r.path).cloned();
        }
        Some(cache)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::file_path().ok_or_else(|| anyhow::anyhow!("no data folder available"))?;
        self.save_to(&path)
    }

    pub fn load(cfg: &Config) -> Option<Self> {
        Self::load_from(&Self::file_path()?, cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitState;

    #[test]
    fn round_trips_and_drops_missing_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let alive = tmp.path().join("alive");
        std::fs::create_dir_all(alive.join(".git")).unwrap();
        let cfg = Config {
            project_roots: vec![tmp.path().to_path_buf()],
            ..Config::default()
        };
        let mut a = Repo::new(alive.clone(), &Config::default());
        a.inspected = true;
        a.size_bytes = Some(42);
        a.last_activity = Some(SystemTime::now());
        a.git = Some(GitState {
            remote_url: Some("https://github.com/a/b".into()),
            ..Default::default()
        });
        let mut gone = Repo::new(tmp.path().join("gone"), &Config::default());
        gone.inspected = true;

        let file = tmp.path().join("cache.toml");
        ScanCache {
            format: FORMAT,
            scanned_at: SystemTime::now(),
            repos: vec![a, gone],
        }
        .save_to(&file)
        .unwrap();

        let back = ScanCache::load_from(&file, &cfg).unwrap();
        // A cache from before `format` existed is dropped, so it gets rescanned.
        let old = tmp.path().join("old.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&old, text.replace(&format!("format = {FORMAT}\n"), "")).unwrap();
        assert!(ScanCache::load_from(&old, &cfg).is_none());
        assert_eq!(back.repos.len(), 1);
        let r = &back.repos[0];
        assert_eq!(r.path, alive);
        assert_eq!(r.size_bytes, Some(42));
        assert_eq!(
            r.project_root.as_deref(),
            Some(tmp.path()),
            "re-classified on load"
        );
        assert!(r.git.as_ref().unwrap().remote_url.is_some());
    }
}
