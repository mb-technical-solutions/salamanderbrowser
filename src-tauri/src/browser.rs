use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxySettings {
  pub proxy_type: String,
  pub host: String,
  pub port: u16,
  pub username: Option<String>,
  pub password: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub vless_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowserType {
  Camoufox,
  Clearcote,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::Camoufox => "camoufox",
      BrowserType::Clearcote => "clearcote",
    }
  }

  /// Brand name for user-facing strings. `as_str` is the internal id and is
  /// the wrong thing to put in a message the user reads.
  pub fn display_name(&self) -> &'static str {
    match self {
      BrowserType::Camoufox => "Camoufox",
      BrowserType::Clearcote => "Clearcote",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "camoufox" => Ok(BrowserType::Camoufox),
      "clearcote" => Ok(BrowserType::Clearcote),
      _ => Err(format!("Unknown browser type: {s}")),
    }
  }
}

#[allow(dead_code)]
pub trait Browser: Send + Sync {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>>;
  fn create_launch_args(
    &self,
    profile_path: &str,
    proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>>;
  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool;
  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>>;
}

// Platform-specific modules
#[cfg(target_os = "macos")]
mod macos {
  use super::*;

  #[allow(dead_code)]
  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On macOS, no special preparation needed
    Ok(())
  }

  /// Camoufox ships as `Camoufox.app`, with an architecture-specific
  /// executable inside `Contents/MacOS` on universal-ish release layouts
  /// (the release zip is per-arch, but a build extracted onto the wrong
  /// machine can leave an incompatible binary sitting there).
  pub fn get_camoufox_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Camoufox app not found")?;

    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    let candidates: Vec<PathBuf> = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .filter(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.starts_with("camoufox") || name.contains("Camoufox")
      })
      .map(|entry| entry.path())
      .collect();

    let current_arch = if cfg!(target_arch = "x86_64") {
      "x86_64"
    } else if cfg!(target_arch = "aarch64") {
      "arm64"
    } else {
      return Err("Unsupported architecture".into());
    };

    for candidate in &candidates {
      if let Ok(output) = std::process::Command::new("file").arg(candidate).output() {
        if output.status.success() {
          if let Ok(output_str) = String::from_utf8(output.stdout) {
            let is_compatible = if current_arch == "x86_64" {
              output_str.contains("x86_64") || output_str.contains("i386")
            } else {
              output_str.contains("arm64") || output_str.contains("aarch64")
            };
            if is_compatible {
              return Ok(candidate.clone());
            }
            log::warn!(
              "Skipping incompatible Camoufox executable: {} (architecture: {})",
              candidate.display(),
              output_str.trim()
            );
          }
        }
      }
    }

    // `file` unavailable or nothing matched: fall back to the first candidate
    // rather than failing outright.
    candidates
      .into_iter()
      .next()
      .ok_or_else(|| "No Camoufox executable found in MacOS directory".into())
  }

  pub fn is_camoufox_version_downloaded(install_dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "app") {
          return true;
        }
      }
    }
    false
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;
  use std::os::unix::fs::PermissionsExt;

  #[allow(dead_code)]
  pub fn prepare_executable(executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On Linux, ensure the executable has proper permissions
    log::info!("Setting execute permissions for: {:?}", executable_path);

    let metadata = std::fs::metadata(executable_path)?;
    let mut permissions = metadata.permissions();

    // Add execute permissions for owner, group, and others
    let mode = permissions.mode();
    permissions.set_mode(mode | 0o755);

    std::fs::set_permissions(executable_path, permissions)?;

    log::info!(
      "Execute permissions set successfully for: {:?}",
      executable_path
    );
    Ok(())
  }

  /// Candidate paths for the Camoufox executable on Linux, in priority order.
  /// Upstream releases ship the binary named `camoufox` or `camoufox-bin` at
  /// the archive root, or nested under a `camoufox/` subdirectory.
  fn camoufox_executable_candidates(install_dir: &Path) -> Vec<PathBuf> {
    const NAMES: [&str; 2] = ["camoufox", "camoufox-bin"];
    let dirs = [
      install_dir.to_path_buf(),
      install_dir.join("camoufox"),
      install_dir.join("camoufox-linux"),
    ];
    dirs
      .iter()
      .flat_map(|dir| NAMES.iter().map(move |name| dir.join(name)))
      .collect()
  }

  pub fn get_camoufox_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    for executable_path in camoufox_executable_candidates(install_dir) {
      if executable_path.exists() && executable_path.is_file() {
        return Ok(executable_path);
      }
    }

    Err(format!("Camoufox executable not found in {}", install_dir.display()).into())
  }

  pub fn is_camoufox_version_downloaded(install_dir: &Path) -> bool {
    camoufox_executable_candidates(install_dir)
      .iter()
      .any(|exe_path| exe_path.exists() && exe_path.is_file())
  }
}

#[cfg(target_os = "windows")]
mod windows {
  use super::*;

  #[allow(dead_code)]
  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On Windows, no special preparation needed
    Ok(())
  }

  /// Candidate paths for the Camoufox executable on Windows, in priority
  /// order. Upstream releases ship `camoufox.exe` at the archive root, or
  /// nested under a `camoufox\` subdirectory.
  fn camoufox_executable_candidates(install_dir: &Path) -> Vec<PathBuf> {
    const NAMES: [&str; 1] = ["camoufox.exe"];
    let dirs = [
      install_dir.to_path_buf(),
      install_dir.join("camoufox"),
      install_dir.join("camoufox-win"),
    ];
    dirs
      .iter()
      .flat_map(|dir| NAMES.iter().map(move |name| dir.join(name)))
      .collect()
  }

  pub fn get_camoufox_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    for path in camoufox_executable_candidates(install_dir) {
      if path.exists() && path.is_file() {
        return Ok(path);
      }
    }

    // Look for any .exe file that might be the browser.
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "exe") && is_pe_executable(&path) {
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.starts_with("camoufox") {
            return Ok(path);
          }
        }
      }
    }

    Err("Camoufox executable not found in Windows installation directory".into())
  }

  pub fn is_camoufox_version_downloaded(install_dir: &Path) -> bool {
    if camoufox_executable_candidates(install_dir)
      .iter()
      .any(|exe_path| exe_path.exists() && exe_path.is_file() && has_sibling_dll(exe_path))
    {
      return true;
    }

    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "exe") && is_pe_executable(&path) {
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.starts_with("camoufox") && has_sibling_dll(&path) {
            return true;
          }
        }
      }
    }

    false
  }
}

#[cfg(target_os = "linux")]
mod clearcote_linux {
  use super::*;
  use std::os::unix::fs::PermissionsExt;

  pub fn prepare_executable(executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = std::fs::metadata(executable_path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(executable_path, permissions)?;
    Ok(())
  }

  pub fn get_executable_path(install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let candidates = [install_dir.join("chrome"), install_dir.join("clearcote")];
    candidates
      .into_iter()
      .find(|path| path.is_file())
      .ok_or_else(|| {
        format!(
          "Clearcote executable not found in {}",
          install_dir.display()
        )
        .into()
      })
  }

  pub fn is_downloaded(install_dir: &Path) -> bool {
    get_executable_path(install_dir).is_ok()
  }
}

#[cfg(target_os = "windows")]
mod clearcote_windows {
  use super::*;

  pub fn get_executable_path(install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let candidates = [
      install_dir.join("chrome.exe"),
      install_dir.join("clearcote.exe"),
    ];
    candidates
      .into_iter()
      .find(|path| path.is_file() && has_sibling_dll(path))
      .ok_or_else(|| {
        format!(
          "Clearcote executable not found in {}",
          install_dir.display()
        )
        .into()
      })
  }

  pub fn is_downloaded(install_dir: &Path) -> bool {
    get_executable_path(install_dir).is_ok()
  }
}

/// Camoufox is a Firefox-based anti-detect browser. Fingerprint injection
/// happens via `CAMOU_CONFIG_N` environment variables set by
/// `camoufox_manager::launch_camoufox`, which is the actual launch path used
/// at runtime; `create_launch_args` below only exists to satisfy the
/// `Browser` trait and is not used for a real launch.
pub struct CamoufoxBrowser;

impl CamoufoxBrowser {
  pub fn new() -> Self {
    Self
  }
}

/// Clearcote is a Chromium build. Its release artifacts currently support only
/// Windows x64 and Linux x64; macOS and ARM64 are rejected explicitly.
pub struct ClearcoteBrowser;

impl ClearcoteBrowser {
  pub fn new() -> Self {
    Self
  }
}

impl Browser for ClearcoteBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return clearcote_linux::get_executable_path(install_dir);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return clearcote_windows::get_executable_path(install_dir);
    #[cfg(target_os = "macos")]
    {
      Err("Clearcote does not publish macOS builds".into())
    }
    #[cfg(all(
      any(target_os = "linux", target_os = "windows"),
      target_arch = "aarch64"
    ))]
    {
      Err("Clearcote does not publish ARM64 builds".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
      Err("Clearcote is unsupported on this platform".into())
    }
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let fingerprint_platform = if cfg!(target_os = "windows") {
      "windows"
    } else if cfg!(target_os = "macos") {
      "macos"
    } else {
      "linux"
    };
    let mut args = vec![
      format!("--user-data-dir={profile_path}"),
      "--no-first-run".to_string(),
      "--no-default-browser-check".to_string(),
      "--disable-background-networking".to_string(),
      // Clearcote's native fingerprint is seeded per persistent profile. The
      // profile UUID is the final path component, so it remains stable across
      // launches without leaking the user's absolute data path to the engine.
      format!(
        "--fingerprint={}",
        Path::new(profile_path)
          .parent()
          .and_then(Path::file_name)
          .and_then(|name| name.to_str())
          .unwrap_or("salamander-profile")
      ),
      format!("--fingerprint-platform={fingerprint_platform}"),
      "--remote-debugging-address=127.0.0.1".to_string(),
    ];
    if let Some(proxy) = proxy_settings {
      args.push(format!(
        "--proxy-server={}://{}:{}",
        proxy.proxy_type, proxy.host, proxy.port
      ));
    }
    if let Some(port) = remote_debugging_port {
      args.push(format!("--remote-debugging-port={port}"));
      args.push(format!("--remote-allow-origins=http://127.0.0.1:{port}"));
    }
    if headless {
      args.push("--headless=new".to_string());
    }
    if let Some(url) = url {
      args.push(url);
    }
    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    let install_dir = binaries_dir.join("clearcote").join(version);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return clearcote_linux::is_downloaded(&install_dir);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return clearcote_windows::is_downloaded(&install_dir);
    #[cfg(any(target_os = "macos", target_arch = "aarch64"))]
    {
      false
    }
    #[cfg(not(any(
      target_os = "macos",
      target_os = "linux",
      target_os = "windows",
      target_arch = "aarch64"
    )))]
    {
      false
    }
  }

  fn prepare_executable(&self, _executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return clearcote_linux::prepare_executable(_executable_path);
    #[cfg(target_os = "windows")]
    return Ok(());
    #[cfg(target_os = "macos")]
    return Err("Clearcote does not publish macOS builds".into());
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Err("Clearcote does not publish ARM64 builds".into());
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    Err("Clearcote is unsupported on this platform".into())
  }
}

impl Browser for CamoufoxBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::get_camoufox_executable_path(install_dir);

    #[cfg(target_os = "linux")]
    return linux::get_camoufox_executable_path(install_dir);

    #[cfg(target_os = "windows")]
    return windows::get_camoufox_executable_path(install_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    _proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec![
      "-profile".to_string(),
      profile_path.to_string(),
      "-no-remote".to_string(),
    ];

    if let Some(port) = remote_debugging_port {
      args.push(format!("--remote-debugging-port={port}"));
    }

    if headless {
      args.push("--headless".to_string());
    }

    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    let install_dir = binaries_dir.join("camoufox").join(version);

    #[cfg(target_os = "macos")]
    return macos::is_camoufox_version_downloaded(&install_dir);

    #[cfg(target_os = "linux")]
    return linux::is_camoufox_version_downloaded(&install_dir);

    #[cfg(target_os = "windows")]
    return windows::is_camoufox_version_downloaded(&install_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    false
  }

  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::prepare_executable(executable_path);

    #[cfg(target_os = "linux")]
    return linux::prepare_executable(executable_path);

    #[cfg(target_os = "windows")]
    return windows::prepare_executable(executable_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }
}

pub struct BrowserFactory;

impl BrowserFactory {
  fn new() -> Self {
    Self
  }

  pub fn instance() -> &'static BrowserFactory {
    &BROWSER_FACTORY
  }

  pub fn create_browser(&self, browser_type: BrowserType) -> Box<dyn Browser> {
    match browser_type {
      BrowserType::Camoufox => Box::new(CamoufoxBrowser::new()),
      BrowserType::Clearcote => Box::new(ClearcoteBrowser::new()),
    }
  }
}

/// Whether the directory holding `exe_path` also contains at least one `.dll`.
///
/// A gutted install (the `.exe` survived a cleanup pass that deleted every
/// `.dll`) is not a healthy one; treating it as downloaded is what made that
/// state permanent, since the registry rescan would re-add it as healthy and
/// no re-download would ever be offered.
#[cfg(any(target_os = "windows", test))]
fn has_sibling_dll(exe_path: &Path) -> bool {
  let Some(dir) = exe_path.parent() else {
    return false;
  };
  let Ok(entries) = std::fs::read_dir(dir) else {
    return false;
  };
  entries.flatten().any(|entry| {
    entry
      .path()
      .extension()
      .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
  })
}

/// Check if a file is a valid PE executable by reading its magic bytes (MZ).
/// Returns false for archive files (.zip starts with PK, etc.) that were
/// incorrectly named with a .exe extension.
#[cfg(target_os = "windows")]
fn is_pe_executable(path: &Path) -> bool {
  use std::io::Read;
  let Ok(mut file) = std::fs::File::open(path) else {
    return false;
  };
  let mut magic = [0u8; 2];
  if file.read_exact(&mut magic).is_err() {
    return false;
  }
  magic == [0x4D, 0x5A] // MZ
}

// Factory function to create browser instances (kept for backward compatibility)
pub fn create_browser(browser_type: BrowserType) -> Box<dyn Browser> {
  BrowserFactory::instance().create_browser(browser_type)
}

// Add GithubRelease and GithubAsset structs to browser.rs if they don't already exist
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubRelease {
  pub tag_name: String,
  #[serde(default)]
  pub name: String,
  pub assets: Vec<GithubAsset>,
  #[serde(default)]
  pub published_at: String,
  #[serde(default)]
  pub is_nightly: bool,
  #[serde(default)]
  pub prerelease: bool,
  #[serde(default)]
  pub draft: bool,
  #[serde(default)]
  pub body: Option<String>,
  #[serde(default)]
  pub html_url: Option<String>,
  #[serde(default)]
  pub id: Option<u64>,
  #[serde(default)]
  pub node_id: Option<String>,
  #[serde(default)]
  pub target_commitish: Option<String>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub tarball_url: Option<String>,
  #[serde(default)]
  pub zipball_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubAsset {
  pub name: String,
  pub browser_download_url: String,
  #[serde(default)]
  pub size: u64,
  #[serde(default)]
  pub download_count: Option<u64>,
  #[serde(default)]
  pub id: Option<u64>,
  #[serde(default)]
  pub node_id: Option<String>,
  #[serde(default)]
  pub label: Option<String>,
  #[serde(default)]
  pub content_type: Option<String>,
  #[serde(default)]
  pub state: Option<String>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
  /// GitHub's SHA-256 digest for the uploaded asset (for example
  /// `sha256:abc123...`). Camoufox publishes this in the release API rather
  /// than as a `.sha256` sidecar file.
  #[serde(default)]
  pub digest: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_proxy_settings_serialization() {
    let proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
      vless_uri: None,
    };

    // Test that it can be serialized (implements Serialize)
    let json = serde_json::to_string(&proxy).expect("Failed to serialize proxy settings");
    assert!(json.contains("127.0.0.1"), "JSON should contain host IP");
    assert!(json.contains("8080"), "JSON should contain port number");
    assert!(json.contains("http"), "JSON should contain proxy type");

    // Test that it can be deserialized (implements Deserialize)
    let deserialized: ProxySettings =
      serde_json::from_str(&json).expect("Failed to deserialize proxy settings");
    assert_eq!(
      deserialized.proxy_type, proxy.proxy_type,
      "Proxy type should match"
    );
    assert_eq!(deserialized.host, proxy.host, "Host should match");
    assert_eq!(deserialized.port, proxy.port, "Port should match");
  }

  #[test]
  fn clearcote_is_a_chromium_browser_with_profile_and_proxy_flags() {
    let browser = ClearcoteBrowser::new();
    let proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
      vless_uri: None,
    };
    let args = browser
      .create_launch_args(
        "C:\\profiles\\one",
        Some(&proxy),
        Some("https://example.com".into()),
        Some(9222),
        true,
      )
      .unwrap();
    assert!(
      args
        .iter()
        .any(|arg| arg == "--user-data-dir=C:\\profiles\\one")
    );
    assert!(
      args
        .iter()
        .any(|arg| arg == "--proxy-server=http://127.0.0.1:8080")
    );
    assert!(args.iter().any(|arg| arg == "--remote-debugging-port=9222"));
    assert!(args.iter().any(|arg| arg == "--headless=new"));
  }

  #[test]
  fn test_camoufox_config_has_no_executable_path() {
    // Verify CamoufoxConfig does not store executable_path
    let config = crate::camoufox_manager::CamoufoxConfig::default();
    let json = serde_json::to_value(&config).unwrap();
    assert!(
      json.get("executable_path").is_none(),
      "CamoufoxConfig should not have executable_path field"
    );
  }

  #[test]
  fn test_profile_data_path_is_dynamic() {
    use crate::profile::BrowserProfile;
    let profiles_dir = std::path::PathBuf::from("/fake/profiles");
    let profile = BrowserProfile {
      id: uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap(),
      name: "test".to_string(),
      browser: "camoufox".to_string(),
      version: "1.0.0".to_string(),
      proxy_id: None,
      vpn_id: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: None,
      browser_config: None,
      group_id: None,
      tags: Vec::new(),
      note: None,
      window_color: None,
      encryption_salt: None,
      host_os: None,
      password_protected: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      dns_blocklist: None,
      created_at: None,
      updated_at: None,
    };

    let path = profile.get_profile_data_path(&profiles_dir);
    assert_eq!(
      path,
      profiles_dir
        .join("12345678-1234-1234-1234-123456789abc")
        .join("profile")
    );
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_FACTORY: BrowserFactory = BrowserFactory::new();
}
