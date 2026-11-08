//! Duplicate clones: the same remote repo cloned into more than one folder,
//! usually because an agent cloned it again instead of using your copy.
//!
//! Works on the repos the scan already found; nothing new is read here.

use crate::model::Repo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `https://github.com/Me/Tool.git`, `git@github.com:me/tool` and
/// `ssh://git@github.com/me/tool/` all become `github.com/me/tool`.
pub fn normalize_remote(url: &str) -> Option<String> {
    let mut s = url.trim().to_lowercase().replace('\\', "/");
    // A Windows drive path (`c:/repos/x`) is a local remote, not a host.
    if s.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    for scheme in ["https://", "http://", "ssh://", "git://", "git+ssh://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest.to_string();
            break;
        }
    }
    // Drop `user@` (and `user:token@`) in front of the host.
    if let Some(at) = s.find('@')
        && s[..at].find('/').is_none()
    {
        s = s[at + 1..].to_string();
    }
    // scp form: `host:owner/repo`. A `:` followed by digits is a port.
    if let Some(colon) = s.find(':') {
        let after = &s[colon + 1..];
        let port_len = after.chars().take_while(char::is_ascii_digit).count();
        if port_len > 0 && after[port_len..].starts_with('/') {
            s = format!("{}{}", &s[..colon], &after[port_len..]);
        } else {
            s = format!("{}/{}", &s[..colon], after.trim_start_matches('/'));
        }
    }
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s).trim_end_matches('/');
    // Local paths and file URLs aren't worth matching.
    (s.contains('/') && !s.starts_with('/') && !s.starts_with("file")).then(|| s.to_string())
}

/// Copies of one remote repo.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Normalized remote, like `github.com/me/tool`.
    pub remote: String,
    /// The copy to keep.
    pub keep: PathBuf,
    /// Every copy, the kept one first, then by last activity.
    pub copies: Vec<PathBuf>,
}

impl Group {
    pub fn extras(&self) -> impl Iterator<Item = &PathBuf> {
        self.copies.iter().filter(move |p| **p != self.keep)
    }
}

/// Which copy to keep when the user hasn't said: one in a project folder,
/// then the most recently used, then the shortest path.
fn default_keep<'a>(copies: &[&'a Repo]) -> &'a Repo {
    copies
        .iter()
        .copied()
        .max_by(|a, b| {
            let rank = |r: &Repo| (r.project_root.is_some(), r.last_activity);
            rank(a)
                .cmp(&rank(b))
                .then_with(|| b.path.as_os_str().len().cmp(&a.path.as_os_str().len()))
        })
        .expect("a group has copies")
}

/// Groups the repos that share a remote. `chosen` maps a normalized remote
/// to the copy the user picked to keep. Biggest waste first.
pub fn groups(repos: &[Repo], chosen: &HashMap<String, PathBuf>) -> Vec<Group> {
    let mut by_remote: HashMap<String, Vec<&Repo>> = HashMap::new();
    for r in repos.iter().filter(|r| r.inspected) {
        if let Some(remote) = r
            .git
            .as_ref()
            .and_then(|g| g.remote_url.as_deref())
            .and_then(normalize_remote)
        {
            by_remote.entry(remote).or_default().push(r);
        }
    }
    let mut out: Vec<(u64, Group)> = by_remote
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(remote, mut copies)| {
            let keep = chosen
                .get(&remote)
                .filter(|p| copies.iter().any(|r| &r.path == *p))
                .cloned()
                .unwrap_or_else(|| default_keep(&copies).path.clone());
            copies.sort_by(|a, b| {
                (b.path == keep)
                    .cmp(&(a.path == keep))
                    .then(b.last_activity.cmp(&a.last_activity))
                    .then(a.path.cmp(&b.path))
            });
            let waste = copies
                .iter()
                .filter(|r| r.path != keep)
                .filter_map(|r| r.size_bytes)
                .sum();
            let group = Group {
                remote,
                keep,
                copies: copies.iter().map(|r| r.path.clone()).collect(),
            };
            (waste, group)
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.remote.cmp(&b.1.remote)));
    out.into_iter().map(|(_, g)| g).collect()
}

/// Extra copies that are safe to recycle in one go: outside your project
/// folders and with nothing that exists only in that copy.
pub fn safe_extras(groups: &[Group], repo: impl Fn(&Path) -> Option<Repo>) -> Vec<PathBuf> {
    groups
        .iter()
        .flat_map(|g| g.extras())
        .filter(|p| {
            repo(p).is_some_and(|r| {
                r.project_root.is_none() && r.git.as_ref().is_some_and(|g| !g.has_local_work())
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitState;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn remotes_normalize() {
        let want = Some("github.com/me/tool".to_string());
        for url in [
            "https://github.com/Me/Tool.git",
            "https://github.com/me/tool/",
            "http://user:tok@github.com/me/tool",
            "git@github.com:me/tool.git",
            "ssh://git@github.com/me/tool",
            "ssh://git@github.com:22/me/tool.git",
        ] {
            assert_eq!(normalize_remote(url), want, "{url}");
        }
        assert_eq!(normalize_remote("/srv/git/tool.git"), None);
        assert_eq!(normalize_remote("file:///srv/git/tool"), None);
        assert_eq!(normalize_remote(r"C:\repos\tool"), None);
        assert_eq!(normalize_remote("C:/repos/tool"), None);
    }

    fn repo(path: &str, remote: &str, project: bool, days_ago: u64, local_work: bool) -> Repo {
        Repo {
            path: PathBuf::from(path),
            project_root: project.then(|| PathBuf::from("/code")),
            inspected: true,
            git: Some(GitState {
                branch: Some("main".into()),
                changed_files: usize::from(local_work),
                unpushed_commits: 0,
                stashes: 0,
                remote_url: Some(remote.into()),
                last_commit: None,
                ..Default::default()
            }),
            error: None,
            size_bytes: Some(100),
            last_activity: Some(UNIX_EPOCH + Duration::from_secs(100_000_000 - days_ago * 86_400)),
        }
    }

    #[test]
    fn groups_and_picks_keeper() {
        let repos = vec![
            repo("/tmp/tool", "https://github.com/me/tool", false, 1, false),
            repo("/code/tool", "git@github.com:me/tool.git", true, 30, false),
            repo("/dl/tool-2", "https://github.com/me/tool", false, 5, true),
            repo("/code/other", "https://github.com/me/other", true, 1, false),
        ];
        let gs = groups(&repos, &HashMap::new());
        assert_eq!(gs.len(), 1);
        let g = &gs[0];
        assert_eq!(g.remote, "github.com/me/tool");
        // The project copy wins even though another was used more recently.
        assert_eq!(g.keep, PathBuf::from("/code/tool"));
        assert_eq!(
            g.copies,
            vec![
                PathBuf::from("/code/tool"),
                PathBuf::from("/tmp/tool"),
                PathBuf::from("/dl/tool-2")
            ]
        );
        let find = |p: &Path| repos.iter().find(|r| r.path == p).cloned();
        // The copy with uncommitted changes is never in the bulk list.
        assert_eq!(safe_extras(&gs, find), vec![PathBuf::from("/tmp/tool")]);

        // The user's pick wins; the project copy becomes an extra, but stays
        // out of the bulk list because it's in a project folder.
        let chosen =
            HashMap::from([("github.com/me/tool".to_string(), PathBuf::from("/tmp/tool"))]);
        let gs = groups(&repos, &chosen);
        assert_eq!(gs[0].keep, PathBuf::from("/tmp/tool"));
        assert_eq!(safe_extras(&gs, find), Vec::<PathBuf>::new());
    }
}
