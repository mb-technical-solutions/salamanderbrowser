//! Export one or more profiles to a single portable, password-protected
//! archive (`.salamander-profile`) and import them back — e.g. to move
//! profiles to another machine, or keep an offline backup. Always a
//! "bundle" of 1..N profiles so a single profile and a batch of profiles
//! share one format, one command pair, and one mental model.
//!
//! ## Two independent encryption layers
//! 1. **Archive transit password** (this module, mandatory on export —
//!    `export_profiles_archive` rejects a missing or under-length password
//!    with `PASSWORD_TOO_SHORT`, since the archive is meant to leave the
//!    machine carrying a fingerprint, cookies and extensions): protects the
//!    exported FILE in transit/at rest. Uses the same AES-256-GCM + Argon2id
//!    primitives as local profile password-protection (`profile::encryption`),
//!    wrapping the whole zip instead of per-file. `import_profiles_archive`
//!    still accepts `None` so archives exported before this requirement
//!    existed keep importing.
//! 2. **Profile at-rest password** (`profile::password`, unchanged, separate
//!    and optional): if a source profile is already password-protected, its
//!    `profile/` directory is already encrypted on disk. Export zips those
//!    bytes as-is and carries `encryption_salt`/`password_protected` in that
//!    profile's manifest — after import it's still locked with its
//!    *original* password, independent of the mandatory archive password
//!    used for transit.
//!
//! ## File format
//! `"SALAMPROF"(9B) | format_version(1B) | encrypted_flag(1B)`, then either
//! the raw zip bytes (unencrypted — only ever produced in tests) or
//! `salt_len(2B-LE) | salt | nonce(12B) | AES-256-GCM(zip bytes)` (encrypted;
//! nonce is part of `encrypt_bytes`'s own output format).
//!
//! ## Zip contents
//! - `bundle.json` — `BundleManifest`: format version, app version, export
//!   timestamp, and how many profiles follow.
//! - `profiles/{i}/manifest.json` — profile metadata for profile `i`, see
//!   `ArchiveManifest`.
//! - `profiles/{i}/profile/...` — recursive copy of that profile's data
//!   directory, verbatim (already-encrypted bytes, if any, stay encrypted).
//! - `profiles/{i}/extensions/{n}/{file_name}` — payload for every non-linked
//!   extension in that profile's extension group. Linked extensions point at
//!   a machine-local folder (`linked_path`) and can never be bundled — they
//!   are reported back as a warning instead of silently dropped.
//!
//! Proxy/VPN are reference IDs local to this machine, so they are dropped on
//! export unless the caller opts in, in which case the resolved settings
//! (not just the ID) are embedded so the destination machine can recreate
//! them.

use crate::extension_manager::{Extension, ExtensionManager};
use crate::profile::encryption::{decrypt_bytes, derive_profile_key, encrypt_bytes, generate_salt};
use crate::profile::manager::ProfileManager;
use crate::profile::types::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 9] = b"SALAMPROF";
const FORMAT_VERSION: u8 = 2;
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn code(c: &str) -> String {
  serde_json::json!({ "code": c }).to_string()
}

fn code_detail(c: &str, detail: impl std::fmt::Display) -> String {
  serde_json::json!({ "code": c, "params": { "detail": detail.to_string() } }).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BundleManifest {
  format_version: u8,
  app_version: String,
  exported_at: u64,
  profile_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivedExtension {
  name: String,
  file_name: String,
  file_type: String,
  #[serde(default)]
  version: Option<String>,
  #[serde(default)]
  description: Option<String>,
  #[serde(default)]
  author: Option<String>,
  #[serde(default)]
  homepage_url: Option<String>,
  source_kind: String,
  /// Index into `profiles/{i}/extensions/{zip_index}/{file_name}` inside the zip.
  zip_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivedVpn {
  name: String,
  vpn_type: crate::vpn::VpnType,
  config_data: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ArchivedNetwork {
  #[serde(default)]
  proxy: Option<crate::browser::ProxySettings>,
  #[serde(default)]
  vpn: Option<ArchivedVpn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchiveManifest {
  original_id: String,
  name: String,
  browser: String,
  version: String,
  release_type: String,
  #[serde(default)]
  camoufox_config: Option<crate::camoufox_manager::CamoufoxConfig>,
  #[serde(default)]
  tags: Vec<String>,
  #[serde(default)]
  note: Option<String>,
  #[serde(default)]
  window_color: Option<String>,
  #[serde(default)]
  encryption_salt: Option<String>,
  #[serde(default)]
  password_protected: bool,
  #[serde(default)]
  host_os: Option<String>,
  #[serde(default)]
  proxy_bypass_rules: Vec<String>,
  #[serde(default)]
  dns_blocklist: Option<String>,
  #[serde(default)]
  clear_on_close: bool,
  #[serde(default)]
  extensions: Vec<ArchivedExtension>,
  #[serde(default)]
  network: ArchivedNetwork,
}

/// Non-fatal issues surfaced to the caller after a successful export, for one
/// profile within the bundle.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ExportWarnings {
  /// Names of extensions that were skipped because they are loaded in place
  /// from a machine-local folder (`linked_path`) and can't be bundled.
  pub skipped_linked_extensions: Vec<String>,
}

/// An [`ExportWarnings`] tagged with which profile in the bundle it came from.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileExportWarning {
  pub profile_name: String,
  pub skipped_linked_extensions: Vec<String>,
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

fn copy_dir_into_zip<W: Write + std::io::Seek>(
  zip: &mut zip::ZipWriter<W>,
  options: zip::write::FileOptions<'_, ()>,
  base: &Path,
  current: &Path,
  zip_root: &str,
) -> Result<(), String> {
  if !current.exists() {
    return Ok(());
  }
  for entry in fs::read_dir(current).map_err(|e| code_detail("INTERNAL_ERROR", e))? {
    let entry = entry.map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    let file_type = entry
      .file_type()
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    let path = entry.path();
    if file_type.is_symlink() {
      return Err(code("ARCHIVE_UNSUPPORTED_SYMLINK"));
    }
    if file_type.is_dir() {
      copy_dir_into_zip(zip, options, base, &path, zip_root)?;
      continue;
    }
    if !file_type.is_file() {
      continue;
    }
    let rel = path
      .strip_prefix(base)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?
      .to_string_lossy()
      .replace('\\', "/");
    let bytes = fs::read(&path).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    zip
      .start_file(format!("{zip_root}/{rel}"), options)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    zip
      .write_all(&bytes)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  }
  Ok(())
}

/// Write one profile's manifest, data directory and bundled extensions into
/// `zip` under `profiles/{index}/...`.
fn write_profile_entry<W: Write + std::io::Seek>(
  zip: &mut zip::ZipWriter<W>,
  options: zip::write::FileOptions<'_, ()>,
  index: usize,
  profile: &BrowserProfile,
  profiles_dir: &Path,
  include_proxy_credentials: bool,
  include_vpn_credentials: bool,
) -> Result<ExportWarnings, String> {
  let mut warnings = ExportWarnings::default();
  let prefix = format!("profiles/{index}");

  // Resolve the profile's extension group; bundle non-linked extensions only.
  let mut archived_extensions = Vec::new();
  let mut payloads: Vec<(String, Extension)> = Vec::new();
  if let Some(group_id) = &profile.extension_group_id {
    let mgr = ExtensionManager::new();
    let groups = mgr.list_groups().unwrap_or_default();
    if let Some(group) = groups.into_iter().find(|g| &g.id == group_id) {
      let all_extensions = mgr.list_extensions().unwrap_or_default();
      for ext_id in &group.extension_ids {
        if let Some(ext) = all_extensions.iter().find(|e| &e.id == ext_id) {
          if ext.linked_path.is_some() {
            warnings.skipped_linked_extensions.push(ext.name.clone());
            continue;
          }
          payloads.push((ext_id.clone(), ext.clone()));
        }
      }
    }
  }
  for (idx, (_, ext)) in payloads.iter().enumerate() {
    archived_extensions.push(ArchivedExtension {
      name: ext.name.clone(),
      file_name: ext.file_name.clone(),
      file_type: ext.file_type.clone(),
      version: ext.version.clone(),
      description: ext.description.clone(),
      author: ext.author.clone(),
      homepage_url: ext.homepage_url.clone(),
      source_kind: ext.source_kind.clone(),
      zip_index: idx,
    });
  }

  // Network credentials, opt-in only and independently so — a proxy/VPN *ID*
  // is meaningless on another machine, so the resolved settings are embedded
  // instead. Proxy and VPN are asked about separately: carrying a VPN along
  // with a profile is a materially different decision (it changes what
  // network the destination machine's traffic exits through) from carrying a
  // proxy, so bundling them into one checkbox hid that choice from the user.
  let mut network = ArchivedNetwork::default();
  if include_proxy_credentials {
    if let Some(proxy_id) = &profile.proxy_id {
      network.proxy = crate::proxy_manager::PROXY_MANAGER.get_proxy_settings_by_id(proxy_id);
    }
  }
  if include_vpn_credentials {
    if let Some(vpn_id) = &profile.vpn_id {
      if let Ok(storage) = crate::vpn::VPN_STORAGE.lock() {
        if let Ok(cfg) = storage.load_config(vpn_id) {
          network.vpn = Some(ArchivedVpn {
            name: cfg.name,
            vpn_type: cfg.vpn_type,
            config_data: cfg.config_data,
          });
        }
      }
    }
  }

  let manifest = ArchiveManifest {
    original_id: profile.id.to_string(),
    name: profile.name.clone(),
    browser: profile.browser.clone(),
    version: profile.version.clone(),
    release_type: profile.release_type.clone(),
    camoufox_config: profile.camoufox_config.clone(),
    tags: profile.tags.clone(),
    note: profile.note.clone(),
    window_color: profile.window_color.clone(),
    encryption_salt: profile.encryption_salt.clone(),
    password_protected: profile.password_protected,
    host_os: profile.host_os.clone(),
    proxy_bypass_rules: profile.proxy_bypass_rules.clone(),
    dns_blocklist: profile.dns_blocklist.clone(),
    clear_on_close: profile.clear_on_close,
    extensions: archived_extensions,
    network,
  };

  zip
    .start_file(format!("{prefix}/manifest.json"), options)
    .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  let manifest_bytes =
    serde_json::to_vec_pretty(&manifest).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  zip
    .write_all(&manifest_bytes)
    .map_err(|e| code_detail("INTERNAL_ERROR", e))?;

  let data_dir = profile.get_profile_data_path(profiles_dir);
  copy_dir_into_zip(zip, options, &data_dir, &data_dir, &format!("{prefix}/profile"))?;

  let mgr = ExtensionManager::new();
  for (idx, (ext_id, ext)) in payloads.iter().enumerate() {
    let file_path = mgr.get_file_dir_public(ext_id).join(&ext.file_name);
    let bytes = fs::read(&file_path).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    zip
      .start_file(
        format!("{prefix}/extensions/{idx}/{}", ext.file_name),
        options,
      )
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    zip
      .write_all(&bytes)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  }

  Ok(warnings)
}

/// Build a bundle zip (before the envelope/encryption layer) containing every
/// profile in `profiles`, in order.
fn build_bundle_zip(
  profiles: &[BrowserProfile],
  profiles_dir: &Path,
  include_proxy_credentials: bool,
  include_vpn_credentials: bool,
) -> Result<(Vec<u8>, Vec<ProfileExportWarning>), String> {
  let bundle_manifest = BundleManifest {
    format_version: FORMAT_VERSION,
    app_version: env!("CARGO_PKG_VERSION").to_string(),
    exported_at: now_secs(),
    profile_count: profiles.len(),
  };

  let mut buf = Vec::new();
  let mut warnings = Vec::new();
  {
    let cursor = std::io::Cursor::new(&mut buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let options: zip::write::FileOptions<'_, ()> =
      zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip
      .start_file("bundle.json", options)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    let bundle_bytes =
      serde_json::to_vec_pretty(&bundle_manifest).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    zip
      .write_all(&bundle_bytes)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;

    for (index, profile) in profiles.iter().enumerate() {
      let w = write_profile_entry(
        &mut zip,
        options,
        index,
        profile,
        profiles_dir,
        include_proxy_credentials,
        include_vpn_credentials,
      )?;
      if !w.skipped_linked_extensions.is_empty() {
        warnings.push(ProfileExportWarning {
          profile_name: profile.name.clone(),
          skipped_linked_extensions: w.skipped_linked_extensions,
        });
      }
    }

    zip.finish().map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  }

  Ok((buf, warnings))
}

/// Wrap zip bytes in the archive envelope, encrypting with `archive_password`
/// if one is given.
fn wrap_envelope(zip_bytes: &[u8], archive_password: Option<&str>) -> Result<Vec<u8>, String> {
  let mut out = Vec::with_capacity(zip_bytes.len() + 64);
  out.extend_from_slice(MAGIC);
  out.push(FORMAT_VERSION);

  match archive_password.filter(|p| !p.is_empty()) {
    Some(password) => {
      out.push(1); // encrypted
      let salt = generate_salt();
      let key =
        derive_profile_key(password, &salt).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
      let encrypted =
        encrypt_bytes(&key, zip_bytes).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
      let salt_bytes = salt.as_bytes();
      out.extend_from_slice(&(salt_bytes.len() as u16).to_le_bytes());
      out.extend_from_slice(salt_bytes);
      out.extend_from_slice(&encrypted);
    }
    None => {
      out.push(0); // not encrypted
      out.extend_from_slice(zip_bytes);
    }
  }

  Ok(out)
}

/// Unwrap the archive envelope back into plain zip bytes.
fn unwrap_envelope(data: &[u8], archive_password: Option<&str>) -> Result<Vec<u8>, String> {
  if data.len() < MAGIC.len() + 2 || &data[..MAGIC.len()] != MAGIC {
    return Err(code("ARCHIVE_CORRUPT"));
  }
  let mut pos = MAGIC.len();
  let version = data[pos];
  pos += 1;
  if version != FORMAT_VERSION {
    return Err(code("ARCHIVE_UNSUPPORTED_VERSION"));
  }
  let encrypted = data[pos];
  pos += 1;

  if encrypted == 0 {
    return Ok(data[pos..].to_vec());
  }

  let Some(password) = archive_password.filter(|p| !p.is_empty()) else {
    return Err(code("ARCHIVE_PASSWORD_REQUIRED"));
  };

  if data.len() < pos + 2 {
    return Err(code("ARCHIVE_CORRUPT"));
  }
  let salt_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
  pos += 2;
  if data.len() < pos + salt_len {
    return Err(code("ARCHIVE_CORRUPT"));
  }
  let salt =
    std::str::from_utf8(&data[pos..pos + salt_len]).map_err(|_| code("ARCHIVE_CORRUPT"))?;
  pos += salt_len;

  let key = derive_profile_key(password, salt).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  decrypt_bytes(&key, &data[pos..]).map_err(|_| code("ARCHIVE_WRONG_PASSWORD"))
}

/// Classification of an arbitrary filesystem path dropped or picked for
/// import, so the frontend can route it to the right import flow (and batch
/// multiple archive files together) without the user having to pick a mode
/// first.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportSourceInfo {
  /// A `.salamander-profile` archive (a bundle of one or more profiles).
  /// `profile_count`/`preview_names` are only populated when the archive
  /// isn't password-protected (reading it requires the password otherwise).
  Archive {
    needs_password: bool,
    profile_count: Option<usize>,
    preview_names: Vec<String>,
  },
  /// A raw Camoufox profile folder (has `metadata.json` at its root).
  ProfileFolder,
  /// A whole previous installation's data directory (has `profiles/`,
  /// `proxies/`, `vpn/`, etc. at its root).
  DataDirectory,
  Unknown,
}

fn inspect_archive_bytes(data: &[u8]) -> ImportSourceInfo {
  if data.len() < MAGIC.len() + 2 || &data[..MAGIC.len()] != MAGIC {
    return ImportSourceInfo::Unknown;
  }
  let version = data[MAGIC.len()];
  if version != FORMAT_VERSION {
    // Still report it as an archive so the UI routes it to the archive-import
    // flow, which already returns a clear ARCHIVE_UNSUPPORTED_VERSION error.
    return ImportSourceInfo::Archive {
      needs_password: false,
      profile_count: None,
      preview_names: Vec::new(),
    };
  }
  let encrypted = data[MAGIC.len() + 1];
  if encrypted == 1 {
    return ImportSourceInfo::Archive {
      needs_password: true,
      profile_count: None,
      preview_names: Vec::new(),
    };
  }

  let zip_bytes = &data[MAGIC.len() + 2..];
  let (profile_count, preview_names) = (|| -> Option<(usize, Vec<String>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).ok()?;
    let bundle: BundleManifest = {
      let mut entry = archive.by_name("bundle.json").ok()?;
      let mut bytes = Vec::new();
      entry.read_to_end(&mut bytes).ok()?;
      drop(entry);
      serde_json::from_slice(&bytes).ok()?
    };

    let mut names = Vec::with_capacity(bundle.profile_count);
    for i in 0..bundle.profile_count {
      let name = (|| -> Option<String> {
        let mut entry = archive.by_name(&format!("profiles/{i}/manifest.json")).ok()?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).ok()?;
        drop(entry);
        let manifest: ArchiveManifest = serde_json::from_slice(&bytes).ok()?;
        Some(manifest.name)
      })();
      if let Some(name) = name {
        names.push(name);
      }
    }
    Some((bundle.profile_count, names))
  })()
  .unwrap_or((0, Vec::new()));

  ImportSourceInfo::Archive {
    needs_password: false,
    profile_count: Some(profile_count),
    preview_names,
  }
}

/// Classify a path dropped onto the window or picked via a file/folder
/// browser, so the import UI can route it automatically instead of requiring
/// the user to choose a mode up front.
#[tauri::command]
pub async fn inspect_import_source(path: String) -> Result<ImportSourceInfo, String> {
  let p = std::path::PathBuf::from(&path);

  if p.is_file() {
    if p.extension().and_then(|e| e.to_str()) != Some("salamander-profile") {
      return Ok(ImportSourceInfo::Unknown);
    }
    let bytes = tokio::fs::read(&p)
      .await
      .map_err(|e| code_detail("IMPORT_SOURCE_NOT_FOUND", e))?;
    return Ok(inspect_archive_bytes(&bytes));
  }

  if p.is_dir() {
    if p.join("metadata.json").is_file() {
      return Ok(ImportSourceInfo::ProfileFolder);
    }
    let looks_like_data_dir = [
      "profiles",
      "proxies",
      "vpn",
      "settings",
      "binaries",
      "extensions",
    ]
    .iter()
    .any(|d| p.join(d).is_dir());
    if looks_like_data_dir {
      return Ok(ImportSourceInfo::DataDirectory);
    }
  }

  Ok(ImportSourceInfo::Unknown)
}

/// Archives leave the machine (another PC, a backup drive, cloud storage a
/// user might sync to) carrying a fingerprint, cookies and extensions, so a
/// password is mandatory rather than opt-in — the same minimum length as
/// local profile at-rest passwords (`profile::password::MIN_PASSWORD_LEN`).
fn validate_archive_password(archive_password: &str) -> Result<(), String> {
  if archive_password.len() < crate::profile::password::MIN_PASSWORD_LEN {
    return Err(
      serde_json::json!({
        "code": "PASSWORD_TOO_SHORT",
        "params": { "min": crate::profile::password::MIN_PASSWORD_LEN.to_string() }
      })
      .to_string(),
    );
  }
  Ok(())
}

/// Export one or more profiles into a single password-protected bundle.
#[tauri::command]
pub async fn export_profiles_archive(
  profile_ids: Vec<String>,
  archive_password: String,
  include_proxy_credentials: bool,
  include_vpn_credentials: bool,
  output_path: String,
) -> Result<Vec<ProfileExportWarning>, String> {
  validate_archive_password(&archive_password)?;

  let profile_manager = ProfileManager::instance();
  let all_profiles = profile_manager
    .list_profiles()
    .map_err(|e| code_detail("INTERNAL_ERROR", e))?;

  let mut selected = Vec::with_capacity(profile_ids.len());
  for id in &profile_ids {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| code("PROFILE_NOT_FOUND"))?;
    let profile = all_profiles
      .iter()
      .find(|p| p.id == uuid)
      .cloned()
      .ok_or_else(|| code("PROFILE_NOT_FOUND"))?;
    selected.push(profile);
  }

  let profile_count = selected.len();
  let profiles_dir = profile_manager.get_profiles_dir();
  let (zip_bytes, warnings) = tokio::task::spawn_blocking(move || {
    build_bundle_zip(
      &selected,
      &profiles_dir,
      include_proxy_credentials,
      include_vpn_credentials,
    )
  })
  .await
  .map_err(|e| code_detail("INTERNAL_ERROR", e))??;

  let archive_bytes = wrap_envelope(&zip_bytes, Some(&archive_password))?;

  tokio::fs::write(&output_path, archive_bytes)
    .await
    .map_err(|e| code_detail("EXPORT_WRITE_FAILED", e))?;

  log::info!("Exported {profile_count} profile(s) to {output_path}");
  Ok(warnings)
}

fn extract_dir_from_zip<R: Read + std::io::Seek>(
  archive: &mut zip::ZipArchive<R>,
  zip_prefix: &str,
  dest_dir: &Path,
  total_written: &mut u64,
) -> Result<(), String> {
  fs::create_dir_all(dest_dir).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  for i in 0..archive.len() {
    let mut entry = archive
      .by_index(i)
      .map_err(|e| code_detail("ARCHIVE_CORRUPT", e))?;
    let name = entry.name().to_string();
    let Some(rel) = name.strip_prefix(zip_prefix) else {
      continue;
    };
    if rel.is_empty() || entry.is_dir() {
      continue;
    }
    let rel_path = Path::new(rel);
    if rel_path.components().any(|component| {
      matches!(
        component,
        std::path::Component::Prefix(_)
          | std::path::Component::RootDir
          | std::path::Component::ParentDir
      )
    }) {
      return Err(code("ARCHIVE_INVALID_PATH"));
    }

    let entry_size = entry.size();
    if entry_size > MAX_ARCHIVE_ENTRY_BYTES
      || (*total_written).saturating_add(entry_size) > MAX_ARCHIVE_TOTAL_BYTES
    {
      return Err(code("ARCHIVE_TOO_LARGE"));
    }

    let dest = dest_dir.join(rel_path);
    let normalized_dest = dest
      .components()
      .collect::<std::path::PathBuf>();
    if !normalized_dest.starts_with(dest_dir) {
      return Err(code("ARCHIVE_INVALID_PATH"));
    }
    if let Some(parent) = dest.parent() {
      fs::create_dir_all(parent).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
    }
    let mut bytes = Vec::new();
    entry
      .read_to_end(&mut bytes)
      .map_err(|e| code_detail("ARCHIVE_CORRUPT", e))?;
    *total_written = (*total_written).saturating_add(bytes.len() as u64);
    fs::write(&dest, &bytes).map_err(|e| code_detail("INTERNAL_ERROR", e))?;
  }
  Ok(())
}

struct PendingImport {
  manifest: ArchiveManifest,
  temp_dir: tempfile::TempDir,
}

/// Import every profile bundled in an archive exported by
/// `export_profiles_archive` (including a bundle of exactly one profile).
#[tauri::command]
pub async fn import_profiles_archive(
  app_handle: tauri::AppHandle,
  archive_path: String,
  archive_password: Option<String>,
) -> Result<Vec<BrowserProfile>, String> {
  let raw = tokio::fs::read(&archive_path)
    .await
    .map_err(|e| code_detail("IMPORT_SOURCE_NOT_FOUND", e))?;

  let zip_bytes = unwrap_envelope(&raw, archive_password.as_deref())?;

  let profile_manager = ProfileManager::instance();
  let profiles_dir = profile_manager.get_profiles_dir();
  let mut existing_names: Vec<String> = profile_manager
    .list_profiles()
    .map_err(|e| code_detail("INTERNAL_ERROR", e))?
    .into_iter()
    .map(|p| p.name)
    .collect();

  // Parsing + extraction is sync/blocking; do it off the async executor.
  let pending: Vec<PendingImport> = tokio::task::spawn_blocking(move || -> Result<_, String> {
    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive =
      zip::ZipArchive::new(cursor).map_err(|e| code_detail("ARCHIVE_CORRUPT", e))?;

    let bundle: BundleManifest = {
      let mut entry = archive
        .by_name("bundle.json")
        .map_err(|_| code("ARCHIVE_CORRUPT"))?;
      let mut bytes = Vec::new();
      entry
        .read_to_end(&mut bytes)
        .map_err(|e| code_detail("ARCHIVE_CORRUPT", e))?;
      drop(entry);
      serde_json::from_slice(&bytes).map_err(|_| code("ARCHIVE_CORRUPT"))?
    };

    if bundle.format_version > FORMAT_VERSION {
      return Err(code("ARCHIVE_UNSUPPORTED_VERSION"));
    }

    let mut result = Vec::with_capacity(bundle.profile_count);
    for index in 0..bundle.profile_count {
      let prefix = format!("profiles/{index}");
      let manifest: ArchiveManifest = {
        let mut entry = archive
          .by_name(&format!("{prefix}/manifest.json"))
          .map_err(|_| code("ARCHIVE_CORRUPT"))?;
        let mut bytes = Vec::new();
        entry
          .read_to_end(&mut bytes)
          .map_err(|e| code_detail("ARCHIVE_CORRUPT", e))?;
        drop(entry);
        serde_json::from_slice(&bytes).map_err(|_| code("ARCHIVE_CORRUPT"))?
      };

      let temp_dir = tempfile::tempdir().map_err(|e| code_detail("INTERNAL_ERROR", e))?;
      let mut total_written = 0;
      extract_dir_from_zip(
        &mut archive,
        &format!("{prefix}/profile/"),
        &temp_dir.path().join("profile"),
        &mut total_written,
      )?;
      for ext in &manifest.extensions {
        extract_dir_from_zip(
          &mut archive,
          &format!("{prefix}/extensions/{}/", ext.zip_index),
          &temp_dir.path().join(format!("ext-{}", ext.zip_index)),
          &mut total_written,
        )?;
      }

      result.push(PendingImport { manifest, temp_dir });
    }

    Ok(result)
  })
  .await
  .map_err(|e| code_detail("INTERNAL_ERROR", e))??;

  let mut imported = Vec::with_capacity(pending.len());
  for PendingImport { manifest, temp_dir } in pending {
    // Recreate proxy/VPN from bundled credentials, if any.
    let mut proxy_id = None;
    if let Some(proxy_settings) = manifest.network.proxy.clone() {
      let name = format!("{} (imported)", manifest.name);
      if let Ok(stored) =
        crate::proxy_manager::PROXY_MANAGER.create_stored_proxy(&app_handle, name, proxy_settings)
      {
        proxy_id = Some(stored.id);
      }
    }
    let mut vpn_id = None;
    if let Some(vpn) = manifest.network.vpn.clone() {
      if let Ok(storage) = crate::vpn::VPN_STORAGE.lock() {
        let name = format!("{} (imported)", manifest.name);
        if let Ok(cfg) = storage.create_config_manual(&name, vpn.vpn_type, &vpn.config_data) {
          vpn_id = Some(cfg.id);
        }
      }
    }

    // Resolve a non-colliding name. `existing_names` grows as we go, so two
    // profiles sharing a name within the SAME bundle also dedupe against
    // each other, not just against what was already here.
    let base_name = manifest.name.clone();
    let mut final_name = base_name.clone();
    let mut suffix = 2;
    while existing_names
      .iter()
      .any(|n| n.to_lowercase() == final_name.to_lowercase())
    {
      final_name = format!("{base_name} ({suffix})");
      suffix += 1;
    }
    existing_names.push(final_name.clone());

    let created = profile_manager
      .create_profile_with_group(
        &app_handle,
        &final_name,
        &manifest.browser,
        &manifest.version,
        &manifest.release_type,
        proxy_id,
        vpn_id,
        manifest.camoufox_config.clone(),
        None, // profile group membership is local organization, not portable
        false,
        manifest.dns_blocklist.clone(),
        None,
      )
      .await
      .map_err(|e| e.to_string())?;

    // Replace the freshly-created (empty) data dir with the archived one.
    let dest_data_dir = created.get_profile_data_path(&profiles_dir);
    let src_data_dir = temp_dir.path().join("profile");
    if src_data_dir.exists() {
      if dest_data_dir.exists() {
        let _ = fs::remove_dir_all(&dest_data_dir);
      }
      copy_directory_recursive(&src_data_dir, &dest_data_dir)
        .map_err(|e| code_detail("IMPORT_COPY_FAILED", e))?;
    }

    // Recreate extensions (fresh IDs) and, if any were bundled, a fresh group.
    let mut extension_group_id = None;
    if !manifest.extensions.is_empty() {
      let mgr = ExtensionManager::new();
      let mut new_ext_ids = Vec::new();
      for ext in &manifest.extensions {
        let ext_dir = temp_dir.path().join(format!("ext-{}", ext.zip_index));
        let file_path = ext_dir.join(&ext.file_name);
        if let Ok(bytes) = fs::read(&file_path) {
          if let Ok(created_ext) = mgr.add_extension(ext.name.clone(), ext.file_name.clone(), bytes) {
            new_ext_ids.push(created_ext.id);
          }
        }
      }
      if !new_ext_ids.is_empty() {
        if let Ok(group) = mgr.create_group(format!("{} (imported)", manifest.name)) {
          let _ = mgr.update_group(&group.id, None, Some(new_ext_ids));
          extension_group_id = Some(group.id);
        }
      }
    }

    // Apply remaining metadata in one save.
    let mut profile = created;
    profile.tags = manifest.tags.clone();
    profile.note = manifest.note.clone();
    profile.window_color = manifest.window_color.clone();
    profile.encryption_salt = manifest.encryption_salt.clone();
    profile.password_protected = manifest.password_protected;
    profile.host_os = manifest.host_os.clone();
    profile.proxy_bypass_rules = manifest.proxy_bypass_rules.clone();
    profile.clear_on_close = manifest.clear_on_close;
    profile.extension_group_id = extension_group_id;
    profile.updated_at = Some(now_secs());
    profile_manager
      .save_profile(&profile)
      .map_err(|e| code_detail("INTERNAL_ERROR", e))?;

    log::info!(
      "Imported profile '{}' from archive '{archive_path}'",
      profile.name
    );
    imported.push(profile);
  }

  if let Err(e) = crate::events::emit_empty("profiles-changed") {
    log::warn!("Failed to emit profiles-changed event: {e}");
  }

  Ok(imported)
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> std::io::Result<()> {
  if !destination.exists() {
    fs::create_dir_all(destination)?;
  }
  for entry in fs::read_dir(source)? {
    let entry = entry?;
    let src = entry.path();
    let dst = destination.join(entry.file_name());
    if src.is_dir() {
      copy_directory_recursive(&src, &dst)?;
    } else {
      fs::copy(&src, &dst)?;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::profile::types::get_host_os;

  fn sample_profile(id: uuid::Uuid, name: &str, data_dir_parent: &Path) -> BrowserProfile {
    let profile_dir = data_dir_parent.join(id.to_string()).join("profile");
    fs::create_dir_all(&profile_dir).unwrap();
    fs::write(profile_dir.join("prefs.js"), b"user_pref(1);").unwrap();

    BrowserProfile {
      id,
      name: name.to_string(),
      browser: "camoufox".to_string(),
      version: "135.0".to_string(),
      proxy_id: None,
      vpn_id: None,
      launch_hook: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: Some(crate::camoufox_manager::CamoufoxConfig {
        proxy: None,
        screen_max_width: None,
        screen_max_height: None,
        screen_min_width: None,
        screen_min_height: None,
        geoip: None,
        block_images: None,
        block_webrtc: None,
        block_webgl: None,
        fingerprint: Some("{\"fp\":true}".to_string()),
        randomize_fingerprint_on_launch: None,
        os: Some(get_host_os()),
        preset_tabs: None,
      }),
      group_id: None,
      tags: vec!["work".to_string()],
      note: Some("a note".to_string()),
      window_color: Some("#abcdef".to_string()),
      encryption_salt: None,
      host_os: Some(get_host_os()),
      ephemeral: false,
      extension_group_id: None,
      proxy_bypass_rules: vec![],
      dns_blocklist: None,
      password_protected: false,
      clear_on_close: false,
      created_at: Some(1),
      updated_at: Some(1),
    }
  }

  #[test]
  fn envelope_roundtrip_plain() {
    let zip = b"fake zip bytes".to_vec();
    let wrapped = wrap_envelope(&zip, None).unwrap();
    assert_eq!(&wrapped[..MAGIC.len()], MAGIC);
    let unwrapped = unwrap_envelope(&wrapped, None).unwrap();
    assert_eq!(unwrapped, zip);
  }

  #[test]
  fn envelope_roundtrip_encrypted() {
    let zip = b"fake zip bytes, but longer this time".to_vec();
    let wrapped = wrap_envelope(&zip, Some("hunter2")).unwrap();
    let unwrapped = unwrap_envelope(&wrapped, Some("hunter2")).unwrap();
    assert_eq!(unwrapped, zip);
  }

  #[test]
  fn envelope_wrong_password_is_rejected() {
    let zip = b"secret".to_vec();
    let wrapped = wrap_envelope(&zip, Some("right")).unwrap();
    let err = unwrap_envelope(&wrapped, Some("wrong")).unwrap_err();
    assert!(err.contains("ARCHIVE_WRONG_PASSWORD"));
  }

  #[test]
  fn envelope_missing_password_is_reported() {
    let zip = b"secret".to_vec();
    let wrapped = wrap_envelope(&zip, Some("right")).unwrap();
    let err = unwrap_envelope(&wrapped, None).unwrap_err();
    assert!(err.contains("ARCHIVE_PASSWORD_REQUIRED"));
  }

  #[test]
  fn envelope_corrupt_data_is_rejected() {
    let err = unwrap_envelope(b"not an archive", None).unwrap_err();
    assert!(err.contains("ARCHIVE_CORRUPT"));
  }

  #[test]
  fn build_bundle_zip_round_trips_a_single_profile() {
    let work = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    let profile = sample_profile(id, "Test Profile", work.path());

    let (zip_bytes, warnings) = build_bundle_zip(&[profile], work.path(), false, false).unwrap();
    assert!(warnings.is_empty());

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();

    let mut bundle_bytes = Vec::new();
    archive
      .by_name("bundle.json")
      .unwrap()
      .read_to_end(&mut bundle_bytes)
      .unwrap();
    let bundle: BundleManifest = serde_json::from_slice(&bundle_bytes).unwrap();
    assert_eq!(bundle.profile_count, 1);
    assert_eq!(bundle.format_version, FORMAT_VERSION);

    let mut manifest_bytes = Vec::new();
    archive
      .by_name("profiles/0/manifest.json")
      .unwrap()
      .read_to_end(&mut manifest_bytes)
      .unwrap();
    let manifest: ArchiveManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.name, "Test Profile");
    assert_eq!(manifest.tags, vec!["work".to_string()]);
    assert!(manifest.camoufox_config.is_some());
    assert!(manifest.network.proxy.is_none());

    let mut prefs = Vec::new();
    archive
      .by_name("profiles/0/profile/prefs.js")
      .unwrap()
      .read_to_end(&mut prefs)
      .unwrap();
    assert_eq!(prefs, b"user_pref(1);");
  }

  #[test]
  fn build_bundle_zip_bundles_multiple_profiles_independently() {
    let work = tempfile::tempdir().unwrap();
    let a = sample_profile(uuid::Uuid::new_v4(), "Alpha", work.path());
    let b = sample_profile(uuid::Uuid::new_v4(), "Bravo", work.path());

    let (zip_bytes, _) = build_bundle_zip(&[a, b], work.path(), false, false).unwrap();
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();

    let mut bundle_bytes = Vec::new();
    archive
      .by_name("bundle.json")
      .unwrap()
      .read_to_end(&mut bundle_bytes)
      .unwrap();
    let bundle: BundleManifest = serde_json::from_slice(&bundle_bytes).unwrap();
    assert_eq!(bundle.profile_count, 2);

    for (index, expected_name) in [(0, "Alpha"), (1, "Bravo")] {
      let mut manifest_bytes = Vec::new();
      archive
        .by_name(&format!("profiles/{index}/manifest.json"))
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
      let manifest: ArchiveManifest = serde_json::from_slice(&manifest_bytes).unwrap();
      assert_eq!(manifest.name, expected_name);
    }
  }

  #[test]
  fn inspect_archive_bytes_reports_profile_count_and_names_for_unencrypted_archive() {
    let work = tempfile::tempdir().unwrap();
    let a = sample_profile(uuid::Uuid::new_v4(), "Alpha", work.path());
    let b = sample_profile(uuid::Uuid::new_v4(), "Bravo", work.path());
    let (zip_bytes, _) = build_bundle_zip(&[a, b], work.path(), false, false).unwrap();
    let wrapped = wrap_envelope(&zip_bytes, None).unwrap();

    match inspect_archive_bytes(&wrapped) {
      ImportSourceInfo::Archive {
        needs_password,
        profile_count,
        preview_names,
      } => {
        assert!(!needs_password);
        assert_eq!(profile_count, Some(2));
        assert_eq!(preview_names, vec!["Alpha".to_string(), "Bravo".to_string()]);
      }
      other => panic!("expected Archive, got {other:?}"),
    }
  }

  #[test]
  fn inspect_archive_bytes_flags_password_protected_archive_without_decrypting() {
    let work = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    let profile = sample_profile(id, "Test Profile", work.path());
    let (zip_bytes, _) = build_bundle_zip(&[profile], work.path(), false, false).unwrap();
    let wrapped = wrap_envelope(&zip_bytes, Some("hunter2")).unwrap();

    match inspect_archive_bytes(&wrapped) {
      ImportSourceInfo::Archive {
        needs_password,
        profile_count,
        preview_names,
      } => {
        assert!(needs_password);
        assert!(profile_count.is_none());
        assert!(preview_names.is_empty());
      }
      other => panic!("expected Archive, got {other:?}"),
    }
  }

  #[test]
  fn inspect_archive_bytes_rejects_garbage() {
    assert!(matches!(
      inspect_archive_bytes(b"not an archive"),
      ImportSourceInfo::Unknown
    ));
  }

  #[tokio::test]
  async fn inspect_import_source_classifies_archive_file() {
    let work = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    let profile = sample_profile(id, "Test Profile", work.path());
    let (zip_bytes, _) = build_bundle_zip(&[profile], work.path(), false, false).unwrap();
    let wrapped = wrap_envelope(&zip_bytes, None).unwrap();
    let archive_path = work.path().join("export.salamander-profile");
    fs::write(&archive_path, &wrapped).unwrap();

    let info = inspect_import_source(archive_path.to_string_lossy().to_string())
      .await
      .unwrap();
    assert!(matches!(info, ImportSourceInfo::Archive { .. }));
  }

  #[tokio::test]
  async fn inspect_import_source_classifies_raw_profile_folder() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("metadata.json"), b"{}").unwrap();

    let info = inspect_import_source(work.path().to_string_lossy().to_string())
      .await
      .unwrap();
    assert!(matches!(info, ImportSourceInfo::ProfileFolder));
  }

  #[tokio::test]
  async fn inspect_import_source_classifies_whole_data_directory() {
    let work = tempfile::tempdir().unwrap();
    fs::create_dir_all(work.path().join("profiles")).unwrap();

    let info = inspect_import_source(work.path().to_string_lossy().to_string())
      .await
      .unwrap();
    assert!(matches!(info, ImportSourceInfo::DataDirectory));
  }

  #[tokio::test]
  async fn inspect_import_source_reports_unknown_for_unrelated_folder() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("notes.txt"), b"hello").unwrap();

    let info = inspect_import_source(work.path().to_string_lossy().to_string())
      .await
      .unwrap();
    assert!(matches!(info, ImportSourceInfo::Unknown));
  }

  #[tokio::test]
  async fn inspect_import_source_reports_unknown_for_nonexistent_path() {
    let info = inspect_import_source("/does/not/exist".to_string())
      .await
      .unwrap();
    assert!(matches!(info, ImportSourceInfo::Unknown));
  }

  #[test]
  fn validate_archive_password_rejects_empty_and_short_passwords() {
    let err = validate_archive_password("").unwrap_err();
    assert!(err.contains("PASSWORD_TOO_SHORT"));

    let err = validate_archive_password("short").unwrap_err();
    assert!(err.contains("PASSWORD_TOO_SHORT"));
    assert!(err.contains(&crate::profile::password::MIN_PASSWORD_LEN.to_string()));
  }

  #[test]
  fn validate_archive_password_accepts_a_password_at_the_minimum_length() {
    let password = "a".repeat(crate::profile::password::MIN_PASSWORD_LEN);
    assert!(validate_archive_password(&password).is_ok());
  }

  #[test]
  fn archive_extraction_rejects_parent_path_entries() {
    let mut bytes = Vec::new();
    {
      let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
      let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
      zip
        .start_file("profiles/0/profile/../../outside.txt", options)
        .unwrap();
      zip.write_all(b"should not escape").unwrap();
      zip.finish().unwrap();
    }

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut total_written = 0;
    let error = extract_dir_from_zip(
      &mut archive,
      "profiles/0/profile/",
      destination.path(),
      &mut total_written,
    )
    .unwrap_err();

    assert!(error.contains("ARCHIVE_INVALID_PATH"));
    assert!(!destination.path().join("../outside.txt").exists());
  }
}
