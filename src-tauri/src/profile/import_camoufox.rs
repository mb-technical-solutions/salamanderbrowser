//! Manually import an existing Camoufox (Firefox-family) profile directory —
//! e.g. one saved via the upstream Python `camoufox` library — as a new
//! Salamander profile.
//!
//! Unlike the old Chromium multi-browser importer (removed along with
//! Wayfern), this needs no OS-keychain secret re-sealing: Firefox's
//! cookie/login secrets (`key4.db`) are not OS-keychain-tied the way
//! Chromium's `os_crypt` is, so a same-machine profile's files are already
//! readable as-is. This is a plain copy into the new profile's data
//! directory, followed by the normal Camoufox profile-creation flow
//! (generates a fresh fingerprint unless `fingerprint_json` is supplied —
//! e.g. one saved separately from the same Python session via
//! `camoufox.generate_data()` / `json.dumps()`, since Camoufox never
//! persists its fingerprint config inside the profile directory itself).

use crate::camoufox_manager::CamoufoxConfig;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::types::BrowserProfile;
use crate::profile::ProfileManager;
use std::fs::{self, create_dir_all};
use std::path::Path;

/// Convert an importer error into the structured `{"code": …}` string the
/// frontend translates. Errors that are already structured pass through.
fn error_to_code_string(e: Box<dyn std::error::Error>) -> String {
  let msg = e.to_string();
  if msg.starts_with('{') {
    msg
  } else {
    serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": msg } }).to_string()
  }
}

/// Whether `dir` looks like a Gecko (Firefox/Camoufox) profile directory. Any
/// one marker alone can appear elsewhere; together they are conclusive.
fn looks_like_firefox_profile(dir: &Path) -> bool {
  let markers = ["prefs.js", "places.sqlite", "cookies.sqlite", "key4.db"];
  markers.iter().filter(|m| dir.join(m).exists()).count() >= 2
}

fn get_default_camoufox_version() -> Result<String, Box<dyn std::error::Error>> {
  let downloaded_versions =
    DownloadedBrowsersRegistry::instance().get_downloaded_versions("camoufox");

  if let Some(version) = downloaded_versions.first() {
    return Ok(version.clone());
  }

  Err(
    serde_json::json!({
      "code": "BROWSER_NOT_DOWNLOADED",
      "params": { "browser": "Camoufox" }
    })
    .to_string()
    .into(),
  )
}

fn copy_directory_recursive(
  source: &Path,
  destination: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
  if !destination.exists() {
    create_dir_all(destination)?;
  }

  for entry in fs::read_dir(source)? {
    let entry = entry?;
    let file_type = entry.file_type()?;
    let source_path = entry.path();
    let dest_path = destination.join(entry.file_name());

    if file_type.is_symlink() {
      return Err(format!(
        "Camoufox profile contains an unsupported symlink: {}",
        source_path.display()
      )
      .into());
    }

    if file_type.is_dir() {
      copy_directory_recursive(&source_path, &dest_path)?;
    } else if file_type.is_file() {
      fs::copy(&source_path, &dest_path)?;
    }
  }

  Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_camoufox_profile_impl(
  app_handle: &tauri::AppHandle,
  source_path: &str,
  new_profile_name: &str,
  proxy_id: Option<String>,
  vpn_id: Option<String>,
  group_id: Option<String>,
  fingerprint_json: Option<String>,
) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
  let profile_manager = ProfileManager::instance();

  let source = Path::new(source_path)
    .canonicalize()
    .map_err(|e| format!("Failed to resolve Camoufox profile path: {e}"))?;
  if !source.is_dir() {
    return Err(
      serde_json::json!({ "code": "IMPORT_SOURCE_NOT_FOUND" })
        .to_string()
        .into(),
    );
  }

  if !looks_like_firefox_profile(&source) {
    return Err(
      serde_json::json!({ "code": "CAMOUFOX_IMPORT_INVALID_FOLDER" })
        .to_string()
        .into(),
    );
  }

  let existing_profiles = profile_manager.list_profiles()?;
  if existing_profiles
    .iter()
    .any(|p| p.name.to_lowercase() == new_profile_name.to_lowercase())
  {
    return Err(
      serde_json::json!({ "code": "PROFILE_NAME_EXISTS", "params": { "name": new_profile_name } })
        .to_string()
        .into(),
    );
  }

  let version = get_default_camoufox_version()?;

  let camoufox_config = CamoufoxConfig {
    fingerprint: fingerprint_json,
    ..CamoufoxConfig::default()
  };

  // Creates the profile's on-disk directories (including an empty
  // `profile/` data dir) and generates a fingerprint if none was supplied.
  let profile = profile_manager
    .create_profile_with_group(
      app_handle,
      new_profile_name,
      "camoufox",
      &version,
      "stable",
      proxy_id,
      vpn_id,
      Some(camoufox_config),
      group_id,
      false,
      None,
      None,
    )
    .await?;

  // Replace the freshly-created empty profile dir's contents with the
  // imported browsing data (cookies, history, extensions, logins).
  let profiles_dir = profile_manager.get_profiles_dir();
  let dest_dir = profile.get_profile_data_path(&profiles_dir);
  if source == dest_dir.canonicalize().unwrap_or_else(|_| dest_dir.clone()) {
    return Err(
      serde_json::json!({ "code": "CAMOUFOX_IMPORT_INVALID_FOLDER" })
        .to_string()
        .into(),
    );
  }

  if let Err(e) = copy_directory_recursive(&source, &dest_dir) {
    // The profile metadata already exists and is otherwise valid — leave it
    // in place (with an empty data dir) rather than deleting a profile the
    // user can still see and retry the import into, and surface exactly
    // what went wrong copying the source.
    log::error!(
      "Camoufox profile '{new_profile_name}' created, but copying imported data from '{}' failed: {e}",
      source.display()
    );
    return Err(
      serde_json::json!({
        "code": "CAMOUFOX_IMPORT_COPY_FAILED",
        "params": { "detail": e.to_string() }
      })
      .to_string()
      .into(),
    );
  }

  log::info!(
    "Imported Camoufox profile '{new_profile_name}' from '{}'",
    source.display()
  );

  if let Err(e) = events::emit_empty("profiles-changed") {
    log::warn!("Warning: Failed to emit profiles-changed event: {e}");
  }

  Ok(profile)
}

#[tauri::command]
pub async fn import_camoufox_profile(
  app_handle: tauri::AppHandle,
  source_path: String,
  new_profile_name: String,
  proxy_id: Option<String>,
  vpn_id: Option<String>,
  group_id: Option<String>,
  fingerprint_json: Option<String>,
) -> Result<BrowserProfile, String> {
  import_camoufox_profile_impl(
    &app_handle,
    &source_path,
    &new_profile_name,
    proxy_id,
    vpn_id,
    group_id,
    fingerprint_json,
  )
  .await
  .map_err(error_to_code_string)
}
