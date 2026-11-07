//! Dependency updates: which of your projects have outdated npm, Cargo or
//! Python packages, using each ecosystem's own tool:
//!
//! - npm: `npm outdated --json` (needs `node_modules`; reads only)
//! - Cargo: `cargo update --dry-run --verbose` (doesn't touch the lockfile)
//! - Python: `<venv> -m pip list --outdated --not-required --format=json`
//!
//! These run tools inside the repo, so they're only used on repos in your
//! project folders, never on stray clones. Nothing here changes files; the
//! update itself is run by the user in a terminal (see `actions`).

use crate::config::Config;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long one check may take before it's given up (npm can hang offline).
const TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Eco {
    Npm,
    Cargo,
    Pip,
}

impl Eco {
    pub fn label(self) -> &'static str {
        match self {
            Eco::Npm => "npm",
            Eco::Cargo => "Cargo",
            Eco::Pip => "Python (pip)",
        }
    }
}

/// One outdated package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outdated {
    pub name: String,
    pub current: String,
    /// What the update command moves it to (within the allowed range).
    pub wanted: String,
    /// Newest release, when it's beyond `wanted`.
    pub latest: Option<String>,
}

impl Outdated {
    /// The update command can't reach the newest version: it's a new major
    /// (or outside the range in package.json / Cargo.toml) and may break.
    pub fn held_back(&self) -> bool {
        self.latest.as_ref().is_some_and(|l| *l != self.wanted)
    }

    /// The update command moves it at all.
    pub fn updatable(&self) -> bool {
        self.wanted != self.current
    }
}

/// Result of checking one ecosystem in one repo.
#[derive(Clone, Debug)]
pub struct Report {
    pub repo: PathBuf,
    pub eco: Eco,
    /// Where the command runs.
    pub dir: PathBuf,
    /// For pip: the venv folder.
    pub venv: Option<PathBuf>,
    pub packages: Vec<Outdated>,
    pub error: Option<String>,
}

impl Report {
    pub fn updatable(&self) -> usize {
        self.packages.iter().filter(|p| p.updatable()).count()
    }

    pub fn held_back(&self) -> usize {
        self.packages.iter().filter(|p| p.held_back()).count()
    }

    /// The command the Update button runs, or None when there's nothing it
    /// would change.
    pub fn update_command(&self) -> Option<String> {
        if self.updatable() == 0 {
            return None;
        }
        match self.eco {
            Eco::Npm => Some("npm update".into()),
            Eco::Cargo => Some("cargo update".into()),
            Eco::Pip => {
                let venv = self.venv.as_ref()?;
                let rel = venv.strip_prefix(&self.dir).unwrap_or(venv);
                let python = if cfg!(windows) {
                    rel.join("Scripts").join("python")
                } else {
                    rel.join("bin").join("python")
                };
                let names: Vec<&str> = self
                    .packages
                    .iter()
                    .filter(|p| p.updatable())
                    .map(|p| p.name.as_str())
                    .filter(|n| safe_name(n))
                    .collect();
                if names.is_empty() {
                    return None;
                }
                Some(format!(
                    "{} -m pip install --upgrade {}",
                    python.display(),
                    names.join(" ")
                ))
            }
        }
    }
}

/// Package names go into a command line, so only plain ones are allowed.
pub fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

// ---- detecting ---------------------------------------------------------

/// The ecosystems a repo uses at its root that can be checked: npm with
/// `node_modules` installed, Cargo with a lockfile, or a venv.
pub fn detect(repo: &Path) -> Vec<(Eco, Option<PathBuf>)> {
    let mut out = Vec::new();
    if repo.join("package.json").is_file() && repo.join("node_modules").is_dir() {
        out.push((Eco::Npm, None));
    }
    if repo.join("Cargo.toml").is_file() && repo.join("Cargo.lock").is_file() {
        out.push((Eco::Cargo, None));
    }
    if let Some(v) = [".venv", "venv", "env"]
        .iter()
        .map(|n| repo.join(n))
        .find(|v| v.join("pyvenv.cfg").is_file() && venv_python(v).is_file())
    {
        out.push((Eco::Pip, Some(v)));
    }
    out
}

pub fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

// ---- parsing -----------------------------------------------------------

/// `npm outdated --json`. Packages without a `current` (not installed) are
/// skipped.
pub fn parse_npm(json: &str) -> Result<Vec<Outdated>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(json).map_err(|e| format!("unexpected npm output: {e}"))?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("npm reported an error");
        return Err(msg.to_string());
    }
    let mut out: Vec<Outdated> = v
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, info)| {
            // Workspaces can list one package several times as an array.
            let info = info.as_array().and_then(|a| a.first()).unwrap_or(info);
            let s = |k: &str| info.get(k).and_then(Value::as_str).map(str::to_string);
            let current = s("current")?;
            let wanted = s("wanted").unwrap_or_else(|| current.clone());
            let latest = s("latest").filter(|l| *l != wanted);
            Some(Outdated {
                name: name.clone(),
                current,
                wanted,
                latest,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `cargo update --dry-run --verbose` (cargo writes this to stderr).
/// "Updating a v1 -> v2" is a compatible update; "Unchanged b v1
/// (available: v2)" is one only a Cargo.toml change gets.
pub fn parse_cargo(text: &str) -> Vec<Outdated> {
    let strip = |v: &str| v.trim().trim_start_matches('v').to_string();
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("Updating") => {
                let rest: Vec<&str> = words.collect();
                if let [name, from, "->", to] = rest.as_slice() {
                    out.push(Outdated {
                        name: (*name).to_string(),
                        current: strip(from),
                        wanted: strip(to),
                        latest: None,
                    });
                }
            }
            Some("Unchanged") => {
                let rest: Vec<&str> = words.collect();
                if let [name, current, avail, latest] = rest.as_slice()
                    && *avail == "(available:"
                {
                    out.push(Outdated {
                        name: (*name).to_string(),
                        current: strip(current),
                        wanted: strip(current),
                        latest: Some(strip(latest.trim_end_matches(')'))),
                    });
                }
            }
            _ => {}
        }
    }
    // An update can also leave it behind latest: merge the two lines.
    out.sort_by(|a, b| a.name.cmp(&b.name).then(b.updatable().cmp(&a.updatable())));
    out.dedup_by(|b, a| {
        if a.name == b.name {
            a.latest = a.latest.take().or(b.latest.take());
            true
        } else {
            false
        }
    });
    out
}

/// `pip list --outdated --format=json`. pip itself is left out.
pub fn parse_pip(json: &str) -> Result<Vec<Outdated>, String> {
    let v: Value =
        serde_json::from_str(json.trim()).map_err(|e| format!("unexpected pip output: {e}"))?;
    let mut out: Vec<Outdated> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
            let name = s("name")?;
            if name.eq_ignore_ascii_case("pip") {
                return None;
            }
            let latest = s("latest_version")?;
            Some(Outdated {
                name,
                current: s("version")?,
                wanted: latest,
                latest: None,
            })
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

// ---- running -----------------------------------------------------------

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs a tool in `dir` with no window and a time limit.
fn run(dir: &Path, program: &str, args: &[&str], cancel: &AtomicBool) -> Result<Output, String> {
    let mut cmd = if cfg!(windows) && program == "npm" {
        // npm is a .cmd script on Windows.
        let mut c = Command::new("cmd");
        c.arg("/C").arg("npm");
        c
    } else {
        Command::new(program)
    };
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{program} isn't installed or isn't on PATH")
        } else {
            format!("couldn't run {program}: {e}")
        }
    })?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_string(&mut s);
            }
            s
        })
    };
    let out_t = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err_t = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) {
                "stopped".into()
            } else {
                format!("{program} took too long (no network?)")
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        code: status.code(),
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
    })
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("npm warn"))
        .unwrap_or("failed")
        .chars()
        .take(200)
        .collect()
}

/// Checks one ecosystem in one repo.
pub fn check(repo: &Path, eco: Eco, venv: Option<PathBuf>, cancel: &AtomicBool) -> Report {
    let mut r = Report {
        repo: repo.to_path_buf(),
        eco,
        dir: repo.to_path_buf(),
        venv: venv.clone(),
        packages: Vec::new(),
        error: None,
    };
    let result = match eco {
        // Exit code 1 just means "something is outdated".
        Eco::Npm => run(repo, "npm", &["outdated", "--json"], cancel).and_then(|o| match o.code {
            Some(0) | Some(1) => parse_npm(&o.stdout),
            _ => Err(first_line(&o.stderr)),
        }),
        Eco::Cargo => run(
            repo,
            "cargo",
            &["update", "--dry-run", "--verbose", "--color", "never"],
            cancel,
        )
        .and_then(|o| {
            if o.code == Some(0) {
                Ok(parse_cargo(&o.stderr))
            } else {
                Err(first_line(&o.stderr))
            }
        }),
        Eco::Pip => match venv {
            None => Err("no virtual env".into()),
            Some(v) => {
                let py = venv_python(&v);
                run(
                    repo,
                    &py.to_string_lossy(),
                    &[
                        "-m",
                        "pip",
                        "list",
                        "--outdated",
                        "--not-required",
                        "--format=json",
                    ],
                    cancel,
                )
                .and_then(|o| {
                    if o.code == Some(0) {
                        parse_pip(&o.stdout)
                    } else {
                        Err(first_line(&o.stderr))
                    }
                })
            }
        },
    };
    match result {
        Ok(p) => r.packages = p,
        Err(e) => r.error = Some(e),
    }
    r
}

/// Repos worth checking: inside a project folder and not hidden.
pub fn eligible(repos: &[PathBuf], cfg: &Config) -> Vec<PathBuf> {
    repos
        .iter()
        .filter(|r| cfg.project_root_for(r).is_some() && !cfg.is_ignored(r))
        .cloned()
        .collect()
}

/// Checks every eligible repo. Reports with nothing outdated and no error
/// are dropped. `done` counts finished repos.
pub fn check_all(repos: &[PathBuf], cancel: &AtomicBool, done: &AtomicUsize) -> Vec<Report> {
    let mut out = Vec::new();
    for repo in repos {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for (eco, venv) in detect(repo) {
            let r = check(repo, eco, venv, cancel);
            if r.error.is_some() || !r.packages.is_empty() {
                out.push(r);
            }
        }
        done.fetch_add(1, Ordering::Relaxed);
    }
    out.sort_by_key(|r| std::cmp::Reverse((r.updatable(), r.packages.len())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_output() {
        let json = r#"{
  "chalk": {"current": "4.0.0", "wanted": "4.1.2", "latest": "6.0.1", "dependent": "x", "location": "x"},
  "lodash": {"current": "4.17.0", "wanted": "4.18.1", "latest": "4.18.1", "dependent": "x", "location": "x"},
  "missing": {"wanted": "1.0.0", "latest": "1.0.0"}
}"#;
        let p = parse_npm(json).unwrap();
        assert_eq!(
            p,
            vec![
                Outdated {
                    name: "chalk".into(),
                    current: "4.0.0".into(),
                    wanted: "4.1.2".into(),
                    latest: Some("6.0.1".into())
                },
                Outdated {
                    name: "lodash".into(),
                    current: "4.17.0".into(),
                    wanted: "4.18.1".into(),
                    latest: None
                },
            ]
        );
        assert!(p[0].held_back() && p[0].updatable());
        assert!(!p[1].held_back());
        assert!(parse_npm("").unwrap().is_empty());
        assert!(parse_npm(r#"{"error":{"code":"E404","summary":"Not found"}}"#).is_err());
    }

    #[test]
    fn cargo_output() {
        let text = "    Updating crates.io index
     Locking 2 packages to latest compatible versions
    Updating anyhow v1.0.50 -> v1.0.104
    Updating serde v1.0.1 -> v1.0.9
   Unchanged itoa v0.4.8 (available: v1.0.18)
   Unchanged serde v1.0.1 (available: v2.0.0)
note: to see how you depend on a package, run `cargo tree --invert <dep>@<ver>`
warning: not updating lockfile due to dry run";
        let p = parse_cargo(text);
        let names: Vec<_> = p
            .iter()
            .map(|o| {
                (
                    o.name.as_str(),
                    o.current.as_str(),
                    o.wanted.as_str(),
                    o.latest.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("anyhow", "1.0.50", "1.0.104", None),
                ("itoa", "0.4.8", "0.4.8", Some("1.0.18")),
                ("serde", "1.0.1", "1.0.9", Some("2.0.0")),
            ]
        );
        assert!(!p[1].updatable() && p[1].held_back());
    }

    #[test]
    fn pip_output() {
        let json = r#"[{"name": "idna", "version": "3.0", "latest_version": "3.20", "latest_filetype": "wheel"}, {"name": "pip", "version": "26.2.2", "latest_version": "26.3", "latest_filetype": "wheel"}]"#;
        let p = parse_pip(json).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "idna");
        assert!(p[0].updatable());
    }

    #[test]
    fn update_commands() {
        let pkg = |n: &str, cur: &str, want: &str| Outdated {
            name: n.into(),
            current: cur.into(),
            wanted: want.into(),
            latest: None,
        };
        let mut r = Report {
            repo: PathBuf::from("/p"),
            eco: Eco::Npm,
            dir: PathBuf::from("/p"),
            venv: None,
            packages: vec![pkg("a", "1.0.0", "1.1.0")],
            error: None,
        };
        assert_eq!(r.update_command().as_deref(), Some("npm update"));
        r.packages = vec![Outdated {
            latest: Some("2.0.0".into()),
            ..pkg("a", "1.0.0", "1.0.0")
        }];
        assert_eq!(r.update_command(), None, "nothing the command can move");

        r.eco = Eco::Pip;
        r.venv = Some(PathBuf::from("/p/.venv"));
        r.packages = vec![pkg("idna", "3.0", "3.20"), pkg("bad&name", "1", "2")];
        let py = if cfg!(windows) {
            Path::new(".venv").join("Scripts").join("python")
        } else {
            Path::new(".venv").join("bin").join("python")
        };
        assert_eq!(
            r.update_command(),
            Some(format!("{} -m pip install --upgrade idna", py.display()))
        );
        assert!(safe_name("typing_extensions") && !safe_name("x;rm") && !safe_name(""));
    }

    #[test]
    fn detects_ecosystems() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        assert!(detect(r).is_empty());
        std::fs::write(r.join("package.json"), "{}").unwrap();
        assert!(detect(r).is_empty(), "not installed yet");
        std::fs::create_dir(r.join("node_modules")).unwrap();
        std::fs::write(r.join("Cargo.toml"), "").unwrap();
        std::fs::write(r.join("Cargo.lock"), "").unwrap();
        let py = venv_python(&r.join(".venv"));
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();
        std::fs::write(r.join(".venv").join("pyvenv.cfg"), "").unwrap();
        let found: Vec<_> = detect(r).into_iter().map(|(e, _)| e).collect();
        assert_eq!(found, vec![Eco::Npm, Eco::Cargo, Eco::Pip]);
    }
}
