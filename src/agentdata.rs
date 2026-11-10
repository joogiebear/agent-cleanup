//! Agent data for deleted projects: Claude Code and Codex keep every
//! session's history, and it stays behind after the project folder is gone.
//!
//! - Claude Code: one folder per project in `~/.claude/projects/<encoded
//!   path>/` holding `<session>.jsonl` logs (plus per-session folders), and
//!   file snapshots in `~/.claude/file-history/<session>/`.
//! - Codex: dated `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` files; each
//!   one names its project in a `session_meta` line.
//!
//! The project path is read from the logs themselves (the folder name can't
//! be decoded reliably). Only reading happens here.

use crate::config::Config;
use crate::trail::Agent;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

/// How many lines of a log to read looking for its project folder.
const HEAD_LINES: usize = 50;
/// Sessions newer than this need a tick before recycling.
pub const RECENT: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);

/// One agent's data for one project that no longer exists.
#[derive(Clone, Debug, PartialEq)]
pub struct Orphan {
    pub agent: Agent,
    /// The project folder the sessions ran in. It's gone.
    pub project: PathBuf,
    /// What recycling removes. The first one names the item.
    pub paths: Vec<PathBuf>,
    pub sessions: usize,
    pub size_bytes: u64,
    pub last_used: Option<SystemTime>,
    /// A folder with the project's name that exists somewhere else: the
    /// project may have been moved or renamed rather than deleted.
    pub moved_to: Option<PathBuf>,
}

impl Orphan {
    /// Stable id for the UI and for hiding: the first path.
    pub fn key(&self) -> &Path {
        &self.paths[0]
    }

    /// Used in the last week: the project may only have moved.
    pub fn is_recent(&self, now: SystemTime) -> bool {
        self.last_used
            .is_some_and(|t| now.duration_since(t).map_or(true, |d| d < RECENT))
    }

    /// Why to look before recycling, if anything, in plain words.
    pub fn warning(&self, now: SystemTime) -> Option<String> {
        if let Some(to) = &self.moved_to {
            return Some(format!(
                "a project with the same name is at {}, so it may have moved",
                to.display()
            ));
        }
        self.is_recent(now).then(|| {
            format!(
                "last used {}",
                self.last_used
                    .map(|t| crate::model::relative_time(t, now))
                    .unwrap_or_default()
            )
        })
    }

    pub fn project_name(&self) -> String {
        self.project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.display().to_string())
    }
}

/// The project folder a session log names, from its first lines.
pub fn log_cwd(path: &Path) -> Option<PathBuf> {
    let f = std::fs::File::open(path).ok()?;
    for line in BufReader::new(f)
        .lines()
        .take(HEAD_LINES)
        .map_while(Result::ok)
    {
        if !line.contains("\"cwd\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let item = v.get("payload").unwrap_or(&v);
        if let Some(c) = item.get("cwd").and_then(Value::as_str)
            && !c.is_empty()
        {
            return Some(PathBuf::from(c));
        }
    }
    None
}

/// Whether `project` is really gone, not just on a drive or share that isn't
/// connected right now.
pub fn is_gone(project: &Path) -> bool {
    if !project.is_absolute() || project.exists() {
        return false;
    }
    project.ancestors().last().is_some_and(Path::exists)
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn size(p: &Path, cancel: &AtomicBool) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => crate::scan::dir_size(p, cancel),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Claude Code project folders whose project is gone.
pub fn claude(home: &Path, cancel: &AtomicBool) -> Vec<Orphan> {
    let root = home.join(".claude");
    let Ok(rd) = std::fs::read_dir(root.join("projects")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in rd.filter_map(Result::ok).map(|e| e.path()) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !dir.is_dir() {
            continue;
        }
        let logs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                    .collect()
            })
            .unwrap_or_default();
        // Any log naming its folder will do; they all ran in the same one.
        let Some(project) = logs.iter().find_map(|l| log_cwd(l)) else {
            continue;
        };
        if !is_gone(&project) {
            continue;
        }
        let mut paths = vec![dir.clone()];
        for l in &logs {
            if let Some(id) = l.file_stem() {
                let history = root.join("file-history").join(id);
                if history.is_dir() {
                    paths.push(history);
                }
            }
        }
        out.push(Orphan {
            agent: Agent::ClaudeCode,
            project,
            sessions: logs.len(),
            size_bytes: paths.iter().map(|p| size(p, cancel)).sum(),
            last_used: logs.iter().filter_map(|l| modified(l)).max(),
            paths,
            moved_to: None,
        });
    }
    out
}

/// Codex session files whose project is gone, one entry per project.
pub fn codex(home: &Path, cancel: &AtomicBool) -> Vec<Orphan> {
    let mut by_project: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for e in walkdir::WalkDir::new(home.join(".codex").join("sessions"))
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
    {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let p = e.path();
        if !e.file_type().is_file() || p.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        if let Some(project) = log_cwd(p).filter(|c| is_gone(c)) {
            by_project.entry(project).or_default().push(p.to_path_buf());
        }
    }
    by_project
        .into_iter()
        .map(|(project, mut paths)| {
            paths.sort();
            Orphan {
                agent: Agent::Codex,
                project,
                sessions: paths.len(),
                size_bytes: paths.iter().map(|p| size(p, cancel)).sum(),
                last_used: paths.iter().filter_map(|p| modified(p)).max(),
                paths,
                moved_to: None,
            }
        })
        .collect()
}

/// Folders that could be where a "deleted" project went: every project copy
/// the scan found, and everything directly inside the project folders.
pub fn known_projects(repos: &[PathBuf], cfg: &Config) -> Vec<PathBuf> {
    let mut out = repos.to_vec();
    for root in &cfg.project_roots {
        if let Ok(rd) = std::fs::read_dir(root) {
            out.extend(
                rd.filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path()),
            );
        }
    }
    out
}

/// Everything found, minus hidden items, biggest first. `known` is from
/// [`known_projects`]: an orphan whose project name matches one of those
/// folders is marked as possibly moved.
pub fn find(home: &Path, cfg: &Config, known: &[PathBuf], cancel: &AtomicBool) -> Vec<Orphan> {
    let mut out = claude(home, cancel);
    out.extend(codex(home, cancel));
    out.retain(|o| !cfg.is_ignored(o.key()));
    for o in &mut out {
        let name = o.project.file_name();
        o.moved_to = known
            .iter()
            .find(|k| {
                name.is_some()
                    && k.file_name()
                        .zip(name)
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                    && **k != o.project
            })
            .cloned();
    }
    out.sort_by_key(|o| std::cmp::Reverse(o.size_bytes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn line(cwd: &Path) -> String {
        serde_json::json!({"type": "user", "cwd": cwd, "timestamp": "2026-11-07T10:00:00Z"})
            .to_string()
    }

    #[test]
    fn finds_data_for_deleted_projects_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let alive = home.join("code").join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        let gone = home.join("code").join("gone");

        let cl = home.join(".claude");
        write(&cl.join("projects/-code-alive/a1.jsonl"), &line(&alive));
        write(
            &cl.join("projects/-code-gone/g1.jsonl"),
            &format!("{{\"type\":\"summary\"}}\n{}\n", line(&gone)),
        );
        write(&cl.join("projects/-code-gone/g2.jsonl"), "{}\n");
        write(&cl.join("file-history/g1/snap"), "xxxx");
        // No cwd at all: unknown, left alone.
        write(&cl.join("projects/-mystery/m.jsonl"), "{}\n");

        let codex_meta = |cwd: &Path| {
            serde_json::json!({"timestamp": "2026-11-07T10:00:00Z", "type": "session_meta", "payload": {"cwd": cwd}})
                .to_string()
        };
        let cx = home.join(".codex/sessions/2026/11/07");
        write(&cx.join("rollout-1.jsonl"), &codex_meta(&gone));
        write(&cx.join("rollout-2.jsonl"), &codex_meta(&gone));
        write(&cx.join("rollout-3.jsonl"), &codex_meta(&alive));

        let cancel = AtomicBool::new(false);
        let found = find(home, &Config::default(), &[], &cancel);
        assert_eq!(found.len(), 2);

        let c = found.iter().find(|o| o.agent == Agent::ClaudeCode).unwrap();
        assert_eq!(c.project, gone);
        assert_eq!(c.sessions, 2);
        assert_eq!(
            c.paths,
            vec![cl.join("projects/-code-gone"), cl.join("file-history/g1")]
        );
        assert_eq!(c.project_name(), "gone");

        let x = found.iter().find(|o| o.agent == Agent::Codex).unwrap();
        assert_eq!(x.sessions, 2);
        assert_eq!(
            x.paths,
            vec![cx.join("rollout-1.jsonl"), cx.join("rollout-2.jsonl")]
        );

        // Hidden items stay hidden.
        let cfg = Config {
            ignored: vec![c.key().to_path_buf()],
            ..Config::default()
        };
        assert_eq!(find(home, &cfg, &[], &cancel).len(), 1);
    }

    #[test]
    fn missing_drive_is_not_gone() {
        assert!(!is_gone(Path::new("relative/path")));
        assert!(!is_gone(&std::env::temp_dir()));
        let tmp = tempfile::tempdir().unwrap();
        assert!(is_gone(&tmp.path().join("deleted")));
    }
}
