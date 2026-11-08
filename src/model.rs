//! A found repo plus the judgement calls about it.

use crate::config::Config;
use crate::git::GitState;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Repo {
    pub path: PathBuf,
    /// The project folder this repo lives in, or `None` if it's a stray.
    pub project_root: Option<PathBuf>,
    /// `false` until git state and size have been read.
    pub inspected: bool,
    pub git: Option<GitState>,
    pub error: Option<String>,
    /// Whole folder, including dependencies and build output.
    pub size_bytes: Option<u64>,
    /// Newest of: last commit, last checkout/fetch/stage.
    pub last_activity: Option<SystemTime>,
}

/// What the app thinks you should do about a repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Stray, no recent activity, nothing that exists only here.
    SafeToRemove,
    /// Stray, but has commits/changes/stashes that exist only on this PC.
    UnsavedWork,
    /// Stray, and git couldn't read it, so what's in it is unknown.
    Unreadable,
    /// Stray but used recently; maybe still in use.
    StrayActive,
    InProject,
    Checking,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::SafeToRemove => "Safe to remove",
            Verdict::UnsavedWork => "Has unsaved work",
            Verdict::Unreadable => "Couldn't check",
            Verdict::StrayActive => "Used recently",
            Verdict::InProject => "In a project",
            Verdict::Checking => "Checking…",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Verdict::SafeToRemove => {
                "A copy of a project outside your project folders. It hasn't been used in a while and everything in it is already saved online (for example on GitHub), so removing it loses nothing."
            }
            Verdict::UnsavedWork => {
                "Outside your project folders, and some of its work is saved only on this PC. Removing it would lose that work, so check it first."
            }
            Verdict::Unreadable => {
                "The app couldn't look inside it, so it can't tell whether it holds unsaved work. Copies in a temp folder are almost always throwaway copies an AI agent made. Anywhere else, open it and look first."
            }
            Verdict::StrayActive => {
                "Outside your project folders, but it was used in the last few days, so it may still be in use."
            }
            Verdict::InProject => "Inside one of your project folders, so the app leaves it alone.",
            Verdict::Checking => "Still checking what's in it.",
        }
    }
}

impl Repo {
    pub fn new(path: PathBuf, cfg: &Config) -> Self {
        let project_root = cfg.project_root_for(&path).cloned();
        Self {
            path,
            project_root,
            inspected: false,
            git: None,
            error: None,
            size_bytes: None,
            last_activity: None,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    pub fn is_stray(&self) -> bool {
        self.project_root.is_none()
    }

    /// Unknown state (git failed) is treated as having local work, to stay safe.
    pub fn has_local_work(&self) -> bool {
        self.git.as_ref().is_none_or(GitState::has_local_work)
    }

    /// Lives in a temp folder (like `AppData\Local\Temp`), where agents put
    /// throwaway clones. A hint, not a guarantee.
    pub fn in_temp_folder(&self) -> bool {
        is_temp_path(&self.path, Some(&std::env::temp_dir()))
    }

    pub fn is_stale(&self, stale_after_days: u32, now: SystemTime) -> bool {
        let limit = Duration::from_secs(u64::from(stale_after_days) * 86_400);
        match self.last_activity {
            Some(t) => now.duration_since(t).unwrap_or_default() >= limit,
            None => true,
        }
    }

    pub fn verdict(&self, cfg: &Config, now: SystemTime) -> Verdict {
        if !self.inspected {
            Verdict::Checking
        } else if !self.is_stray() {
            Verdict::InProject
        } else if self.error.is_some() || self.git.is_none() {
            Verdict::Unreadable
        } else if self.has_local_work() {
            Verdict::UnsavedWork
        } else if self.is_stale(cfg.stale_after_days, now) {
            Verdict::SafeToRemove
        } else {
            Verdict::StrayActive
        }
    }

    /// What to do about this repo, in plain language, most important first.
    pub fn next_steps(&self, cfg: &Config, now: SystemTime) -> Vec<String> {
        let v = self.verdict(cfg, now);
        let mut steps = Vec::new();
        match v {
            Verdict::Checking => return steps,
            Verdict::InProject => {
                steps.push("Nothing to do. This is one of your projects.".to_string());
                return steps;
            }
            _ => {}
        }
        if v == Verdict::Unreadable {
            steps.push(match &self.error {
                Some(e) => format!("The app couldn't look inside it. Git (the tool that tracks its history) said: {e}"),
                None => "The app couldn't look inside it.".to_string(),
            });
            if self.in_temp_folder() {
                steps.push(
                    "It's in a temp folder, which is where agents put throwaway copies. It's almost always fine to recycle."
                        .to_string(),
                );
            } else {
                steps.push(
                    "Open the folder and look. If you see files you wrote or changed yourself, keep it, or copy them out first."
                        .to_string(),
                );
                steps.push(
                    "If it's just a copy of a project you have elsewhere or on GitHub, it's fine to recycle."
                        .to_string(),
                );
            }
            steps.push(
                "Recycling moves it to the Recycle Bin, so you can restore it if you change your mind."
                    .to_string(),
            );
            return steps;
        }
        if let Some(g) = &self.git {
            if g.changed_files > 0 {
                steps.push(format!(
                    "It has {} that {} never saved into its history (committed). To keep them, ask the AI agent that made it to commit them, or run `git status` in a terminal here to see them.",
                    if g.changed_files == 1 {
                        "1 edited file".to_string()
                    } else {
                        format!("{} edited files", g.changed_files)
                    },
                    if g.changed_files == 1 { "was" } else { "were" }
                ));
            }
            if g.stashes > 0 {
                steps.push(
                    "It has changes set aside for later (a stash), which are lost if the folder is removed. Run `git stash list` in a terminal here to see them."
                        .to_string(),
                );
            }
            match (&g.remote_url, g.unpushed_commits) {
                (Some(_), n) if n > 0 => steps.push(format!(
                    "It has {} saved on this PC but not uploaded (pushed). To keep them, ask your agent to push them, or run `git push` in a terminal here.",
                    if n == 1 { "1 saved version".to_string() } else { format!("{n} saved versions") }
                )),
                (None, _) => steps.push(
                    "It isn't on GitHub or anywhere else online, so this folder is the only copy. If it matters, ask your agent to put it on GitHub, or mark this folder as a project."
                        .to_string(),
                ),
                _ => {}
            }
        }
        match v {
            Verdict::SafeToRemove => steps.push(format!(
                "Nothing to save. Recycle it to free {}. It's saved online, so it can be downloaded (cloned) again later.",
                self.size_bytes.map(human_size).unwrap_or_else(|| "space".into())
            )),
            Verdict::StrayActive => steps.push(
                "It was used recently, so it might still be in use. If it's your real work, mark it as a project. Otherwise check back in a few days."
                    .to_string(),
            ),
            Verdict::UnsavedWork => {
                steps.push(
                    "If it was only an experiment and you don't need it, you can recycle it anyway. You'll confirm first, and it goes to the Recycle Bin."
                        .to_string(),
                );
                steps.push(
                    "Once the work is saved, scan again and it will show as safe to remove."
                        .to_string(),
                );
            }
            _ => {}
        }
        steps
    }

    /// One-line git summary for the table.
    pub fn git_summary(&self) -> String {
        if let Some(e) = &self.error {
            return format!("Couldn't look inside: {e}");
        }
        let Some(g) = &self.git else {
            return String::new();
        };
        let reasons = g.local_work_reasons();
        if reasons.is_empty() {
            "Nothing unsaved, all saved online".to_string()
        } else {
            let mut s = reasons.join(", ");
            if let Some(first) = s.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            s
        }
    }
}

/// True if `path` is inside the system temp folder or any folder named
/// `Temp` or `tmp`.
pub fn is_temp_path(path: &Path, temp_dir: Option<&Path>) -> bool {
    if let Some(t) = temp_dir
        && !t.as_os_str().is_empty()
        && path.starts_with(t)
    {
        return true;
    }
    path.parent().is_some_and(|parent| {
        parent.components().any(|c| {
            let n = c.as_os_str().to_string_lossy();
            n.eq_ignore_ascii_case("temp") || n.eq_ignore_ascii_case("tmp")
        })
    })
}

/// Shortens paths under the home folder to `~\...` for display.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        let sep = std::path::MAIN_SEPARATOR;
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~{sep}{}", rest.display())
        };
    }
    path.display().to_string()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

pub fn relative_time(t: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(t).unwrap_or_default().as_secs();
    let (n, unit) = match secs {
        0..60 => return "just now".to_string(),
        60..3_600 => (secs / 60, "minute"),
        3_600..86_400 => (secs / 3_600, "hour"),
        86_400..1_209_600 => (secs / 86_400, "day"),
        1_209_600..5_184_000 => (secs / 604_800, "week"),
        5_184_000..31_536_000 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspected(path: &str, git: GitState, age_days: u64, cfg: &Config, now: SystemTime) -> Repo {
        let mut r = Repo::new(PathBuf::from(path), cfg);
        r.inspected = true;
        r.git = Some(git);
        r.last_activity = Some(now - Duration::from_secs(age_days * 86_400));
        r
    }

    #[test]
    fn verdicts() {
        let cfg = Config {
            project_roots: vec![PathBuf::from("/work")],
            stale_after_days: 14,
            ..Config::default()
        };
        let now = SystemTime::now();
        let pushed = GitState {
            remote_url: Some("https://github.com/a/b".into()),
            ..Default::default()
        };
        let dirty = GitState {
            changed_files: 2,
            ..pushed.clone()
        };

        assert_eq!(
            inspected("/tmp/old", pushed.clone(), 30, &cfg, now).verdict(&cfg, now),
            Verdict::SafeToRemove
        );
        assert_eq!(
            inspected("/tmp/new", pushed.clone(), 2, &cfg, now).verdict(&cfg, now),
            Verdict::StrayActive
        );
        assert_eq!(
            inspected("/tmp/dirty", dirty, 30, &cfg, now).verdict(&cfg, now),
            Verdict::UnsavedWork
        );
        assert_eq!(
            inspected("/work/app", pushed, 300, &cfg, now).verdict(&cfg, now),
            Verdict::InProject
        );

        let mut failed = Repo::new(PathBuf::from("/tmp/broken"), &cfg);
        failed.inspected = true;
        assert_eq!(
            failed.verdict(&cfg, now),
            Verdict::Unreadable,
            "unknown state must not look safe"
        );
        assert!(failed.has_local_work(), "and still counts as unsaved");
    }

    #[test]
    fn next_steps_explain_what_to_do() {
        let cfg = Config::default();
        let now = SystemTime::now();
        let dirty_unpushed = GitState {
            changed_files: 2,
            unpushed_commits: 1,
            remote_url: Some("https://github.com/a/b".into()),
            ..Default::default()
        };
        let steps = inspected("/tmp/x", dirty_unpushed, 30, &cfg, now).next_steps(&cfg, now);
        assert!(steps[0].contains("2 edited files"));
        assert!(steps[1].contains("git push"));
        assert!(steps.iter().any(|s| s.contains("Recycle Bin")));
        assert!(steps.last().unwrap().contains("scan again"));

        let lone = inspected("/tmp/y", GitState::default(), 30, &cfg, now).next_steps(&cfg, now);
        assert!(lone.iter().any(|s| s.contains("isn't on GitHub")));

        let pushed = GitState {
            remote_url: Some("x".into()),
            ..Default::default()
        };
        let safe = inspected("/tmp/z", pushed, 30, &cfg, now).next_steps(&cfg, now);
        assert_eq!(safe.len(), 1);
        assert!(safe[0].starts_with("Nothing to save"));
    }

    #[test]
    fn temp_folders_are_spotted() {
        let t = Path::new("/sys-temp");
        assert!(is_temp_path(Path::new("/sys-temp/clone"), Some(t)));
        assert!(is_temp_path(
            Path::new("/home/me/AppData/Local/Temp/royal/x"),
            None
        ));
        assert!(is_temp_path(Path::new("/tmp/x"), None));
        assert!(!is_temp_path(Path::new("/home/me/code/app"), Some(t)));
        // The repo itself being called "temp" doesn't count.
        assert!(!is_temp_path(Path::new("/home/me/code/temp"), None));
    }

    #[test]
    fn unreadable_steps_depend_on_location() {
        let cfg = Config::default();
        let now = SystemTime::now();
        let mut tmp = Repo::new(PathBuf::from("/x/Temp/clone"), &cfg);
        tmp.inspected = true;
        tmp.error = Some("not a git repository".into());
        let steps = tmp.next_steps(&cfg, now);
        assert!(steps[0].contains("not a git repository"));
        assert!(steps.iter().any(|s| s.contains("temp folder")));
        assert!(steps.last().unwrap().contains("Recycle Bin"));

        let mut real = Repo::new(PathBuf::from("/x/code/app"), &cfg);
        real.inspected = true;
        let steps = real.next_steps(&cfg, now);
        assert!(steps.iter().any(|s| s.contains("Open the folder")));
    }

    #[test]
    fn home_paths_are_shortened() {
        let home = Path::new("/home/me");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            display_path(Path::new("/home/me/Downloads/x"), Some(home)),
            format!("~{sep}Downloads/x")
        );
        assert_eq!(display_path(Path::new("/home/me"), Some(home)), "~");
        assert_eq!(display_path(Path::new("/opt/x"), Some(home)), "/opt/x");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(250 * 1024 * 1024), "250 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn times() {
        let now = SystemTime::now();
        let ago = |s: u64| relative_time(now - Duration::from_secs(s), now);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(3_600), "1 hour ago");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(ago(21 * 86_400), "3 weeks ago");
        assert_eq!(ago(90 * 86_400), "3 months ago");
        assert_eq!(ago(800 * 86_400), "2 years ago");
    }
}
