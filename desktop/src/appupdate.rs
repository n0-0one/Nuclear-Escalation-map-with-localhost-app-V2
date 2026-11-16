//! Updates of the app itself.
//!
//! Asks GitHub for this project's latest release. An installed copy
//! downloads that release's installer, checks it against the SHA-256 listed
//! in the release's SHA256SUMS.txt, starts it and closes itself so the
//! installer can replace the files. A portable copy opens the release page
//! instead, because it was not installed and cannot install itself.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

const REPO: &str = "n0-0one/Nuclear-Escalation-map-with-localhost-app-V2";
const INSTALLER: &str = "Nuclear-Thunder-Live-Setup.exe";
const SUMS: &str = "SHA256SUMS.txt";
const MAX_INSTALLER: u64 = 300 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub current: String,
    pub latest: String,
    pub newer: bool,
    pub portable: bool,
    pub page: String,
}

fn version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

/// The NSIS installer leaves an uninstaller next to the program; a copy
/// unpacked from the portable ZIP has none.
pub fn is_portable() -> bool {
    let Ok(exe) = std::env::current_exe() else { return true };
    let Some(dir) = exe.parent() else { return true };
    !dir.join("uninstall.exe").exists()
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "No release has been published yet.".to_string(),
            ureq::Error::Status(403 | 429, _) => "GitHub is limiting requests right now. Try again in a few minutes.".to_string(),
            ureq::Error::Status(code, _) => format!("GitHub answered with error {code}."),
            ureq::Error::Transport(_) => "Could not reach GitHub. Check your internet connection.".to_string(),
        })?;
    let mut body = Vec::new();
    response.into_reader().take(limit + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
    if body.len() as u64 > limit {
        return Err("A downloaded file was larger than expected.".into());
    }
    Ok(body)
}

struct Release {
    tag: String,
    page: String,
    installer: Option<String>,
    sums: Option<String>,
}

fn latest(agent: &ureq::Agent) -> Result<Release, String> {
    let body = get(agent, &format!("https://api.github.com/repos/{REPO}/releases/latest"), 2 * 1024 * 1024)?;
    let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| "GitHub's answer could not be read.".to_string())?;
    let tag = value.get("tag_name").and_then(|v| v.as_str()).ok_or("The latest release has no version.")?.to_string();
    let page = value.get("html_url").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    // Only accept downloads from this project's own release files.
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    let asset = |name: &str| {
        value.get("assets")?.as_array()?.iter().find_map(|asset| {
            let url = asset.get("browser_download_url")?.as_str()?;
            (asset.get("name")?.as_str()? == name && url.starts_with(&prefix)).then(|| url.to_string())
        })
    };
    Ok(Release { installer: asset(INSTALLER), sums: asset(SUMS), tag, page })
}

pub fn check() -> Result<Check, String> {
    let agent = crate::update::agent()?;
    let release = latest(&agent)?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    let newer = match (version(&release.tag), version(&current)) {
        (Some(latest), Some(mine)) => latest > mine,
        _ => false,
    };
    Ok(Check { latest: release.tag.trim_start_matches('v').to_string(), current, newer, portable: is_portable(), page: release.page })
}

fn open_in_browser(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/") {
        return Err("Unexpected address for the release page.".into());
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    result.map(|_| ()).map_err(|e| format!("Could not open the browser ({e})."))
}

/// Downloads, verifies and starts the installer. Returns a message for the
/// user and whether the app should now close.
pub fn install() -> Result<(String, bool), String> {
    let agent = crate::update::agent()?;
    let release = latest(&agent)?;
    if is_portable() {
        open_in_browser(&release.page)?;
        return Ok(("The download page is open in your browser. Download the new portable ZIP and unpack it over this one.".into(), false));
    }
    let (Some(installer_url), Some(sums_url)) = (release.installer, release.sums) else {
        return Err("This release has no installer yet. Try again in a few minutes.".into());
    };
    let sums = String::from_utf8_lossy(&get(&agent, &sums_url, 64 * 1024)?).to_string();
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == INSTALLER && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
        })
        .ok_or("The release does not list a checksum for the installer.")?;
    let bytes = get(&agent, &installer_url, MAX_INSTALLER)?;
    let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err("The downloaded installer did not match its checksum, so it was not started.".into());
    }
    let dir = std::env::temp_dir().join("nuclear-thunder-live-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join(format!("Nuclear-Thunder-Live-Setup-{}.exe", release.tag.trim_start_matches('v')));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    std::process::Command::new(&path).spawn().map_err(|e| format!("The installer could not be started ({e})."))?;
    Ok(("The installer is starting. The app will close so it can be updated.".into(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(version("v2.2.0") > version("2.1.0"));
        assert!(version("v2.10.0") > version("v2.9.3"));
        assert_eq!(version("2.1"), Some((2, 1, 0)));
        assert_eq!(version("beta"), None);
    }
}
