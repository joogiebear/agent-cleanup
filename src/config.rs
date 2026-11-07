//! User settings, stored as TOML in the per-user config folder
//! (`%APPDATA%\agent-cleanup\config\config.toml` on Windows).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Folders that hold your real projects. Repos inside these are "yours";
    /// anything found elsewhere is treated as a possible leftover.
    pub project_roots: Vec<PathBuf>,
    /// Where to look for repos. Defaults to your home folder.
    pub scan_roots: Vec<PathBuf>,
    /// Repos you've told the app to stop showing.
    pub ignored: Vec<PathBuf>,
    /// Folder names never descended into (dependency caches, toolchains, ...).
    pub skip_dir_names: Vec<String>,
    /// Absolute folders never descended into.
    pub skip_paths: Vec<PathBuf>,
    /// A repo with no activity for this many days counts as stale.
    pub stale_after_days: u32,
}

impl Default for Config {
    fn default() -> Self {
        let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
        let skip_paths = home
            .iter()
            .flat_map(|h| {
                [
                    h.join("AppData").join("Local").join("Microsoft"),
                    h.join("AppData").join("Local").join("Packages"),
                    h.join("AppData").join("Roaming").join("Microsoft"),
                ]
            })
            .collect();
        Self {
            project_roots: Vec::new(),
            scan_roots: home.into_iter().collect(),
            ignored: Vec::new(),
            skip_dir_names: [
                "node_modules",
                "target",
                ".venv",
                "venv",
                "__pycache__",
                ".cargo",
                ".rustup",
                ".gradle",
                ".m2",
                ".nuget",
                ".npm",
                "$Recycle.Bin",
                "System Volume Information",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            skip_paths,
            stale_after_days: 14,
        }
    }
}

impl Config {
    /// Folder holding the config file and the action log.
    pub fn data_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "agent-cleanup")
            .map(|d| d.config_dir().to_path_buf())
    }

    pub fn file_path() -> Option<PathBuf> {
        Self::data_dir().map(|d| d.join("config.toml"))
    }

    /// Loads the saved config, or the defaults if there is none (or it can't be read).
    pub fn load() -> Self {
        Self::file_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path =
            Self::file_path().ok_or_else(|| anyhow::anyhow!("no config folder available"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// The project folder that contains `path`, if any.
    pub fn project_root_for(&self, path: &Path) -> Option<&PathBuf> {
        self.project_roots.iter().find(|r| path_within(path, r))
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        self.ignored.iter().any(|i| same_path(i, path))
    }

    pub fn is_skipped_path(&self, path: &Path) -> bool {
        self.skip_paths.iter().any(|s| same_path(s, path))
    }
}

/// Comparable form of a path: forward slashes, no trailing slash,
/// lowercase on Windows (where paths are case-insensitive).
fn normalize(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    if cfg!(windows) { s.to_lowercase() } else { s }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// True if `path` is `root` or somewhere inside it.
pub fn path_within(path: &Path, root: &Path) -> bool {
    let (p, r) = (normalize(path), normalize(root));
    p == r || (p.starts_with(&r) && p[r.len()..].starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_checks_whole_components() {
        assert!(path_within(
            Path::new("/home/a/projects/x"),
            Path::new("/home/a/projects")
        ));
        assert!(path_within(
            Path::new("/home/a/projects"),
            Path::new("/home/a/projects/")
        ));
        assert!(!path_within(
            Path::new("/home/a/projects-old/x"),
            Path::new("/home/a/projects")
        ));
    }

    #[test]
    fn project_root_lookup() {
        let cfg = Config {
            project_roots: vec![PathBuf::from("/work")],
            ..Config::default()
        };
        assert_eq!(
            cfg.project_root_for(Path::new("/work/app")),
            Some(&PathBuf::from("/work"))
        );
        assert_eq!(cfg.project_root_for(Path::new("/tmp/clone")), None);
    }

    #[test]
    fn config_round_trips_and_tolerates_missing_fields() {
        let cfg = Config::default();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, back);
        let partial: Config = toml::from_str("stale_after_days = 30").unwrap();
        assert_eq!(partial.stale_after_days, 30);
        assert!(!partial.skip_dir_names.is_empty());
    }
}
