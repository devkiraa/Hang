use anyhow::{Context, Result};
use serde::Deserialize;
use std::cmp::Ordering;

use crate::constants::{GITHUB_RELEASES_API, VERSION};

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    #[allow(dead_code)]
    pub release_notes: String,
    pub is_update_available: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Check for updates from GitHub releases
pub async fn check_for_updates() -> Result<UpdateInfo> {
    let client = reqwest::Client::builder()
        .user_agent("Hang-Client")
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(GITHUB_RELEASES_API)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("Failed to fetch release info")?;

    if !response.status().is_success() {
        anyhow::bail!("GitHub API returned status: {}", response.status());
    }

    let release: GitHubRelease = response
        .json()
        .await
        .context("Failed to parse release info")?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let current_version = VERSION.to_string();
    let is_update_available = compare_versions(&current_version, &latest_version) == Ordering::Less;

    // Find the MSI download URL, fallback to release page
    let download_url = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".msi"))
        .map(|a| a.browser_download_url.clone())
        .unwrap_or_else(|| release.html_url.clone());

    let release_notes = release.body.unwrap_or_default();

    Ok(UpdateInfo {
        current_version,
        latest_version,
        download_url,
        release_notes,
        is_update_available,
    })
}

/// Compare semantic versions (e.g., "1.2.3" vs "1.3.0")
fn compare_versions(current: &str, latest: &str) -> Ordering {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .collect()
    };

    let current_parts = parse_version(current);
    let latest_parts = parse_version(latest);

    for i in 0..3 {
        let c = current_parts.get(i).copied().unwrap_or(0);
        let l = latest_parts.get(i).copied().unwrap_or(0);
        match c.cmp(&l) {
            Ordering::Less => return Ordering::Less,
            Ordering::Greater => return Ordering::Greater,
            Ordering::Equal => continue,
        }
    }
    Ordering::Equal
}

/// Open the download URL in the default browser
pub fn open_download_page(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0", "1.0.1"), Ordering::Less);
        assert_eq!(compare_versions("1.0.1", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.1.0"), Ordering::Less);
        assert_eq!(compare_versions("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Ordering::Equal);
    }
}
