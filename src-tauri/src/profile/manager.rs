use crate::browser::{BrowserType, create_browser};
use crate::camoufox_manager::CamoufoxConfig;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::types::{BrowserProfile, get_host_os, is_host_os};
use crate::proxy_manager::PROXY_MANAGER;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

/// Derive a stable, deterministic pastel window-frame color from a profile's
/// id, so every profile gets a distinct, stable color with no stored state.
pub fn derive_profile_color(id: &uuid::Uuid) -> String {
  // FNV-1a over the 16 id bytes -> hue in [0,360). The hue varies per profile
  // while saturation/lightness are fixed to a pastel band (see below).
  let mut h: u32 = 2166136261;
  for &b in id.as_bytes() {
    h = (h ^ u32::from(b)).wrapping_mul(16777619);
  }
  let hue = f64::from(h % 360);
  // Pastel: high lightness + soft saturation so windows stay easy to tell apart
  // without a garish frame.
  let (r, g, b) = hsl_to_rgb(hue, 0.6, 0.8);
  format!("#{r:02x}{g:02x}{b:02x}")
}

/// Convert HSL (h in [0,360), s/l in [0,1]) to 8-bit RGB.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
  let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
  let hp = h / 60.0;
  let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
  let (r1, g1, b1) = match hp as i32 {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  let m = l - c / 2.0;
  let to_u8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
  (to_u8(r1), to_u8(g1), to_u8(b1))
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
    let source_path = entry.path();
    let dest_path = destination.join(entry.file_name());

    if source_path.is_dir() {
      copy_directory_recursive(&source_path, &dest_path)?;
    } else {
      fs::copy(&source_path, &dest_path)?;
    }
  }

  Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
  let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
    Some(ext) => format!("{ext}.tmp"),
    None => "tmp".to_string(),
  });
  {
    let mut f = fs::File::create(&tmp)?;
    use std::io::Write;
    f.write_all(data)?;
    f.sync_all()?;
  }
  fs::rename(&tmp, path)
}

pub struct ProfileManager {
  camoufox_manager: &'static crate::camoufox_manager::CamoufoxManager,
}

impl ProfileManager {
  fn new() -> Self {
    Self {
      camoufox_manager: crate::camoufox_manager::CamoufoxManager::instance(),
    }
  }

  pub fn instance() -> &'static ProfileManager {
    &PROFILE_MANAGER
  }

  pub fn get_profiles_dir(&self) -> PathBuf {
    crate::app_dirs::profiles_dir()
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    crate::app_dirs::binaries_dir()
  }

  #[allow(clippy::too_many_arguments)]
  pub async fn create_profile_with_group(
    &self,
    app_handle: &tauri::AppHandle,
    name: &str,
    browser: &str,
    version: &str,
    release_type: &str,
    proxy_id: Option<String>,
    vpn_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
    browser_config: Option<serde_json::Value>,
    group_id: Option<String>,
    dns_blocklist: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    if name.trim().is_empty() {
      return Err(
        serde_json::json!({ "code": "NAME_CANNOT_BE_EMPTY" })
          .to_string()
          .into(),
      );
    }

    if proxy_id.is_some() && vpn_id.is_some() {
      return Err("Cannot set both proxy_id and vpn_id".into());
    }

    log::info!("Attempting to create profile: {name}");

    // Check if a profile with this name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == name.to_lowercase())
    {
      return Err(format!("Profile with name '{name}' already exists").into());
    }

    // Generate a new UUID for this profile
    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let profile_data_dir = profile_uuid_dir.join("profile");
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Create profile directory with UUID and profile subdirectory
    create_dir_all(&profile_uuid_dir)?;
    create_dir_all(&profile_data_dir)?;

    // For Camoufox profiles, generate a fingerprint during creation
    let final_camoufox_config = if browser == "camoufox" {
      let mut config = camoufox_config.unwrap_or_else(|| {
        log::info!("Creating default Camoufox config for profile: {name}");
        crate::camoufox_manager::CamoufoxConfig::default()
      });

      if let Some(proxy_id_ref) = &proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
          let proxy_url = if let (Some(username), Some(password)) =
            (&proxy_settings.username, &proxy_settings.password)
          {
            format!(
              "{}://{}:{}@{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              username,
              password,
              proxy_settings.host,
              proxy_settings.port
            )
          } else {
            format!(
              "{}://{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              proxy_settings.host,
              proxy_settings.port
            )
          };
          config.proxy = Some(proxy_url);
        }
      }

      if config.fingerprint.is_none() {
        log::info!("Generating fingerprint for Camoufox profile: {name}");

        let temp_profile = BrowserProfile {
          id: uuid::Uuid::new_v4(),
          name: name.to_string(),
          browser: browser.to_string(),
          version: version.to_string(),
          proxy_id: proxy_id.clone(),
          vpn_id: None,
          process_id: None,
          last_launch: None,
          release_type: release_type.to_string(),
          camoufox_config: None,
          browser_config: None,
          group_id: group_id.clone(),
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

        match self
          .camoufox_manager
          .generate_fingerprint_config(app_handle, &temp_profile, &config)
          .await
        {
          Ok(generated) => {
            config.fingerprint = Some(generated);
            log::info!("Successfully generated fingerprint for Camoufox profile: {name}");
          }
          Err(e) => {
            return Err(
              format!("Failed to generate fingerprint for Camoufox profile '{name}': {e}").into(),
            );
          }
        }
      } else {
        log::info!("Using provided fingerprint for Camoufox profile: {name}");
      }

      config.proxy = None;

      Some(config)
    } else {
      camoufox_config.clone()
    };

    let profile = BrowserProfile {
      id: profile_id,
      name: name.to_string(),
      browser: browser.to_string(),
      version: version.to_string(),
      proxy_id: proxy_id.clone(),
      vpn_id: vpn_id.clone(),
      process_id: None,
      last_launch: None,
      release_type: release_type.to_string(),
      camoufox_config: final_camoufox_config,
      browser_config,
      group_id: group_id.clone(),
      tags: Vec::new(),
       note: None,
      // A random-looking pastel derived from the (random) profile id, so every
      // new profile gets a distinct, stable window color it can later override.
       window_color: Some(derive_profile_color(&profile_id)),
       encryption_salt: None,
       host_os: Some(get_host_os()),
       password_protected: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      dns_blocklist,
      created_at: Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .map(|d| d.as_secs())
          .unwrap_or(0),
      ),
      updated_at: Some(crate::proxy_manager::now_secs()),
    };

    // Save profile info
    self.save_profile(&profile)?;

    // Verify the profile was saved correctly
    if !profile_file.exists() {
      return Err(format!("Failed to create profile file for '{name}'").into());
    }

    log::info!("Profile '{name}' created successfully with ID: {profile_id}");

    // Emit profile creation event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn save_profile(&self, profile: &BrowserProfile) -> Result<(), Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Ensure the UUID directory exists
    create_dir_all(&profile_uuid_dir)?;

    let json = serde_json::to_string_pretty(profile)?;
    atomic_write(&profile_file, json.as_bytes())?;

    // Update tag suggestions after any save
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    Ok(())
  }

  pub fn list_profiles(&self) -> Result<Vec<BrowserProfile>, Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    if !profiles_dir.exists() {
      return Ok(vec![]);
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(profiles_dir)? {
      let entry = entry?;
      let path = entry.path();

      // Look for UUID directories containing metadata.json
      if path.is_dir() {
        let metadata_file = path.join("metadata.json");
        if metadata_file.exists() {
          let content = match fs::read_to_string(&metadata_file) {
            Ok(c) => c,
            Err(e) => {
              log::warn!(
                "Skipping profile at {}: failed to read metadata.json: {e}",
                path.display()
              );
              continue;
            }
          };
          let mut profile: BrowserProfile = match serde_json::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
              log::warn!(
                "Skipping profile at {}: invalid metadata.json: {e}",
                path.display()
              );
              continue;
            }
          };

          // Backfill host_os from browser config for profiles created before
          // the field existed (or synced without it), and repair any profile
          // already stamped with a fingerprint-only OS.
          //
          // Only a real host OS may be stored here — `is_host_os` filters out
          // anything `resolved_os`'s fallback couldn't actually match a
          // machine to, so a mismatch here is a genuine cross-OS profile, not
          // a value that would incorrectly lock the profile out forever.
          let needs_repair = profile.host_os.as_deref().is_some_and(|os| !is_host_os(os));
          if profile.host_os.is_none() || needs_repair {
            let inferred_os = profile
              .resolved_os()
              .filter(|os| is_host_os(os))
              .map(str::to_string);
            if inferred_os != profile.host_os {
              profile.host_os = inferred_os;
              if let Ok(json) = serde_json::to_string_pretty(&profile) {
                let _ = atomic_write(&metadata_file, json.as_bytes());
              }
            }
          }

          profiles.push(profile);
        }
      }
    }

    Ok(profiles)
  }

  pub fn rename_profile(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    new_name: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    if new_name.trim().is_empty() {
      return Err(
        serde_json::json!({ "code": "NAME_CANNOT_BE_EMPTY" })
          .to_string()
          .into(),
      );
    }

    // Check if new name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_name}' already exists").into());
    }

    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let mut profile = existing_profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Update profile name (no need to move directories since we use UUID)
    profile.name = new_name.to_string();
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    // Save profile with new name
    self.save_profile(&profile)?;

    // Keep tag suggestions up to date after name change (rebuild from all profiles)
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile rename event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn delete_profile(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Attempting to delete profile with ID: {profile_id}");

    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Check if browser is running (cross-OS profiles can't be running locally)
    if profile.process_id.is_some() && !profile.is_cross_os() {
      return Err(
        "Cannot delete profile while browser is running. Please stop the browser first.".into(),
      );
    }

    // Launch-gate acknowledgements are keyed by profile id and are not synced,
    // so nothing else would ever clean them up.
    crate::launch_gate_prefs::forget_profile(profile_id);

    // Per-domain traffic history lives outside the profile directory, so it
    // survives the delete otherwise. It is already zero-overwritten on removal.
    crate::traffic_stats::delete_traffic_stats(profile_id);

    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

    // Delete the entire UUID directory (contains both metadata.json and profile data)
    if profile_uuid_dir.exists() {
      log::info!("Deleting profile directory: {}", profile_uuid_dir.display());
      fs::remove_dir_all(&profile_uuid_dir)?;
      log::info!("Profile directory deleted successfully");
    }

    // Verify deletion was successful
    if profile_uuid_dir.exists() {
      return Err(format!("Failed to completely delete profile '{}'", profile.name).into());
    }

    log::info!(
      "Profile '{}' (ID: {}) deleted successfully",
      profile.name,
      profile_id
    );

    // Rebuild tag suggestions after deletion
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Always perform cleanup after profile deletion to remove unused binaries
    if let Err(e) = DownloadedBrowsersRegistry::instance().cleanup_unused_binaries() {
      log::warn!("Warning: Failed to cleanup unused binaries after profile deletion: {e}");
    }

    // Emit profile deletion event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub fn update_profile_version(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    version: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Check if the browser is currently running
    if profile.process_id.is_some() {
      return Err(
        "Cannot update version while browser is running. Please stop the browser first.".into(),
      );
    }

    // Verify the new version is downloaded
    let browser_type = BrowserType::from_str(&profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", profile.browser))?;
    let browser = create_browser(browser_type.clone());
    let binaries_dir = self.get_binaries_dir();

    if !browser.is_version_downloaded(version, &binaries_dir) {
      return Err(format!("Browser version {version} is not downloaded").into());
    }

    // Update version
    profile.version = version.to_string();

    profile.release_type = "stable".to_string();

    // Save the updated profile
    self.save_profile(&profile)?;

    // Emit profile update event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn assign_profiles_to_group(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_ids: Vec<String>,
    group_id: Option<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_id in profile_ids {
      let profile_uuid = uuid::Uuid::parse_str(&profile_id)
        .map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
      let mut profile = profiles
        .iter()
        .find(|p| p.id == profile_uuid)
        .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?
        .clone();

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(format!(
          "Cannot modify group for profile '{}' while browser is running. Please stop the browser first.", profile.name
        ).into());
      }

      profile.group_id = group_id.clone();
      profile.updated_at = Some(crate::proxy_manager::now_secs());
      self.save_profile(&profile)?;
    }

    // Rebuild tag suggestions after group changes just in case
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile group assignment event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub fn update_profile_tags(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    tags: Vec<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<String> = Vec::with_capacity(tags.len());
    for t in tags.into_iter() {
      if seen.insert(t.clone()) {
        deduped.push(t);
      }
    }
    profile.tags = deduped;
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    // Save profile
    self.save_profile(&profile)?;

    // Update global tag suggestions from all profiles
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile tags update event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_note(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    note: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Update note (trim whitespace, set to None if empty)
    profile.note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    // Save profile
    self.save_profile(&profile)?;

    // Emit profile note update event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_window_color(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    window_color: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Normalize to lowercase #RRGGBB, or clear (None) for an invalid/empty value
    // so it reverts to the auto id-derived color at next launch.
    profile.window_color = window_color.and_then(|c| {
      let hex = c.trim().trim_start_matches('#');
      (hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_lowercase()))
    });
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    self.save_profile(&profile)?;
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_proxy_bypass_rules(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    rules: Vec<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    profile.proxy_bypass_rules = rules;
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    self.save_profile(&profile)?;

    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_dns_blocklist(
    &self,
    profile_id: &str,
    dns_blocklist: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    profile.dns_blocklist = dns_blocklist;
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    self.save_profile(&profile)?;

    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn delete_multiple_profiles(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_ids: Vec<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_id in profile_ids {
      let profile_uuid = uuid::Uuid::parse_str(&profile_id)
        .map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
      let profile = profiles
        .iter()
        .find(|p| p.id == profile_uuid)
        .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

      // Check if browser is running (cross-OS profiles can't be running locally)
      if profile.process_id.is_some() && !profile.is_cross_os() {
        return Err(
          format!(
            "Cannot delete profile '{}' while browser is running. Please stop the browser first.",
            profile.name
          )
          .into(),
        );
      }

      // Delete the profile
      let profiles_dir = self.get_profiles_dir();
      let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

      if profile_uuid_dir.exists() {
        std::fs::remove_dir_all(&profile_uuid_dir)?;
      }
    }

    // Emit profile deletion event
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  fn generate_clone_name(&self, original_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;
    let existing_names: std::collections::HashSet<String> =
      profiles.iter().map(|p| p.name.clone()).collect();

    let candidate = format!("{original_name} (Copy)");
    if !existing_names.contains(&candidate) {
      return Ok(candidate);
    }

    for i in 2.. {
      let candidate = format!("{original_name} (Copy {i})");
      if !existing_names.contains(&candidate) {
        return Ok(candidate);
      }
    }

    unreachable!()
  }

  pub fn clone_profile(
    &self,
    profile_id: &str,
    custom_name: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let source = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    if source.process_id.is_some() {
      return Err(
        "Cannot clone profile while browser is running. Please stop the browser first.".into(),
      );
    }

    let new_id = uuid::Uuid::new_v4();
    let clone_name = match custom_name {
      Some(name) if !name.trim().is_empty() => name.trim().to_string(),
      _ => self.generate_clone_name(&source.name)?,
    };

    let profiles_dir = self.get_profiles_dir();
    let source_dir = profiles_dir.join(source.id.to_string());
    let dest_dir = profiles_dir.join(new_id.to_string());

    if source_dir.exists() {
      copy_directory_recursive(&source_dir, &dest_dir)?;
    } else {
      fs::create_dir_all(&dest_dir)?;
    }

    let mut new_profile = BrowserProfile {
      id: new_id,
      name: clone_name,
      browser: source.browser,
      version: source.version,
      proxy_id: source.proxy_id,
      vpn_id: source.vpn_id,
      process_id: None,
      last_launch: None,
      release_type: source.release_type,
      camoufox_config: source.camoufox_config,
      browser_config: source.browser_config,
      group_id: source.group_id,
      tags: source.tags,
      note: source.note,
       window_color: source.window_color,
       encryption_salt: None,
       host_os: Some(get_host_os()),
       password_protected: false,
      extension_group_id: source.extension_group_id,
      proxy_bypass_rules: source.proxy_bypass_rules,
      dns_blocklist: source.dns_blocklist,
      created_at: Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .map(|d| d.as_secs())
          .unwrap_or(0),
      ),
      updated_at: Some(crate::proxy_manager::now_secs()),
    };

    // Salamander: a clone must NOT be linkable to its source. The source
    // camoufox_config embeds the persisted fingerprint, so copying it
    // verbatim would make the clone emit identical device signals as the
    // source — trivially linkable if both run concurrently. Clear the
    // fingerprint so the launch path mints a fresh one, exactly as
    // create_profile does when fingerprint.is_none(). NOTE: the user-data-dir
    // copy above still duplicates cookies/localStorage/TLS state — a separate
    // storage-linkage vector the user must clear if they want full isolation
    // between a clone and its source.
    if let Some(cfg) = new_profile.camoufox_config.as_mut() {
      cfg.fingerprint = None;
    }

    self.save_profile(&new_profile)?;

    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(new_profile)
  }

  pub async fn update_camoufox_config(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: &str,
    config: CamoufoxConfig,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile with ID '{profile_id}' not found").into()
      })?;

    let is_running = self
      .check_browser_status(app_handle.clone(), &profile)
      .await?;

    if is_running {
      return Err(
        "Cannot update Camoufox configuration while browser is running. Please stop the browser first.".into(),
      );
    }

    profile.camoufox_config = Some(config);

    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    log::info!(
      "Camoufox configuration updated for profile '{}' (ID: {}).",
      profile.name,
      profile_id
    );

    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub async fn update_browser_config(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: &str,
    config: serde_json::Value,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles = self.list_profiles().map_err(|e| format!("Failed to list profiles: {e}"))?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    if self.check_browser_status(app_handle, &profile).await? {
      return Err("Cannot update browser configuration while browser is running. Please stop the browser first.".into());
    }

    profile.browser_config = Some(config);
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }
    Ok(())
  }

  pub async fn update_profile_proxy(
    &self,
    _app_handle: tauri::AppHandle,
    profile_id: &str,
    proxy_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by ID
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;

    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile with ID '{profile_id}' not found").into()
      })?;

    // Remember old proxy_id for cleanup (not used yet, but may be needed for cleanup)
    let _old_proxy_id = profile.proxy_id.clone();

    // Update proxy settings and clear VPN (mutual exclusion)
    profile.proxy_id = proxy_id.clone();
    profile.vpn_id = None;
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    // Save the updated profile
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    // Emit profile update event so frontend UIs can refresh immediately (e.g. proxy manager)
    if let Err(e) = events::emit("profile-updated", &profile) {
      log::warn!("Warning: Failed to emit profile update event: {e}");
    }

    // Emit general profiles changed event for profile list updates
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub async fn update_profile_vpn(
    &self,
    _app_handle: tauri::AppHandle,
    profile_id: &str,
    vpn_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;

    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile with ID '{profile_id}' not found").into()
      })?;

    // Update VPN and clear proxy (mutual exclusion)
    profile.vpn_id = vpn_id.clone();
    profile.proxy_id = None;
    profile.updated_at = Some(crate::proxy_manager::now_secs());

    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    if let Err(e) = events::emit("profile-updated", &profile) {
      log::warn!("Warning: Failed to emit profile update event: {e}");
    }

    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_extension_group(
    &self,
    profile_id: &str,
    extension_group_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    profile.extension_group_id = extension_group_id;
    profile.updated_at = Some(crate::proxy_manager::now_secs());
    self.save_profile(&profile)?;

    if let Err(e) = events::emit("profile-updated", &profile) {
      log::warn!("Failed to emit profile update event: {e}");
    }
    if let Err(e) = events::emit_empty("profiles-changed") {
      log::warn!("Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let inner_profile = profile.clone();
    let system = System::new_with_specifics(
      RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    let mut is_running = false;
    let mut found_pid: Option<u32> = None;

    // First check if the stored PID is still valid
    if let Some(pid) = profile.process_id {
      if let Some(process) = system.process(Pid::from(pid as usize)) {
        let cmd = process.cmd();
        // Verify this process is actually our browser with the correct profile
        let profiles_dir = self.get_profiles_dir();
         let profile_data_path = profile.get_profile_data_path(&profiles_dir);
        let profile_data_path_str = profile_data_path.to_string_lossy();
        let profile_path_match = cmd.iter().any(|s| {
          let arg = s.to_str().unwrap_or("");
          // Match the Chromium --user-data-dir flag or an exact profile path argument
          arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
            || arg == profile_data_path_str
        });

        if profile_path_match {
          is_running = true;
          found_pid = Some(pid);
        }
      }
    }

    // If we didn't find the browser with the stored PID, search all processes
    if !is_running {
      for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.len() >= 2 {
          // Check if this is the right browser executable first
          let exe_name = process.name().to_string_lossy().to_lowercase();
          let is_correct_browser = match profile.browser.as_str() {
            "camoufox" => exe_name.contains("camoufox"),
            "clearcote" => {
              exe_name.contains("clearcote") || exe_name == "chrome" || exe_name == "chrome.exe"
            }
            _ => false,
          };

          if !is_correct_browser {
            continue;
          }

          // Check for profile path match
          let profiles_dir = self.get_profiles_dir();
           let profile_data_path = profile.get_profile_data_path(&profiles_dir);
          let profile_data_path_str = profile_data_path.to_string_lossy();
          let profile_path_match = cmd.iter().any(|s| {
            let arg = s.to_str().unwrap_or("");
            // Match the Chromium --user-data-dir flag or an exact profile path argument
            arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
              || arg == profile_data_path_str
          });

          if profile_path_match {
            // Found a matching process
            found_pid = Some(pid.as_u32());
            is_running = true;
            log::info!(
              "Found browser process with PID: {} for profile: {}",
              pid.as_u32(),
              profile.name
            );
            break;
          }
        }
      }
    }

    // Only persist status changes if the profile metadata still exists on disk
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
    let metadata_file = profile_uuid_dir.join("metadata.json");
    let metadata_exists = metadata_file.exists();

    if metadata_exists {
      // Load the latest profile from disk to avoid overwriting fields like proxy_id
      let latest_profile: BrowserProfile = match std::fs::read_to_string(&metadata_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
      {
        Some(p) => p,
        None => inner_profile.clone(),
      };

      let mut merged = latest_profile.clone();
      let mut detected_stop = false;

      if let Some(pid) = found_pid {
        if merged.process_id != Some(pid) {
          let old_pid = merged.process_id;
          merged.process_id = Some(pid);
          if let Err(e) = self.save_profile(&merged) {
            log::warn!("Warning: Failed to update profile with new PID: {e}");
          }
          // Re-point the worker at the browser's NEW identity. update_proxy_pid
          // persists it, so a browser that re-execs mid-session doesn't leave
          // the detached worker watching a process that no longer exists.
          if let Some(prev) = old_pid {
            let _ = crate::proxy_manager::PROXY_MANAGER.update_proxy_pid(prev, pid);
          } else {
            // First sighting this session (e.g. the GUI restarted while the
            // browser kept running): nothing to re-key, but the worker still
            // needs a live owner recorded or nothing will ever reap it.
            crate::proxy_manager::PROXY_MANAGER
              .set_browser_pid_for_profile(&merged.id.to_string(), pid);
          }
        }
      } else if merged.process_id.is_some() {
        // Clear the PID if no process found
        merged.process_id = None;
        if let Err(e) = self.save_profile(&merged) {
          log::warn!("Warning: Failed to clear profile PID: {e}");
        }
        detected_stop = true;
      }

      if detected_stop {
        if let Some(updated) = crate::auto_updater::AutoUpdater::instance()
          .update_profile_to_latest_installed(&app_handle, &merged)
        {
          merged = updated;
        }
      }

      // Emit profile update event to frontend
      if let Err(e) = events::emit("profile-updated", &merged) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }
    }

    Ok(is_running)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use tempfile::TempDir;

  fn create_test_profile_manager() -> (&'static ProfileManager, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    // Mock the base directories by setting environment variables
    unsafe {
      std::env::set_var("HOME", temp_dir.path());
    }

    let profile_manager = ProfileManager::instance();
    (profile_manager, temp_dir)
  }

  #[test]
  fn test_profile_manager_creation() {
    let (_manager, _temp_dir) = create_test_profile_manager();
    // If we get here without panicking, the test passes
  }

  #[test]
  fn test_get_profiles_dir() {
    let (manager, _temp_dir) = create_test_profile_manager();
    let profiles_dir = manager.get_profiles_dir();

    assert!(
      profiles_dir.to_string_lossy().contains("SalamanderBrowser"),
      "Profiles dir should contain SalamanderBrowser"
    );
    assert!(
      profiles_dir.to_string_lossy().contains("profiles"),
      "Profiles dir should contain profiles"
    );
  }

  #[test]
  fn test_get_binaries_dir() {
    let (manager, _temp_dir) = create_test_profile_manager();

    let binaries_dir = manager.get_binaries_dir();
    let path_str = binaries_dir.to_string_lossy();

    assert!(
      path_str.contains("SalamanderBrowser"),
      "Binaries dir should contain SalamanderBrowser"
    );
    assert!(
      path_str.contains("binaries"),
      "Binaries dir should contain binaries"
    );
  }

}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_browser_profile_with_group(
  app_handle: tauri::AppHandle,
  name: String,
  browser: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  vpn_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
    browser_config: Option<serde_json::Value>,
  group_id: Option<String>,
  dns_blocklist: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .create_profile_with_group(
      &app_handle,
      &name,
      &browser,
      &version,
      &release_type,
      proxy_id,
      vpn_id,
      camoufox_config,
      browser_config,
      group_id,
      dns_blocklist,
    )
    .await
    .map_err(|e| crate::wrap_backend_error(e, "Failed to create profile"))
}

#[tauri::command]
pub fn list_browser_profiles() -> Result<Vec<BrowserProfile>, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
}

#[tauri::command]
pub async fn update_profile_proxy(
  app_handle: tauri::AppHandle,
  profile_id: String,
  proxy_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_proxy(app_handle, &profile_id, proxy_id)
    .await
    .map_err(|e| format!("Failed to update profile: {e}"))
}

#[tauri::command]
pub async fn update_profile_vpn(
  app_handle: tauri::AppHandle,
  profile_id: String,
  vpn_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_vpn(app_handle, &profile_id, vpn_id)
    .await
    .map_err(|e| format!("Failed to update profile VPN: {e}"))
}

#[tauri::command]
pub fn update_profile_tags(
  app_handle: tauri::AppHandle,
  profile_id: String,
  tags: Vec<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_tags(&app_handle, &profile_id, tags)
    .map_err(|e| format!("Failed to update profile tags: {e}"))
}

#[tauri::command]
pub fn update_profile_note(
  app_handle: tauri::AppHandle,
  profile_id: String,
  note: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_note(&app_handle, &profile_id, note)
    .map_err(|e| format!("Failed to update profile note: {e}"))
}

#[tauri::command]
pub fn update_profile_window_color(
  app_handle: tauri::AppHandle,
  profile_id: String,
  window_color: Option<String>,
) -> Result<BrowserProfile, String> {
  ProfileManager::instance()
    .update_profile_window_color(&app_handle, &profile_id, window_color)
    .map_err(|e| format!("Failed to update profile window color: {e}"))
}

#[tauri::command]
pub fn update_profile_proxy_bypass_rules(
  app_handle: tauri::AppHandle,
  profile_id: String,
  rules: Vec<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_proxy_bypass_rules(&app_handle, &profile_id, rules)
    .map_err(|e| format!("Failed to update proxy bypass rules: {e}"))
}

#[tauri::command]
pub fn update_profile_dns_blocklist(
  profile_id: String,
  dns_blocklist: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_dns_blocklist(&profile_id, dns_blocklist)
    .map_err(|e| format!("Failed to update DNS blocklist: {e}"))
}

#[tauri::command]
pub async fn check_browser_status(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<bool, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .check_browser_status(app_handle, &profile)
    .await
    .map_err(|e| format!("Failed to check browser status: {e}"))
}

#[tauri::command]
pub fn rename_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
  new_name: String,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .rename_profile(&app_handle, &profile_id, &new_name)
    .map_err(|e| crate::wrap_backend_error(e, "Failed to rename profile"))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_browser_profile_new(
  app_handle: tauri::AppHandle,
  name: String,
  browser_str: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  vpn_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
  browser_config: Option<serde_json::Value>,
  group_id: Option<String>,
  dns_blocklist: Option<String>,
) -> Result<BrowserProfile, String> {
  // A dead/unreachable proxy or VPN (or a 402 from an expired proxy
  // subscription) cancels creation with a translatable error.
  crate::validate_profile_network(proxy_id.as_deref(), vpn_id.as_deref()).await?;

  let browser_type =
    BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
  create_browser_profile_with_group(
    app_handle,
    name,
    browser_type.as_str().to_string(),
    version,
    release_type,
    proxy_id,
    vpn_id,
    camoufox_config,
    browser_config,
    group_id,
    dns_blocklist,
  )
  .await
}

#[tauri::command]
pub async fn update_camoufox_config(
  app_handle: tauri::AppHandle,
  profile_id: String,
  config: CamoufoxConfig,
) -> Result<(), String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_camoufox_config(app_handle, &profile_id, config)
    .await
    .map_err(|e| format!("Failed to update Camoufox config: {e}"))
}

#[tauri::command]
pub async fn update_browser_config(
  app_handle: tauri::AppHandle,
  profile_id: String,
  config: serde_json::Value,
) -> Result<(), String> {
  ProfileManager::instance()
    .update_browser_config(app_handle, &profile_id, config)
    .await
    .map_err(|e| format!("Failed to update browser config: {e}"))
}

#[tauri::command]
pub fn clone_profile(profile_id: String, name: Option<String>) -> Result<BrowserProfile, String> {
  ProfileManager::instance()
    .clone_profile(&profile_id, name)
    .map_err(|e| format!("Failed to clone profile: {e}"))
}

#[tauri::command]
pub fn delete_profile(app_handle: tauri::AppHandle, profile_id: String) -> Result<(), String> {
  ProfileManager::instance()
    .delete_profile(&app_handle, &profile_id)
    .map_err(|e| format!("Failed to delete profile: {e}"))
}

lazy_static::lazy_static! {
  static ref PROFILE_MANAGER: ProfileManager = ProfileManager::new();
}
