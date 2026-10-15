//! Feed updater: explicit consent, bounded downloads and fail-closed SHA-256.
use openmgmt_sync_client::updates::{ReleaseFile, ReleaseManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State};

// App version is 2026.10.1 in tauri.conf.json, with monotonic code 20261001.
// Release engineering must increase this code alongside future version changes
// and put the same code in release.json. Do not derive ordering from tag/semver strings.
pub const UPDATE_VERSION_CODE: u64 = 20261001;
const FEED: &str =
    "https://api.github.com/repos/black-candle-technologies/openmgmt/releases?per_page=10";
const PREFIX: &str = "https://github.com/black-candle-technologies/openmgmt/releases/download/";
#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    pub version: String,
    pub file: ReleaseFile,
    url: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub last_check: Option<u64>,
    pub available: Option<Offer>,
    pub error: Option<String>,
    pub downloading: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
}
#[derive(Default)]
pub struct Updater(pub Mutex<UpdateStatus>, pub tokio::sync::Mutex<()>);
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("OpenMGMT-updater")
        .https_only(true)
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}
fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}
fn installer(name: &str) -> bool {
    match platform() {
        "macos" => name.ends_with(".dmg") || name.ends_with(".pkg"),
        "windows" => name.ends_with(".msi") || name.ends_with(".exe"),
        _ => name.ends_with(".deb") || name.ends_with(".rpm"),
    }
}
#[tauri::command]
pub fn updater_status(updater: State<'_, Updater>) -> UpdateStatus {
    updater.0.lock().unwrap().clone()
}
#[tauri::command]
pub async fn check_updates(
    app: tauri::AppHandle,
    updater: State<'_, Updater>,
    force: bool,
) -> Result<UpdateStatus, String> {
    let _guard = updater.1.lock().await;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = dir.join("update-last-check");
    let last = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    if !force && last.is_some_and(|t| now().saturating_sub(t) < 3600) {
        updater.0.lock().unwrap().last_check = last;
        return Ok(updater.0.lock().unwrap().clone());
    }
    let checked = now();
    std::fs::write(stamp, checked.to_string()).map_err(|e| e.to_string())?;
    {
        let mut status = updater.0.lock().unwrap();
        status.last_check = Some(checked);
        status.error = None;
    }
    let result = find_update().await;
    let mut status = updater.0.lock().unwrap();
    match result {
        Ok(offer) => status.available = offer,
        Err(e) => status.error = Some(e),
    }
    Ok(status.clone())
}
async fn find_update() -> Result<Option<Offer>, String> {
    let client = http()?;
    let response = client
        .get(FEED)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let releases: Vec<Release> = response.json().await.map_err(|e| e.to_string())?;
    let mut offer: Option<Offer> = None;
    for release in releases.into_iter().filter(|r| !r.draft && !r.prerelease) {
        let Some(asset) = release
            .assets
            .iter()
            .find(|a| a.name == "release.json" && a.browser_download_url.starts_with(PREFIX))
        else {
            continue;
        };
        let mut response = client
            .get(&asset.browser_download_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("Release manifest too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let manifest = ReleaseManifest::parse(&bytes)?;
        let Some(file) =
            manifest.newer_file(platform(), std::env::consts::ARCH, UPDATE_VERSION_CODE)
        else {
            continue;
        };
        if !installer(&file.name) {
            continue;
        }
        let Some(asset) = release
            .assets
            .iter()
            .find(|a| a.name == file.name && a.browser_download_url.starts_with(PREFIX))
        else {
            continue;
        };
        if offer
            .as_ref()
            .is_none_or(|o| file.version_code > o.file.version_code)
        {
            offer = Some(Offer {
                version: release.tag_name,
                file: file.clone(),
                url: asset.browser_download_url.clone(),
            });
        }
    }
    Ok(offer)
}
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    updater: State<'_, Updater>,
) -> Result<(), String> {
    // No URL/path from the webview: install only the offer selected by this process.
    let _guard = updater.1.lock().await;
    let offer = updater
        .0
        .lock()
        .unwrap()
        .available
        .clone()
        .ok_or("No update available")?;
    {
        let mut s = updater.0.lock().unwrap();
        s.downloading = true;
        s.downloaded = 0;
        s.error = None;
    }
    let result = download_and_open(&app, &updater, &offer).await;
    {
        let mut s = updater.0.lock().unwrap();
        s.downloading = false;
        if let Err(e) = &result {
            s.error = Some(e.clone());
        }
    }
    result
}
async fn download_and_open(
    app: &tauri::AppHandle,
    updater: &Updater,
    offer: &Offer,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("updates");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    let mut response = http()?
        .get(&offer.url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    updater.0.lock().unwrap().total = response.content_length();
    let mut hash = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        size += chunk.len() as u64;
        if size > 2 * 1024 * 1024 * 1024 {
            return Err("Installer exceeds download limit".into());
        }
        temporary.write_all(&chunk).map_err(|e| e.to_string())?;
        hash.update(&chunk);
        updater.0.lock().unwrap().downloaded = size;
    }
    if format!("{:x}", hash.finalize()) != offer.file.sha256.to_lowercase() {
        return Err("SHA-256 mismatch; installer was deleted and will not be opened".into());
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let target = dir.join(format!("{}-{}", offer.file.version_code, offer.file.name));
    temporary.persist(&target).map_err(|e| e.to_string())?;
    open::that(target).map_err(|e| e.to_string())?;
    Ok(())
}
