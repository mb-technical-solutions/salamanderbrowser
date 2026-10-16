use crate::browser::{GithubAsset, GithubRelease};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionComponent {
  pub major: u32,
  pub minor: u32,
  pub patch: u32,
  pub build: u32,
}

impl VersionComponent {
  pub fn parse(version: &str) -> Self {
    let version = version.trim();
    let version = if version.starts_with('v') || version.starts_with('V') {
      &version[1..]
    } else {
      version
    };
    let version = version.to_lowercase();

    let prefix_end = Self::numeric_prefix_end(&version);
    let numeric_part = version[..prefix_end].trim_end_matches('.');

    let mut parts: Vec<u32> = numeric_part
      .split('.')
      .filter_map(|part| part.parse().ok())
      .collect();

    // A trailing pre-release/build number after the dotted numeric prefix
    // (Camoufox tags releases `152.0.4-beta.30`) is the real tiebreaker
    // between releases that otherwise share the same major.minor.patch.
    // Without it, every beta of "152.0.4" parses identically and a
    // supposedly-newest-first sort degrades to whatever arbitrary order the
    // caller happened to hand in — which is exactly how an old beta with a
    // known-broken search-service build ends up selected as "latest".
    if parts.len() < 4 {
      if let Some(trailing) = Self::last_digit_run(&version[prefix_end..]) {
        parts.resize(3, 0);
        parts.push(trailing);
      }
    }

    VersionComponent {
      major: parts.first().copied().unwrap_or(0),
      minor: parts.get(1).copied().unwrap_or(0),
      patch: parts.get(2).copied().unwrap_or(0),
      build: parts.get(3).copied().unwrap_or(0),
    }
  }

  /// End index of the leading run of digits and `.` in `version` (already
  /// lowercased). Stopping at any other character — not just letters — keeps
  /// a trailing separator like the `-` in `152.0.4-beta.30` out of the
  /// numeric prefix, where it would otherwise attach to the last digit group
  /// and make it unparseable.
  fn numeric_prefix_end(version: &str) -> usize {
    version
      .char_indices()
      .find(|&(_, ch)| !(ch.is_ascii_digit() || ch == '.'))
      .map(|(i, _)| i)
      .unwrap_or(version.len())
  }

  /// The last maximal run of ASCII digits anywhere in `s`.
  fn last_digit_run(s: &str) -> Option<u32> {
    let mut last = None;
    let mut current = String::new();
    for ch in s.chars() {
      if ch.is_ascii_digit() {
        current.push(ch);
      } else if !current.is_empty() {
        last = current.parse().ok();
        current.clear();
      }
    }
    if !current.is_empty() {
      last = current.parse().ok();
    }
    last
  }
}

impl PartialOrd for VersionComponent {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for VersionComponent {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    (self.major, self.minor, self.patch, self.build).cmp(&(
      other.major,
      other.minor,
      other.patch,
      other.build,
    ))
  }
}

pub fn sort_versions(versions: &mut [String]) {
  versions.sort_by(|a, b| {
    let version_a = VersionComponent::parse(a);
    let version_b = VersionComponent::parse(b);
    version_b.cmp(&version_a)
  });
}

pub fn compare_versions(version1: &str, version2: &str) -> std::cmp::Ordering {
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a.cmp(&version_b)
}

pub fn is_version_newer(version1: &str, version2: &str) -> bool {
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a > version_b
}

/// Sort GitHub releases newest-first by their tag name.
pub fn sort_github_releases(releases: &mut [GithubRelease]) {
  releases.sort_by(|a, b| {
    let version_a = VersionComponent::parse(&a.tag_name);
    let version_b = VersionComponent::parse(&b.tag_name);
    version_b.cmp(&version_a)
  });
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserRelease {
  pub version: String,
  pub date: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedVersionData {
  releases: Vec<BrowserRelease>,
  timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedGithubData {
  releases: Vec<GithubRelease>,
  timestamp: u64,
}

pub struct ApiClient {
  client: Client,
}

impl ApiClient {
  pub fn new() -> Self {
    let client = Client::builder()
      .timeout(std::time::Duration::from_secs(30))
      .build()
      .unwrap_or_else(|_| Client::new());

    Self { client }
  }

  pub fn instance() -> &'static ApiClient {
    &API_CLIENT
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = crate::app_dirs::cache_dir().join("version_cache");
    fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
  }

  fn get_current_timestamp() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs()
  }

  fn is_cache_valid(timestamp: u64) -> bool {
    let current_time = Self::get_current_timestamp();
    let cache_duration = 10 * 60;
    current_time - timestamp < cache_duration
  }

  pub fn load_cached_versions(&self, browser: &str) -> Option<Vec<BrowserRelease>> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    if let Ok(cached) = serde_json::from_str::<CachedVersionData>(&content) {
      log::info!("Using cached versions for {browser}");
      return Some(cached.releases);
    }

    if let Ok(legacy_versions) = serde_json::from_str::<Vec<String>>(&content) {
      log::info!("Using legacy cached versions for {browser}; upgrading in-memory");
      let releases: Vec<BrowserRelease> = legacy_versions
        .into_iter()
        .map(|version| BrowserRelease {
          version,
          date: "".to_string(),
        })
        .collect();
      return Some(releases);
    }

    None
  }

  pub fn is_cache_expired(&self, browser: &str) -> bool {
    let cache_dir = match Self::get_cache_dir() {
      Ok(dir) => dir,
      Err(_) => return true,
    };
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return true;
    }

    let content = match fs::read_to_string(&cache_file) {
      Ok(content) => content,
      Err(_) => return true,
    };

    let cached_data: CachedVersionData = match serde_json::from_str(&content) {
      Ok(data) => data,
      Err(_) => return true,
    };

    !Self::is_cache_valid(cached_data.timestamp)
  }

  pub fn save_cached_versions(
    &self,
    browser: &str,
    releases: &[BrowserRelease],
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    let cached_data = CachedVersionData {
      releases: releases.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    log::info!("Cached {} versions for {}", releases.len(), browser);
    Ok(())
  }

  fn get_platform_info() -> (String, String) {
    let os = if cfg!(target_os = "windows") {
      "windows"
    } else if cfg!(target_os = "linux") {
      "linux"
    } else if cfg!(target_os = "macos") {
      "macos"
    } else {
      "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
      "x64"
    } else if cfg!(target_arch = "aarch64") {
      "arm64"
    } else {
      "unknown"
    };

    (os.to_string(), arch.to_string())
  }

  fn load_cached_github_releases(&self, browser: &str) -> Option<Vec<GithubRelease>> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join(format!("{browser}_github.json"));

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    let cached_data: CachedGithubData = serde_json::from_str(&content).ok()?;
    Some(cached_data.releases)
  }

  fn save_cached_github_releases(
    &self,
    browser: &str,
    releases: &[GithubRelease],
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_github.json"));

    let cached_data = CachedGithubData {
      releases: releases.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    log::info!("Cached {} GitHub releases for {}", releases.len(), browser);
    Ok(())
  }

  async fn fetch_github_releases_multiple_pages(
    &self,
    base_releases_url: &str,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{base_releases_url}?per_page=100&page=1");
    let response = self
      .client
      .get(&url)
      .header(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
      )
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("GitHub API returned status: {}", response.status()).into());
    }

    let text = response.text().await?;
    let releases: Vec<GithubRelease> = serde_json::from_str(&text).map_err(|e| {
      log::error!("Failed to parse GitHub API response: {e}");
      format!("Failed to parse GitHub API response: {e}")
    })?;

    Ok(releases)
  }

  /// Check if a Camoufox release has an asset compatible with the given
  /// platform/architecture. Real upstream naming:
  /// `camoufox-{version}-beta.{n}-{os}.{arch}.zip`, e.g.
  /// `camoufox-135.0.1-beta.24-lin.x86_64.zip`.
  fn camoufox_asset_pattern(os: &str, arch: &str) -> Option<String> {
    let (os_name, arch_name) = match (os, arch) {
      ("windows", "x64") => ("win", "x86_64"),
      ("windows", "arm64") => ("win", "arm64"),
      ("linux", "x64") => ("lin", "x86_64"),
      ("linux", "arm64") => ("lin", "arm64"),
      ("macos", "x64") => ("mac", "x86_64"),
      ("macos", "arm64") => ("mac", "arm64"),
      _ => return None,
    };
    Some(format!("-{os_name}.{arch_name}.zip"))
  }

  fn has_compatible_camoufox_asset(assets: &[GithubAsset], os: &str, arch: &str) -> bool {
    let Some(pattern) = Self::camoufox_asset_pattern(os, arch) else {
      return false;
    };
    assets.iter().any(|asset| {
      let name = asset.name.to_lowercase();
      name.starts_with("camoufox-") && name.ends_with(&pattern)
    })
  }

  /// Find the download URL for the asset matching the current platform in a
  /// specific Camoufox release.
  pub fn find_camoufox_asset<'a>(
    release: &'a GithubRelease,
    os: &str,
    arch: &str,
  ) -> Option<&'a GithubAsset> {
    let pattern = Self::camoufox_asset_pattern(os, arch)?;
    release.assets.iter().find(|asset| {
      let name = asset.name.to_lowercase();
      name.starts_with("camoufox-") && name.ends_with(&pattern)
    })
  }

  /// Fetch Camoufox releases from `github.com/daijro/camoufox`, filtered to
  /// releases that have an asset compatible with the current platform, newest
  /// first. The GitHub releases list never expires once cached; callers pass
  /// `no_caching: true` for background refreshes.
  pub async fn fetch_camoufox_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_github_releases("camoufox") {
        log::info!(
          "Using cached Camoufox releases, count: {}",
          cached_releases.len()
        );
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Camoufox releases from GitHub API");
    let releases: Vec<GithubRelease> = self
      .fetch_github_releases_multiple_pages("https://api.github.com/repos/daijro/camoufox/releases")
      .await?;

    let (os, arch) = Self::get_platform_info();
    let mut compatible_releases: Vec<GithubRelease> = releases
      .into_iter()
      .filter(|release| Self::has_compatible_camoufox_asset(&release.assets, &os, &arch))
      .collect();

    sort_github_releases(&mut compatible_releases);

    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("camoufox", &compatible_releases) {
        log::error!("Failed to cache Camoufox releases: {e}");
      }
    }

    Ok(compatible_releases)
  }

  fn has_compatible_clearcote_asset(assets: &[GithubAsset], os: &str, arch: &str) -> bool {
    let suffix = match (os, arch) {
      ("windows", "x64") => "-windows-x64.zip",
      ("linux", "x64") => "-linux-x64.tar.xz",
      _ => return false,
    };
    assets.iter().any(|asset| {
      asset.name.to_lowercase().starts_with("clearcote-")
        && asset.name.to_lowercase().ends_with(suffix)
    })
  }

  pub fn find_clearcote_asset<'a>(
    release: &'a GithubRelease,
    os: &str,
    arch: &str,
  ) -> Option<&'a GithubAsset> {
    let suffix = match (os, arch) {
      ("windows", "x64") => "-windows-x64.zip",
      ("linux", "x64") => "-linux-x64.tar.xz",
      _ => return None,
    };
    release.assets.iter().find(|asset| {
      asset.name.to_lowercase().starts_with("clearcote-")
        && asset.name.to_lowercase().ends_with(suffix)
    })
  }

  pub async fn fetch_clearcote_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    if !no_caching {
      if let Some(releases) = self.load_cached_github_releases("clearcote") {
        return Ok(releases);
      }
    }
    let releases = self
      .fetch_github_releases_multiple_pages(
        "https://api.github.com/repos/clearcotelabs/clearcote-browser/releases",
      )
      .await?;
    let (os, arch) = Self::get_platform_info();
    let mut compatible: Vec<GithubRelease> = releases
      .into_iter()
      .filter(|release| Self::has_compatible_clearcote_asset(&release.assets, &os, &arch))
      .collect();
    sort_github_releases(&mut compatible);
    if !no_caching {
      self.save_cached_github_releases("clearcote", &compatible)?;
    }
    Ok(compatible)
  }

  pub fn clear_all_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;

    if cache_dir.exists() {
      for entry in fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
          fs::remove_file(&path)?;
          log::info!("Removed cache file: {path:?}");
        }
      }
      log::info!("All version cache cleared successfully");
    }

    Ok(())
  }
}

lazy_static::lazy_static! {
  static ref API_CLIENT: ApiClient = ApiClient::new();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_version_parsing() {
    let v1 = VersionComponent::parse("1.2.3");
    assert_eq!(v1.major, 1);
    assert_eq!(v1.minor, 2);
    assert_eq!(v1.patch, 3);

    let v2 = VersionComponent::parse("138.0.7204.50");
    assert_eq!(v2.major, 138);
    assert_eq!(v2.minor, 0);
    assert_eq!(v2.patch, 7204);
    assert_eq!(v2.build, 50);

    let v3 = VersionComponent::parse("137.0b5");
    assert_eq!(v3.major, 137);
    assert_eq!(v3.minor, 0);
    assert_eq!(v3.patch, 0);

    // Camoufox's own tag shape: `v<major>.<minor>.<patch>-beta.<n>`. The
    // trailing beta number must land in `build`, not get merged into (or
    // silently drop) the patch component.
    let v4 = VersionComponent::parse("v152.0.4-beta.30");
    assert_eq!(v4.major, 152);
    assert_eq!(v4.minor, 0);
    assert_eq!(v4.patch, 4);
    assert_eq!(v4.build, 30);

    // Some Camoufox tags omit the patch component entirely.
    let v5 = VersionComponent::parse("v132.0-beta.14");
    assert_eq!(v5.major, 132);
    assert_eq!(v5.minor, 0);
    assert_eq!(v5.patch, 0);
    assert_eq!(v5.build, 14);
  }

  #[test]
  fn test_version_comparison() {
    assert!(VersionComponent::parse("1.2.4") > VersionComponent::parse("1.2.3"));
    assert!(VersionComponent::parse("2.0.0") > VersionComponent::parse("1.9.9"));
    assert!(VersionComponent::parse("138.0.7204.50") > VersionComponent::parse("138.0.7204.49"));

    // Regression: two Camoufox betas sharing "152.0.4" must not compare
    // equal — the newer beta number must sort ahead of the older one, or an
    // "always fetch fresh, sort newest-first" pipeline can silently settle
    // on a stale (possibly broken) build instead of the real latest.
    assert!(
      VersionComponent::parse("v152.0.4-beta.30") > VersionComponent::parse("v152.0.4-beta.26")
    );
  }

  #[test]
  fn test_sort_github_releases_orders_camoufox_betas_newest_first() {
    let tags = [
      "v152.0.4-beta.27",
      "v152.0.4-beta.30",
      "v152.0.4-beta.26",
      "v152.0.4-beta.28",
      "v152.0.4-beta.29",
    ];
    let mut releases: Vec<GithubRelease> = tags
      .iter()
      .map(|tag| {
        serde_json::from_value(serde_json::json!({ "tag_name": tag, "assets": [] }))
          .expect("minimal release JSON deserializes")
      })
      .collect();

    sort_github_releases(&mut releases);

    let sorted_tags: Vec<&str> = releases.iter().map(|r| r.tag_name.as_str()).collect();
    assert_eq!(
      sorted_tags,
      vec![
        "v152.0.4-beta.30",
        "v152.0.4-beta.29",
        "v152.0.4-beta.28",
        "v152.0.4-beta.27",
        "v152.0.4-beta.26",
      ]
    );
  }

  #[test]
  fn clearcote_asset_lookup_matches_only_supported_x64_archives() {
    let release: GithubRelease = serde_json::from_value(serde_json::json!({
      "tag_name": "v0.1.0-pre.23",
      "assets": [
        {"name": "clearcote-150.0.7871.114-linux-x64.tar.xz", "browser_download_url": "https://example.invalid/linux"},
        {"name": "clearcote-150.0.7871.114-windows-x64.zip", "browser_download_url": "https://example.invalid/windows"}
      ]
    })).unwrap();
    assert_eq!(
      ApiClient::find_clearcote_asset(&release, "linux", "x64")
        .unwrap()
        .name,
      "clearcote-150.0.7871.114-linux-x64.tar.xz"
    );
    assert_eq!(
      ApiClient::find_clearcote_asset(&release, "windows", "x64")
        .unwrap()
        .name,
      "clearcote-150.0.7871.114-windows-x64.zip"
    );
    assert!(ApiClient::find_clearcote_asset(&release, "linux", "arm64").is_none());
  }

  #[test]
  fn test_version_sorting() {
    let mut versions = vec![
      "138.0.7204.50".to_string(),
      "138.0.7204.49".to_string(),
      "139.0.7204.1".to_string(),
      "137.0.7204.99".to_string(),
    ];

    sort_versions(&mut versions);

    assert_eq!(versions[0], "139.0.7204.1");
    assert_eq!(versions[1], "138.0.7204.50");
    assert_eq!(versions[2], "138.0.7204.49");
    assert_eq!(versions[3], "137.0.7204.99");
  }
}
