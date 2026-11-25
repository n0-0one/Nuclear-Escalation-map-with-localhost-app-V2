//! Updates of the app itself.
//!
//! Asks GitHub for this project's latest release and checks every download
//! against the SHA-256 listed in the release's SHA256SUMS.txt.
//!
//! * An installed copy starts the new installer and closes so it can run.
//! * A portable copy unpacks the new ZIP over its own folder and restarts.
//!   The running program is renamed to `.old` first (Windows allows that),
//!   the old `data` folder to `data.old`; both are removed on the next start.
//!   The `user-data` folder with settings and recordings is never touched.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

const REPO: &str = "n0-0one/Nuclear-Escalation-map-with-localhost-app-V2";
const INSTALLER: &str = "Nuclear-Thunder-Live-Setup.exe";
const PORTABLE: &str = "Nuclear-Thunder-Live-Portable.zip";
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
    let mut parts = text
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// The NSIS installer leaves an uninstaller next to the program; a copy
/// unpacked from the portable ZIP has none.
pub fn is_portable() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return true;
    };
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
            ureq::Error::Status(403 | 429, _) => {
                "GitHub is limiting requests right now. Try again in a few minutes.".to_string()
            }
            ureq::Error::Status(code, _) => format!("GitHub answered with error {code}."),
            ureq::Error::Transport(_) => {
                "Could not reach GitHub. Check your internet connection.".to_string()
            }
        })?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() as u64 > limit {
        return Err("A downloaded file was larger than expected.".into());
    }
    Ok(body)
}

struct Release {
    tag: String,
    page: String,
    installer: Option<String>,
    portable: Option<String>,
    sums: Option<String>,
}

fn latest(agent: &ureq::Agent) -> Result<Release, String> {
    let body = get(
        agent,
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        2 * 1024 * 1024,
    )?;
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| "GitHub's answer could not be read.".to_string())?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("The latest release has no version.")?
        .to_string();
    let page = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    // Only accept downloads from this project's own release files.
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    let asset = |name: &str| {
        value.get("assets")?.as_array()?.iter().find_map(|asset| {
            let url = asset.get("browser_download_url")?.as_str()?;
            (asset.get("name")?.as_str()? == name && url.starts_with(&prefix))
                .then(|| url.to_string())
        })
    };
    Ok(Release {
        installer: asset(INSTALLER),
        portable: asset(PORTABLE),
        sums: asset(SUMS),
        tag,
        page,
    })
}

pub fn check() -> Result<Check, String> {
    let agent = crate::update::agent()?;
    let release = latest(&agent)?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    let newer = match (version(&release.tag), version(&current)) {
        (Some(latest), Some(mine)) => latest > mine,
        _ => false,
    };
    Ok(Check {
        latest: release.tag.trim_start_matches('v').to_string(),
        current,
        newer,
        portable: is_portable(),
        page: release.page,
    })
}

/// Opens a folder or a github.com page with the system's default handler.
pub fn open_path(target: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(target).spawn();
    result
        .map(|_| ())
        .map_err(|e| format!("Could not open it ({e})."))
}

/// Downloads one release file and checks it against SHA256SUMS.txt.
fn verified(agent: &ureq::Agent, sums: &str, name: &str, url: &str) -> Result<Vec<u8>, String> {
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let file = parts.next()?.trim_start_matches('*');
            (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| hash.to_ascii_lowercase())
        })
        .ok_or("The release does not list a checksum for this download.")?;
    let bytes = get(agent, url, MAX_INSTALLER)?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err("The download did not match its checksum, so it was not used.".into());
    }
    Ok(bytes)
}

/// Removes what the last portable update left behind.
pub fn cleanup_previous() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else { return };
    let _ = std::fs::remove_dir_all(dir.join("data.old"));
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".exe.old") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Unpacks the new portable ZIP over the folder this copy runs from.
fn update_portable(zip_bytes: Vec<u8>) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("Unknown program folder.")?.to_path_buf();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|_| "The downloaded ZIP could not be opened.".to_string())?;

    // Everything in the ZIP sits under one top folder; files go into `dir`
    // without it. Nothing may land outside `dir` or inside `user-data`.
    let mut files: Vec<(usize, PathBuf)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            return Err("The ZIP contains an unsafe file name.".into());
        };
        let relative: PathBuf = path.components().skip(1).collect();
        if relative.as_os_str().is_empty() || relative.starts_with("user-data") {
            continue;
        }
        files.push((index, relative));
    }
    let new_exe = files
        .iter()
        .find(|(_, path)| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                && path.components().count() == 1
        })
        .map(|(_, path)| dir.join(path))
        .ok_or("The ZIP does not contain the program.")?;

    // Make way: the running program and the data folder are renamed, not deleted.
    let old_exe = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old_exe);
    std::fs::rename(&exe, &old_exe).map_err(|e| format!("Could not replace the program ({e})."))?;
    let data = dir.join("data");
    let data_old = dir.join("data.old");
    let _ = std::fs::remove_dir_all(&data_old);
    if data.exists() {
        if let Err(error) = std::fs::rename(&data, &data_old) {
            let _ = std::fs::rename(&old_exe, &exe);
            return Err(format!("Could not replace the map data ({error})."));
        }
    }
    let result = (|| -> Result<(), String> {
        for (index, relative) in &files {
            let mut entry = archive.by_index(*index).map_err(|e| e.to_string())?;
            let target = dir.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Put the working copy back.
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::rename(&data_old, &data);
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::rename(&old_exe, &exe);
        return Err(format!(
            "The update could not be unpacked ({error}); nothing was changed."
        ));
    }
    Ok(new_exe)
}

/// Updates this copy. Returns a message for the user and whether the app
/// should now close.
pub fn install() -> Result<(String, bool), String> {
    let agent = crate::update::agent()?;
    let release = latest(&agent)?;
    let Some(sums_url) = release.sums.clone() else {
        return Err("This release is not complete yet. Try again in a few minutes.".into());
    };
    let sums = String::from_utf8_lossy(&get(&agent, &sums_url, 64 * 1024)?).to_string();
    if is_portable() {
        let Some(url) = release.portable else {
            if release.page.starts_with("https://github.com/") {
                open_path(&release.page)?;
            }
            return Ok((
                "This release has no portable ZIP; its page is open in your browser.".into(),
                false,
            ));
        };
        let bytes = verified(&agent, &sums, PORTABLE, &url)?;
        let new_exe = update_portable(bytes)?;
        std::process::Command::new(&new_exe).spawn().map_err(|e| {
            format!("Updated, but the new version could not be started ({e}). Start it yourself.")
        })?;
        return Ok(("Updated. The new version is starting.".into(), true));
    }
    let Some(installer_url) = release.installer else {
        return Err("This release has no installer yet. Try again in a few minutes.".into());
    };
    let bytes = verified(&agent, &sums, INSTALLER, &installer_url)?;
    let dir = std::env::temp_dir().join("nuclear-thunder-live-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join(format!(
        "Nuclear-Thunder-Live-Setup-{}.exe",
        release.tag.trim_start_matches('v')
    ));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("The installer could not be started ({e})."))?;
    Ok((
        "The installer is starting. The app will close so it can be updated.".into(),
        true,
    ))
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
