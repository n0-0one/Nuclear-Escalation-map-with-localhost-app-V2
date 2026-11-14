//! Map-data updates.
//!
//! The data pack is rebuilt from the public datamine by this project's
//! scheduled workflow and published as plain files. The app downloads a new
//! pack next to the installed one, checks every file against the SHA-256 in
//! the manifest, makes sure the engine can load it, and only then switches
//! to it. A failed or interrupted update leaves the working pack untouched.
//!
//! The pack is data only (JSON and images). Nothing in it is executed.

use ntl_engine::Pack;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const FEED: &str = "https://raw.githubusercontent.com/n0-0one/Nuclear-Escalation-map-with-localhost-app-V2/data-feed/data/";
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 250 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct FileEntry {
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    #[serde(default)]
    pub game_version: String,
    #[serde(default)]
    pub generated: String,
    files: BTreeMap<String, FileEntry>,
}

pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).ok()?).ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Pack file names are plain relative paths; anything else is rejected.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    let plain = candidate.components().all(|c| matches!(c, Component::Normal(_)));
    let tame = path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'));
    (plain && tame && !path.is_empty() && path.len() < 200).then(|| candidate.to_path_buf())
}

fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(30))
        .redirects(2)
        .user_agent("nuclear-thunder-live")
        .build())
}

fn download(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(404, _) => "No published map data was found yet.".to_string(),
        ureq::Error::Status(code, _) => format!("The update server answered with error {code}."),
        ureq::Error::Transport(_) => "Could not reach the update server. Check your internet connection.".to_string(),
    })?;
    let mut body = Vec::new();
    response.into_reader().take(limit + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
    if body.len() as u64 > limit {
        return Err("A downloaded file was larger than allowed.".into());
    }
    Ok(body)
}

pub enum Outcome {
    UpToDate,
    Downloaded { game_version: String },
    NeedsNewerApp,
}

/// `current`: the pack in use. `root`: writable folder that holds `pack/`.
pub fn update(current: &Path, root: &Path) -> Result<Outcome, String> {
    let agent = agent()?;
    let manifest_bytes = download(&agent, &format!("{FEED}manifest.json"), 1024 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|_| "The published manifest could not be read.".to_string())?;
    if manifest.schema != ntl_engine::pack::SCHEMA {
        return Ok(Outcome::NeedsNewerApp);
    }
    let have = read_manifest(current);
    if have.as_ref().is_some_and(|m| m.generated >= manifest.generated) {
        return Ok(Outcome::UpToDate);
    }
    if manifest.files.values().map(|f| f.bytes).sum::<u64>() > MAX_TOTAL || manifest.files.len() > 2000 {
        return Err("The published data pack is larger than allowed.".into());
    }

    let staging = root.join("pack.next");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        for (name, entry) in &manifest.files {
            let relative = safe_relative(name).ok_or("The published data pack lists an invalid file name.")?;
            if entry.bytes > MAX_FILE {
                return Err("A published file is larger than allowed.".into());
            }
            // Reuse files that did not change; images rarely do.
            let reused = have
                .as_ref()
                .and_then(|m| m.files.get(name))
                .filter(|old| old.sha256 == entry.sha256)
                .and_then(|_| std::fs::read(current.join(&relative)).ok())
                .filter(|bytes| sha256_hex(bytes) == entry.sha256);
            let bytes = match reused {
                Some(bytes) => bytes,
                None => {
                    let bytes = download(&agent, &format!("{FEED}{name}"), MAX_FILE)?;
                    if sha256_hex(&bytes) != entry.sha256 {
                        return Err(format!("{name} did not match its checksum; the update was discarded."));
                    }
                    bytes
                }
            };
            let target = staging.join(&relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        }
        std::fs::write(staging.join("manifest.json"), &manifest_bytes).map_err(|e| e.to_string())?;
        Pack::load(&staging).map_err(|e| format!("The new data did not pass its checks ({e}); keeping the current data."))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let (live, old) = (root.join("pack"), root.join("pack.old"));
    let _ = std::fs::remove_dir_all(&old);
    if live.exists() {
        std::fs::rename(&live, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, &live).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(Outcome::Downloaded { game_version: manifest.game_version })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_that_leave_the_pack() {
        assert!(safe_relative("levels/archipelago.json").is_some());
        assert!(safe_relative("../settings.json").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("levels\\x.json").is_none());
        assert!(safe_relative("").is_none());
    }

    #[test]
    fn checksum_is_lowercase_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
