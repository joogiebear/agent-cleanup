//! Ready-to-paste requests for an AI coding agent (Claude Code, Codex...),
//! so people can hand a group of items to their agent instead of working
//! through git themselves.
//!
//! A prompt only ever holds what the page already shows: paths, the app's
//! plain-language notes about them, and package names and versions. Never
//! remote URLs (they can carry tokens) or anything else read from disk.

use crate::library::Filter;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// One item to list in a prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// Short note shown after the path, like "2 edited files not saved".
    pub note: Option<String>,
}

impl Entry {
    pub fn new(path: impl Into<PathBuf>, note: Option<String>) -> Self {
        Self {
            path: path.into(),
            note: note.filter(|n| !n.is_empty()),
        }
    }
}

const RULES: &str = "Rules:
- Don't delete anything. I'll remove what's left over myself with Agent Cleanup, which uses the Recycle Bin.
- Don't force-push, rewrite git history or change git settings.
- Ask me before creating any new GitHub repository.
- When you're done, give me a short, plain-language summary: what you did, and anything I still need to decide.";

const UPDATE_RULES: &str = "Rules:
- Don't force-push, rewrite git history or change git settings.
- When you're done, give me a short, plain-language summary: what you changed, and anything I still need to decide.";

fn build(intro: &str, entries: &[Entry], task: &str) -> String {
    build_with(intro, entries, task, RULES)
}

fn build_with(intro: &str, entries: &[Entry], task: &str, rules: &str) -> String {
    let mut s = String::new();
    s.push_str(intro.trim());
    s.push_str("\n\n");
    for e in entries {
        let _ = write!(s, "- {}", e.path.display());
        if let Some(n) = &e.note {
            let _ = write!(s, " ({n})");
        }
        s.push('\n');
    }
    s.push('\n');
    s.push_str(task.trim());
    s.push_str("\n\n");
    s.push_str(rules);
    s.push('\n');
    s
}

/// For a Leftovers view. `None` when there's nothing useful to ask.
pub fn leftovers(filter: Filter, entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let (intro, task) = match filter {
        Filter::UnsavedWork => (
            "These folders on my PC are copies of git repositories that have work not saved anywhere else (edits that aren't committed, commits that aren't pushed, stashes, no online copy at all, or files git ignores on purpose like .env files and local databases). Many were probably made by an AI coding agent:",
            "For each one, look at what's unsaved and tell me in plain words what it is. If it's real work worth keeping, commit it and push it to its existing remote. If it has no remote, ask me whether to create one. Never commit or upload files that git ignores (like .env files, which can hold passwords, or databases): tell me what each one is and whether it looks worth keeping, so I can copy it somewhere safe. If it's just a throwaway experiment, say so, and leave it as it is so I can decide.",
        ),
        Filter::Unreadable => (
            "Git can't read these folders on my PC, so I can't tell whether they hold unsaved work. The note after each one is the error git gave:",
            "For each one, find out why git can't read it (for example a half-finished clone, a broken worktree, or not really a repository). Tell me whether it contains anything worth keeping. If it does, tell me where it is so I can save it. Don't try to repair or re-clone anything without asking me first.",
        ),
        Filter::SafeToRemove => (
            "These folders on my PC look like leftover copies of git repositories. Agent Cleanup says everything in them is already pushed and they haven't been used in a while:",
            "Double-check each one: no uncommitted changes, no unpushed commits on any branch, no stashes, and no files git ignores that exist only here, like .env files or local databases (downloaded packages and build output don't count). Tell me which are safe to delete and flag any that aren't, with the reason.",
        ),
        Filter::StrayActive => (
            "These git repositories on my PC are outside my main project folders but were used recently:",
            "For each one, tell me what it seems to be and whether it looks like real work I'm still doing or a temporary copy an agent made. Point out anything that isn't saved anywhere else.",
        ),
        Filter::Strays | Filter::All => (
            "These git repositories on my PC are outside my main project folders, and many were probably left behind by AI coding agents:",
            "For each one, tell me what it is, whether anything in it isn't saved anywhere else (including files git ignores, like .env files or local databases), and whether it looks safe to delete. If something has real unsaved work, commit and push it to its existing remote, but never commit or upload ignored files: they can hold passwords.",
        ),
    };
    Some(build(intro, entries, task))
}

/// For one Agent trail group: things an agent made while working on `project`.
pub fn trail_group(project: &Path, entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let intro = if project.as_os_str().is_empty() {
        "An AI coding agent created these files and folders on my PC outside the project it was working on:".to_string()
    } else {
        format!(
            "While working on the project in {}, an AI coding agent created these files and folders outside that project:",
            project.display()
        )
    };
    let task = "For each one, tell me what it was for and whether I still need it. If something belongs in the project, move it into the project and say where you put it. If a folder is a git repository with unsaved work, tell me what the work is. List the rest as not needed so I can clean them up.";
    Some(build(&intro, entries, task))
}

/// For one Downloads group. `project` is the folder they seem to be for.
pub fn downloads_group(project: Option<&Path>, entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let (intro, task) = match project {
        Some(p) => (
            format!(
                "These files in my Downloads folder arrived while I (or an AI agent) was working on the project in {}:",
                p.display()
            ),
            "Look at each one and tell me whether the project needs it. Move the ones it needs into a sensible place in the project (copy, don't delete the original) and tell me where. List the rest as not needed.".to_string(),
        ),
        None => (
            "These files are in my Downloads folder and I'm not sure what they're for:".to_string(),
            "Look at each one and tell me in plain words what it is and whether it's likely I still need it. Don't open or run any programs or installers.".to_string(),
        ),
    };
    Some(build(&intro, entries, &task))
}

/// For one Duplicates group: the same repository in several folders.
pub fn dupes_group(keep: &Path, entries: &[Entry]) -> Option<String> {
    if entries.len() < 2 {
        return None;
    }
    let intro = "These folders on my PC are all copies of the same git repository:";
    let task = format!(
        "I want to keep only {}. Compare the copies and tell me if any of the others has work the kept copy doesn't (uncommitted edits, commits on any branch that aren't pushed, stashes). If so, push that work to the remote (or bring it into the kept copy) and tell me what you moved. Then tell me which copies have nothing left that I'd lose.",
        keep.display()
    );
    Some(build(intro, entries, &task))
}

/// For the Agent data page: chat history whose project folder is gone.
pub fn agentdata(entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let intro = "Claude Code and Codex still keep chat history for these project folders, but the folders no longer exist on my PC:";
    let task = "Check whether any of these projects was moved or renamed rather than deleted. Search my user folder for a project with the same name and tell me where it is now. Don't change or delete any chat history.";
    Some(build(intro, entries, task))
}

/// One outdated package for [`updates`].
pub struct Package<'a> {
    pub name: &'a str,
    pub current: &'a str,
    pub target: &'a str,
}

/// For one Updates card: outdated packages in a project.
pub fn updates(repo: &Path, ecosystem: &str, packages: &[Package]) -> Option<String> {
    if packages.is_empty() {
        return None;
    }
    let entries: Vec<Entry> = packages
        .iter()
        .map(|p| Entry::new(p.name, Some(format!("{} -> {}", p.current, p.target))))
        .collect();
    let intro = format!(
        "These {ecosystem} packages are out of date in the project at {}:",
        repo.display()
    );
    let task = "Update them, then build the project and run its tests. Fix anything the updates break. If a major version needs bigger code changes, tell me what's involved before doing it. Commit the result with a clear message, but don't push until I say so.";
    Some(build_with(&intro, &entries, task, UPDATE_RULES))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_paths_notes_and_rules() {
        let e = [
            Entry::new(
                "/tmp/a",
                Some("2 edited files not saved (committed)".into()),
            ),
            Entry::new("/tmp/b", Some(String::new())),
        ];
        let p = leftovers(Filter::UnsavedWork, &e).unwrap();
        assert!(p.contains("- /tmp/a (2 edited files not saved (committed))\n"));
        assert!(p.contains("- /tmp/b\n"), "empty notes are dropped");
        assert!(p.contains("commit it and push"));
        assert!(p.contains("Don't delete anything"));
        assert!(leftovers(Filter::UnsavedWork, &[]).is_none());
    }

    #[test]
    fn every_page_has_a_prompt() {
        let e = [Entry::new("/x/one", None), Entry::new("/x/two", None)];
        assert!(
            trail_group(Path::new("/code/app"), &e)
                .unwrap()
                .contains("/code/app")
        );
        assert!(trail_group(Path::new(""), &e).is_some());
        assert!(
            downloads_group(Some(Path::new("/code/site")), &e)
                .unwrap()
                .contains("/code/site")
        );
        assert!(
            downloads_group(None, &e)
                .unwrap()
                .contains("Don't open or run")
        );
        assert!(
            dupes_group(Path::new("/x/one"), &e)
                .unwrap()
                .contains("keep only /x/one")
        );
        assert!(dupes_group(Path::new("/x/one"), &e[..1]).is_none());
        assert!(agentdata(&e).unwrap().contains("moved or renamed"));
        let pk = [Package {
            name: "serde",
            current: "1.0.1",
            target: "1.0.9",
        }];
        let u = updates(Path::new("/code/app"), "Cargo", &pk).unwrap();
        assert!(u.contains("- serde (1.0.1 -> 1.0.9)"));
        assert!(u.contains("don't push"));
    }
}
