//! Reads a repo's state by running the `git` command line tool (read-only).
//!
//! Using the user's own git keeps the binary small and matches exactly what
//! they see in a terminal.

use anyhow::{Context, bail};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
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
}

impl GitState {
    /// Anything that exists only on this PC and would be lost if the folder went away.
    pub fn has_local_work(&self) -> bool {
        self.changed_files > 0
            || self.unpushed_commits > 0
            || self.stashes > 0
            || self.remote_url.is_none()
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
                "{} not committed",
                plural(self.changed_files, "1 changed file", "changed files")
            ));
        }
        if self.unpushed_commits > 0 && self.remote_url.is_some() {
            out.push(format!(
                "{} not pushed to any remote",
                plural(self.unpushed_commits, "1 commit", "commits")
            ));
        }
        if self.stashes > 0 {
            out.push(plural(self.stashes, "1 stash", "stashes"));
        }
        if self.remote_url.is_none() {
            out.push("no remote, so this repo only exists on this PC".to_string());
        }
        out
    }
}

fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let mut cmd = Command::new("git");
    // safe.directory: read repos owned by other accounts (common for clones made
    // by tools running elevated) instead of failing with "dubious ownership".
    cmd.arg("-c")
        .arg("safe.directory=*")
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
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("Git isn't installed or isn't on PATH")
        } else {
            anyhow::Error::new(e).context("couldn't run git")
        }
    })?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn inspect(repo: &Path) -> anyhow::Result<GitState> {
    let status = git(
        repo,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
        ],
    )
    .context("git status failed")?;
    let (branch, changed_files) = parse_status_v2(&status);

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
    })
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
                "1 changed file not committed",
                "3 commits not pushed to any remote"
            ]
        );
        assert!(
            GitState::default().has_local_work(),
            "no remote counts as local work"
        );
    }
}
