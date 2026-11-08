//! Reads a repo's state by running the `git` command line tool (read-only).
//!
//! Using the user's own git keeps the binary small and matches exactly what
//! they see in a terminal.

use anyhow::bail;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GitState {
    /// Current branch, or `None` for a detached HEAD.
    pub branch: Option<String>,
    /// Modified, staged, conflicted and untracked entries.
    pub changed_files: usize,
    /// Commits on local branches that aren't on any remote.
    pub unpushed_commits: usize,
    pub stashes: usize,
    /// URL of the first remote (credentials stripped), if the repo has one.
    pub remote_url: Option<String>,
    /// Newest commit on any branch.
    pub last_commit: Option<SystemTime>,
    /// Files git is told to leave out (`.gitignore`) that no command brings
    /// back, like `.env` or a local database: never committed, so never
    /// uploaded. Relative to the repo, at most [`MAX_LISTED`] of them.
    pub private_files: Vec<String>,
    /// How many there are in all (`private_files` may be cut short).
    pub private_files_total: usize,
}

pub use crate::private::MAX_LISTED;

impl GitState {
    /// Anything that exists only on this PC and would be lost if the folder went away.
    pub fn has_local_work(&self) -> bool {
        self.changed_files > 0
            || self.unpushed_commits > 0
            || self.stashes > 0
            || self.remote_url.is_none()
            || self.private_files_total > 0
    }

    /// Up to `n` private file names, with "and N more" when there are more.
    pub fn private_files_preview(&self, n: usize) -> String {
        crate::private::preview(&self.private_files, self.private_files_total, n)
    }

    /// Plain-language list of what would be lost, for warnings.
    pub fn local_work_reasons(&self) -> Vec<String> {
        let mut out = Vec::new();
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                one.to_string()
            } else {
                format!("{n} {many}")
            }
        };
        if self.changed_files > 0 {
            out.push(format!(
                "{} not saved (committed)",
                plural(self.changed_files, "1 edited file", "edited files")
            ));
        }
        if self.unpushed_commits > 0 && self.remote_url.is_some() {
            out.push(format!(
                "{} not uploaded (pushed)",
                plural(self.unpushed_commits, "1 saved version", "saved versions")
            ));
        }
        if self.stashes > 0 {
            out.push(plural(
                self.stashes,
                "1 set-aside change (stash)",
                "set-aside changes (stashes)",
            ));
        }
        if self.remote_url.is_none() {
            out.push("not online anywhere, so this folder is the only copy".to_string());
        }
        if self.private_files_total > 0 {
            out.push(format!(
                "{} git leaves out on purpose, so only on this PC ({})",
                plural(self.private_files_total, "1 file", "files"),
                self.private_files_preview(3)
            ));
        }
        out
    }
}

fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = run(repo, args.iter().map(std::ffi::OsStr::new))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Which of `paths` (inside `repo`) git ignores. Tracked paths are never
/// reported as ignored. Errors if git can't answer, so callers can treat
/// "don't know" as "not ignored".
pub fn ignored(
    repo: &Path,
    paths: &[std::path::PathBuf],
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let rel: Vec<std::path::PathBuf> = paths
        .iter()
        .filter_map(|p| p.strip_prefix(repo).ok().map(Path::to_path_buf))
        .collect();
    let mut args: Vec<&std::ffi::OsStr> = vec!["check-ignore".as_ref(), "--".as_ref()];
    args.extend(rel.iter().map(|p| p.as_os_str()));
    let out = run(repo, args)?;
    // 0: some are ignored, 1: none are, anything else: an error.
    match out.status.code() {
        Some(0) | Some(1) => {}
        _ => bail!("{}", String::from_utf8_lossy(&out.stderr).trim()),
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let listed: Vec<&str> = text.lines().map(str::trim).collect();
    Ok(rel
        .iter()
        .filter(|r| {
            let r = r.to_string_lossy();
            listed
                .iter()
                .any(|l| *l == r || l.replace('\\', "/") == r.replace('\\', "/"))
        })
        .map(|r| repo.join(r))
        .collect())
}

fn run<'a>(
    repo: &Path,
    args: impl IntoIterator<Item = &'a std::ffi::OsStr>,
) -> anyhow::Result<std::process::Output> {
    let mut cmd = Command::new("git");
    // safe.directory: read repos owned by other accounts (common for clones made
    // by tools running elevated) instead of failing with "dubious ownership".
    // core.longpaths: read repos with paths past Windows' 260-character limit.
    cmd.arg("-c")
        .arg("safe.directory=*")
        .arg("-c")
        .arg("core.longpaths=true")
        .arg("-C")
        .arg(repo)
        .args(args);
    // Don't let `git status` take the index lock or refresh it on disk.
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("Git isn't installed or isn't on PATH")
        } else {
            anyhow::Error::new(e).context("couldn't run git")
        }
    })
}

/// Why git can't read `repo`, in plain words, if the `.git` itself shows it.
fn broken_dot_git(repo: &Path) -> Option<String> {
    let dot = repo.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_file() {
        let text = std::fs::read_to_string(&dot).unwrap_or_default();
        let Some(target) = text.trim().strip_prefix("gitdir:").map(str::trim) else {
            return Some("its .git is a file that doesn't point to a repo".to_string());
        };
        let target = repo.join(target);
        if !target.exists() {
            return Some(format!(
                "it's a worktree or submodule, and the repo it belongs to is gone ({})",
                target.display()
            ));
        }
        None
    } else if !dot.join("HEAD").exists() {
        Some("its .git folder is incomplete, probably a partial copy".to_string())
    } else {
        None
    }
}

/// git's error output boiled down to one readable line.
pub fn explain_git_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or_else(|| stderr.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("git gave no reason");
    let msg = line
        .trim_start_matches("fatal:")
        .trim_start_matches("error:")
        .trim();
    let lower = msg.to_lowercase();
    if lower.contains("not a git repository") {
        format!("git says it isn't a repo ({msg})")
    } else if lower.contains("filename too long") {
        "some paths are too long for Windows".to_string()
    } else if lower.contains("index file")
        || lower.contains("bad object")
        || lower.contains("corrupt")
    {
        format!("the repo is damaged ({msg})")
    } else {
        msg.to_string()
    }
}

pub fn inspect(repo: &Path) -> anyhow::Result<GitState> {
    if let Some(why) = broken_dot_git(repo) {
        bail!("{why}");
    }
    let status = git(
        repo,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
            // List ignored files too, but stop at an ignored folder instead
            // of walking into it (node_modules can hold 100,000 files).
            "--ignored=matching",
        ],
    )
    .map_err(|e| anyhow::anyhow!("{}", explain_git_error(&e.to_string())))?;
    let (branch, changed_files) = parse_status_v2(&status);
    let private: Vec<String> = parse_ignored(&status)
        .into_iter()
        .filter(|p| !is_disposable(repo, p))
        .collect();
    let private_files_total = private.len();
    let private_files = private.into_iter().take(MAX_LISTED).collect();

    let remotes = git(repo, &["remote", "-v"]).unwrap_or_default();
    let remote_url = parse_first_remote(&remotes).map(|u| redact_credentials(&u));

    // With no remote every commit is local-only; otherwise count commits on any
    // local branch that no remote-tracking ref contains.
    let unpushed_args: &[&str] = if remote_url.is_some() {
        &["rev-list", "--count", "--branches", "--not", "--remotes"]
    } else {
        &["rev-list", "--count", "--branches"]
    };
    let unpushed_commits = git(repo, unpushed_args)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let stashes = git(repo, &["stash", "list"])
        .map(|s| s.lines().count())
        .unwrap_or(0);

    // Fails on a repo with no commits yet; that just means "no last commit".
    let last_commit = git(repo, &["log", "-1", "--all", "--format=%ct"])
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));

    Ok(GitState {
        branch,
        changed_files,
        unpushed_commits,
        stashes,
        remote_url,
        last_commit,
        private_files,
        private_files_total,
    })
}

/// Ignored entries (`! path`) from `git status --porcelain=v2 --ignored`.
/// Folders keep their trailing `/`.
pub fn parse_ignored(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| l.strip_prefix("! "))
        .map(|p| {
            // git quotes paths with unusual characters.
            let p = p.trim();
            p.strip_prefix('"')
                .and_then(|p| p.strip_suffix('"'))
                .unwrap_or(p)
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// See [`crate::private::is_disposable`].
pub fn is_disposable(repo: &Path, rel: &str) -> bool {
    crate::private::is_disposable(repo, rel)
}

/// Parses `git status --porcelain=v2 --branch` into (branch, changed entry count).
pub fn parse_status_v2(s: &str) -> (Option<String>, usize) {
    let mut branch = None;
    let mut changed = 0;
    for line in s.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            if head != "(detached)" {
                branch = Some(head.to_string());
            }
        } else if line.starts_with(['1', '2', 'u', '?']) {
            changed += 1;
        }
    }
    (branch, changed)
}

fn parse_first_remote(s: &str) -> Option<String> {
    s.lines()
        .find_map(|l| l.split_whitespace().nth(1).map(String::from))
}

/// Removes `user:token@` from URLs so access tokens never show up on screen.
pub fn redact_credentials(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let host_start = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest[..host_start].rfind('@') {
            return format!("{scheme}://{}", &rest[at + 1..]);
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let s = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +1 -0\n\
                 1 .M N... 100644 100644 100644 a b src/x.rs\n? notes.md\n";
        assert_eq!(parse_status_v2(s), (Some("main".into()), 2));
        assert_eq!(parse_status_v2("# branch.head (detached)\n"), (None, 0));
    }

    #[test]
    fn redacts_tokens() {
        assert_eq!(
            redact_credentials("https://x-access-token:ghp_secret@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(
            redact_credentials("git@github.com:a/b.git"),
            "git@github.com:a/b.git"
        );
        assert_eq!(
            redact_credentials("https://github.com/a/b"),
            "https://github.com/a/b"
        );
    }

    #[test]
    fn reasons_are_plain_language() {
        let g = GitState {
            changed_files: 1,
            unpushed_commits: 3,
            remote_url: Some("x".into()),
            ..Default::default()
        };
        assert_eq!(
            g.local_work_reasons(),
            vec![
                "1 edited file not saved (committed)",
                "3 saved versions not uploaded (pushed)"
            ]
        );
        assert!(
            GitState::default().has_local_work(),
            "no remote counts as local work"
        );
    }

    #[test]
    fn spots_ignored_files_that_are_the_only_copy() {
        let s = "# branch.head main\n! .env\n! node_modules/\n! \"we\\\"ird.db\"\n? new.txt\n";
        assert_eq!(
            parse_ignored(s),
            vec![".env", "node_modules/", "we\\\"ird.db"]
        );
        assert_eq!(parse_status_v2(s).1, 1, "ignored files aren't edits");

        let repo = Path::new("/nowhere");
        for junk in [
            "node_modules/",
            "packages/app/node_modules/",
            "src/__pycache__/",
            "debug.log",
            "app/foo.pyc",
            ".DS_Store",
            "Thumbs.db",
            "pkg.egg-info/",
            ".claude/settings.local.json",
            ".vscode/",
            "tsconfig.tsbuildinfo",
        ] {
            assert!(is_disposable(repo, junk), "{junk} should be disposable");
        }
        for keep in [
            ".env",
            ".env.local",
            "app.db",
            "db.sqlite3",
            "data/",
            "uploads/",
            "secrets.json",
            "terraform.tfstate",
            "env/",
            "settings.local.json",
        ] {
            assert!(!is_disposable(repo, keep), "{keep} may be the only copy");
        }
    }

    #[test]
    fn inspect_lists_private_ignored_files() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(r)
                    .output()
                    .unwrap()
                    .status
                    .success()
            )
        };
        git(&["init", "-q"]);
        std::fs::write(r.join(".gitignore"), "node_modules/\n.env\n*.db\n*.log\n").unwrap();
        std::fs::create_dir_all(r.join("node_modules/x")).unwrap();
        std::fs::write(r.join("node_modules/x/a.js"), "").unwrap();
        std::fs::write(r.join("run.log"), "").unwrap();
        std::fs::write(r.join(".env"), "KEY=1").unwrap();
        std::fs::write(r.join("app.db"), "").unwrap();
        let g = inspect(r).unwrap();
        assert_eq!(g.private_files, vec![".env", "app.db"]);
        assert_eq!(g.private_files_total, 2);
        assert!(g.has_local_work());
        assert!(
            g.local_work_reasons().iter().any(
                |r| r == "2 files git leaves out on purpose, so only on this PC (.env, app.db)"
            ),
            "{:?}",
            g.local_work_reasons()
        );
    }

    #[test]
    fn explains_broken_repos() {
        let tmp = tempfile::tempdir().unwrap();
        // A worktree whose main repo was deleted.
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        let e = inspect(&wt).unwrap_err().to_string();
        assert!(e.contains("the repo it belongs to is gone"), "{e}");
        // A .git folder copied without its contents.
        let half = tmp.path().join("half");
        std::fs::create_dir_all(half.join(".git")).unwrap();
        let e = inspect(&half).unwrap_err().to_string();
        assert!(e.contains("incomplete"), "{e}");

        assert_eq!(
            explain_git_error("warning: x\nfatal: not a git repository: 'C:/a/.git/worktrees/b'\n"),
            "git says it isn't a repo (not a git repository: 'C:/a/.git/worktrees/b')"
        );
        assert_eq!(
            explain_git_error("error: unable to stat 'x': Filename too long"),
            "some paths are too long for Windows"
        );
    }
}
