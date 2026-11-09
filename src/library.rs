//! The list of found repos and the views of it the window shows: filters,
//! search, sorting, selection and the headline numbers. No UI code here, so
//! it can be tested.

use crate::config::Config;
use crate::model::{Repo, Verdict};
use crate::scan::ScanEvent;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Everything outside the project folders.
    Strays,
    SafeToRemove,
    UnsavedWork,
    Unreadable,
    StrayActive,
    /// Including repos inside project folders.
    All,
}

impl Filter {
    pub const ALL: [Filter; 6] = [
        Filter::Strays,
        Filter::SafeToRemove,
        Filter::UnsavedWork,
        Filter::Unreadable,
        Filter::StrayActive,
        Filter::All,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Strays => "All leftovers",
            Filter::SafeToRemove => "Safe to remove",
            Filter::UnsavedWork => "Unsaved work",
            Filter::Unreadable => "Couldn't check",
            Filter::StrayActive => "Recent",
            Filter::All => "Everything",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Filter::Strays => {
                "Every project copy (git repo) found outside your project folders. These are the ones to look at."
            }
            Filter::SafeToRemove => Verdict::SafeToRemove.explanation(),
            Filter::UnsavedWork => Verdict::UnsavedWork.explanation(),
            Filter::Unreadable => Verdict::Unreadable.explanation(),
            Filter::StrayActive => Verdict::StrayActive.explanation(),
            Filter::All => {
                "Everything found, including the projects inside your own project folders."
            }
        }
    }

    pub fn keeps(self, v: Verdict, stray: bool) -> bool {
        match self {
            Filter::Strays => stray,
            Filter::SafeToRemove => v == Verdict::SafeToRemove,
            Filter::UnsavedWork => v == Verdict::UnsavedWork,
            Filter::Unreadable => v == Verdict::Unreadable,
            Filter::StrayActive => v == Verdict::StrayActive,
            Filter::All => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Size,
    Activity,
    Name,
    Status,
}

impl SortBy {
    pub const ALL: [SortBy; 4] = [SortBy::Size, SortBy::Activity, SortBy::Name, SortBy::Status];

    pub fn label(self) -> &'static str {
        match self {
            SortBy::Size => "Largest first",
            SortBy::Activity => "Oldest first",
            SortBy::Name => "Name",
            SortBy::Status => "Status",
        }
    }
}

/// Headline numbers for the stat cards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub strays: usize,
    pub safe: usize,
    pub safe_bytes: u64,
    pub unsaved: usize,
    /// Strays git couldn't read.
    pub unreadable: usize,
    pub recent: usize,
    pub in_projects: usize,
    pub checking: usize,
    pub stray_bytes: u64,
}

#[derive(Default)]
pub struct Library {
    repos: Vec<Repo>,
    index: HashMap<PathBuf, usize>,
    /// Ticked repos, for bulk actions.
    pub checked: HashSet<PathBuf>,
}

impl Library {
    pub fn new(repos: Vec<Repo>) -> Self {
        let mut lib = Self {
            repos,
            ..Self::default()
        };
        lib.reindex();
        lib
    }

    fn reindex(&mut self) {
        self.index = self
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.clone(), i))
            .collect();
    }

    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    pub fn get(&self, path: &Path) -> Option<&Repo> {
        self.index.get(path).map(|&i| &self.repos[i])
    }

    pub fn clear(&mut self) {
        self.repos.clear();
        self.index.clear();
        self.checked.clear();
    }

    /// Applies one event from a running scan. Returns the scan's summary once
    /// it's done: `(dirs, elapsed, cancelled)`.
    pub fn apply(
        &mut self,
        ev: ScanEvent,
        cfg: &Config,
    ) -> Option<(u64, std::time::Duration, bool)> {
        match ev {
            ScanEvent::Progress { .. } => {}
            ScanEvent::Found(path) => {
                if !cfg.is_ignored(&path) && !self.index.contains_key(&path) {
                    self.index.insert(path.clone(), self.repos.len());
                    self.repos.push(Repo::new(path, cfg));
                }
            }
            ScanEvent::Inspected(repo) => {
                if let Some(&i) = self.index.get(&repo.path) {
                    self.repos[i] = *repo;
                }
            }
            ScanEvent::Done {
                dirs,
                elapsed,
                cancelled,
            } => return Some((dirs, elapsed, cancelled)),
        }
        None
    }

    pub fn push(&mut self, repo: Repo) {
        if !self.index.contains_key(&repo.path) {
            self.index.insert(repo.path.clone(), self.repos.len());
            self.repos.push(repo);
        }
    }

    /// Removes and returns the repos at `paths` (for recycle and hide).
    pub fn remove(&mut self, paths: &[PathBuf]) -> Vec<Repo> {
        let gone: HashSet<&PathBuf> = paths.iter().collect();
        let mut removed = Vec::new();
        self.repos.retain(|r| {
            if gone.contains(&r.path) {
                removed.push(r.clone());
                false
            } else {
                true
            }
        });
        self.checked.retain(|p| !gone.contains(p));
        self.reindex();
        removed
    }

    /// Reads git again for the repos in `paths` that the last scan found
    /// nothing unsaved in, right before they're recycled. The scan can be
    /// days old (results are saved between runs), and work may have been done
    /// in a repo since. Any that now hold unsaved work, or that git can no
    /// longer read, are updated in the list and returned, so the caller
    /// leaves them where they are.
    pub fn recheck_clean(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for p in paths {
            let Some(&i) = self.index.get(p) else {
                continue;
            };
            let r = &mut self.repos[i];
            // Already flagged: the confirm dialog showed what would be lost.
            // Gone from disk: recycling reports that itself.
            if !r.inspected || r.has_local_work() || !p.exists() {
                continue;
            }
            match crate::git::inspect(p) {
                Ok(g) if !g.has_local_work() => r.git = Some(g),
                Ok(g) => {
                    r.git = Some(g);
                    changed.push(p.clone());
                }
                Err(e) => {
                    r.git = None;
                    r.error = Some(e.to_string());
                    (r.private_files, r.private_files_total) = crate::private::find(p);
                    changed.push(p.clone());
                }
            }
        }
        changed
    }

    /// Re-reads which project folder each repo is in after settings change.
    pub fn reclassify(&mut self, cfg: &Config) {
        for r in &mut self.repos {
            r.project_root = cfg.project_root_for(&r.path).cloned();
        }
    }

    pub fn stats(&self, cfg: &Config, now: SystemTime) -> Stats {
        let mut s = Stats::default();
        for r in &self.repos {
            let size = r.size_bytes.unwrap_or(0);
            if r.is_stray() {
                s.strays += 1;
                s.stray_bytes += size;
            }
            match r.verdict(cfg, now) {
                Verdict::SafeToRemove => {
                    s.safe += 1;
                    s.safe_bytes += size;
                }
                Verdict::UnsavedWork => s.unsaved += 1,
                Verdict::Unreadable => s.unreadable += 1,
                Verdict::StrayActive => s.recent += 1,
                Verdict::InProject => s.in_projects += 1,
                Verdict::Checking => s.checking += 1,
            }
        }
        s
    }

    pub fn count(&self, filter: Filter, cfg: &Config, now: SystemTime) -> usize {
        self.repos
            .iter()
            .filter(|r| filter.keeps(r.verdict(cfg, now), r.is_stray()))
            .count()
    }

    /// Paths of the repos to show, in display order.
    pub fn visible(
        &self,
        filter: Filter,
        search: &str,
        sort: SortBy,
        cfg: &Config,
        now: SystemTime,
    ) -> Vec<PathBuf> {
        let needle = search.trim().to_lowercase();
        let mut rows: Vec<&Repo> = self
            .repos
            .iter()
            .filter(|r| {
                filter.keeps(r.verdict(cfg, now), r.is_stray())
                    && (needle.is_empty()
                        || r.path.to_string_lossy().to_lowercase().contains(&needle))
            })
            .collect();
        rows.sort_by(|a, b| match sort {
            SortBy::Size => b.size_bytes.cmp(&a.size_bytes),
            // Never-used (None) sorts first: it's the oldest.
            SortBy::Activity => a.last_activity.cmp(&b.last_activity),
            SortBy::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
            SortBy::Status => a
                .verdict(cfg, now)
                .cmp(&b.verdict(cfg, now))
                .then(b.size_bytes.cmp(&a.size_bytes)),
        });
        rows.into_iter().map(|r| r.path.clone()).collect()
    }

    pub fn safe_paths(&self, cfg: &Config, now: SystemTime) -> Vec<PathBuf> {
        self.repos
            .iter()
            .filter(|r| r.verdict(cfg, now) == Verdict::SafeToRemove)
            .map(|r| r.path.clone())
            .collect()
    }

    /// Repos git couldn't read that sit in a temp folder: almost always
    /// throwaway agent clones.
    pub fn unreadable_temp_paths(&self, cfg: &Config, now: SystemTime) -> Vec<PathBuf> {
        self.repos
            .iter()
            .filter(|r| r.verdict(cfg, now) == Verdict::Unreadable && r.is_throwaway_temp_copy())
            .map(|r| r.path.clone())
            .collect()
    }

    /// Ticked repos that still exist, in list order.
    pub fn checked_paths(&self) -> Vec<PathBuf> {
        self.repos
            .iter()
            .filter(|r| self.checked.contains(&r.path))
            .map(|r| r.path.clone())
            .collect()
    }

    pub fn toggle_checked(&mut self, path: &Path) {
        if !self.checked.remove(path) {
            self.checked.insert(path.to_path_buf());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitState;
    use std::time::Duration;

    fn repo(path: &str, cfg: &Config, now: SystemTime, age_days: u64, dirty: bool) -> Repo {
        let mut r = Repo::new(PathBuf::from(path), cfg);
        r.inspected = true;
        r.size_bytes = Some(age_days * 1000);
        r.last_activity = Some(now - Duration::from_secs(age_days * 86_400));
        r.git = Some(GitState {
            remote_url: Some("https://github.com/a/b".into()),
            changed_files: usize::from(dirty),
            ..GitState::default()
        });
        r
    }

    fn setup() -> (Config, SystemTime, Library) {
        let cfg = Config {
            project_roots: vec![PathBuf::from("/work")],
            ..Config::default()
        };
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let lib = Library::new(vec![
            repo("/tmp/old-clone", &cfg, now, 30, false),
            repo("/tmp/dirty", &cfg, now, 40, true),
            repo("/dl/fresh", &cfg, now, 1, false),
            repo("/work/mine", &cfg, now, 50, false),
            repo("/tmp/older-clone", &cfg, now, 60, false),
        ]);
        (cfg, now, lib)
    }

    #[test]
    fn stats_count_each_verdict() {
        let (cfg, now, lib) = setup();
        let s = lib.stats(&cfg, now);
        assert_eq!(s.strays, 4);
        assert_eq!(s.safe, 2);
        assert_eq!(s.safe_bytes, 90_000);
        assert_eq!(s.unsaved, 1);
        assert_eq!(s.recent, 1);
        assert_eq!(s.in_projects, 1);
        assert_eq!(lib.count(Filter::All, &cfg, now), 5);
    }

    #[test]
    fn visible_filters_searches_and_sorts() {
        let (cfg, now, lib) = setup();
        let safe = lib.visible(Filter::SafeToRemove, "", SortBy::Size, &cfg, now);
        assert_eq!(
            safe,
            vec![
                PathBuf::from("/tmp/older-clone"),
                PathBuf::from("/tmp/old-clone")
            ]
        );
        let found = lib.visible(Filter::Strays, "DIRTY", SortBy::Name, &cfg, now);
        assert_eq!(found, vec![PathBuf::from("/tmp/dirty")]);
        let by_age = lib.visible(Filter::Strays, "", SortBy::Activity, &cfg, now);
        assert_eq!(by_age.first(), Some(&PathBuf::from("/tmp/older-clone")));
        assert!(!by_age.contains(&PathBuf::from("/work/mine")));
    }

    #[test]
    fn recheck_catches_work_done_since_the_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        let up = tmp.path().join("up");
        std::fs::create_dir_all(&up).unwrap();
        git(&up, &["init", "-q"]);
        git(&up, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(tmp.path(), &["clone", "-q", "up", "same"]);
        git(tmp.path(), &["clone", "-q", "up", "edited"]);
        git(tmp.path(), &["clone", "-q", "up", "broken"]);
        let cfg = Config::default();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let paths: Vec<PathBuf> = ["same", "edited", "broken", "up"]
            .iter()
            .map(|n| tmp.path().join(n))
            .collect();
        let mut lib = Library::new(
            paths
                .iter()
                .map(|p| crate::scan::inspect_repo(p, &cfg, &cancel))
                .collect(),
        );
        assert!(lib.recheck_clean(&paths).is_empty(), "nothing changed yet");

        // Work done after the scan: a new file, and a repo that got damaged.
        std::fs::write(paths[1].join("notes.txt"), "x").unwrap();
        std::fs::remove_file(paths[2].join(".git").join("HEAD")).unwrap();
        // `up` has no remote, so it was already flagged and isn't re-read.
        assert_eq!(
            lib.recheck_clean(&paths),
            vec![paths[1].clone(), paths[2].clone()]
        );
        let now = SystemTime::now() + Duration::from_secs(365 * 86_400);
        assert_eq!(
            lib.get(&paths[0]).unwrap().verdict(&cfg, now),
            Verdict::SafeToRemove
        );
        assert_eq!(
            lib.get(&paths[1]).unwrap().verdict(&cfg, now),
            Verdict::UnsavedWork
        );
        assert_eq!(
            lib.get(&paths[2]).unwrap().verdict(&cfg, now),
            Verdict::Unreadable
        );
        // Asking again doesn't report them twice: they're flagged now.
        assert!(lib.recheck_clean(&paths).is_empty());
    }

    #[test]
    fn remove_and_push_keep_index_and_selection_in_sync() {
        let (_, _, mut lib) = setup();
        let p = PathBuf::from("/tmp/dirty");
        lib.toggle_checked(&p);
        lib.toggle_checked(Path::new("/dl/fresh"));
        assert_eq!(lib.checked_paths().len(), 2);
        let removed = lib.remove(std::slice::from_ref(&p));
        assert_eq!(removed.len(), 1);
        assert!(lib.get(&p).is_none());
        assert_eq!(lib.checked_paths(), vec![PathBuf::from("/dl/fresh")]);
        assert!(lib.get(Path::new("/tmp/older-clone")).is_some());
        lib.push(removed.into_iter().next().unwrap());
        assert!(lib.get(&p).is_some());
        assert_eq!(lib.repos().len(), 5);
    }
}
