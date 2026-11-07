//! Build junk: dependency and build-output folders inside your repos that a
//! single command recreates (`node_modules`, `target`, `.venv`, `dist`, ...).
//!
//! A folder only counts when both hold:
//! - its name and a marker agree on what made it (`node_modules` next to a
//!   `package.json`, `target` next to a `Cargo.toml`, a venv with
//!   `pyvenv.cfg` inside), and
//! - git ignores it, so it isn't part of the project's source. If git can't
//!   answer, nothing in that repo is offered.

use crate::config::Config;
use crate::git;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

/// How deep inside a repo to look (monorepos keep `packages/*/node_modules`).
const MAX_DEPTH: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    /// A file with this name next to the folder.
    Sibling(&'static str),
    /// A file with this extension next to the folder.
    SiblingExt(&'static str),
    /// A file with this name inside the folder.
    Inside(&'static str),
    /// The folder name alone is specific enough (tool caches).
    None,
}

/// One kind of build folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub label: &'static str,
    /// What brings it back, shown to the user.
    pub rebuild: &'static str,
}

const NODE: Kind = Kind {
    label: "Node packages",
    rebuild: "npm install",
};
const NODE_BUILD: Kind = Kind {
    label: "JavaScript build output",
    rebuild: "npm run build",
};
const NODE_CACHE: Kind = Kind {
    label: "Framework cache",
    rebuild: "recreated on the next build",
};
const RUST: Kind = Kind {
    label: "Rust build output",
    rebuild: "cargo build",
};
const VENV: Kind = Kind {
    label: "Python virtual env",
    rebuild: "python -m venv, then pip install",
};
const PY_CACHE: Kind = Kind {
    label: "Python tool cache",
    rebuild: "recreated on the next run",
};
const GRADLE: Kind = Kind {
    label: "Gradle build output",
    rebuild: "gradle build",
};
const DOTNET: Kind = Kind {
    label: ".NET build output",
    rebuild: "dotnet build",
};

/// Folder name, what must be there to trust it, and what it is.
const RULES: &[(&str, Marker, Kind)] = &[
    ("node_modules", Marker::Sibling("package.json"), NODE),
    ("target", Marker::Sibling("Cargo.toml"), RUST),
    (".venv", Marker::Inside("pyvenv.cfg"), VENV),
    ("venv", Marker::Inside("pyvenv.cfg"), VENV),
    ("env", Marker::Inside("pyvenv.cfg"), VENV),
    (".next", Marker::Sibling("package.json"), NODE_CACHE),
    (".nuxt", Marker::Sibling("package.json"), NODE_CACHE),
    (".svelte-kit", Marker::Sibling("package.json"), NODE_CACHE),
    (".turbo", Marker::Sibling("package.json"), NODE_CACHE),
    (".parcel-cache", Marker::Sibling("package.json"), NODE_CACHE),
    (".angular", Marker::Sibling("package.json"), NODE_CACHE),
    ("dist", Marker::Sibling("package.json"), NODE_BUILD),
    ("build", Marker::Sibling("package.json"), NODE_BUILD),
    ("out", Marker::Sibling("package.json"), NODE_BUILD),
    ("coverage", Marker::Sibling("package.json"), NODE_BUILD),
    ("build", Marker::Sibling("build.gradle"), GRADLE),
    ("build", Marker::Sibling("build.gradle.kts"), GRADLE),
    (".gradle", Marker::Sibling("settings.gradle"), GRADLE),
    (".gradle", Marker::Sibling("settings.gradle.kts"), GRADLE),
    ("bin", Marker::SiblingExt("csproj"), DOTNET),
    ("obj", Marker::SiblingExt("csproj"), DOTNET),
    ("bin", Marker::SiblingExt("fsproj"), DOTNET),
    ("obj", Marker::SiblingExt("fsproj"), DOTNET),
    (".pytest_cache", Marker::None, PY_CACHE),
    (".mypy_cache", Marker::None, PY_CACHE),
    (".ruff_cache", Marker::None, PY_CACHE),
    (".tox", Marker::None, PY_CACHE),
];

/// A build folder found on disk.
#[derive(Clone, Debug)]
pub struct Item {
    pub path: PathBuf,
    /// The repo it belongs to.
    pub repo: PathBuf,
    pub kind: Kind,
    pub size_bytes: u64,
    /// When the repo was last worked on, if known.
    pub repo_activity: Option<SystemTime>,
}

impl Item {
    pub fn name(&self) -> String {
        self.path
            .strip_prefix(&self.repo)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| self.path.display().to_string())
    }

    pub fn repo_name(&self) -> String {
        self.repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.repo.display().to_string())
    }

    /// The repo hasn't been touched in `days` days. Unknown activity counts
    /// as recent, so it isn't offered by "Clean up".
    pub fn is_idle(&self, days: u32, now: SystemTime) -> bool {
        self.repo_activity.is_some_and(|t| {
            now.duration_since(t).unwrap_or_default()
                > Duration::from_secs(u64::from(days) * 86_400)
        })
    }
}

/// What kind of build folder `dir` is, if any.
pub fn classify(dir: &Path) -> Option<Kind> {
    let name = dir.file_name()?.to_str()?;
    let parent = dir.parent()?;
    RULES
        .iter()
        .filter(|(n, _, _)| *n == name)
        .find(|(_, marker, _)| match marker {
            Marker::Sibling(f) => parent.join(f).is_file(),
            Marker::SiblingExt(ext) => std::fs::read_dir(parent).is_ok_and(|rd| {
                rd.filter_map(Result::ok)
                    .any(|e| e.path().extension().is_some_and(|x| x == *ext))
            }),
            Marker::Inside(f) => dir.join(f).is_file(),
            Marker::None => true,
        })
        .map(|(_, _, k)| *k)
}

/// Build folders inside one repo that match a rule, before the git check.
/// Doesn't look inside `.git`, nested repos (they're scanned on their own)
/// or a match (no `node_modules` inside `node_modules`).
pub fn candidates(repo: &Path, cancel: &AtomicBool) -> Vec<(PathBuf, Kind)> {
    let mut out = Vec::new();
    let mut it = WalkDir::new(repo)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter();
    while let Some(entry) = it.next() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() || entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".git") || path.join(".git").exists() {
            it.skip_current_dir();
            continue;
        }
        if let Some(kind) = classify(path) {
            out.push((path.to_path_buf(), kind));
            it.skip_current_dir();
        }
    }
    out
}

/// Build folders in one repo that git ignores, with their sizes.
pub fn find_in_repo(
    repo: &Path,
    repo_activity: Option<SystemTime>,
    cancel: &AtomicBool,
) -> Vec<Item> {
    let found = candidates(repo, cancel);
    if found.is_empty() {
        return Vec::new();
    }
    let paths: Vec<PathBuf> = found.iter().map(|(p, _)| p.clone()).collect();
    // Unknown means not ignored: offer nothing rather than guess.
    let Ok(ignored) = git::ignored(repo, &paths) else {
        return Vec::new();
    };
    found
        .into_iter()
        .filter(|(p, _)| ignored.contains(p))
        .map(|(path, kind)| Item {
            size_bytes: crate::scan::dir_size(&path, cancel),
            path,
            repo: repo.to_path_buf(),
            kind,
            repo_activity,
        })
        .collect()
}

/// Looks through `repos` (path and last activity), skipping hidden ones.
/// `done` counts finished repos for a progress display. Biggest first.
pub fn find(
    repos: &[(PathBuf, Option<SystemTime>)],
    cfg: &Config,
    cancel: &AtomicBool,
    done: &AtomicUsize,
) -> Vec<Item> {
    let mut items = Vec::new();
    for (repo, activity) in repos {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !cfg.is_ignored(repo) {
            items.extend(
                find_in_repo(repo, *activity, cancel)
                    .into_iter()
                    .filter(|i| !cfg.is_ignored(&i.path)),
            );
        }
        done.fetch_add(1, Ordering::Relaxed);
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.size_bytes));
    items
}

/// Totals for the page header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub count: usize,
    pub bytes: u64,
    pub idle_count: usize,
    pub idle_bytes: u64,
}

pub fn stats(items: &[Item], idle_days: u32, now: SystemTime) -> Stats {
    let mut s = Stats::default();
    for i in items {
        s.count += 1;
        s.bytes += i.size_bytes;
        if i.is_idle(idle_days, now) {
            s.idle_count += 1;
            s.idle_bytes += i.size_bytes;
        }
    }
    s
}

/// Paths "Clean up" recycles: build folders in repos idle for `idle_days`.
pub fn idle_paths(items: &[Item], idle_days: u32, now: SystemTime) -> Vec<PathBuf> {
    items
        .iter()
        .filter(|i| i.is_idle(idle_days, now))
        .map(|i| i.path.clone())
        .collect()
}

/// Items grouped by repo, biggest repo first; items keep their order.
pub fn by_repo(items: &[Item]) -> Vec<(PathBuf, Vec<&Item>)> {
    let mut groups: Vec<(PathBuf, Vec<&Item>)> = Vec::new();
    for it in items {
        match groups.iter_mut().find(|(r, _)| *r == it.repo) {
            Some((_, v)) => v.push(it),
            None => groups.push((it.repo.clone(), vec![it])),
        }
    }
    groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.iter().map(|i| i.size_bytes).sum::<u64>()));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn touch(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn classify_needs_markers() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("node_modules")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::create_dir_all(r.join("build")).unwrap();
        fs::create_dir_all(r.join("venv")).unwrap();
        fs::create_dir_all(r.join(".pytest_cache")).unwrap();
        // No markers yet: only the specific cache name counts.
        assert_eq!(classify(&r.join("node_modules")), None);
        assert_eq!(classify(&r.join("target")), None);
        assert_eq!(classify(&r.join("build")), None);
        assert_eq!(classify(&r.join("venv")), None);
        assert_eq!(classify(&r.join(".pytest_cache")), Some(PY_CACHE));

        touch(&r.join("package.json"), "{}");
        touch(&r.join("Cargo.toml"), "");
        touch(&r.join("venv/pyvenv.cfg"), "");
        assert_eq!(classify(&r.join("node_modules")), Some(NODE));
        assert_eq!(classify(&r.join("target")), Some(RUST));
        assert_eq!(classify(&r.join("build")), Some(NODE_BUILD));
        assert_eq!(classify(&r.join("venv")), Some(VENV));

        let cs = r.join("svc");
        touch(&cs.join("Svc.csproj"), "");
        fs::create_dir_all(cs.join("bin")).unwrap();
        assert_eq!(classify(&cs.join("bin")), Some(DOTNET));
    }

    #[test]
    fn candidates_skip_nested_repos_and_insides() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        touch(&r.join("package.json"), "{}");
        touch(&r.join("node_modules/dep/package.json"), "{}");
        fs::create_dir_all(r.join("node_modules/dep/node_modules")).unwrap();
        touch(&r.join("packages/web/package.json"), "{}");
        fs::create_dir_all(r.join("packages/web/node_modules")).unwrap();
        touch(&r.join("vendor/other/package.json"), "{}");
        fs::create_dir_all(r.join("vendor/other/.git")).unwrap();
        fs::create_dir_all(r.join("vendor/other/node_modules")).unwrap();
        let mut found: Vec<PathBuf> = candidates(r, &AtomicBool::new(false))
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![r.join("node_modules"), r.join("packages/web/node_modules")]
        );
    }

    fn has_git() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn only_git_ignored_folders_count() {
        if !has_git() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(r)
                .status()
                .unwrap()
                .success()
        );
        touch(&r.join("package.json"), "{}");
        touch(&r.join(".gitignore"), "node_modules/\n");
        touch(&r.join("node_modules/a/index.js"), "12345");
        // `dist` matches a rule but isn't ignored: it might be committed output.
        touch(&r.join("dist/app.js"), "x");
        let items = find_in_repo(r, None, &AtomicBool::new(false));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, r.join("node_modules"));
        assert_eq!(items[0].kind, NODE);
        assert_eq!(items[0].size_bytes, 5);
        assert_eq!(items[0].name(), "node_modules");

        // Not a repo: git can't answer, so nothing is offered.
        let plain = tempfile::tempdir().unwrap();
        touch(&plain.path().join("package.json"), "{}");
        fs::create_dir_all(plain.path().join("node_modules")).unwrap();
        assert!(find_in_repo(plain.path(), None, &AtomicBool::new(false)).is_empty());
    }

    #[test]
    fn idle_stats_and_groups() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 * 86_400);
        let item = |repo: &str, name: &str, size: u64, age_days: Option<u64>| Item {
            path: PathBuf::from(repo).join(name),
            repo: PathBuf::from(repo),
            kind: NODE,
            size_bytes: size,
            repo_activity: age_days.map(|d| now - Duration::from_secs(d * 86_400)),
        };
        let items = vec![
            item("/a", "node_modules", 500, Some(60)),
            item("/b", "target", 300, Some(2)),
            item("/a", "web/node_modules", 100, Some(60)),
            item("/c", "node_modules", 50, None),
        ];
        let s = stats(&items, 14, now);
        assert_eq!(
            s,
            Stats {
                count: 4,
                bytes: 950,
                idle_count: 2,
                idle_bytes: 600
            }
        );
        assert_eq!(
            idle_paths(&items, 14, now),
            vec![
                PathBuf::from("/a/node_modules"),
                PathBuf::from("/a/web/node_modules")
            ]
        );
        let groups = by_repo(&items);
        let order: Vec<_> = groups.iter().map(|(r, v)| (r.clone(), v.len())).collect();
        assert_eq!(
            order,
            vec![
                (PathBuf::from("/a"), 2),
                (PathBuf::from("/b"), 1),
                (PathBuf::from("/c"), 1)
            ]
        );
        assert_eq!(
            items[2].name(),
            Path::new("web/node_modules").display().to_string()
        );
    }
}
