//! Checks GitHub for a newer release and replaces the running .exe with it.

use serde::Deserialize;
use std::io::{Read, Write};
use std::path::PathBuf;

pub const REPO: &str = "joogiebear/agent-cleanup";

/// Suffix of the release asset this app updates itself from.
const ASSET_SUFFIX: &str = "-windows-x64.exe";

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub download_url: String,
    pub page_url: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `"v1.2.3"` or `"1.2.3"` to `(1, 2, 3)`.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let mut next = || {
        parts
            .next()?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    };
    Some((next()?, next()?, next()?))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// The update offered by `release`, if it's newer than `current` and has a Windows .exe.
pub fn pick_update(release: &GhRelease, current: &str) -> Option<Update> {
    if release.draft || release.prerelease || !is_newer(&release.tag_name, current) {
        return None;
    }
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(ASSET_SUFFIX))?;
    Some(Update {
        version: release.tag_name.trim_start_matches('v').to_string(),
        download_url: asset.browser_download_url.clone(),
        page_url: release.html_url.clone(),
        size: asset.size,
    })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .user_agent(concat!("agent-cleanup/", env!("CARGO_PKG_VERSION")))
        // Trust the Windows certificate store, so it works behind company
        // proxies that inspect HTTPS.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

/// Asks GitHub for the latest release. `Ok(None)` means you're up to date.
pub fn check() -> anyhow::Result<Option<Update>> {
    let release: GhRelease = agent()
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;
    Ok(pick_update(&release, current_version()))
}

/// Downloads the update and swaps it in for the running .exe. The new version
/// runs after [`relaunch`]. `progress` gets (bytes so far, total if known).
pub fn install(update: &Update, progress: impl Fn(u64, Option<u64>)) -> anyhow::Result<()> {
    if !cfg!(windows) {
        anyhow::bail!("automatic updates are only available on Windows");
    }
    let exe = std::env::current_exe()?;
    // Download next to the exe so the final swap is a same-drive rename.
    let tmp: PathBuf = exe.with_file_name("agent-cleanup.update.exe");
    let result = download(update, &tmp, progress).and_then(|()| {
        self_replace::self_replace(&tmp)?;
        Ok(())
    });
    let _ = std::fs::remove_file(&tmp);
    result?;
    crate::actions::log_action(&format!("updated to v{}", update.version), &exe);
    Ok(())
}

/// Downloads the update's .exe to `dest` and checks it looks complete and valid.
pub fn download(
    update: &Update,
    dest: &std::path::Path,
    progress: impl Fn(u64, Option<u64>),
) -> anyhow::Result<()> {
    let mut resp = agent().get(&update.download_url).call()?;
    let total = resp
        .body()
        .content_length()
        .or(Some(update.size).filter(|s| *s > 0));
    let mut reader = resp
        .body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .reader();
    let mut file = std::fs::File::create(dest)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut first = Vec::new();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if first.len() < 2 {
            first.extend_from_slice(&buf[..n.min(2 - first.len())]);
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()?;
    verify_download(&first, done, total)
}

/// Sanity checks before replacing ourselves: complete, and actually a Windows program.
fn verify_download(first_bytes: &[u8], got: u64, expected: Option<u64>) -> anyhow::Result<()> {
    if let Some(total) = expected
        && got != total
    {
        anyhow::bail!("download was incomplete ({got} of {total} bytes)");
    }
    if got < 1024 * 1024 || !first_bytes.starts_with(b"MZ") {
        anyhow::bail!("the downloaded file isn't a valid Windows program");
    }
    Ok(())
}

/// Starts the (updated) exe again and exits this one.
pub fn relaunch() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            html_url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|n| GhAsset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example.com/{n}"),
                    size: 5_000_000,
                })
                .collect(),
        }
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("nope"), None);
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.9", "0.2.0"));
    }

    #[test]
    fn picks_the_windows_exe_from_newer_releases_only() {
        let r = release(
            "v0.3.0",
            &["notes.txt", "agent-cleanup-v0.3.0-windows-x64.exe"],
        );
        let u = pick_update(&r, "0.2.0").unwrap();
        assert_eq!(u.version, "0.3.0");
        assert!(
            u.download_url
                .ends_with("agent-cleanup-v0.3.0-windows-x64.exe")
        );
        assert!(pick_update(&r, "0.3.0").is_none());
        assert!(pick_update(&release("v0.4.0", &["source.zip"]), "0.2.0").is_none());
        let mut pre = release("v0.5.0", &["agent-cleanup-v0.5.0-windows-x64.exe"]);
        pre.prerelease = true;
        assert!(pick_update(&pre, "0.2.0").is_none());
    }

    #[test]
    fn parses_github_json() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://x","draft":false,"prerelease":false,
            "assets":[{"name":"agent-cleanup-v0.2.0-windows-x64.exe","browser_download_url":"https://d","size":7}],
            "body":"ignored"}"#;
        let r: GhRelease = serde_json::from_str(json).unwrap();
        assert_eq!(pick_update(&r, "0.1.0").unwrap().size, 7);
    }

    #[test]
    fn rejects_bad_downloads() {
        assert!(verify_download(b"MZ", 5_000_000, Some(5_000_000)).is_ok());
        assert!(verify_download(b"MZ", 4_000_000, Some(5_000_000)).is_err());
        assert!(verify_download(b"<h", 5_000_000, None).is_err());
        assert!(verify_download(b"MZ", 100, None).is_err());
    }
}

#[cfg(test)]
mod live {
    /// Talks to the real GitHub API. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn check_against_github() {
        let latest = super::check().expect("GitHub API reachable");
        println!("latest update offered: {latest:?}");

        // Pretend to be an old version so the current release counts as an update.
        let release: super::GhRelease = super::agent()
            .get(&format!(
                "https://api.github.com/repos/{}/releases/latest",
                super::REPO
            ))
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        let update = super::pick_update(&release, "0.0.1").expect("release has a Windows exe");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.exe");
        super::download(&update, &dest, |_, _| {}).expect("download works");
        println!(
            "downloaded {} bytes",
            std::fs::metadata(&dest).unwrap().len()
        );
    }
}
