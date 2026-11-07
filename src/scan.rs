//! Finding repos on disk and inspecting them in the background.

use crate::config::Config;
use crate::git;
use crate::model::Repo;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

pub enum ScanEvent {
    /// Folders looked at so far.
    Progress {
        dirs: u64,
    },
    /// A repo was found; its details arrive later as `Inspected`.
    Found(PathBuf),
    Inspected(Box<Repo>),
    Done {
        dirs: u64,
        elapsed: Duration,
        cancelled: bool,
    },
}

/// Walks `cfg.scan_roots` and calls `on_found` for every folder that contains
/// `.git`. Keeps descending into repos (to catch clones nested inside other
/// repos) but never into `.git` itself or the configured skip lists.
pub fn find_repos(
    cfg: &Config,
    cancel: &AtomicBool,
    mut on_found: impl FnMut(PathBuf),
    mut on_progress: impl FnMut(u64),
) -> u64 {
    let mut dirs = 0u64;
    let mut seen = HashSet::new();
    for root in &cfg.scan_roots {
        let mut it = WalkDir::new(root).follow_links(false).into_iter();
        while let Some(entry) = it.next() {
            if cancel.load(Ordering::Relaxed) {
                return dirs;
            }
            // Unreadable folders (permissions, junctions like "Application Data") are skipped.
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if entry.depth() > 0
                && (name == ".git"
                    || cfg.skip_dir_names.iter().any(|s| s.as_str() == name)
                    || cfg.is_skipped_path(entry.path()))
            {
                it.skip_current_dir();
                continue;
            }
            dirs += 1;
            if dirs.is_multiple_of(500) {
                on_progress(dirs);
            }
            let path = entry.path();
            if path.join(".git").exists() && seen.insert(path.to_path_buf()) {
                on_found(path.to_path_buf());
            }
        }
    }
    dirs
}

/// Reads git state, folder size and last activity for one repo.
pub fn inspect_repo(path: &Path, cfg: &Config, cancel: &AtomicBool) -> Repo {
    let mut repo = Repo::new(path.to_path_buf(), cfg);
    match git::inspect(path) {
        Ok(g) => repo.git = Some(g),
        Err(e) => repo.error = Some(e.to_string()),
    }
    repo.size_bytes = Some(dir_size(path, cancel));
    repo.last_activity = last_activity(path, repo.git.as_ref().and_then(|g| g.last_commit));
    repo.inspected = true;
    repo
}

/// Total size of every file under `path`, without following links.
pub fn dir_size(path: &Path, cancel: &AtomicBool) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Newest of the last commit and the files git touches on checkout, fetch,
/// stage or pull. Catches repos that were cloned and used but never committed to.
fn last_activity(repo: &Path, last_commit: Option<SystemTime>) -> Option<SystemTime> {
    let dot_git = repo.join(".git");
    ["index", "HEAD", "FETCH_HEAD", "ORIG_HEAD", "logs/HEAD"]
        .iter()
        .filter_map(|f| {
            std::fs::metadata(dot_git.join(f))
                .and_then(|m| m.modified())
                .ok()
        })
        .chain(std::fs::metadata(&dot_git).and_then(|m| m.modified()).ok())
        .chain(last_commit)
        .max()
}

/// Runs a full scan on background threads. Events go to `tx`; `notify` is
/// called after each one so the UI can repaint.
pub fn start(
    cfg: Config,
    tx: Sender<ScanEvent>,
    cancel: Arc<AtomicBool>,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let started = Instant::now();
        let send = |ev: ScanEvent| {
            let _ = tx.send(ev);
            notify();
        };
        let (work_tx, work_rx) = mpsc::channel::<PathBuf>();
        let work_rx = Arc::new(Mutex::new(work_rx));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let (work_rx, tx, cfg, cancel, notify) = (
                    work_rx.clone(),
                    tx.clone(),
                    cfg.clone(),
                    cancel.clone(),
                    notify.clone(),
                );
                thread::spawn(move || {
                    loop {
                        let next = work_rx.lock().map(|rx| rx.recv());
                        let Ok(Ok(path)) = next else { break };
                        if cancel.load(Ordering::Relaxed) {
                            continue;
                        }
                        let _ = tx.send(ScanEvent::Inspected(Box::new(inspect_repo(
                            &path, &cfg, &cancel,
                        ))));
                        notify();
                    }
                })
            })
            .collect();

        let dirs = find_repos(
            &cfg,
            &cancel,
            |p| {
                send(ScanEvent::Found(p.clone()));
                let _ = work_tx.send(p);
            },
            |dirs| send(ScanEvent::Progress { dirs }),
        );
        drop(work_tx);
        for w in workers {
            let _ = w.join();
        }
        send(ScanEvent::Done {
            dirs,
            elapsed: started.elapsed(),
            cancelled: cancel.load(Ordering::Relaxed),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git must be installed for tests")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn make_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join("README.md"), "hi").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "init"]);
    }

    #[test]
    fn finds_repos_including_nested_and_skips_ignored_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        make_repo(&root.join("projects/app"));
        make_repo(&root.join("projects/app/vendor/clone"));
        make_repo(&root.join("Downloads/some-tool"));
        make_repo(&root.join("node_modules/pkg"));

        let cfg = Config {
            scan_roots: vec![root.to_path_buf()],
            skip_paths: vec![],
            ..Config::default()
        };
        let mut found = Vec::new();
        find_repos(&cfg, &AtomicBool::new(false), |p| found.push(p), |_| {});
        found.sort();
        let rel: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_path_buf())
            .collect();
        assert_eq!(
            rel,
            vec![
                PathBuf::from("Downloads/some-tool"),
                PathBuf::from("projects/app"),
                PathBuf::from("projects/app/vendor/clone"),
            ]
        );
    }

    #[test]
    fn inspects_local_work() {
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream");
        make_repo(&upstream);
        let clone = tmp.path().join("clone");
        git(
            tmp.path(),
            &[
                "clone",
                "-q",
                upstream.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );

        let cfg = Config::default();
        let cancel = AtomicBool::new(false);
        let clean = inspect_repo(&clone, &cfg, &cancel);
        let g = clean.git.as_ref().unwrap();
        assert_eq!((g.changed_files, g.unpushed_commits, g.stashes), (0, 0, 0));
        assert!(g.remote_url.is_some());
        assert!(!clean.has_local_work());
        assert!(clean.size_bytes.unwrap() > 0);
        assert!(clean.last_activity.is_some());

        std::fs::write(clone.join("new.txt"), "x").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-qm", "local"]);
        std::fs::write(clone.join("scratch.txt"), "x").unwrap();
        let dirty = inspect_repo(&clone, &cfg, &cancel);
        let g = dirty.git.unwrap();
        assert_eq!((g.changed_files, g.unpushed_commits), (1, 1));

        // The upstream itself has no remote, so all of it is local-only.
        let lone = inspect_repo(&upstream, &cfg, &cancel);
        assert!(lone.has_local_work());
    }

    #[test]
    fn background_scan_reports_everything() {
        let tmp = tempfile::tempdir().unwrap();
        make_repo(&tmp.path().join("a"));
        make_repo(&tmp.path().join("b"));
        let cfg = Config {
            scan_roots: vec![tmp.path().to_path_buf()],
            ..Config::default()
        };
        let (tx, rx) = mpsc::channel();
        start(cfg, tx, Arc::new(AtomicBool::new(false)), Arc::new(|| {}))
            .join()
            .unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        let found = events
            .iter()
            .filter(|e| matches!(e, ScanEvent::Found(_)))
            .count();
        let inspected = events
            .iter()
            .filter(|e| matches!(e, ScanEvent::Inspected(_)))
            .count();
        assert_eq!((found, inspected), (2, 2));
        assert!(matches!(
            events.last(),
            Some(ScanEvent::Done {
                cancelled: false,
                ..
            })
        ));
    }
}
