//! Spotting files that are probably the only copy of something private:
//! passwords and settings (`.env`), keys, password stores and databases.
//!
//! git never uploads these when they're ignored, and an agent-made folder
//! that isn't a repo has no online copy at all, so removing the folder would
//! lose them. Everything here only reads names on disk, never file contents.

use std::path::Path;
use walkdir::WalkDir;

/// How many private files to keep by name.
pub const MAX_LISTED: usize = 20;
/// Stop looking after this many entries, so a huge folder can't stall a scan.
const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 8;

/// File endings for databases, keys, certificates and password stores.
const PRIVATE_EXTS: &[&str] = &[
    "db", "sqlite", "sqlite3", "db3", "mdb", "accdb", "pem", "key", "p12", "pfx", "ppk", "kdbx",
    "keystore", "jks", "tfstate", "ovpn", "gpg", "asc",
];

/// Exact names of files that hold passwords or access tokens.
const PRIVATE_NAMES: &[&str] = &[
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "credentials",
    "credentials.json",
    "secrets.json",
    "secrets.yaml",
    "secrets.yml",
    "secrets.toml",
    "service-account.json",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".htpasswd",
    ".git-credentials",
];

/// `.env` templates meant to be shared, holding no real values.
const ENV_TEMPLATES: &[&str] = &["example", "sample", "template", "dist", "defaults"];

/// True if a file with this name is likely private and not recreated by
/// anything: `.env` files, keys, password stores, databases.
pub fn looks_private(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == ".env" || lower.ends_with(".env") && lower.len() > 4 && !lower.starts_with('.') {
        return true;
    }
    if let Some(rest) = lower.strip_prefix(".env.") {
        return !ENV_TEMPLATES
            .iter()
            .any(|t| rest.split('.').any(|p| p == *t));
    }
    if PRIVATE_NAMES.contains(&lower.as_str()) {
        return true;
    }
    // Database side files (`app.db-wal`, `app.sqlite-journal`).
    let base = lower
        .strip_suffix("-wal")
        .or_else(|| lower.strip_suffix("-shm"))
        .or_else(|| lower.strip_suffix("-journal"))
        .unwrap_or(&lower);
    // `Thumbs.db` is a Windows picture cache, not a database.
    if base == "thumbs.db" {
        return false;
    }
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => PRIVATE_EXTS.contains(&ext),
        _ => false,
    }
}

/// Private-looking files under `dir`, relative to it, at most
/// [`MAX_LISTED`] of them, plus how many there are in all. Skips folders a
/// command recreates (like `node_modules`) and git's own `.git` folder.
pub fn find(dir: &Path) -> (Vec<String>, usize) {
    let mut listed = Vec::new();
    let mut total = 0;
    let walk = WalkDir::new(dir)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_dir()
                || !is_skipped_dir(&e.file_name().to_string_lossy())
        })
        .take(MAX_ENTRIES)
        .filter_map(Result::ok);
    for e in walk {
        if !e.file_type().is_file() || !looks_private(&e.file_name().to_string_lossy()) {
            continue;
        }
        total += 1;
        if listed.len() < MAX_LISTED
            && let Ok(rel) = e.path().strip_prefix(dir)
        {
            listed.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    (listed, total)
}

/// Folders [`find`] doesn't walk into: git's data and dependency folders
/// that hold thousands of files from the internet.
fn is_skipped_dir(name: &str) -> bool {
    [
        ".git",
        "node_modules",
        "bower_components",
        ".venv",
        "venv",
        "__pycache__",
        "target",
    ]
    .iter()
    .any(|d| d.eq_ignore_ascii_case(name))
}

/// "a, b, c and 4 more" for up to `n` names.
pub fn preview(listed: &[String], total: usize, n: usize) -> String {
    let mut s = listed
        .iter()
        .take(n)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let shown = listed.len().min(n);
    if total > shown {
        s.push_str(&format!(" and {} more", total - shown));
    }
    s
}

/// Folders tools recreate, or that only hold editor settings, whatever is
/// around them. Their names are specific enough to trust.
const DISPOSABLE_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".angular",
    ".expo",
    ".docusaurus",
    ".vercel",
    ".netlify",
    ".cache",
    ".nyc_output",
    ".sass-cache",
    ".dart_tool",
    ".gradle",
    ".terraform",
    "Pods",
    "DerivedData",
    "storybook-static",
    ".idea",
    ".vs",
    ".vscode",
];

/// Common words for build output. People also use them for their own
/// folders, so they only count with a build file right beside them.
const BUILD_DIRS: &[(&str, &[&str])] = &[
    (
        "dist",
        &[
            "package.json",
            "setup.py",
            "pyproject.toml",
            "build.gradle",
            "build.gradle.kts",
            "pom.xml",
        ],
    ),
    (
        "build",
        &[
            "package.json",
            "setup.py",
            "pyproject.toml",
            "build.gradle",
            "build.gradle.kts",
            "CMakeLists.txt",
            "Makefile",
        ],
    ),
    (
        "out",
        &[
            "package.json",
            "CMakeLists.txt",
            "build.gradle",
            "build.gradle.kts",
        ],
    ),
    ("coverage", &["package.json", "pyproject.toml", "setup.py"]),
    ("target", &["Cargo.toml", "pom.xml"]),
    ("vendor", &["composer.json", "go.mod", "Gemfile"]),
];

/// `bin`/`obj` count next to a .NET project file.
const DOTNET_DIRS: &[&str] = &["bin", "obj"];
const DOTNET_EXTS: &[&str] = &["csproj", "fsproj", "vbproj", "sln"];

/// File endings for logs, caches, compiled output and OS clutter.
const DISPOSABLE_EXTS: &[&str] = &[
    "log",
    "pyc",
    "pyo",
    "class",
    "o",
    "obj",
    "pdb",
    "ilk",
    "tsbuildinfo",
    "swp",
    "swo",
    "tmp",
];

const DISPOSABLE_FILES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    ".eslintcache",
    ".stylelintcache",
];

/// True for a path git ignores (relative to `repo`, folders ending in `/`)
/// that's safe to lose: build output, caches, logs, editor settings.
/// Everything else (`.env`, `*.db`, `data/`...) may be the only copy.
/// When unsure, it says no, so the repo gets a closer look.
pub fn is_disposable(repo: &Path, rel: &str) -> bool {
    let rel = rel.trim_end_matches('/');
    let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some(name) = parts.last() else {
        return true;
    };
    if looks_private(name) {
        return false;
    }
    for (i, p) in parts.iter().enumerate() {
        if DISPOSABLE_DIRS.iter().any(|d| d.eq_ignore_ascii_case(p))
            || p.to_ascii_lowercase().ends_with(".egg-info")
        {
            return true;
        }
        let parent = repo.join(parts[..i].join("/"));
        if let Some((_, markers)) = BUILD_DIRS.iter().find(|(d, _)| d.eq_ignore_ascii_case(p))
            && markers.iter().any(|m| parent.join(m).is_file())
        {
            return true;
        }
        if DOTNET_DIRS.iter().any(|d| d.eq_ignore_ascii_case(p)) && has_dotnet_project(&parent) {
            return true;
        }
    }
    // `env` is often a Python virtual env, but only trust it with the marker.
    if name.eq_ignore_ascii_case("env") && repo.join(rel).join("pyvenv.cfg").is_file() {
        return true;
    }
    // An agent's personal settings for this folder.
    if name.eq_ignore_ascii_case("settings.local.json")
        && parts.iter().any(|p| p.eq_ignore_ascii_case(".claude"))
    {
        return true;
    }
    if DISPOSABLE_FILES
        .iter()
        .any(|f| f.eq_ignore_ascii_case(name))
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with('~')
        || lower.starts_with("npm-debug.log")
        || lower.starts_with("yarn-error.log")
    {
        return true;
    }
    match lower.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => DISPOSABLE_EXTS.contains(&ext),
        _ => false,
    }
}

fn has_dotnet_project(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(Result::ok).any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| DOTNET_EXTS.iter().any(|d| x.eq_ignore_ascii_case(d)))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_private_names() {
        for yes in [
            ".env",
            ".env.local",
            ".env.production",
            "prod.env",
            "app.db",
            "data.sqlite3",
            "app.db-wal",
            "server.key",
            "cert.pem",
            "id_ed25519",
            "Passwords.kdbx",
            "terraform.tfstate",
            "secrets.json",
            ".npmrc",
        ] {
            assert!(looks_private(yes), "{yes}");
        }
        for no in [
            ".env.example",
            ".env.sample",
            ".env.local.example",
            "Thumbs.db",
            "main.rs",
            "package.json",
            "README.md",
            ".envrc",
            "db",
            "keys.txt",
        ] {
            assert!(!looks_private(no), "{no}");
        }
    }

    #[test]
    fn finds_private_files_but_not_in_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(d.join("config")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
        std::fs::write(d.join(".env"), "").unwrap();
        std::fs::write(d.join("config/app.db"), "").unwrap();
        std::fs::write(d.join("node_modules/pkg/.env"), "").unwrap();
        std::fs::write(d.join("notes.txt"), "").unwrap();
        let (mut listed, total) = find(d);
        listed.sort();
        assert_eq!(listed, vec![".env", "config/app.db"]);
        assert_eq!(total, 2);
        assert_eq!(preview(&listed, 5, 1), ".env and 4 more");
    }

    #[test]
    fn build_folder_names_need_a_build_file() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        assert!(
            !is_disposable(r, "build/"),
            "no build file: could be anything"
        );
        assert!(!is_disposable(r, "bin/"));
        assert!(!is_disposable(r, "target/"));
        std::fs::write(r.join("package.json"), "{}").unwrap();
        std::fs::write(r.join("Cargo.toml"), "").unwrap();
        std::fs::write(r.join("App.csproj"), "").unwrap();
        assert!(is_disposable(r, "build/"));
        assert!(is_disposable(r, "target/"));
        assert!(is_disposable(r, "bin/"));
        std::fs::create_dir_all(r.join("web")).unwrap();
        assert!(!is_disposable(r, "web/dist/"), "marker must be beside it");
        std::fs::write(r.join("web/package.json"), "{}").unwrap();
        assert!(is_disposable(r, "web/dist/"));
        assert!(!is_disposable(r, "build.db"), "private names always count");
    }
}
