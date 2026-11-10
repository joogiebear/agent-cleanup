//! Moving things to the Recycle Bin and back, as a list of steps a
//! background thread can run so the window never waits on the disk.

use crate::actions;
use crate::config::path_within;
use crate::git::{self, GitState};
use std::path::{Path, PathBuf};

/// One step of a job.
#[derive(Clone, Debug)]
pub enum Op {
    /// Recycle these together: if one fails, the ones already moved are
    /// put back. With `recheck`, git is read first and the folder is left
    /// alone if it now holds unsaved work (see [`recheck`]).
    Recycle { paths: Vec<PathBuf>, recheck: bool },
    /// Put these back from the Recycle Bin. With `ok_if_present`, a path
    /// that's already on disk counts as done: it came back with the folder
    /// it sits in.
    Restore {
        paths: Vec<PathBuf>,
        ok_if_present: bool,
    },
    /// Undo a move into a project.
    MoveBack { from: PathBuf, to: PathBuf },
}

/// What a second look at a project copy found.
#[derive(Clone, Debug)]
pub struct Recheck {
    /// The fresh git state, or why git couldn't read it.
    pub git: Result<GitState, String>,
    /// Private-looking files, looked for when git couldn't read it.
    pub private_files: Vec<String>,
    pub private_files_total: usize,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Done,
    /// Left where it is: it has unsaved work it didn't have before.
    Changed(Box<Recheck>),
    Failed(String),
}

/// Reads git again for a project copy that looked fully saved. `None` means
/// it still is (or it's gone, which recycling reports itself). The lists in
/// the app can be days old, since scan results are saved between runs.
pub fn recheck(path: &Path) -> Option<Recheck> {
    if !path.exists() {
        return None;
    }
    match git::inspect(path) {
        Ok(g) if !g.has_local_work() => None,
        Ok(g) => Some(Recheck {
            git: Ok(g),
            private_files: Vec::new(),
            private_files_total: 0,
        }),
        Err(e) => {
            let (private_files, private_files_total) = crate::private::find(path);
            Some(Recheck {
                git: Err(e.to_string()),
                private_files,
                private_files_total,
            })
        }
    }
}

/// Runs the steps in order and returns one outcome per step.
pub fn run(ops: Vec<Op>) -> Vec<Outcome> {
    // Folders recycled so far: anything inside one went with it.
    let mut gone: Vec<PathBuf> = Vec::new();
    ops.into_iter().map(|op| run_one(op, &mut gone)).collect()
}

fn run_one(op: Op, gone: &mut Vec<PathBuf>) -> Outcome {
    match op {
        Op::Recycle {
            paths,
            recheck: look,
        } => {
            if paths
                .iter()
                .all(|p| gone.iter().any(|g| g != p && path_within(p, g)))
            {
                return Outcome::Done;
            }
            if look && let Some(found) = paths.first().and_then(|p| recheck(p)) {
                return Outcome::Changed(Box::new(found));
            }
            let mut done: Vec<&PathBuf> = Vec::new();
            for p in &paths {
                if let Err(e) = actions::recycle(p) {
                    for d in done {
                        let _ = actions::restore(d);
                    }
                    return Outcome::Failed(e.to_string());
                }
                done.push(p);
            }
            gone.extend(paths);
            Outcome::Done
        }
        Op::Restore {
            paths,
            ok_if_present,
        } => {
            let errs: Vec<String> = paths
                .iter()
                .filter(|p| !(ok_if_present && p.exists()))
                .filter_map(|p| actions::restore(p).err().map(|e| e.to_string()))
                .collect();
            if errs.is_empty() {
                Outcome::Done
            } else {
                Outcome::Failed(errs.join(", "))
            }
        }
        Op::MoveBack { from, to } => match crate::downloads::move_back(&from, &to) {
            Ok(()) => Outcome::Done,
            Err(e) => Outcome::Failed(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
    }

    #[test]
    fn recycles_nested_folders_once_and_puts_them_back() {
        let tmp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let outer = tmp.path().join(format!("outer-{}", std::process::id()));
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("a.txt"), "hi").unwrap();
        let recycle = |p: &Path| Op::Recycle {
            paths: vec![p.to_path_buf()],
            recheck: false,
        };
        let out = run(vec![recycle(&outer), recycle(&inner)]);
        assert!(matches!(out[..], [Outcome::Done, Outcome::Done]), "{out:?}");
        assert!(!outer.exists());

        let restore = |p: &Path| Op::Restore {
            paths: vec![p.to_path_buf()],
            ok_if_present: true,
        };
        let out = run(vec![restore(&outer), restore(&inner)]);
        assert!(matches!(out[..], [Outcome::Done, Outcome::Done]), "{out:?}");
        assert_eq!(std::fs::read_to_string(inner.join("a.txt")).unwrap(), "hi");
    }

    #[test]
    fn leaves_a_copy_that_gained_unsaved_work() {
        let tmp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let up = tmp.path().join("up");
        std::fs::create_dir_all(&up).unwrap();
        git(&up, &["init", "-q"]);
        git(&up, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(tmp.path(), &["clone", "-q", "up", "copy"]);
        let copy = tmp.path().join("copy");
        assert!(recheck(&copy).is_none(), "a fresh clone is fully saved");

        std::fs::write(copy.join("notes.txt"), "x").unwrap();
        let out = run(vec![Op::Recycle {
            paths: vec![copy.clone()],
            recheck: true,
        }]);
        let [Outcome::Changed(found)] = &out[..] else {
            panic!("{out:?}");
        };
        assert!(found.git.as_ref().unwrap().has_local_work());
        assert!(copy.join("notes.txt").exists(), "it stayed where it was");
        assert!(recheck(&tmp.path().join("missing")).is_none());
    }
}
