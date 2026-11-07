//! Agent trail: reads Claude Code and Codex session logs to find files and
//! clones an agent created outside the project it was working in.
//!
//! - Claude Code keeps one JSONL file per session under
//!   `~/.claude/projects/<encoded cwd>/<session id>.jsonl`. Lines carry `cwd`
//!   and `timestamp`; assistant lines hold `tool_use` blocks (`Write`,
//!   `Edit`, `MultiEdit`, `NotebookEdit`, `Bash`).
//! - Codex keeps `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Lines are
//!   `{timestamp, type, payload}`: `session_meta` and `turn_context` carry
//!   `cwd`; `response_item` payloads are `function_call` (JSON string
//!   `arguments`), `custom_tool_call` (`input`, e.g. `apply_patch`) or
//!   `local_shell_call` (`action.command`).
//!
//! Only reading happens here. Parsing is lenient: unknown lines are skipped.

use crate::config::{Config, path_within};
use crate::git::{self, GitState};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// `git clone` / `gh repo clone`.
    Clone,
    /// `curl -o`, `wget`.
    Download,
    /// Created with a file tool or a patch.
    File,
    /// An existing file the agent changed. It was there before, so it's
    /// probably not clutter.
    Edited,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Clone => "Cloned repo",
            Kind::Download => "Download",
            Kind::File => "File written",
            Kind::Edited => "File changed",
        }
    }
}

/// Something a session created, as found in its log.
#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
    pub path: PathBuf,
    pub kind: Kind,
    pub when: Option<SystemTime>,
}

/// One session's log, reduced to what matters here.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub cwd: Option<PathBuf>,
    pub touches: Vec<Touch>,
}

/// A leftover on disk, with where it came from.
#[derive(Clone, Debug)]
pub struct Item {
    pub path: PathBuf,
    pub kind: Kind,
    pub agent: Agent,
    /// The folder the agent was working in.
    pub project: PathBuf,
    pub when: Option<SystemTime>,
    pub is_dir: bool,
    pub size_bytes: u64,
    /// Read when the item is a git repo.
    pub git: Option<GitState>,
    /// It was on disk before the agent first touched it: an edited file, or
    /// one whose creation time is older than the session's first mention.
    pub existed_before: bool,
}

impl Item {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// A repo with commits, changes or stashes that exist only here.
    pub fn has_local_work(&self) -> bool {
        self.git.as_ref().is_some_and(GitState::has_local_work)
    }

    pub fn is_repo(&self) -> bool {
        self.kind == Kind::Clone && self.is_dir
    }

    /// Reasons to think twice before recycling, in plain words. Empty means
    /// it looks like something the agent made and nothing else depends on.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.existed_before {
            out.push("it was already there before the agent touched it".to_string());
        }
        if self.is_repo() {
            match &self.git {
                // Unknown git state counts as unsaved work.
                None => out.push("git couldn't read it".to_string()),
                Some(g) => out.extend(g.local_work_reasons()),
            }
        }
        out
    }

    pub fn needs_care(&self) -> bool {
        !self.warnings().is_empty()
    }
}

/// How far back the trail reads logs.
pub const DAYS: u64 = 90;

/// Items grouped by the project the agent was working in, biggest group
/// first. Items keep their order (newest first) inside a group.
pub fn by_project(items: &[Item]) -> Vec<(PathBuf, Vec<&Item>)> {
    let mut groups: Vec<(PathBuf, Vec<&Item>)> = Vec::new();
    for it in items {
        match groups.iter_mut().find(|(p, _)| *p == it.project) {
            Some((_, v)) => v.push(it),
            None => groups.push((it.project.clone(), vec![it])),
        }
    }
    groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.iter().map(|i| i.size_bytes).sum::<u64>()));
    groups
}

/// Items that look safe to recycle without a second look.
pub fn safe_paths(items: &[Item]) -> Vec<PathBuf> {
    items
        .iter()
        .filter(|i| !i.needs_care())
        .map(|i| i.path.clone())
        .collect()
}

// ---- parsing -----------------------------------------------------------

/// Parses a Claude Code session log.
pub fn parse_claude<R: BufRead>(reader: R, home: Option<&Path>) -> Session {
    let mut s = Session::default();
    for line in reader.lines().map_while(Result::ok) {
        // Cheap filter before parsing: only lines that can carry what we use.
        if !line.contains("\"cwd\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        if s.cwd.is_none() {
            s.cwd.clone_from(&cwd);
        }
        if !line.contains("tool_use") {
            continue;
        }
        let when = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso8601);
        let base = cwd.or_else(|| s.cwd.clone()).unwrap_or_default();
        let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for b in blocks {
            if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let name = b.get("name").and_then(Value::as_str).unwrap_or_default();
            let input = b.get("input").cloned().unwrap_or(Value::Null);
            match name {
                "Write" => {
                    if let Some(p) = input.get("file_path").and_then(Value::as_str) {
                        push(&mut s, resolve(p, &base, home), Kind::File, when);
                    }
                }
                "Edit" | "MultiEdit" => {
                    if let Some(p) = input.get("file_path").and_then(Value::as_str) {
                        push(&mut s, resolve(p, &base, home), Kind::Edited, when);
                    }
                }
                "NotebookEdit" => {
                    if let Some(p) = input.get("notebook_path").and_then(Value::as_str) {
                        push(&mut s, resolve(p, &base, home), Kind::Edited, when);
                    }
                }
                "Bash" | "PowerShell" => {
                    if let Some(cmd) = input.get("command").and_then(Value::as_str) {
                        for (path, kind) in shell_targets(cmd, &base, home) {
                            push(&mut s, path, kind, when);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    s
}

/// Parses a Codex rollout log.
pub fn parse_codex<R: BufRead>(reader: R, home: Option<&Path>) -> Session {
    let mut s = Session::default();
    let mut cwd: Option<PathBuf> = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let when = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso8601);
        // Newer logs wrap items as {type, payload}; older ones don't.
        let item = v.get("payload").unwrap_or(&v);
        if let Some(c) = item.get("cwd").and_then(Value::as_str) {
            cwd = Some(PathBuf::from(c));
            if s.cwd.is_none() {
                s.cwd.clone_from(&cwd);
            }
        }
        let base = cwd.clone().unwrap_or_default();
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let args: Value = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or(Value::Null);
                let dir = args
                    .get("workdir")
                    .and_then(Value::as_str)
                    .map(|w| resolve(w, &base, home))
                    .unwrap_or_else(|| base.clone());
                if name == "apply_patch" {
                    if let Some(p) = args.get("input").and_then(Value::as_str) {
                        for (path, kind) in patch_targets(p, &dir, home) {
                            push(&mut s, path, kind, when);
                        }
                    }
                    continue;
                }
                let cmd = command_text(args.get("command").or_else(|| args.get("cmd")));
                if let Some(cmd) = cmd {
                    for (path, kind) in shell_targets(&cmd, &dir, home) {
                        push(&mut s, path, kind, when);
                    }
                    // `apply_patch` can also arrive through the shell tool.
                    if cmd.contains("*** Begin Patch") {
                        for (path, kind) in patch_targets(&cmd, &dir, home) {
                            push(&mut s, path, kind, when);
                        }
                    }
                }
            }
            Some("custom_tool_call") => {
                if let Some(input) = item.get("input").and_then(Value::as_str)
                    && input.contains("*** Begin Patch")
                {
                    for (path, kind) in patch_targets(input, &base, home) {
                        push(&mut s, path, kind, when);
                    }
                }
            }
            Some("local_shell_call") => {
                let action = item.get("action").cloned().unwrap_or(Value::Null);
                let dir = action
                    .get("working_directory")
                    .and_then(Value::as_str)
                    .map(|w| resolve(w, &base, home))
                    .unwrap_or_else(|| base.clone());
                if let Some(cmd) = command_text(action.get("command")) {
                    for (path, kind) in shell_targets(&cmd, &dir, home) {
                        push(&mut s, path, kind, when);
                    }
                }
            }
            _ => {}
        }
    }
    s
}

fn push(s: &mut Session, path: PathBuf, kind: Kind, when: Option<SystemTime>) {
    if !path.as_os_str().is_empty() {
        s.touches.push(Touch { path, kind, when });
    }
}

/// A shell command as one string. `["bash", "-lc", "..."]` becomes the script.
fn command_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [shell, flag, script]
                    if flag.starts_with('-')
                        && (flag.contains('c') || flag.eq_ignore_ascii_case("-command"))
                        && !shell.is_empty() =>
                {
                    Some((*script).to_string())
                }
                _ => Some(parts.join(" ")),
            }
        }
        _ => None,
    }
}

/// Files added (or moved into place) and files updated by an `apply_patch`
/// envelope.
pub fn patch_targets(patch: &str, cwd: &Path, home: Option<&Path>) -> Vec<(PathBuf, Kind)> {
    patch
        .lines()
        .filter_map(|l| {
            l.strip_prefix("*** Add File: ")
                .or_else(|| l.strip_prefix("*** Move to: "))
                .map(|p| (p, Kind::File))
                .or_else(|| {
                    l.strip_prefix("*** Update File: ")
                        .map(|p| (p, Kind::Edited))
                })
        })
        .map(|(p, k)| (resolve(p.trim(), cwd, home), k))
        .collect()
}

/// Splits a shell script into simple commands and tokens. Handles quotes,
/// `&&`, `||`, `;`, `|` and newlines; enough for what agents type.
fn split_commands(script: &str) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut tok = String::new();
    let mut in_tok = false;
    let mut chars = script.chars().peekable();
    let mut quote: Option<char> = None;
    let flush_tok = |tok: &mut String, in_tok: &mut bool, cur: &mut Vec<String>| {
        if *in_tok {
            cur.push(std::mem::take(tok));
            *in_tok = false;
        }
    };
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' {
                if let Some(n) = chars.next() {
                    tok.push(n);
                }
            } else {
                tok.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                in_tok = true;
            }
            ' ' | '\t' => flush_tok(&mut tok, &mut in_tok, &mut cur),
            '\n' | ';' | '|' | '&' => {
                flush_tok(&mut tok, &mut in_tok, &mut cur);
                // `&&`, `||`, and a lone `&` all end the command.
                if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                if !cur.is_empty() {
                    cmds.push(std::mem::take(&mut cur));
                }
            }
            '\\' => {
                if let Some(n) = chars.next()
                    && n != '\n'
                {
                    tok.push(n);
                    in_tok = true;
                }
            }
            _ => {
                tok.push(c);
                in_tok = true;
            }
        }
    }
    flush_tok(&mut tok, &mut in_tok, &mut cur);
    if !cur.is_empty() {
        cmds.push(cur);
    }
    cmds
}

/// Clones and downloads a shell script creates, following `cd`.
pub fn shell_targets(script: &str, cwd: &Path, home: Option<&Path>) -> Vec<(PathBuf, Kind)> {
    let mut out = Vec::new();
    let mut dir = cwd.to_path_buf();
    for cmd in split_commands(script) {
        // Skip `VAR=value` prefixes and `sudo`/`env`.
        let mut i = 0;
        while i < cmd.len()
            && (cmd[i].contains('=') && !cmd[i].starts_with('-')
                || cmd[i] == "sudo"
                || cmd[i] == "env")
        {
            i += 1;
        }
        let args = &cmd[i..];
        let Some(prog) = args.first() else { continue };
        let prog = prog
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(prog)
            .trim_end_matches(".exe");
        match prog {
            "cd" | "pushd" | "Set-Location" => {
                if let Some(d) = args.get(1).filter(|d| !d.starts_with('-')) {
                    dir = resolve(d, &dir, home);
                }
            }
            "git" => {
                // git [-C dir] clone ...
                let mut j = 1;
                let mut gdir = dir.clone();
                while j < args.len() && args[j].starts_with('-') {
                    if args[j] == "-C" {
                        if let Some(d) = args.get(j + 1) {
                            gdir = resolve(d, &dir, home);
                        }
                        j += 1;
                    } else if args[j] == "-c" {
                        j += 1;
                    }
                    j += 1;
                }
                if args.get(j).map(String::as_str) == Some("clone")
                    && let Some(p) = clone_dest(&args[j + 1..], &gdir, home)
                {
                    out.push((p, Kind::Clone));
                }
            }
            "gh" => {
                if args.get(1).map(String::as_str) == Some("repo")
                    && args.get(2).map(String::as_str) == Some("clone")
                    && let Some(p) = clone_dest(&args[3..], &dir, home)
                {
                    out.push((p, Kind::Clone));
                }
            }
            "curl" => {
                // `-o FILE` names the file; `-O` saves each URL under its own
                // name in the current folder.
                let mut remote_name = false;
                let mut urls = Vec::new();
                let mut j = 1;
                while j < args.len() {
                    let a = &args[j];
                    if a == "-o" || a == "--output" {
                        if let Some(f) = args.get(j + 1)
                            && f != "-"
                            && f != "/dev/null"
                            && !f.eq_ignore_ascii_case("nul")
                        {
                            out.push((resolve(f, &dir, home), Kind::Download));
                        }
                        j += 2;
                        continue;
                    }
                    if a == "-O" || a == "--remote-name" || a == "--remote-name-all" {
                        remote_name = true;
                    } else if !a.starts_with('-') && a.contains("://") {
                        urls.push(a.clone());
                    }
                    j += 1;
                }
                if remote_name {
                    for u in urls {
                        out.push((dir.join(url_basename(&u)), Kind::Download));
                    }
                }
            }
            "wget" => {
                let mut target_dir = dir.clone();
                let mut explicit = None;
                let mut urls = Vec::new();
                let mut j = 1;
                while j < args.len() {
                    let a = &args[j];
                    if a == "-O" || a == "--output-document" {
                        explicit = args.get(j + 1).cloned();
                        j += 2;
                        continue;
                    }
                    if a == "-P" || a == "--directory-prefix" {
                        if let Some(d) = args.get(j + 1) {
                            target_dir = resolve(d, &dir, home);
                        }
                        j += 2;
                        continue;
                    }
                    if a.contains("://") {
                        urls.push(a.clone());
                    }
                    j += 1;
                }
                match explicit {
                    Some(f) if f != "-" => out.push((resolve(&f, &dir, home), Kind::Download)),
                    Some(_) => {}
                    None => {
                        for u in urls {
                            out.push((target_dir.join(url_basename(&u)), Kind::Download));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Where `git clone <args>` puts the repo.
fn clone_dest(args: &[String], cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    // Options that take a separate value.
    const WITH_VALUE: [&str; 14] = [
        "-b",
        "--branch",
        "-o",
        "--origin",
        "--depth",
        "-c",
        "--config",
        "--reference",
        "-j",
        "--jobs",
        "--filter",
        "--template",
        "-u",
        "--upload-pack",
    ];
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            pos.extend(args[i + 1..].iter().cloned());
            break;
        }
        if a.starts_with('-') {
            if WITH_VALUE.contains(&a.as_str()) {
                i += 1;
            }
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    let url = pos.first()?;
    match pos.get(1) {
        Some(d) => Some(resolve(d, cwd, home)),
        None => {
            let name = url_basename(url);
            let name = name.strip_suffix(".git").unwrap_or(&name).to_string();
            (!name.is_empty()).then(|| cwd.join(name))
        }
    }
}

fn url_basename(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    url.trim_end_matches('/')
        .rsplit(['/', ':', '\\'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Turns a path from a log into an absolute, normalized path: expands `~`,
/// maps Git Bash style `/c/Users/...` to `C:\Users\...` on Windows, joins
/// relative paths onto `cwd`, and removes `.` and `..`.
pub fn resolve(p: &str, cwd: &Path, home: Option<&Path>) -> PathBuf {
    let p = p.trim();
    let raw: PathBuf = if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        match home {
            Some(h) => h.join(rest),
            None => PathBuf::from(p),
        }
    } else if p == "~" {
        home.map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(p))
    } else if let Some(win) = git_bash_to_windows(p) {
        PathBuf::from(win)
    } else {
        let pb = PathBuf::from(p);
        if pb.is_absolute() || is_windows_absolute(p) {
            pb
        } else {
            cwd.join(pb)
        }
    };
    normalize(&raw)
}

fn is_windows_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// `/c/Users/x` -> `C:\Users\x`, only on Windows (elsewhere it's a real path).
fn git_bash_to_windows(p: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
        Some(format!(
            "{}:\\{}",
            (b[1] as char).to_ascii_uppercase(),
            p[3..].replace('/', "\\")
        ))
    } else {
        None
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `2026-11-07T17:23:43.986Z` (fractional seconds optional) to a time.
pub fn parse_iso8601(s: &str) -> Option<SystemTime> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let time = time.split(['+', '.']).next()?;
    let mut t = time.split(':').map(|x| x.parse::<i64>());
    let (hh, mm, ss) = (
        t.next()?.ok()?,
        t.next()?.ok()?,
        t.next().and_then(Result::ok).unwrap_or(0),
    );
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3_600 + mm * 60 + ss;
    u64::try_from(secs)
        .ok()
        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

// ---- collecting --------------------------------------------------------

/// Log files to read: `(agent, path)`, newest first, modified after `since`.
pub fn log_files(home: &Path, since: SystemTime) -> Vec<(Agent, PathBuf)> {
    let mut out: Vec<(Agent, PathBuf, SystemTime)> = Vec::new();
    let mut walk = |agent: Agent, root: PathBuf, depth: usize| {
        for e in walkdir::WalkDir::new(root)
            .max_depth(depth)
            .into_iter()
            .filter_map(Result::ok)
        {
            if e.file_type().is_file()
                && e.path().extension().is_some_and(|x| x == "jsonl")
                && let Ok(m) = e.metadata()
                && let Ok(t) = m.modified()
                && t >= since
            {
                out.push((agent, e.into_path(), t));
            }
        }
    };
    walk(Agent::ClaudeCode, home.join(".claude").join("projects"), 2);
    walk(Agent::Codex, home.join(".codex").join("sessions"), 4);
    out.sort_by_key(|x| std::cmp::Reverse(x.2));
    out.into_iter().map(|(a, p, _)| (a, p)).collect()
}

/// Whether a touched path is a leftover worth showing: outside the session's
/// own folder and the user's projects, not hidden, not inside the agents'
/// own data folders.
pub fn is_stray(
    path: &Path,
    session_cwd: Option<&Path>,
    cfg: &Config,
    home: Option<&Path>,
) -> bool {
    if session_cwd.is_some_and(|c| !c.as_os_str().is_empty() && path_within(path, c)) {
        return false;
    }
    if cfg.project_root_for(path).is_some() || cfg.is_ignored(path) {
        return false;
    }
    if let Some(h) = home {
        for own in [
            ".claude", ".codex", ".config", ".cache", ".npm", ".cargo", ".rustup",
        ] {
            if path_within(path, &h.join(own)) {
                return false;
            }
        }
        if crate::config::same_path(path, h) {
            return false;
        }
    }
    true
}

/// Reads every recent session log and returns the stray things that still
/// exist, newest first, one entry per path.
pub fn collect(cfg: &Config, home: &Path, since: SystemTime, cancel: &AtomicBool) -> Vec<Item> {
    let mut found: HashMap<PathBuf, Item> = HashMap::new();
    // Earliest mention of each path, to tell created from pre-existing.
    let mut first: HashMap<PathBuf, (Option<SystemTime>, Kind)> = HashMap::new();
    for (agent, log) in log_files(home, since) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Ok(f) = std::fs::File::open(&log) else {
            continue;
        };
        let reader = BufReader::new(f);
        let session = match agent {
            Agent::ClaudeCode => parse_claude(reader, Some(home)),
            Agent::Codex => parse_codex(reader, Some(home)),
        };
        let fallback = std::fs::metadata(&log).and_then(|m| m.modified()).ok();
        for t in session.touches {
            // A file inside a clone the same session made is part of the clone.
            let path = t.path;
            if !is_stray(&path, session.cwd.as_deref(), cfg, Some(home)) {
                continue;
            }
            let when = t.when.or(fallback);
            let f = first.entry(path.clone()).or_insert((when, t.kind));
            if when < f.0 {
                *f = (when, t.kind);
            }
            match found.get_mut(&path) {
                Some(existing) => {
                    if when > existing.when {
                        existing.when = when;
                    }
                    existing.kind = existing.kind.min(t.kind);
                }
                None => {
                    found.insert(
                        path.clone(),
                        Item {
                            path,
                            kind: t.kind,
                            agent,
                            project: session.cwd.clone().unwrap_or_default(),
                            when,
                            is_dir: false,
                            size_bytes: 0,
                            git: None,
                            existed_before: false,
                        },
                    );
                }
            }
        }
    }

    // Collapse files that live inside another found item (e.g. files written
    // into a clone) into that item.
    let mut paths: Vec<PathBuf> = found.keys().cloned().collect();
    paths.sort();
    let mut parents: Vec<PathBuf> = Vec::new();
    for p in paths {
        if parents
            .iter()
            .any(|parent| p != *parent && path_within(&p, parent))
        {
            found.remove(&p);
        } else {
            parents.push(p);
        }
    }

    let mut items: Vec<Item> = found
        .into_values()
        .filter_map(|mut it| {
            let meta = std::fs::metadata(&it.path).ok()?;
            it.is_dir = meta.is_dir();
            // Written then edited means created; edited first means it was
            // already there. Clones and downloads keep their kind.
            let (first_when, first_kind) = first.get(&it.path).copied().unwrap_or((None, it.kind));
            if matches!(it.kind, Kind::File | Kind::Edited) {
                it.kind = first_kind.max(Kind::File);
            }
            // Older on disk than the first mention (with slack for clocks):
            // the agent didn't create it.
            let older = match (meta.created(), first_when) {
                (Ok(c), Some(w)) => c + Duration::from_secs(120) < w,
                _ => false,
            };
            it.existed_before = it.kind == Kind::Edited || older;
            if it.is_dir {
                it.size_bytes = crate::scan::dir_size(&it.path, cancel);
                if it.path.join(".git").exists() {
                    it.git = git::inspect(&it.path).ok();
                    it.kind = Kind::Clone;
                }
            } else {
                it.size_bytes = meta.len();
            }
            Some(it)
        })
        .collect();
    items.sort_by_key(|x| std::cmp::Reverse(x.when));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn shell_clones_downloads_and_cd() {
        let home = p("/home/me");
        let t = shell_targets(
            "cd /tmp && git clone --depth 1 https://github.com/a/tool.git && cd tool && npm i",
            &p("/home/me/work"),
            Some(&home),
        );
        assert_eq!(t, vec![(p("/tmp/tool"), Kind::Clone)]);

        let t = shell_targets(
            "git clone -b main git@github.com:a/b.git ~/scratch/b2; gh repo clone o/r",
            &p("/w"),
            Some(&home),
        );
        assert_eq!(
            t,
            vec![
                (p("/home/me/scratch/b2"), Kind::Clone),
                (p("/w/r"), Kind::Clone)
            ]
        );

        let t = shell_targets(
            "curl -sSL -o ../data.zip https://x.io/d.zip\nwget -P /tmp/dl https://x.io/a/file.tar.gz?x=1\ncurl https://x.io > /dev/null",
            &p("/w/proj"),
            Some(&home),
        );
        assert_eq!(
            t,
            vec![
                (p("/w/data.zip"), Kind::Download),
                (p("/tmp/dl/file.tar.gz"), Kind::Download)
            ]
        );

        let t = shell_targets("curl -L https://x.io/r/tool.exe -O", &p("/w"), None);
        assert_eq!(t, vec![(p("/w/tool.exe"), Kind::Download)]);

        let t = shell_targets("git -C /opt clone https://h/x/y", &p("/w"), None);
        assert_eq!(t, vec![(p("/opt/y"), Kind::Clone)]);
        let t = shell_targets("echo 'git clone https://h/x/y'", &p("/w"), None);
        assert!(t.is_empty());
    }

    #[test]
    fn claude_log() {
        let log = r#"{"type":"user","cwd":"/home/me/proj","timestamp":"2026-11-07T10:00:00.000Z","message":{"content":"hi"}}
{"type":"assistant","cwd":"/home/me/proj","timestamp":"2026-11-07T10:01:00.000Z","message":{"content":[{"type":"text","text":"ok"},{"type":"tool_use","name":"Write","input":{"file_path":"/home/me/notes/research.md","content":"x"}},{"type":"tool_use","name":"Write","input":{"file_path":"src/main.rs","content":"x"}},{"type":"tool_use","name":"Bash","input":{"command":"cd ~/Downloads && git clone https://github.com/a/demo"}}]}}
not json
"#;
        let s = parse_claude(Cursor::new(log), Some(Path::new("/home/me")));
        assert_eq!(s.cwd, Some(p("/home/me/proj")));
        let paths: Vec<_> = s.touches.iter().map(|t| (t.path.clone(), t.kind)).collect();
        assert_eq!(
            paths,
            vec![
                (p("/home/me/notes/research.md"), Kind::File),
                (p("/home/me/proj/src/main.rs"), Kind::File),
                (p("/home/me/Downloads/demo"), Kind::Clone),
            ]
        );
        assert_eq!(
            s.touches[0].when,
            Some(UNIX_EPOCH + Duration::from_secs(1_794_045_660))
        );
    }

    #[test]
    fn codex_log() {
        let log = r#"{"timestamp":"2026-11-07T10:00:00Z","type":"session_meta","payload":{"id":"x","timestamp":"2026-11-07T10:00:00Z","cwd":"/home/me/proj"}}
{"timestamp":"2026-11-07T10:01:00Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"git clone https://github.com/a/lib /tmp/lib\"],\"workdir\":\"/home/me/proj\"}","call_id":"c1"}}
{"timestamp":"2026-11-07T10:02:00Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"c2","input":"*** Begin Patch\n*** Add File: ../scratch/test.py\n+print(1)\n*** Update File: src/a.rs\n*** End Patch"}}
{"timestamp":"2026-11-07T10:03:00Z","type":"response_item","payload":{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"curl -o /home/me/Desktop/x.json https://api.x.io/x\",\"workdir\":\"/home/me/proj\"}","call_id":"c3"}}
{"timestamp":"2026-11-07T10:04:00Z","type":"response_item","payload":{"type":"local_shell_call","call_id":"c4","status":"completed","action":{"type":"exec","command":["wget","https://x.io/f.bin"],"working_directory":"/srv"}}}
"#;
        let s = parse_codex(Cursor::new(log), None);
        assert_eq!(s.cwd, Some(p("/home/me/proj")));
        let paths: Vec<_> = s.touches.iter().map(|t| (t.path.clone(), t.kind)).collect();
        assert_eq!(
            paths,
            vec![
                (p("/tmp/lib"), Kind::Clone),
                (p("/home/me/scratch/test.py"), Kind::File),
                (p("/home/me/proj/src/a.rs"), Kind::Edited),
                (p("/home/me/Desktop/x.json"), Kind::Download),
                (p("/srv/f.bin"), Kind::Download),
            ]
        );
    }

    #[test]
    fn stray_rules() {
        let cfg = Config {
            project_roots: vec![p("/home/me/code")],
            ignored: vec![p("/tmp/hidden")],
            ..Config::default()
        };
        let home = Some(Path::new("/home/me"));
        let cwd = Some(Path::new("/home/me/proj"));
        assert!(is_stray(Path::new("/tmp/lib"), cwd, &cfg, home));
        assert!(!is_stray(
            Path::new("/home/me/proj/src/a.rs"),
            cwd,
            &cfg,
            home
        ));
        assert!(!is_stray(Path::new("/home/me/code/other"), cwd, &cfg, home));
        assert!(!is_stray(Path::new("/tmp/hidden"), cwd, &cfg, home));
        assert!(!is_stray(
            Path::new("/home/me/.claude/plans/p.md"),
            cwd,
            &cfg,
            home
        ));
    }

    #[test]
    fn collect_finds_existing_strays_only() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let proj = home.join("proj");
        let downloads = home.join("Downloads");
        let stray = downloads.join("notes.md");
        let edited = downloads.join("settings.ini");
        let rewritten = downloads.join("old.txt");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(&stray, "hello").unwrap();
        std::fs::write(&edited, "a=1").unwrap();
        std::fs::write(&rewritten, "x").unwrap();
        let logs = home.join(".claude").join("projects").join("-proj");
        std::fs::create_dir_all(&logs).unwrap();
        let line = |tool: &str, path: &Path, ts: &str| {
            serde_json::json!({
                "type": "assistant",
                "cwd": proj,
                "timestamp": ts,
                "message": {"content": [{"type": "tool_use", "name": tool, "input": {"file_path": path, "content": ""}}]}
            })
            .to_string()
        };
        let past = "2026-01-01T10:00:00Z";
        let text = [
            line("Write", &stray, past),
            line("Edit", &stray, "2026-01-01T10:05:00Z"),
            line("Write", &home.join("gone.txt"), past),
            line("Write", &proj.join("in-project.txt"), past),
            line("Edit", &edited, past),
            // Logged long after the file was made: the agent overwrote it.
            line("Write", &rewritten, "2099-01-01T00:00:00Z"),
        ]
        .join("\n");
        std::fs::write(logs.join("s1.jsonl"), text).unwrap();
        let mut items = collect(
            &Config::default(),
            home,
            UNIX_EPOCH,
            &AtomicBool::new(false),
        );
        items.sort_by(|a, b| a.path.cmp(&b.path));
        let names: Vec<_> = items.iter().map(Item::name).collect();
        assert_eq!(names, ["notes.md", "old.txt", "settings.ini"]);

        let notes = &items[0];
        assert_eq!(notes.kind, Kind::File);
        assert_eq!(notes.size_bytes, 5);
        assert_eq!(notes.agent, Agent::ClaudeCode);
        assert_eq!(notes.project, proj);
        assert!(!notes.needs_care());

        assert_eq!(items[2].kind, Kind::Edited);
        assert!(items[2].existed_before);
        if std::fs::metadata(&rewritten).unwrap().created().is_ok() {
            assert!(items[1].existed_before);
        }
    }

    #[test]
    fn iso_dates() {
        assert_eq!(
            parse_iso8601("1970-01-02T00:00:01Z"),
            Some(UNIX_EPOCH + Duration::from_secs(86_401))
        );
        assert_eq!(
            parse_iso8601("2026-11-07T17:23:43.986Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_794_072_223))
        );
        assert!(parse_iso8601("nope").is_none());
    }
}
