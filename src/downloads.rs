//! Downloads matched to projects: guesses which project each file in the
//! Downloads folder was for, from what was happening when it arrived.
//!
//! Evidence, strongest first:
//! - an agent session fetched that exact file (`curl -o`, `wget`);
//! - git commits in a project shortly before or after it arrived, read from
//!   `.git/logs/HEAD`;
//! - a Claude Code or Codex session working in a project around then.
//!
//! Only reading happens here, except [`move_into`] and [`move_back`], which
//! the UI calls after the user clicks.

use crate::config::{Config, path_within};
use crate::trail::{self, Agent, Kind};
use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How far from a download's arrival an event still counts.
pub const WINDOW: Duration = Duration::from_secs(2 * 3600);
/// How far back downloads are listed (the same as the session logs).
pub const DAYS: u64 = trail::DAYS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A commit, checkout or pull in the repo.
    Git,
    /// An agent session working in the project.
    Session(Agent),
    /// An agent session downloaded this exact file.
    Fetched(Agent),
}

/// Something that happened in a project at a moment in time.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub project: PathBuf,
    pub when: SystemTime,
    pub source: Source,
    /// Set for [`Source::Fetched`]: the file the agent saved.
    pub path: Option<PathBuf>,
}

/// Why a download was matched to a project.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub project: PathBuf,
    /// The event closest to the arrival time.
    pub source: Source,
    /// Seconds from arrival to that event; negative means before it arrived.
    pub offset_secs: i64,
}

impl Match {
    /// Plain words for the row, like "You committed in it 12 min after it
    /// arrived".
    pub fn reason(&self) -> String {
        let gap = gap_words(self.offset_secs.unsigned_abs());
        let side = if self.offset_secs >= 0 {
            "after it arrived"
        } else {
            "before it arrived"
        };
        match self.source {
            Source::Fetched(a) => format!("{} downloaded it while working there", a.label()),
            Source::Git => format!("You used git there {gap} {side}"),
            Source::Session(a) => format!("{} was working there {gap} {side}", a.label()),
        }
    }
}

fn gap_words(secs: u64) -> String {
    match secs {
        0..=89 => "a minute".to_string(),
        90..=3599 => format!("{} min", secs.div_ceil(60)),
        _ => {
            let h = secs as f64 / 3600.;
            format!("{:.1} h", h).replace(".0 h", " h")
        }
    }
}

/// A file or folder at the top of the Downloads folder.
#[derive(Clone, Debug)]
pub struct Download {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size_bytes: u64,
    /// When it landed: creation time, or last change if that's unknown.
    pub arrived: SystemTime,
    pub matched: Option<Match>,
}

impl Download {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// The app's suggestion for this download and why, in plain words.
    pub fn advice(&self) -> (DownloadAdvice, String) {
        self.advice_at(SystemTime::now())
    }

    pub fn advice_at(&self, now: SystemTime) -> (DownloadAdvice, String) {
        let name = self.name().to_ascii_lowercase();
        let ext = self
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let is_self = std::env::current_exe().is_ok_and(|me| me == self.path);
        let installer = !self.is_dir
            && !is_self
            && (matches!(ext.as_str(), "msi" | "msix")
                || (ext == "exe" && ["setup", "install"].iter().any(|w| name.contains(w))));
        // A new installer may not have been run yet.
        let waited = now
            .duration_since(self.arrived)
            .is_ok_and(|d| d >= INSTALLER_WAIT);
        if installer && !waited {
            return (
                DownloadAdvice::YourCall,
                "An installer that arrived in the last week. If you've already installed the program you don't need it, and you can get it again from the program's website.".to_string(),
            );
        }
        if installer {
            return (
                DownloadAdvice::Safe,
                "An installer. Once the program is installed you don't need it, and you can get it again from the program's website.".to_string(),
            );
        }
        if self.looks_private() {
            return (
                DownloadAdvice::YourCall,
                "This looks like a private file: passwords and settings (.env), a key, a password store or a database. It may be the only copy, so keep it somewhere safe. Don't move it into a project folder, where it could be uploaded to GitHub by accident.".to_string(),
            );
        }
        if let Some(m) = &self.matched {
            let project = m
                .project
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            return (
                DownloadAdvice::Move,
                format!(
                    "It looks like it was for {project}. Move it there to keep it with the project, or recycle it if you don't need it any more."
                ),
            );
        }
        let text = match ext.as_str() {
            "zip" | "7z" | "rar" | "gz" | "tgz" | "tar" | "xz" | "bz2" | "zst" => {
                "A compressed file. If you've already unpacked it, the compressed copy isn't needed."
            }
            "exe" => {
                "A program. If you installed it or don't use it, it's fine to remove. If you run it straight from Downloads, keep it."
            }
            "iso" | "img" => "A disk image. If you've already used it, it's fine to remove.",
            _ if self.is_dir => "A folder. Open it to see what's inside before deciding.",
            _ => {
                "The app can't tell what this is for. Open it to check, then keep, hide or recycle it."
            }
        };
        (DownloadAdvice::YourCall, text.to_string())
    }

    /// A file whose name says it holds passwords, keys or data.
    pub fn looks_private(&self) -> bool {
        !self.is_dir && crate::private::looks_private(&self.name())
    }

    /// Where [`move_into`] would put it. Nowhere for a private-looking file:
    /// inside a project it could be uploaded with the project by accident.
    pub fn destination(&self) -> Option<PathBuf> {
        if self.looks_private() {
            return None;
        }
        let m = self.matched.as_ref()?;
        Some(m.project.join(self.path.file_name()?))
    }
}

/// How long an installer sits in Downloads before it's cleaned in one click.
pub const INSTALLER_WAIT: Duration = Duration::from_secs(7 * 86_400);

/// What the app suggests doing with a download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadAdvice {
    /// An installer: not needed once installed.
    Safe,
    /// Matched to a project: move it there.
    Move,
    /// Only you can tell.
    YourCall,
}

impl DownloadAdvice {
    pub fn label(self) -> &'static str {
        match self {
            DownloadAdvice::Safe => "Safe to clean",
            DownloadAdvice::Move => "Move to project",
            DownloadAdvice::YourCall => "Your call",
        }
    }
}

/// The user's Downloads folder.
pub fn downloads_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|u| u.download_dir().map(Path::to_path_buf))
}

/// Unfinished downloads and system files that never belong to a project.
pub fn is_noise(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('.')
        || lower.starts_with("~$")
        || matches!(lower.as_str(), "desktop.ini" | "thumbs.db")
        || [".crdownload", ".part", ".partial", ".tmp", ".download"]
            .iter()
            .any(|x| lower.ends_with(x))
}

/// Lists the top level of `dir`: things that arrived after `since`, newest
/// first. Folders get their full size.
pub fn list(dir: &Path, since: SystemTime, cfg: &Config, cancel: &AtomicBool) -> Vec<Download> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Download> = rd
        .filter_map(Result::ok)
        .filter(|e| !is_noise(&e.file_name().to_string_lossy()))
        .filter(|e| !cfg.is_ignored(&e.path()))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let arrived = meta.created().or_else(|_| meta.modified()).ok()?;
            (arrived >= since).then_some((e.path(), meta, arrived))
        })
        .map(|(path, meta, arrived)| {
            let is_dir = meta.is_dir();
            let size_bytes = if is_dir {
                crate::scan::dir_size(&path, cancel)
            } else {
                meta.len()
            };
            Download {
                path,
                is_dir,
                size_bytes,
                arrived,
                matched: None,
            }
        })
        .collect();
    out.sort_by_key(|d| std::cmp::Reverse(d.arrived));
    out
}

/// Times from a `.git/logs/HEAD` reflog: commits, checkouts, pulls.
/// Lines look like `old new Name <email> 1762536223 +0000\tcommit: msg`.
pub fn parse_reflog(text: &str) -> Vec<SystemTime> {
    text.lines()
        .filter_map(|line| {
            let head = line.split('\t').next()?;
            let after_email = &head[head.rfind('>')? + 1..];
            let secs: u64 = after_email.split_whitespace().next()?.parse().ok()?;
            Some(UNIX_EPOCH + Duration::from_secs(secs))
        })
        .collect()
}

/// The folder in `projects` that holds `path`, the deepest one if nested.
pub fn project_for<'a>(path: &Path, projects: &'a [PathBuf]) -> Option<&'a PathBuf> {
    projects
        .iter()
        .filter(|p| path_within(path, p))
        .max_by_key(|p| p.components().count())
}

/// Folders downloads can be matched to: the given repos that sit inside a
/// project folder, plus the folders directly inside each project folder.
pub fn candidate_projects(repos: &[PathBuf], cfg: &Config) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = repos
        .iter()
        .filter(|r| cfg.project_root_for(r).is_some() && !cfg.is_ignored(r))
        .cloned()
        .collect();
    for root in &cfg.project_roots {
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            if !hidden
                && e.file_type().is_ok_and(|t| t.is_dir())
                && !cfg.is_ignored(&p)
                && !out.iter().any(|o| path_within(o, &p) || path_within(&p, o))
            {
                out.push(p);
            }
        }
    }
    out
}

/// Git activity in each project since `since`.
pub fn git_events(projects: &[PathBuf], since: SystemTime) -> Vec<Event> {
    let mut out = Vec::new();
    for p in projects {
        let Ok(text) = std::fs::read_to_string(p.join(".git").join("logs").join("HEAD")) else {
            continue;
        };
        out.extend(
            parse_reflog(&text)
                .into_iter()
                .filter(|t| *t >= since)
                .map(|when| Event {
                    project: p.clone(),
                    when,
                    source: Source::Git,
                    path: None,
                }),
        );
    }
    out
}

/// Agent activity in each project, and downloads agents saved, from the
/// session logs in `home` since `since`.
pub fn session_events(
    home: &Path,
    projects: &[PathBuf],
    since: SystemTime,
    cancel: &AtomicBool,
) -> Vec<Event> {
    let mut out = Vec::new();
    for (agent, log) in trail::log_files(home, since) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(f) = std::fs::File::open(&log) else {
            continue;
        };
        let reader = BufReader::new(f);
        let s = match agent {
            Agent::ClaudeCode => trail::parse_claude(reader, Some(home)),
            Agent::Codex => trail::parse_codex(reader, Some(home)),
        };
        for (when, cwd) in &s.active {
            if let Some(p) = project_for(cwd, projects) {
                out.push(Event {
                    project: p.clone(),
                    when: *when,
                    source: Source::Session(agent),
                    path: None,
                });
            }
        }
        let Some(cwd) = s.cwd.as_deref() else {
            continue;
        };
        let Some(project) = project_for(cwd, projects) else {
            continue;
        };
        for t in s.touches.iter().filter(|t| t.kind == Kind::Download) {
            out.push(Event {
                project: project.clone(),
                when: t.when.unwrap_or(since),
                source: Source::Fetched(agent),
                path: Some(t.path.clone()),
            });
        }
    }
    out
}

/// Picks the project a download most likely belongs to. Each event within
/// [`WINDOW`] scores more the closer it is; a project needs a clear lead
/// over the runner-up, otherwise nothing is guessed.
pub fn best_match(d: &Download, events: &[Event]) -> Option<Match> {
    let secs = |e: &Event| -> i64 {
        match e.when.duration_since(d.arrived) {
            Ok(x) => x.as_secs() as i64,
            Err(x) => -(x.duration().as_secs() as i64),
        }
    };
    if let Some(e) = events.iter().find(|e| {
        matches!(e.source, Source::Fetched(_))
            && e.path
                .as_deref()
                .is_some_and(|p| crate::config::same_path(p, &d.path))
    }) {
        return Some(Match {
            project: e.project.clone(),
            source: e.source,
            offset_secs: secs(e),
        });
    }
    let window = WINDOW.as_secs() as f64;
    let mut scores: HashMap<&Path, (f64, &Event)> = HashMap::new();
    for e in events
        .iter()
        .filter(|e| !matches!(e.source, Source::Fetched(_)))
    {
        let gap = secs(e).unsigned_abs() as f64;
        if gap > window {
            continue;
        }
        let w = 1. - gap / window;
        let entry = scores.entry(&e.project).or_insert((0., e));
        entry.0 += w;
        if secs(e).abs() < secs(entry.1).abs() {
            entry.1 = e;
        }
    }
    let mut ranked: Vec<(f64, &Event)> = scores.into_values().collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (top, e) = *ranked.first()?;
    let runner_up = ranked.get(1).map_or(0., |r| r.0);
    if top < 0.3 || top < runner_up * 2. {
        return None;
    }
    Some(Match {
        project: e.project.clone(),
        source: e.source,
        offset_secs: secs(e),
    })
}

/// Lists the Downloads folder and matches every entry.
pub fn find(
    dir: &Path,
    home: Option<&Path>,
    repos: &[PathBuf],
    cfg: &Config,
    cancel: &AtomicBool,
) -> Vec<Download> {
    let since = SystemTime::now() - Duration::from_secs(DAYS * 86_400);
    let mut items = list(dir, since, cfg, cancel);
    if items.is_empty() {
        return items;
    }
    let projects = candidate_projects(repos, cfg);
    // Events slightly older than the oldest download still count.
    let oldest = items.iter().map(|d| d.arrived).min().unwrap_or(since);
    let from = oldest.checked_sub(WINDOW).unwrap_or(oldest);
    let mut events = git_events(&projects, from);
    if let Some(h) = home {
        events.extend(session_events(h, &projects, from, cancel));
    }
    for d in &mut items {
        d.matched = best_match(d, &events);
    }
    items
}

/// Downloads grouped by the project they matched, biggest group first;
/// unmatched ones come last under an empty path.
pub fn by_project(items: &[Download]) -> Vec<(PathBuf, Vec<&Download>)> {
    let mut groups: Vec<(PathBuf, Vec<&Download>)> = Vec::new();
    for d in items {
        let key = d
            .matched
            .as_ref()
            .map(|m| m.project.clone())
            .unwrap_or_default();
        match groups.iter_mut().find(|(p, _)| *p == key) {
            Some((_, v)) => v.push(d),
            None => groups.push((key, vec![d])),
        }
    }
    groups.sort_by_key(|(p, v)| {
        (
            p.as_os_str().is_empty(),
            std::cmp::Reverse(v.iter().map(|d| d.size_bytes).sum::<u64>()),
        )
    });
    groups
}

/// Moves `from` into `project`, keeping its name. Refuses to overwrite and
/// to copy across drives (a move there would be a copy plus a delete).
pub fn move_into(from: &Path, project: &Path) -> anyhow::Result<PathBuf> {
    let name = from
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("it has no file name"))?;
    let to = project.join(name);
    rename(from, &to)?;
    crate::actions::log_action(&format!("moved to {}", to.display()), from);
    Ok(to)
}

/// Undoes [`move_into`].
pub fn move_back(moved_to: &Path, original: &Path) -> anyhow::Result<()> {
    rename(moved_to, original)?;
    crate::actions::log_action(&format!("moved back from {}", moved_to.display()), original);
    Ok(())
}

fn rename(from: &Path, to: &Path) -> anyhow::Result<()> {
    if to.exists() {
        anyhow::bail!("{} already exists", to.display());
    }
    std::fs::rename(from, to).map_err(|e| {
        if e.kind() == std::io::ErrorKind::CrossesDevices {
            anyhow::anyhow!("the project is on another drive, so move it yourself")
        } else {
            anyhow::anyhow!(e)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn dl(path: &str, arrived: u64) -> Download {
        Download {
            path: PathBuf::from(path),
            is_dir: false,
            size_bytes: 1,
            arrived: at(arrived),
            matched: None,
        }
    }

    fn ev(project: &str, when: u64, source: Source) -> Event {
        Event {
            project: PathBuf::from(project),
            when: at(when),
            source,
            path: None,
        }
    }

    #[test]
    fn reads_reflog_times() {
        let text = "0000 abcd Jo Bear <jo@x.io> 1762536223 +0000\tcommit (initial): first\n\
                    abcd ef01 Jo <jo@x.io> 1762539823 -0500\tcheckout: moving from a to b\n\
                    garbage line\n";
        assert_eq!(parse_reflog(text), vec![at(1762536223), at(1762539823)]);
    }

    #[test]
    fn closest_busy_project_wins() {
        let d = dl("/dl/spec.pdf", 10_000);
        let events = vec![
            ev("/code/a", 10_300, Source::Git),
            ev("/code/a", 10_600, Source::Session(Agent::ClaudeCode)),
            ev("/code/b", 16_000, Source::Git),
        ];
        let m = best_match(&d, &events).unwrap();
        assert_eq!(m.project, PathBuf::from("/code/a"));
        assert_eq!(m.source, Source::Git);
        assert_eq!(m.offset_secs, 300);
        assert_eq!(m.reason(), "You used git there 5 min after it arrived");
    }

    #[test]
    fn no_guess_when_unclear_or_far() {
        let d = dl("/dl/x.zip", 10_000);
        // Two projects equally busy.
        let tie = vec![
            ev("/code/a", 10_100, Source::Git),
            ev("/code/b", 9_900, Source::Git),
        ];
        assert_eq!(best_match(&d, &tie), None);
        // Only outside the window.
        let far = vec![ev("/code/a", 10_000 + 3 * 3600, Source::Git)];
        assert_eq!(best_match(&d, &far), None);
    }

    #[test]
    fn agent_fetch_is_definite() {
        let d = dl("/dl/data.csv", 10_000);
        let events = vec![
            ev("/code/a", 10_010, Source::Git),
            ev("/code/a", 10_020, Source::Git),
            Event {
                project: PathBuf::from("/code/b"),
                when: at(9_000),
                source: Source::Fetched(Agent::Codex),
                path: Some(PathBuf::from("/dl/data.csv")),
            },
        ];
        let m = best_match(&d, &events).unwrap();
        assert_eq!(m.project, PathBuf::from("/code/b"));
        assert_eq!(m.reason(), "Codex downloaded it while working there");
    }

    #[test]
    fn deepest_project_and_noise() {
        let projects = vec![PathBuf::from("/code"), PathBuf::from("/code/app")];
        assert_eq!(
            project_for(Path::new("/code/app/src"), &projects),
            Some(&PathBuf::from("/code/app"))
        );
        assert_eq!(project_for(Path::new("/other"), &projects), None);
        assert!(is_noise("big.iso.crdownload"));
        assert!(is_noise("desktop.ini"));
        assert!(!is_noise("report.pdf"));
    }

    #[test]
    fn moves_and_refuses_to_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let dl_dir = tmp.path().join("Downloads");
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(&dl_dir).unwrap();
        std::fs::create_dir_all(&proj).unwrap();
        let f = dl_dir.join("a.txt");
        std::fs::write(&f, "x").unwrap();
        // `rename` is what move_into and move_back do, minus the log line.
        let to = proj.join("a.txt");
        rename(&f, &to).unwrap();
        assert!(to.exists() && !f.exists());
        std::fs::write(&f, "y").unwrap();
        assert!(rename(&f, &to).is_err());
        assert!(rename(&to, &f).is_err());
        std::fs::remove_file(&f).unwrap();
        rename(&to, &f).unwrap();
        assert!(f.exists());
    }

    #[test]
    fn gap_wording() {
        assert_eq!(gap_words(30), "a minute");
        assert_eq!(gap_words(600), "10 min");
        assert_eq!(gap_words(5400), "1.5 h");
        assert_eq!(gap_words(7200), "2 h");
    }

    #[test]
    fn advice_for_downloads() {
        let d = |name: &str, matched: Option<Match>| Download {
            path: PathBuf::from("/dl").join(name),
            is_dir: false,
            size_bytes: 1,
            arrived: SystemTime::UNIX_EPOCH,
            matched,
        };
        assert_eq!(
            d("App-Setup-1.2.exe", None).advice().0,
            DownloadAdvice::Safe
        );
        assert_eq!(d("thing.msi", None).advice().0, DownloadAdvice::Safe);
        let msi = d("thing.msi", None);
        let soon = msi.arrived + Duration::from_secs(86_400);
        assert_eq!(
            msi.advice_at(soon).0,
            DownloadAdvice::YourCall,
            "may not have been run yet"
        );
        assert_eq!(msi.advice_at(soon + INSTALLER_WAIT).0, DownloadAdvice::Safe);
        assert_eq!(d("tool.exe", None).advice().0, DownloadAdvice::YourCall);
        let m = Match {
            project: PathBuf::from("/code/site"),
            source: Source::Git,
            offset_secs: 60,
        };
        let (a, why) = d("logo.png", Some(m.clone())).advice();
        assert_eq!(a, DownloadAdvice::Move);
        assert!(why.contains("site"));
        assert!(d("logo.png", Some(m.clone())).destination().is_some());
        // A private-looking file is never offered a move into a project.
        let env = d(".env", Some(m));
        assert_eq!(env.advice().0, DownloadAdvice::YourCall);
        assert_eq!(env.destination(), None);
        assert!(d("data.zip", None).advice().1.contains("unpacked"));
    }
}
