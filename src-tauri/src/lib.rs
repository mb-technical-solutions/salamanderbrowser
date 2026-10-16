// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
#[cfg(not(feature = "e2e"))]
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_log::{Target, TargetKind};

// Store pending URLs that need to be handled when the window is ready
static PENDING_URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Set to true once the user has confirmed they want to quit, so the close
// interceptor lets the next CloseRequested through instead of looping back
// to the confirmation dialog.
static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

pub(crate) fn backend_error(code: &str) -> String {
  serde_json::json!({ "code": code }).to_string()
}

pub(crate) fn backend_error_with_detail(code: &str, detail: impl std::fmt::Display) -> String {
  serde_json::json!({ "code": code, "params": { "detail": detail.to_string() } }).to_string()
}

/// A VLESS URI Salamander cannot use, carrying which part is unsupported so the UI
/// can say so instead of implying a typo.
pub(crate) fn vless_config_error(error: &crate::xray::XrayError) -> String {
  serde_json::json!({
    "code": "VLESS_CONFIG_INVALID",
    "params": { "reason": error.reason_code(), "detail": error.to_string() }
  })
  .to_string()
}

fn e2e_automation_enabled() -> bool {
  #[cfg(feature = "e2e")]
  {
    std::env::var("TAURI_AUTOMATION")
      .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
  }
  #[cfg(not(feature = "e2e"))]
  {
    false
  }
}

#[cfg(feature = "e2e")]
fn e2e_automation_profile_dir() -> Option<std::path::PathBuf> {
  e2e_automation_enabled()
    .then(|| std::env::var_os("TAURI_AUTOMATION_PROFILE_DIR").map(std::path::PathBuf::from))
    .flatten()
}

mod api_client;
mod app_auto_updater;
pub mod app_dirs;
mod auto_updater;
mod browser;
mod browser_runner;
mod browser_version_manager;
pub mod camoufox;
mod camoufox_manager;
mod checksum;
mod default_browser;
pub mod dns_blocklist;
mod downloaded_browsers_registry;
mod downloader;
mod ephemeral_dirs;
mod extension_manager;
mod extraction;
mod fingerprint_consistency;
mod fs_secure;
mod geoip_downloader;
mod geolocation;
mod group_manager;
mod ip_utils;
mod launch_gate;
mod launch_gate_prefs;
mod log_redaction;
mod platform_browser;
mod profile;
mod proxy_manager;
pub mod proxy_runner;
pub mod proxy_server;
pub mod proxy_storage;
mod settings_manager;
pub mod socks5_local;
pub mod traffic_stats;
mod window_decorations;
// mod theme_detector; // removed: theme detection handled in webview via CSS prefers-color-scheme
mod cookie_manager;
mod cookie_paste;
pub mod events;
mod tag_manager;
mod version_updater;
pub mod vpn;
mod vpn_extension_detect;
pub mod vpn_worker_runner;
pub mod vpn_worker_storage;
pub mod xray;
pub mod xray_worker_runner;
pub mod xray_worker_storage;

use browser_runner::{
  check_browser_exists, kill_browser_profile, launch_browser_profile, open_url_with_profile,
};

use profile::manager::{
  check_browser_status, clone_profile, create_browser_profile_new, delete_profile,
  list_browser_profiles, rename_profile, update_browser_config, update_camoufox_config,
  update_profile_dns_blocklist, update_profile_note,
  update_profile_proxy, update_profile_proxy_bypass_rules, update_profile_tags, update_profile_vpn,
  update_profile_window_color,
};
use profile::password::{
  change_profile_password, is_profile_locked, lock_profile, remove_profile_password,
  set_profile_password, unlock_profile, verify_profile_password,
};

use browser_version_manager::{
  fetch_browser_versions_cached_first, fetch_browser_versions_with_count,
  fetch_browser_versions_with_count_cached_first, get_supported_browsers,
  is_browser_supported_on_platform,
};

use downloaded_browsers_registry::{
  check_missing_binaries, ensure_active_browsers_downloaded, ensure_all_binaries_exist,
  get_downloaded_browser_versions,
};

use downloader::{cancel_download, download_browser};

use settings_manager::{
  complete_onboarding, dismiss_window_resize_warning, get_app_settings, get_onboarding_completed,
  get_system_info, get_system_language, get_table_sorting_settings,
  get_window_resize_warning_dismissed, open_log_directory, read_log_files, save_app_settings,
  save_table_sorting_settings,
};

use tag_manager::get_all_tags;

use default_browser::{is_default_browser, set_as_default_browser};

use version_updater::{
  clear_all_version_cache_and_refetch, get_version_update_status, get_version_updater,
  trigger_manual_version_update,
};

use auto_updater::{
  check_for_browser_updates, complete_browser_update_with_auto_update, dismiss_update_notification,
};

use app_auto_updater::{
  check_for_app_updates, check_for_app_updates_manual, download_and_prepare_app_update,
  restart_application,
};

use profile::import_camoufox::import_camoufox_profile;

use profile::archive::{export_profiles_archive, import_profiles_archive, inspect_import_source};

struct ImportedProxySource {
  id: String,
  name: String,
  settings: browser::ProxySettings,
}

struct ImportedVpnSource {
  id: String,
  name: String,
  vpn_type: vpn::VpnType,
  config_data: String,
}

/// Counts reported back after a successful `import_data_directory`, so the UI
/// can tell the user what actually came in.
#[derive(serde::Serialize)]
struct ImportDataSummary {
  profiles: usize,
  proxies: usize,
  vpn_configs: usize,
}

/// Manually bring in an entire Salamander (or pre-rename Donut Browser) data
/// directory picked by the user — profiles, proxies, VPN configs, extensions
/// and settings — by copying anything missing into the live data directory.
/// This is the user-driven counterpart to `app_dirs::migrate_from_previous_brand`,
/// which only catches the same-machine, default-location case automatically;
/// this covers a different machine, a hand-copied backup, or a non-default
/// install the automatic check can't find on its own.
#[tauri::command]
async fn import_data_directory(
  app_handle: tauri::AppHandle,
  source_path: String,
) -> Result<ImportDataSummary, String> {
  let source = std::path::PathBuf::from(&source_path);

  let looks_like_data_dir = [
    "profiles",
    "proxies",
    "vpn",
    "settings",
    "binaries",
    "extensions",
  ]
  .iter()
  .any(|d| source.join(d).is_dir());
  if !source.is_dir() || !looks_like_data_dir {
    return Err(backend_error("IMPORT_SOURCE_NOT_FOUND"));
  }

  let profile_manager = profile::manager::ProfileManager::instance();
  let pre_existing_ids: std::collections::HashSet<_> = profile_manager
    .list_profiles()
    .map(|p| p.into_iter().map(|profile| profile.id).collect())
    .unwrap_or_default();
  let pre_existing_proxy_count = proxy_manager::PROXY_MANAGER.get_stored_proxies().len();
  let pre_existing_vpn_count = vpn::VPN_STORAGE
    .lock()
    .ok()
    .and_then(|s| s.list_configs().ok())
    .map(|c| c.len())
    .unwrap_or(0);
  let existing_vpn_configs: Vec<vpn::VpnConfig> = vpn::VPN_STORAGE
    .lock()
    .ok()
    .and_then(|storage| {
      storage.list_configs().ok().map(|configs| {
        configs
          .into_iter()
          .filter_map(|summary| storage.load_config(&summary.id).ok())
          .collect()
      })
    })
    .unwrap_or_default();

  // A VPN storage file is encrypted with the key next to the source
  // installation. Read the source configs before copying anything, then
  // recreate them with the destination key and remap profile references.
  let source_vpn_storage = vpn::VpnStorage::with_dir(&source.join("vpn"));
  let source_vpns: Vec<ImportedVpnSource> = source_vpn_storage
    .list_configs()
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|summary| {
      source_vpn_storage.load_config(&summary.id).ok().map(|config| ImportedVpnSource {
        id: config.id,
        name: config.name,
        vpn_type: config.vpn_type,
        config_data: config.config_data,
      })
    })
    .collect();

  let source_proxies: Vec<ImportedProxySource> = std::fs::read_dir(source.join("proxies"))
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|entry| {
      let path = entry.ok()?.path();
      if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return None;
      }
      let proxy: proxy_manager::StoredProxy =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
      Some(ImportedProxySource {
        id: proxy.id,
        name: proxy.name,
        settings: proxy.proxy_settings,
      })
    })
    .collect();

  app_dirs::import_data_tree_from(&source)
    .map_err(|e| backend_error_with_detail("INTERNAL_ERROR", e))?;

  let mut proxy_id_map = std::collections::HashMap::new();
  for proxy in source_proxies {
    let mut name = format!("{} (imported)", proxy.name);
    let mut suffix = 2;
    let imported = loop {
      match proxy_manager::PROXY_MANAGER.create_stored_proxy(
        &app_handle,
        name.clone(),
        proxy.settings.clone(),
      ) {
        Ok(value) => break Some(value),
        Err(_) if suffix < 1000 => {
          name = format!("{} (imported {})", proxy.name, suffix);
          suffix += 1;
        }
        Err(_) => break None,
      }
    };
    if let Some(imported) = imported {
      proxy_id_map.insert(proxy.id, imported.id);
    }
  }

  // Rebuild destination VPN storage from its pre-import plaintext snapshot and
  // the source configs. This avoids leaving records encrypted by the source
  // installation's unrelated `.vpn_key`.
  let vpn_storage_path = app_dirs::vpn_dir().join("vpn_configs.json");
  let _ = std::fs::remove_file(&vpn_storage_path);
  let mut vpn_id_map = std::collections::HashMap::new();
  if let Ok(storage) = vpn::VPN_STORAGE.lock() {
    for config in existing_vpn_configs {
      let _ = storage.save_config(&config);
    }
    for vpn in source_vpns {
      if let Ok(imported) = storage.create_config_manual(
        &format!("{} (imported)", vpn.name),
        vpn.vpn_type,
        &vpn.config_data,
      ) {
        vpn_id_map.insert(vpn.id, imported.id);
      }
    }
  }

  // Source worker files are runtime state, not portable proxy definitions.
  // Remove copies whose definitions were recreated above.
  for source_id in proxy_id_map.keys() {
    let _ = std::fs::remove_file(app_dirs::proxies_dir().join(format!("{source_id}.json")));
  }

  // The copy brings in profile folders verbatim by UUID, so two profiles
  // that happen to share a display name (e.g. both named "data" because one
  // was copied from another installation) now coexist unchanged. Rename only
  // the newly-introduced ones on collision, the same "{name} (2)" pattern
  // `import_profiles_archive` already uses, so existing local profiles are
  // never touched.
  if let Ok(all_profiles) = profile_manager.list_profiles() {
    let mut taken_names: std::collections::HashSet<String> = all_profiles
      .iter()
      .filter(|p| pre_existing_ids.contains(&p.id))
      .map(|p| p.name.to_lowercase())
      .collect();

    for mut imported in all_profiles
      .into_iter()
      .filter(|p| !pre_existing_ids.contains(&p.id))
    {
      let mut changed = false;
      if let Some(source_id) = imported.proxy_id.as_ref() {
        imported.proxy_id = proxy_id_map.get(source_id).cloned();
        changed = true;
      }
      if let Some(source_id) = imported.vpn_id.as_ref() {
        imported.vpn_id = vpn_id_map.get(source_id).cloned();
        changed = true;
      }
      let lower = imported.name.to_lowercase();
      if taken_names.contains(&lower) {
        let base_name = imported.name.clone();
        let mut suffix = 2;
        let mut final_name;
        loop {
          final_name = format!("{base_name} ({suffix})");
          if !taken_names.contains(&final_name.to_lowercase()) {
            break;
          }
          suffix += 1;
        }
        imported.name = final_name;
        changed = true;
      }
      taken_names.insert(imported.name.to_lowercase());
      if changed {
        let _ = profile_manager.save_profile(&imported);
      }
    }
  }

  let profiles = profile_manager
    .list_profiles()
    .map(|all| {
      all
        .iter()
        .filter(|profile| !pre_existing_ids.contains(&profile.id))
        .count()
    })
    .unwrap_or(0);
  let proxies = proxy_manager::PROXY_MANAGER
    .get_stored_proxies()
    .len()
    .saturating_sub(pre_existing_proxy_count);
  let vpn_configs = vpn::VPN_STORAGE
    .lock()
    .ok()
    .and_then(|s| s.list_configs().ok())
    .map(|c| c.len())
    .unwrap_or(0)
    .saturating_sub(pre_existing_vpn_count);

  let _ = events::emit_empty("profiles-changed");
  let _ = events::emit_empty("proxies-changed");
  let _ = events::emit_empty("stored-proxies-changed");
  let _ = events::emit_empty("vpn-configs-changed");
  let _ = events::emit_empty("groups-changed");

  log::info!(
    "Imported data directory from '{}': {} profiles, {} proxies, {} VPN configs",
    source.display(),
    profiles,
    proxies,
    vpn_configs
  );

  Ok(ImportDataSummary {
    profiles,
    proxies,
    vpn_configs,
  })
}

use extension_manager::{
  add_extension, add_extension_to_group, add_unpacked_extension, assign_extension_group_to_profile,
  create_extension_group, delete_extension, delete_extension_group,
  get_extension_group_for_profile, get_extension_icon, list_extension_groups, list_extensions,
  remove_extension_from_group, update_extension, update_extension_from_path,
  update_extension_group,
};

use group_manager::{
  assign_profiles_to_group, create_profile_group, delete_profile_group, delete_selected_profiles,
  get_groups_with_profile_counts, get_profile_groups, update_profile_group,
};

use geoip_downloader::{check_missing_geoip_database, GeoIPDownloader};

use browser_version_manager::get_browser_release_types;

// Trait to extend WebviewWindow with transparent titlebar functionality
pub trait WindowExt {
  #[cfg(target_os = "macos")]
  fn set_transparent_titlebar(&self, transparent: bool) -> Result<(), String>;
  #[cfg(target_os = "macos")]
  fn disable_native_fullscreen(&self) -> Result<(), String>;
}

impl<R: Runtime> WindowExt for WebviewWindow<R> {
  #[cfg(target_os = "macos")]
  fn set_transparent_titlebar(&self, transparent: bool) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSWindow, NSWindowStyleMask, NSWindowTitleVisibility};

    unsafe {
      let ns_window: Retained<NSWindow> =
        Retained::retain(self.ns_window().unwrap().cast()).unwrap();

      if transparent {
        // Hide the title text
        ns_window.setTitleVisibility(NSWindowTitleVisibility(1)); // NSWindowTitleHidden

        // Make titlebar transparent
        ns_window.setTitlebarAppearsTransparent(true);

        // Set full size content view
        let current_mask = ns_window.styleMask();
        let new_mask = NSWindowStyleMask(current_mask.0 | (1 << 15)); // NSFullSizeContentViewWindowMask
        ns_window.setStyleMask(new_mask);
      } else {
        // Show the title text
        ns_window.setTitleVisibility(NSWindowTitleVisibility(0)); // NSWindowTitleVisible

        // Make titlebar opaque
        ns_window.setTitlebarAppearsTransparent(false);

        // Remove full size content view
        let current_mask = ns_window.styleMask();
        let new_mask = NSWindowStyleMask(current_mask.0 & !(1 << 15));
        ns_window.setStyleMask(new_mask);
      }
    }

    Ok(())
  }

  #[cfg(target_os = "macos")]
  fn disable_native_fullscreen(&self) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    unsafe {
      let ns_window: Retained<NSWindow> =
        Retained::retain(self.ns_window().unwrap().cast()).unwrap();

      // Make the green title-bar button (and titlebar double-click) "zoom"
      // the window to fill the screen as an ordinary window instead of
      // entering immersive native fullscreen that hides the menu bar and
      // moves to its own Space. Mirrors Electron's `fullscreenable: false`:
      // clear FullScreenPrimary and set FullScreenNone. AppKit then maps the
      // green button to the standard zoom, expanding to the visible screen
      // frame while keeping the window chrome and the current Space.
      const FULL_SCREEN_PRIMARY: usize = 1 << 7;
      const FULL_SCREEN_NONE: usize = 1 << 9;
      let current = ns_window.collectionBehavior();
      let updated =
        NSWindowCollectionBehavior((current.0 & !FULL_SCREEN_PRIMARY) | FULL_SCREEN_NONE);
      ns_window.setCollectionBehavior(updated);
    }

    Ok(())
  }
}

// Called internally for deep-link / startup URL handling — not invoked from the
// frontend, so it is intentionally not a `#[tauri::command]`.
async fn handle_url_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
  log::info!("Handling URL open request");

  // Check if the main window exists and is ready
  if let Some(window) = app.get_webview_window("main") {
    log::debug!("Main window exists");

    // Try to show and focus the window first
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.unminimize();

    events::emit("show-profile-selector", url.clone())
      .map_err(|e| format!("Failed to emit URL open event: {e}"))?;
  } else {
    // Window doesn't exist yet - add to pending URLs
    log::debug!("Main window doesn't exist, adding URL to pending list");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url);
  }

  Ok(())
}

/// Preserve structured backend errors and wrap lower-level diagnostics in the
/// generic structured error shape expected by the frontend.
pub(crate) fn wrap_backend_error(e: impl std::fmt::Display, context: &str) -> String {
  let msg = e.to_string();
  if msg.starts_with('{') {
    msg
  } else {
    backend_error_with_detail("INTERNAL_ERROR", format!("{context}: {msg}"))
  }
}

#[tauri::command]
async fn create_stored_proxy(
  app_handle: tauri::AppHandle,
  name: String,
  proxy_settings: Option<crate::browser::ProxySettings>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  if let Some(settings) = proxy_settings {
    crate::proxy_manager::PROXY_MANAGER
      .create_stored_proxy(&app_handle, name, settings)
      .map_err(|e| wrap_backend_error(e, "Failed to create stored proxy"))
  } else {
    Err("proxy_settings is required".to_string())
  }
}

/// Validate a VLESS URI without touching the network, so the proxy form can
/// tell the user their setup is unsupported while they are still editing it
/// rather than only after they try to save or launch.
#[tauri::command]
fn validate_vless_uri(uri: String) -> Result<(), String> {
  crate::xray::parse_vless_uri(uri.trim())
    .map(|_| ())
    .map_err(|error| vless_config_error(&error))
}

/// Fetch a subscription URL and parse every VLESS line it carries. Per-line
/// failures are reported in the result rather than failing the whole
/// request, since a mixed subscription (some supported, some not) is the
/// common case — only a genuine fetch failure (unreachable URL, oversized
/// body) is returned as an error.
#[tauri::command]
async fn fetch_vless_subscription(
  url: String,
) -> Result<crate::xray::SubscriptionImportResult, String> {
  crate::xray::fetch_vless_subscription(url.trim())
    .await
    .map_err(|error| backend_error_with_detail("VLESS_SUBSCRIPTION_FETCH_FAILED", error))
}

#[tauri::command]
async fn get_stored_proxies() -> Result<Vec<crate::proxy_manager::StoredProxy>, String> {
  Ok(crate::proxy_manager::PROXY_MANAGER.get_stored_proxies())
}

#[tauri::command]
async fn update_stored_proxy(
  app_handle: tauri::AppHandle,
  proxy_id: String,
  name: Option<String>,
  proxy_settings: Option<crate::browser::ProxySettings>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  crate::proxy_manager::PROXY_MANAGER
    .update_stored_proxy(&app_handle, &proxy_id, name, proxy_settings)
    .map_err(|e| wrap_backend_error(e, "Failed to update stored proxy"))
}

#[tauri::command]
async fn delete_stored_proxy(app_handle: tauri::AppHandle, proxy_id: String) -> Result<(), String> {
  crate::proxy_manager::PROXY_MANAGER
    .delete_stored_proxy(&app_handle, &proxy_id)
    .map_err(|e| format!("Failed to delete stored proxy: {e}"))
}

#[tauri::command]
async fn check_proxy_validity(
  proxy_id: String,
  proxy_settings: Option<crate::browser::ProxySettings>,
) -> Result<crate::proxy_manager::ProxyCheckResult, String> {
  let settings = if let Some(s) = proxy_settings {
    s
  } else {
    crate::proxy_manager::PROXY_MANAGER
      .get_proxy_settings_by_id(&proxy_id)
      .ok_or_else(|| format!("Proxy '{proxy_id}' not found"))?
  };
  crate::proxy_manager::PROXY_MANAGER
    .check_proxy_validity(&proxy_id, &settings)
    .await
}

#[tauri::command]
fn get_cached_proxy_check(proxy_id: String) -> Option<crate::proxy_manager::ProxyCheckResult> {
  crate::proxy_manager::PROXY_MANAGER.get_cached_proxy_check(&proxy_id)
}

#[tauri::command]
fn export_proxies(format: String) -> Result<String, String> {
  match format.as_str() {
    "json" => crate::proxy_manager::PROXY_MANAGER.export_proxies_json(),
    "txt" => Ok(crate::proxy_manager::PROXY_MANAGER.export_proxies_txt()),
    _ => Err(format!("Unsupported export format: {format}")),
  }
}

#[tauri::command]
async fn import_proxies_json(
  app_handle: tauri::AppHandle,
  content: String,
) -> Result<crate::proxy_manager::ProxyImportResult, String> {
  crate::proxy_manager::PROXY_MANAGER
    .import_proxies_json(&app_handle, &content)
    .map_err(|e| format!("Failed to import proxies: {e}"))
}

#[tauri::command]
fn parse_txt_proxies(content: String) -> Vec<crate::proxy_manager::ProxyParseResult> {
  crate::proxy_manager::ProxyManager::parse_txt_proxies(&content)
}

#[tauri::command]
async fn import_proxies_from_parsed(
  app_handle: tauri::AppHandle,
  parsed_proxies: Vec<crate::proxy_manager::ParsedProxyLine>,
  name_prefix: Option<String>,
) -> Result<crate::proxy_manager::ProxyImportResult, String> {
  crate::proxy_manager::PROXY_MANAGER
    .import_proxies_from_parsed(&app_handle, parsed_proxies, name_prefix)
    .map_err(|e| format!("Failed to import proxies: {e}"))
}

#[tauri::command]
async fn read_profile_cookies(
  profile_id: String,
) -> Result<cookie_manager::CookieReadResult, String> {
  tokio::task::spawn_blocking(move || cookie_manager::CookieManager::read_cookies(&profile_id))
    .await
    .map_err(|e| format!("Failed to read profile cookies: {e}"))?
}

#[tauri::command]
async fn get_profile_cookie_stats(
  profile_id: String,
) -> Result<cookie_manager::CookieStats, String> {
  tokio::task::spawn_blocking(move || cookie_manager::CookieManager::read_stats(&profile_id))
    .await
    .map_err(|e| format!("Failed to read profile cookie stats: {e}"))?
}

#[tauri::command]
async fn copy_profile_cookies(
  app_handle: tauri::AppHandle,
  request: cookie_manager::CookieCopyRequest,
) -> Result<Vec<cookie_manager::CookieCopyResult>, String> {
  cookie_manager::CookieManager::copy_cookies(&app_handle, request).await
}

#[tauri::command]
async fn analyze_pasted_cookies(
  app_handle: tauri::AppHandle,
  profile_id: String,
  content: String,
  site: Option<String>,
) -> Result<cookie_manager::CookiePasteAnalysis, String> {
  cookie_manager::CookieManager::analyze_paste(&app_handle, &profile_id, &content, site.as_deref())
    .await
}

#[tauri::command]
async fn import_pasted_cookies(
  app_handle: tauri::AppHandle,
  profile_id: String,
  content: String,
  site: Option<String>,
  mode: cookie_manager::CookieWriteMode,
  include_expired: bool,
) -> Result<cookie_manager::CookiePasteImportResult, String> {
  let result = cookie_manager::CookieManager::import_paste(
    &app_handle,
    &profile_id,
    &content,
    site.as_deref(),
    mode,
    include_expired,
  )
  .await?;

  Ok(result)
}

#[tauri::command]
async fn export_profile_cookies(profile_id: String, format: String) -> Result<String, String> {
  cookie_manager::CookieManager::export_cookies(&profile_id, &format)
}

#[tauri::command]
async fn is_geoip_database_available() -> Result<bool, String> {
  Ok(GeoIPDownloader::is_geoip_database_available())
}

#[tauri::command]
async fn get_all_traffic_snapshots() -> Result<Vec<crate::traffic_stats::TrafficSnapshot>, String> {
  // Use real-time snapshots that merge in-memory data with disk data
  Ok(crate::traffic_stats::get_all_traffic_snapshots_realtime())
}

#[tauri::command]
async fn get_profile_traffic_snapshot(
  profile_id: String,
) -> Result<Option<crate::traffic_stats::TrafficSnapshot>, String> {
  Ok(crate::traffic_stats::get_traffic_snapshot_for_profile(
    &profile_id,
  ))
}

#[tauri::command]
async fn clear_all_traffic_stats() -> Result<(), String> {
  crate::traffic_stats::clear_all_traffic_stats()
    .map_err(|e| format!("Failed to clear traffic stats: {e}"))
}

#[tauri::command]
async fn clear_profile_traffic_stats(profile_id: String) -> Result<(), String> {
  crate::traffic_stats::delete_traffic_stats(&profile_id);
  let _ = events::emit_empty("traffic-stats-changed");
  Ok(())
}

#[tauri::command]
async fn get_traffic_stats_for_period(
  profile_id: String,
  seconds: u64,
) -> Result<Option<crate::traffic_stats::FilteredTrafficStats>, String> {
  Ok(crate::traffic_stats::get_traffic_stats_for_period(
    &profile_id,
    seconds,
  ))
}

#[tauri::command]
async fn download_geoip_database(app_handle: tauri::AppHandle) -> Result<(), String> {
  let downloader = GeoIPDownloader::instance();
  downloader
    .download_geoip_database(&app_handle)
    .await
    .map_err(|e| format!("Failed to download GeoIP database: {e}"))
}

// VPN commands
#[tauri::command]
async fn import_vpn_config(
  content: String,
  filename: String,
  name: Option<String>,
) -> Result<vpn::VpnImportResult, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  match storage.import_config(&content, &filename, name.clone()) {
    Ok(config) => Ok(vpn::VpnImportResult {
      success: true,
      vpn_id: Some(config.id),
      vpn_type: Some(config.vpn_type),
      name: config.name,
      error: None,
    }),
    Err(e) => Ok(vpn::VpnImportResult {
      success: false,
      vpn_id: None,
      vpn_type: None,
      name: name.unwrap_or_else(|| filename.clone()),
      error: Some(e.to_string()),
    }),
  }
}

#[tauri::command]
async fn list_vpn_configs() -> Result<Vec<vpn::VpnConfig>, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .list_configs()
    .map_err(|e| format!("Failed to list VPN configs: {e}"))
}

#[tauri::command]
async fn get_vpn_config(vpn_id: String) -> Result<vpn::VpnConfig, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .load_config(&vpn_id)
    .map_err(|e| format!("Failed to load VPN config: {e}"))
}

#[tauri::command]
async fn delete_vpn_config(vpn_id: String) -> Result<(), String> {
  // First disconnect if connected (stop VPN worker)
  let _ = vpn_worker_runner::stop_vpn_worker_by_vpn_id(&vpn_id).await;

  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .delete_config(&vpn_id)
    .map_err(|e| format!("Failed to delete VPN config: {e}"))?;

  let _ = events::emit("vpn-configs-changed", ());

  Ok(())
}

#[tauri::command]
async fn create_vpn_config_manual(
  name: String,
  vpn_type: vpn::VpnType,
  config_data: String,
) -> Result<vpn::VpnConfig, String> {
  let config = {
    let storage = vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

    storage
      .create_config_manual(&name, vpn_type, &config_data)
      .map_err(|e| format!("Failed to create VPN config: {e}"))?
  };

  Ok(config)
}

#[tauri::command]
async fn update_vpn_config(vpn_id: String, name: String) -> Result<vpn::VpnConfig, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .update_config_name(&vpn_id, &name)
    .map_err(|e| format!("Failed to update VPN config: {e}"))
}

#[tauri::command]
async fn check_vpn_validity(
  vpn_id: String,
) -> Result<crate::proxy_manager::ProxyCheckResult, String> {
  check_vpn_validity_core(&vpn_id).await
}

pub async fn check_vpn_validity_core(
  vpn_id: &str,
) -> Result<crate::proxy_manager::ProxyCheckResult, String> {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();

  let had_existing_worker = vpn_worker_storage::find_vpn_worker_by_vpn_id(vpn_id).is_some();

  let vpn_worker = vpn_worker_runner::start_vpn_worker(vpn_id)
    .await
    .map_err(|e| format!("Failed to start VPN worker: {e}"))?;

  let socks_url = format!(
    "socks5://127.0.0.1:{}",
    vpn_worker.local_port.unwrap_or_default()
  );

  let local_proxy = crate::proxy_runner::start_proxy_process(Some(socks_url), None)
    .await
    .map_err(|error| error.to_string());
  let local_proxy = match local_proxy {
    Ok(proxy) => proxy,
    Err(error_message) => {
      if !had_existing_worker {
        let _ = vpn_worker_runner::stop_vpn_worker(&vpn_worker.id).await;
      }
      return Err(format!("Failed to start validation proxy: {error_message}"));
    }
  };

  let local_proxy_url = format!(
    "http://127.0.0.1:{}",
    local_proxy.local_port.unwrap_or_default()
  );

  let mut result = None;
  let mut last_error = None;
  for attempt in 0..3 {
    if attempt > 0 {
      tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    match ip_utils::fetch_public_ip(Some(&local_proxy_url)).await {
      Ok(ip) => {
        let (city, country, country_code) =
          crate::proxy_manager::ProxyManager::get_ip_geolocation(&ip)
            .await
            .unwrap_or_default();

        result = Some(crate::proxy_manager::ProxyCheckResult {
          ip,
          city,
          country,
          country_code,
          timestamp: now,
          is_valid: true,
        });
        break;
      }
      Err(error) => {
        last_error = Some(error.to_string());
        log::warn!(
          "VPN validation attempt {} failed to fetch public IP through salamander-proxy: {}",
          attempt + 1,
          error
        );
      }
    }
  }

  // Grab the worker's own log before it's (possibly) stopped below — it's
  // the only place that shows whether the tunnel ever got as far as a
  // handshake attempt. Without it, a failed check here is indistinguishable
  // from "the VPN never tried anything" and "the peer never answered",
  // which otherwise means debugging the same failure only showed generic
  // HTTP timeouts, with no way to tell those two apart from the UI alone.
  let worker_log_tail = vpn_worker_runner::read_worker_log_tail(&vpn_worker.id, 20);

  let _ = crate::proxy_runner::stop_proxy_process(&local_proxy.id).await;
  if !had_existing_worker {
    let _ = vpn_worker_runner::stop_vpn_worker(&vpn_worker.id).await;
  }

  result.ok_or_else(|| {
    let network_error = last_error.unwrap_or_else(|| "no public IP response".to_string());
    match worker_log_tail {
      Some(log) => format!(
        "VPN traffic check failed after 3 attempts: {network_error}\n\nVPN worker log:\n{log}"
      ),
      None => format!("VPN traffic check failed after 3 attempts: {network_error}"),
    }
  })
}

#[tauri::command]
async fn connect_vpn(vpn_id: String) -> Result<(), String> {
  // Start VPN worker process (detached, survives GUI shutdown)
  vpn_worker_runner::start_vpn_worker(&vpn_id)
    .await
    .map_err(|e| format!("Failed to connect VPN: {e}"))?;

  // Update last_used timestamp
  {
    let storage = vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;
    let _ = storage.update_last_used(&vpn_id);
  }

  Ok(())
}

#[tauri::command]
async fn disconnect_vpn(vpn_id: String) -> Result<(), String> {
  vpn_worker_runner::stop_vpn_worker_by_vpn_id(&vpn_id)
    .await
    .map_err(|e| format!("Failed to disconnect VPN: {e}"))?;
  Ok(())
}

#[tauri::command]
async fn get_vpn_status(vpn_id: String) -> Result<vpn::VpnStatus, String> {
  use crate::proxy_storage::is_process_running;

  if let Some(worker) = vpn_worker_storage::find_vpn_worker_by_vpn_id(&vpn_id) {
    let connected = worker.pid.map(is_process_running).unwrap_or(false);
    Ok(vpn::VpnStatus {
      connected,
      vpn_id,
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    })
  } else {
    Ok(vpn::VpnStatus {
      connected: false,
      vpn_id,
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    })
  }
}

#[tauri::command]
async fn list_active_vpn_connections() -> Result<Vec<vpn::VpnStatus>, String> {
  use crate::proxy_storage::is_process_running;

  let workers = vpn_worker_storage::list_vpn_worker_configs();
  Ok(
    workers
      .into_iter()
      .filter(|w| w.pid.map(is_process_running).unwrap_or(false))
      .map(|w| vpn::VpnStatus {
        connected: true,
        vpn_id: w.vpn_id,
        connected_at: None,
        bytes_sent: None,
        bytes_received: None,
        last_handshake: None,
      })
      .collect(),
  )
}

/// Validate that a profile's selected proxy or VPN actually works before the
/// profile is created, so a dead/unreachable proxy or VPN (or a 402 from an
/// expired proxy subscription) fails creation with a clear reason. Returns
/// structured `{ "code": ... }` error strings the frontend translates via
/// backend-errors.ts.
pub async fn validate_profile_network(
  proxy_id: Option<&str>,
  vpn_id: Option<&str>,
) -> Result<(), String> {
  if let Some(vpn_id) = vpn_id.filter(|s| !s.is_empty()) {
    // Profile creation must not depend on third-party public-IP services.
    // Those services are useful for the explicit Check action, but a blocked
    // or temporarily unavailable checker must not prevent attaching a valid
    // VPN config to a profile. Starting the worker still validates the native
    // key/config and makes runtime failures visible immediately.
    let had_existing_worker = vpn_worker_storage::find_vpn_worker_by_vpn_id(vpn_id).is_some();
    vpn_worker_runner::start_vpn_worker(vpn_id)
      .await
      .map_err(|e| format!("Failed to start VPN worker: {e}"))?;
    if !had_existing_worker {
      if let Some(worker) = vpn_worker_storage::find_vpn_worker_by_vpn_id(vpn_id) {
        let _ = vpn_worker_runner::stop_vpn_worker(&worker.id).await;
      }
    }
    return Ok(());
  }

  if let Some(proxy_id) = proxy_id.filter(|s| !s.is_empty()) {
    let settings = crate::proxy_manager::PROXY_MANAGER
      .get_proxy_settings_by_id(proxy_id)
      .ok_or_else(|| format!("Proxy '{proxy_id}' not found"))?;
    match crate::proxy_manager::PROXY_MANAGER
      .check_proxy_validity(proxy_id, &settings)
      .await
    {
      Ok(result) if result.is_valid => {}
      _ => {
        return Err(serde_json::json!({ "code": "PROXY_NOT_WORKING" }).to_string());
      }
    }
  }

  Ok(())
}

/// Stop every browser this app process has a `process_id` for, so quitting
/// never leaves camoufox (or the proxy/VPN workers riding its lifecycle)
/// orphaned behind a closed window. Neither `confirm_quit` nor the tray's
/// "Quit" ever called into `browser_runner` before this — the window just
/// closed and `app_handle.exit(0)` ended this process, leaving every spawned
/// child (an independent OS process with no job-object tether on Windows)
/// running headless.
async fn kill_all_running_browsers(app_handle: &tauri::AppHandle) {
  let profile_manager = crate::profile::manager::ProfileManager::instance();
  let Ok(profiles) = profile_manager.list_profiles() else {
    return;
  };
  let browser_runner = crate::browser_runner::BrowserRunner::instance();
  for profile in profiles {
    if profile.process_id.is_some() {
      if let Err(e) = browser_runner
        .kill_browser_process(app_handle.clone(), &profile)
        .await
      {
        log::warn!(
          "Failed to stop profile '{}' while quitting: {e}",
          profile.name
        );
      }
    }
  }
}

/// VPN workers are intentionally spawned detached so a tunnel started from
/// the VPN tab (not tied to any running profile) survives the window being
/// hidden to the tray. But a real quit — the user picking "Quit" rather than
/// closing the window — must still stop them: otherwise they outlive the GUI
/// as orphaned background processes, same failure mode `kill_all_running_browsers`
/// exists to prevent for profiles, and the leftover process holds the sidecar
/// .exe open, breaking the next rebuild (`PermissionDenied` removing it).
async fn stop_all_vpn_workers() {
  for worker in crate::vpn_worker_storage::list_vpn_worker_configs() {
    if let Err(e) = crate::vpn_worker_runner::stop_vpn_worker(&worker.id).await {
      log::warn!("Failed to stop VPN worker '{}' while quitting: {e}", worker.id);
    }
  }
}

/// Confirm a quit chosen from the close-confirmation dialog and exit the app.
#[tauri::command]
async fn confirm_quit(app_handle: tauri::AppHandle) {
  kill_all_running_browsers(&app_handle).await;
  stop_all_vpn_workers().await;
  QUIT_CONFIRMED.store(true, Ordering::SeqCst);
  app_handle.exit(0);
}

/// Hide the main window so the app keeps running behind its tray icon.
#[tauri::command]
fn hide_to_tray(app_handle: tauri::AppHandle) -> Result<(), String> {
  if let Some(window) = app_handle.get_webview_window("main") {
    window.hide().map_err(|e| e.to_string())?;
  }
  Ok(())
}

#[cfg(not(feature = "e2e"))]
fn show_main_window(app_handle: &tauri::AppHandle) {
  if let Some(window) = app_handle.get_webview_window("main") {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
  }
}

/// Update the tray menu labels with localized strings pushed from the frontend
/// (which owns the active language). The item ids are unchanged so the existing
/// menu-event handler keeps matching.
#[tauri::command]
fn update_tray_menu(
  app_handle: tauri::AppHandle,
  show_label: String,
  quit_label: String,
) -> Result<(), String> {
  use tauri::menu::{MenuBuilder, MenuItemBuilder};
  if let Some(tray) = app_handle.tray_by_id("main") {
    let show_item = MenuItemBuilder::with_id("tray_show", show_label)
      .build(&app_handle)
      .map_err(|e| e.to_string())?;
    let quit_item = MenuItemBuilder::with_id("tray_quit", quit_label)
      .build(&app_handle)
      .map_err(|e| e.to_string())?;
    let menu = MenuBuilder::new(&app_handle)
      .item(&show_item)
      .separator()
      .item(&quit_item)
      .build()
      .map_err(|e| e.to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
  }
  Ok(())
}

/// Build the system tray. Best-effort: on Linux the tray depends on
/// libayatana-appindicator at runtime, so any failure here must not abort app
/// startup — the caller logs and continues without a tray.
#[cfg(not(feature = "e2e"))]
fn setup_system_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
  use std::sync::atomic::Ordering;
  use tauri::menu::{MenuBuilder, MenuItemBuilder};
  use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

  // Bootstrap labels only — the frontend pushes localized labels via
  // `update_tray_menu` on mount and on language change, and the menu is only
  // opened after a minimize-to-tray (post-mount), so these are never shown.
  let show_item = MenuItemBuilder::with_id("tray_show", "Show Salamander Browser").build(app)?;
  let quit_item = MenuItemBuilder::with_id("tray_quit", "Quit").build(app)?;
  let tray_menu = MenuBuilder::new(app)
    .item(&show_item)
    .separator()
    .item(&quit_item)
    .build()?;

  // macOS uses the black icon as a template — the OS tints it for the light or
  // dark menu bar. Linux (and other non-Windows desktops) get a white-bodied
  // icon with a dark outline so it stays legible on both dark and light
  // panels: Tauri feeds the SNI/AppIndicator a fixed pixmap with no template
  // tinting, so the icon has to carry its own contrast (a solid black icon is
  // invisible on GNOME's dark top bar). Windows keeps its own solid icon.
  #[cfg(target_os = "macos")]
  let tray_icon_bytes: &[u8] = include_bytes!("../icons/tray-icon-44.png");
  #[cfg(target_os = "windows")]
  let tray_icon_bytes: &[u8] = include_bytes!("../icons/tray-icon-win-44.png");
  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  let tray_icon_bytes: &[u8] = include_bytes!("../icons/tray-icon-linux-44.png");
  let tray_rgba = image::load_from_memory(tray_icon_bytes)?.into_rgba8();
  let (tray_w, tray_h) = tray_rgba.dimensions();
  let tray_image = tauri::image::Image::new_owned(tray_rgba.into_raw(), tray_w, tray_h);

  TrayIconBuilder::with_id("main")
    .icon(tray_image)
    .icon_as_template(cfg!(target_os = "macos"))
    .tooltip("Salamander Browser")
    .menu(&tray_menu)
    .show_menu_on_left_click(false)
    .on_menu_event(|app_handle, event| match event.id().as_ref() {
      "tray_show" => show_main_window(app_handle),
      "tray_quit" => {
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
          kill_all_running_browsers(&app_handle).await;
          stop_all_vpn_workers().await;
          QUIT_CONFIRMED.store(true, Ordering::SeqCst);
          app_handle.exit(0);
        });
      }
      _ => {}
    })
    .on_tray_icon_event(|tray, event| {
      // Click events are not delivered on Linux (AppIndicator/SNI only drives
      // the menu), so left-click-to-restore is macOS/Windows only — Linux users
      // restore via the "Show Salamander Browser" menu item.
      if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
      } = event
      {
        show_main_window(tray.app_handle());
      }
    })
    .build(app)?;

  Ok(())
}

/// Pick the things to open out of a command line.
///
/// This is how the desktop hands a browser its work. Windows and Linux both
/// start the executable with the target as an argument: a URL for a link, and a
/// plain path for a file, because the ProgId command in the registry passes
/// `%1` through unchanged. A path becomes a `file://` URL here, so callers only
/// ever deal with URLs.
///
/// A path that does not exist is ignored. Guessing at one would turn a stray
/// flag into a navigation. The first argument is the executable's own path and
/// is never a target.
fn urls_from_args<'a>(args: impl IntoIterator<Item = &'a String>) -> Vec<String> {
  args
    .into_iter()
    .skip(1)
    .filter_map(|arg| {
      if arg.starts_with("http://") || arg.starts_with("https://") {
        return Some(arg.clone());
      }

      let path = std::path::Path::new(arg);
      if !path.is_file() {
        return None;
      }

      let absolute = if path.is_absolute() {
        path.to_path_buf()
      } else {
        env::current_dir().ok()?.join(path)
      };

      url::Url::from_file_path(absolute)
        .ok()
        .map(|url| url.to_string())
    })
    .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  run_with_builder(|builder| builder);
}

#[doc(hidden)]
pub fn run_with_builder(
  configure_builder: impl FnOnce(tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry>,
) {
  // Must run before anything touches app_dirs::data_dir() (profiles, binaries,
  // proxies, VPNs, settings) — see its doc comment for why.
  app_dirs::migrate_from_previous_brand();

  let args: Vec<String> = env::args().collect();
  let startup_url = urls_from_args(args.iter()).into_iter().next();

  if let Some(url) = startup_url.clone() {
    log::info!("Found startup URL in command line");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url.clone());
  }

  let log_file_name = app_dirs::app_name();

  // Honor SALAMANDERBROWSER_DATA_ROOT and portable mode: logs go to <root>/logs or
  // <exe dir>/logs instead of the platform default app log dir, so all on-disk
  // state lives under one root rather than leaking onto the host machine.
  let file_log_target = match app_dirs::log_dir_override() {
    Some(path) => Target::new(TargetKind::Folder {
      path,
      file_name: Some(log_file_name.to_string()),
    }),
    None => Target::new(TargetKind::LogDir {
      file_name: Some(log_file_name.to_string()),
    }),
  };

  let builder = configure_builder(tauri::Builder::default());

  let builder = builder.plugin(
    tauri_plugin_log::Builder::new()
      .clear_targets() // Clear default targets to avoid duplicates
      .target(Target::new(TargetKind::Stdout))
      .target(Target::new(TargetKind::Webview))
      .target(file_log_target)
      // Keep enough context for customer support without letting a long-running
      // installation accumulate logs without bound.
      .max_file_size(5 * 1024 * 1024)
      .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(10))
      .level(log::LevelFilter::Info)
      .format(|out, message, record| {
        use chrono::Local;
        let now = Local::now();
        let timestamp = format!(
          "{}.{:03}",
          now.format("%Y-%m-%d %H:%M:%S"),
          now.timestamp_subsec_millis()
        );
        out.finish(format_args!(
          "[{}][{}][{}] {}",
          timestamp,
          record.target(),
          record.level(),
          message
        ))
      })
      .build(),
  );

  #[cfg(not(feature = "e2e"))]
  let builder = builder.plugin(tauri_plugin_single_instance::init(
    |app_handle, args, _cwd| {
      log::info!("Single instance triggered with args: {args:?}");
      if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.unminimize();
      }

      // A second launch is how the desktop hands a running browser its next
      // link. The shell starts the executable with the target in argv, this
      // callback receives that argv, and the second process exits. The callback
      // used to log the arguments and drop them, so clicking a link did nothing
      // whenever Salamander was already open, which is every time after the first.
      for url in urls_from_args(args.iter()) {
        let handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
          if let Err(e) = handle_url_open(handle, url).await {
            log::error!("Failed to handle a forwarded URL: {e}");
          }
        });
      }
    },
  ));

  let builder = builder
    .plugin(tauri_plugin_deep_link::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_macos_permissions::init())
    .plugin(tauri_plugin_clipboard_manager::init());

  #[cfg(not(feature = "e2e"))]
  let builder = builder
    // Persist window size/position across restarts. VISIBLE is excluded
    // because the app hides to tray: restoring visibility would otherwise
    // relaunch with an invisible window after quitting from the tray while
    // hidden. FULLSCREEN is excluded because native fullscreen is disabled
    // (the green button zooms instead) — the maximized flag captures the
    // "filled screen" state, including green-button zoom on macOS.
    .plugin(
      {
        let mut window_state = tauri_plugin_window_state::Builder::default();
        // Keep window geometry with the rest of the relocated state instead of
        // the host's app-config dir. The plugin only lets us name the file, so
        // the name is an absolute path; see `window_state_path_override`.
        if let Some(path) = app_dirs::window_state_path_override() {
          if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
              log::warn!("Failed to create the window-state directory: {e}");
            }
          }
          window_state = window_state.with_filename(path.to_string_lossy().into_owned());
        }
        window_state
      }
      .with_state_flags(
        tauri_plugin_window_state::StateFlags::all()
            & !tauri_plugin_window_state::StateFlags::VISIBLE
            & !tauri_plugin_window_state::StateFlags::FULLSCREEN
            // Whether the window is decorated is decided per-session by
            // `window_decorations::use_client_side_decorations()`, not by what
            // a previous run saved. Restoring it would put a real titlebar back
            // on top of the one the app draws — or strip both.
            & !tauri_plugin_window_state::StateFlags::DECORATIONS,
      )
      .build(),
    );

  builder
    .setup(|app| {
      // Extract icons and metadata for existing extensions that don't have them yet
      {
        let mgr = extension_manager::ExtensionManager::new();
        mgr.ensure_icons_extracted();
      }

      // Create the main window programmatically
      #[allow(unused_variables)]
      let win_builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Salamander Browser")
        .inner_size(880.0, 500.0)
        .min_inner_size(640.0, 400.0)
        .resizable(true)
        .fullscreen(false)
        .center()
        .focused(true)
        .visible(true);

      #[cfg(feature = "e2e")]
      let win_builder = match e2e_automation_profile_dir() {
        Some(profile_dir) => win_builder
          .data_directory(profile_dir.join("webview"))
          // WKWebView ignores data_directory on macOS. Incognito gives every
          // launched app process a non-persistent data store there, and also
          // prevents WebView2/WebKitGTK caches from escaping the session on
          // the other platforms. Durable app state is still exercised via
          // SALAMANDERBROWSER_DATA_ROOT; only browser-engine storage is temporary.
          .incognito(true),
        None => win_builder,
      };

      // The app draws its own titlebar. macOS keeps the native one and makes
      // it transparent (below); Windows and Linux drop decorations entirely and
      // render their own controls.
      #[cfg(target_os = "windows")]
      let win_builder = win_builder.decorations(false);

      // Linux opts out on the one configuration where dropping decorations can
      // make things worse rather than better — see `use_client_side_decorations`.
      #[cfg(target_os = "linux")]
      let win_builder = if window_decorations::use_client_side_decorations() {
        win_builder.decorations(false)
      } else {
        win_builder
      };

      #[allow(unused_variables)]
      let window = win_builder.build().unwrap();

      // System tray so the user can keep the app running after the close
      // dialog's "Minimize" action hides the window. Best-effort: a tray
      // failure (e.g. missing libayatana-appindicator on Linux) must never
      // prevent the app from launching, so we log and continue without it.
      #[cfg(not(feature = "e2e"))]
      {
        if let Err(e) = setup_system_tray(app.handle()) {
          log::warn!("System tray unavailable, continuing without it: {e}");
        }
      }

      // Intercept the window close so the frontend can ask the user whether
      // to minimize or quit. The app exits when `confirm_quit` flips
      // QUIT_CONFIRMED — until then, every CloseRequested is held back.
      {
        let app_handle = app.handle().clone();
        window.on_window_event(move |event| {
          if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            if QUIT_CONFIRMED.load(Ordering::SeqCst) {
              return;
            }
            api.prevent_close();
            if let Err(e) = app_handle.emit("close-confirm-requested", ()) {
              log::warn!("Failed to emit close-confirm-requested: {e}");
            }
          }
        });
      }

      // Publish the desktop's titlebar button layout to the frontend. Runs
      // here because `setup` is the GTK main thread, which `gtk::Settings`
      // requires.
      //
      // The decorated state is logged alongside it: "my window has no titlebar"
      // and "my window has two titlebars" are both reports that hinge on this
      // one boolean, and it is otherwise invisible after the fact.
      #[cfg(target_os = "linux")]
      {
        log::info!(
          "Linux window decorations: server-side = {:?}",
          window.is_decorated()
        );

        // tao makes the window visible before it clears the decorations, so it
        // is realized while still framed and the frame extents come out of the
        // size we asked for (a requested 880x500 arrives noticeably smaller).
        //
        // Only correct that on a first run. Once window-state has geometry
        // saved, that geometry is the user's and has already been restored —
        // re-applying the default here would move and resize their window on
        // every launch, and the plugin would then persist the reset.
        // Must resolve through the same helper the plugin was configured with:
        // probing the platform default while the plugin writes elsewhere would
        // read "first run" on every launch and reset the user's window.
        let has_saved_geometry = app_dirs::window_state_path(app.handle())
          .map(|path| path.exists())
          .unwrap_or(false);
        if window_decorations::use_client_side_decorations() && !has_saved_geometry {
          if let Err(e) = window.set_size(tauri::LogicalSize::new(880.0, 500.0)) {
            log::warn!("Failed to re-apply the window size after dropping decorations: {e}");
          }
          if let Err(e) = window.center() {
            log::warn!("Failed to re-center the window after dropping decorations: {e}");
          }
        }
      }
      window_decorations::init(app.handle());

      // Set transparent titlebar for macOS
      #[cfg(target_os = "macos")]
      {
        if let Err(e) = window.set_transparent_titlebar(true) {
          log::warn!("Failed to set transparent titlebar: {e}");
        }
        // Green title-bar button maximizes (zoom) the window rather than
        // entering immersive native fullscreen.
        if let Err(e) = window.disable_native_fullscreen() {
          log::warn!("Failed to disable native fullscreen: {e}");
        }
      }

      // Set up deep link handler
      let handle = app.handle().clone();

      // Initialize the global event emitter for the events module
      let emitter = std::sync::Arc::new(events::TauriEmitter::new(handle.clone()));
      if let Err(e) = events::set_global_emitter(emitter) {
        log::warn!("Failed to set global event emitter: {e}");
      }

      #[cfg(all(windows, not(feature = "e2e")))]
      {
        // For Windows, register all deep links at runtime
        if let Err(e) = app.deep_link().register_all() {
          log::warn!("Failed to register deep links: {e}");
        }
      }

      #[cfg(all(target_os = "macos", not(feature = "e2e")))]
      {
        // On macOS, try to register deep links for development builds
        if let Err(e) = app.deep_link().register_all() {
          log::debug!(
            "Note: Deep link registration failed on macOS (this is normal for production): {e}"
          );
        }
      }

      #[cfg(not(feature = "e2e"))]
      {
        app.deep_link().on_open_url({
          let handle = handle.clone();
          move |event| {
            let urls = event.urls();
            log::info!("Deep link event received with {} URLs", urls.len());

            for url in urls {
              let url_string = url.to_string();
              log::info!("Processing deep link URL");
              let handle_clone = handle.clone();

              tauri::async_runtime::spawn(async move {
                if let Err(e) = handle_url_open(handle_clone, url_string.clone()).await {
                  log::error!("Failed to handle deep link URL: {e}");
                }
              });
            }
          }
        });
      }

      if let Some(startup_url) = startup_url {
        let handle_clone = handle.clone();
        tauri::async_runtime::spawn(async move {
          log::info!("Processing startup URL from command line");
          if let Err(e) = handle_url_open(handle_clone, startup_url.clone()).await {
            log::error!("Failed to handle startup URL: {e}");
          }
        });
      }

      if !e2e_automation_enabled() {
        // Initialize and start background version updater
        let app_handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
          let version_updater = get_version_updater();

          {
            let mut updater_guard = version_updater.lock().await;
            updater_guard.set_app_handle(app_handle);
          }

          {
            let updater_guard = version_updater.lock().await;
            if let Err(e) = updater_guard.start_background_updates().await {
              log::error!("Failed to start background updates: {e}");
            }
          }
        });

        tauri::async_runtime::spawn(async move {
          version_updater::VersionUpdater::run_background_task().await;
        });
      }

      // Clear stale process IDs from profiles (processes that died while app was closed).
      // Runs synchronously here, before the frontend's first `list_browser_profiles`
      // call, so a genuinely-dead PID is never even visible to the UI. The event is
      // still emitted for the case the frontend already has a subscriber attached
      // from a prior window (e.g. a reload) — `use-profile-events.ts` hydrates
      // `runningProfiles` from the initial list but documents that it expects
      // corrections to arrive as `profile-running-changed`, same as every other
      // running-state transition.
      {
        let profile_manager = crate::profile::ProfileManager::instance();
        if let Ok(profiles) = profile_manager.list_profiles() {
          let system = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing()
              .with_processes(sysinfo::ProcessRefreshKind::everything()),
          );
          for profile in profiles {
            if let Some(pid) = profile.process_id {
              let sysinfo_pid = sysinfo::Pid::from_u32(pid);
              if system.process(sysinfo_pid).is_none() {
                log::info!(
                  "Clearing stale process_id {} for profile {}",
                  pid,
                  profile.name
                );
                let mut updated = profile.clone();
                updated.process_id = None;
                let _ = profile_manager.save_profile(&updated);
                let _ = events::emit(
                  "profile-running-changed",
                  serde_json::json!({ "id": updated.id.to_string(), "is_running": false }),
                );
              }
            }
          }
        }
      }

      // Kill orphaned proxy and VPN worker processes from previous app runs.
      // Since active_proxies is an in-memory map that starts empty, any running
      // salamander-proxy workers on disk must be orphans the current app can't track.
      // Without this cleanup, users on Windows accumulate dozens of idle workers
      // (one per profile launch) that the periodic cleanup won't touch because
      // profile-associated workers are deliberately skipped to avoid regressions.
      //
      // Preserves workers whose associated profile still has a running browser
      // process — if the app crashed while a browser was running, its detached
      // browser keeps going and needs the proxy/VPN worker to stay alive.
      tauri::async_runtime::spawn(async move {
        use crate::proxy_storage::{delete_proxy_config, is_process_running, list_proxy_configs};
        use crate::vpn_worker_storage::{delete_vpn_worker_config, list_vpn_worker_configs};

        // Build sets of (profile_id, vpn_id) whose browsers are still running
        let profile_manager = crate::profile::ProfileManager::instance();
        let profiles = profile_manager.list_profiles().unwrap_or_default();

        let running_profile_ids: std::collections::HashSet<String> = profiles
          .iter()
          .filter(|p| p.process_id.is_some_and(is_process_running))
          .map(|p| p.id.to_string())
          .collect();

        let running_vpn_ids: std::collections::HashSet<String> = profiles
          .iter()
          .filter(|p| p.process_id.is_some_and(is_process_running))
          .filter_map(|p| p.vpn_id.clone())
          .collect();

        for config in list_proxy_configs() {
          // Prefer the owner identity the worker itself recorded: it pins the
          // browser to one exact process, so a recycled PID cannot make a dead
          // browser look alive and strand this worker for good. Configs written
          // before that field existed fall back to the profile's stored PID,
          // which is all the information those workers have.
          let has_running_browser = if config.browser_pid_start_time.is_some() {
            crate::proxy_storage::browser_owner_is_alive(&config)
          } else {
            config
              .profile_id
              .as_ref()
              .is_some_and(|pid| running_profile_ids.contains(pid))
          };
          if has_running_browser {
            log::info!(
              "Startup: preserving proxy worker {} (browser still running)",
              config.id
            );
            continue;
          }

          if let Some(pid) = config.pid {
            if is_process_running(pid) {
              log::info!(
                "Startup: killing orphaned proxy worker {} (PID {})",
                config.id,
                pid
              );
              let _ = crate::proxy_runner::stop_proxy_process(&config.id).await;
              continue;
            }
          }
          delete_proxy_config(&config.id);
        }

        for worker in list_vpn_worker_configs() {
          if running_vpn_ids.contains(&worker.vpn_id) {
            log::info!(
              "Startup: preserving VPN worker {} (profile browser using vpn_id {} still running)",
              worker.id,
              worker.vpn_id
            );
            continue;
          }

          if let Some(pid) = worker.pid {
            if is_process_running(pid) {
              log::info!(
                "Startup: killing orphaned VPN worker {} (PID {})",
                worker.id,
                pid
              );
              let _ = crate::vpn_worker_runner::stop_vpn_worker(&worker.id).await;
              continue;
            }
          }
          delete_vpn_worker_config(&worker.id);
        }
      });

      // Immediately bump non-running profiles to the latest installed browser version.
      // This runs synchronously before any network calls so profiles are updated on launch.
      {
        let app_handle_bump = app.handle().clone();
        match auto_updater::AutoUpdater::instance()
          .update_profiles_to_latest_installed(&app_handle_bump)
        {
          Ok(updated) => {
            if !updated.is_empty() {
              log::info!(
                "Startup: bumped {} profiles to latest installed versions: {:?}",
                updated.len(),
                updated
              );
            }
          }
          Err(e) => {
            log::error!("Startup: failed to bump profiles to latest installed versions: {e}");
          }
        }
      }

      if !e2e_automation_enabled() {
        let app_handle_auto_updater = app.handle().clone();
        tauri::async_runtime::spawn(async move {
          auto_updater::check_for_updates_with_progress(app_handle_auto_updater).await;
        });
      }

      // Handle any pending URLs that were received before the window was ready
      let handle_pending = handle.clone();
      tauri::async_runtime::spawn(async move {
        // Wait a bit for the window to be fully ready
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        let pending_urls = {
          let mut pending = PENDING_URLS.lock().unwrap();
          let urls = pending.clone();
          pending.clear();
          urls
        };

        for url in pending_urls {
          log::info!("Processing pending URL");
          if let Err(e) = handle_url_open(handle_pending.clone(), url).await {
            log::error!("Failed to handle pending URL: {e}");
          }
        }
      });

      if !e2e_automation_enabled() {
        // Start periodic cleanup task for unused binaries.
        tauri::async_runtime::spawn(async move {
          let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(43200));
          loop {
            interval.tick().await;

            let registry =
              crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
            if let Err(e) = registry.cleanup_unused_binaries() {
              log::error!("Periodic cleanup failed: {e}");
            } else {
              log::debug!("Periodic cleanup completed successfully");
            }
          }
        });

        tauri::async_runtime::spawn(async move {
          let manager = dns_blocklist::BlocklistManager::instance();
          let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(43200));
          interval.tick().await;
          loop {
            interval.tick().await;
            manager.refresh_all_stale().await;
          }
        });

        tauri::async_runtime::spawn(async move {
          let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3 * 60 * 60));
          loop {
            interval.tick().await;
            log::info!("Checking for app updates...");
            match app_auto_updater::check_for_app_updates().await {
              Ok(Some(update_info)) => {
                log::info!(
                  "App update available: {} -> {}",
                  update_info.current_version,
                  update_info.new_version
                );
                if let Err(e) = events::emit("app-update-available", &update_info) {
                  log::error!("Failed to emit app update event: {e}");
                }
              }
              Ok(None) => log::debug!("No app updates available"),
              Err(e) => log::error!("Failed to check for app updates: {e}"),
            }
          }
        });

        let app_handle_geoip = app.handle().clone();
        tauri::async_runtime::spawn(async move {
          tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
          let geoip_downloader = crate::geoip_downloader::GeoIPDownloader::instance();
          match geoip_downloader.check_missing_geoip_database() {
            Ok(true) => {
              log::info!(
                "GeoIP database is missing for Camoufox profiles, downloading at startup..."
              );
              let geoip_downloader = GeoIPDownloader::instance();
              if let Err(e) = geoip_downloader
                .download_geoip_database(&app_handle_geoip)
                .await
              {
                log::error!("Failed to download GeoIP database at startup: {e}");
              } else {
                log::info!("GeoIP database downloaded successfully at startup");
              }
            }
            Ok(false) => {}
            Err(e) => log::error!("Failed to check GeoIP database status at startup: {e}"),
          }
        });
      }

      // Start proxy cleanup task for dead browser processes
      let app_handle_proxy_cleanup = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

        loop {
          interval.tick().await;

          match crate::proxy_manager::PROXY_MANAGER
            .cleanup_dead_proxies(app_handle_proxy_cleanup.clone())
            .await
          {
            Ok(dead_pids) => {
              if !dead_pids.is_empty() {
                log::info!(
                  "Cleaned up proxies for {} dead browser processes",
                  dead_pids.len()
                );
              }
            }
            Err(e) => {
              log::error!("Error during proxy cleanup: {e}");
            }
          }
        }
      });

      // Periodically broadcast browser running status to the frontend.
      // When no profiles have stored PIDs (nothing was ever launched this
      // session), we use a long interval (30s) to avoid burning CPU on
      // full process-table scans via sysinfo. Once any profile is running
      // we switch to the fast interval (5s) for responsive UI updates.
      let app_handle_status = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        const FAST_INTERVAL_SECS: u64 = 5;
        const IDLE_INTERVAL_SECS: u64 = 30;

        let mut interval =
          tokio::time::interval(tokio::time::Duration::from_secs(FAST_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_running_states: std::collections::HashMap<String, bool> =
          std::collections::HashMap::new();
        let mut current_interval_secs = FAST_INTERVAL_SECS;

        loop {
          interval.tick().await;

          let runner = crate::browser_runner::BrowserRunner::instance();
          let profiles = match runner.profile_manager.list_profiles() {
            Ok(p) => p,
            Err(e) => {
              log::warn!("Failed to list profiles in status checker: {e}");
              continue;
            }
          };

          // If no profile has a stored PID and we have no previously-known
          // running states, there's nothing to check — skip the expensive
          // process scan entirely.
          let any_has_pid = profiles.iter().any(|p| p.process_id.is_some());
          let any_was_running = last_running_states.values().any(|&v| v);

          if !any_has_pid && !any_was_running {
            // Switch to the idle interval to reduce CPU
            if current_interval_secs != IDLE_INTERVAL_SECS {
              current_interval_secs = IDLE_INTERVAL_SECS;
              interval =
                tokio::time::interval(tokio::time::Duration::from_secs(IDLE_INTERVAL_SECS));
              interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            }
            continue;
          }

          // At least one profile might be running — use the fast interval
          if current_interval_secs != FAST_INTERVAL_SECS {
            current_interval_secs = FAST_INTERVAL_SECS;
            interval = tokio::time::interval(tokio::time::Duration::from_secs(FAST_INTERVAL_SECS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
          }

          // Only walk profiles that either have a stored PID or that we last
          // saw as running — for users with hundreds of idle profiles this
          // turns an O(N) sysinfo scan into an O(running) scan. The Rust
          // launch path always emits profile-running-changed when a profile
          // STARTS, so newly-running profiles still get tracked here.
          let profiles_to_check: Vec<_> = profiles
            .into_iter()
            .filter(|p| {
              p.process_id.is_some()
                || last_running_states
                  .get(&p.id.to_string())
                  .copied()
                  .unwrap_or(false)
            })
            .collect();

          for profile in profiles_to_check {
            let had_pid = profile.process_id.is_some();
            // Check browser status and track changes
            match runner
              .check_browser_status(app_handle_status.clone(), &profile)
              .await
            {
              Ok(is_running) => {
                let profile_id = profile.id.to_string();
                let last_state = last_running_states
                  .get(&profile_id)
                  .copied()
                  .unwrap_or(false);

                // Emit when the running state changed, or when we still had a
                // stored PID but the browser is gone — the launch path sets the
                // frontend to "running" immediately, and a missed transition
                // here leaves the stop button stuck.
                let should_emit = last_state != is_running || (!is_running && had_pid);

                  if should_emit {
                  log::debug!(
                    "Status checker detected change for profile {}: {} -> {}",
                    profile.name,
                    last_state,
                    is_running
                  );

                  #[derive(serde::Serialize)]
                  struct RunningChangedPayload {
                    id: String,
                    is_running: bool,
                  }

                  let payload = RunningChangedPayload {
                    id: profile_id.clone(),
                    is_running,
                  };

                  if let Err(e) = events::emit("profile-running-changed", &payload) {
                    log::warn!("Failed to emit profile running changed event: {e}");
                  } else {
                    log::debug!(
                      "Status checker emitted profile-running-changed event for {}: running={}",
                      profile.name,
                      is_running
                    );
                  }

                  if !is_running && (last_state || had_pid) && profile.password_protected {
                    crate::profile::password::complete_after_quit_and_wait(&profile).await;
                  }

                  last_running_states.insert(profile_id, is_running);
                } else {
                  // Update the state even if unchanged to ensure we have it tracked
                  last_running_states.insert(profile_id, is_running);
                }
              }
              Err(e) => {
                log::warn!("Status check failed for profile {}: {}", profile.name, e);
                continue;
              }
            }
          }
        }
      });

      // Nodecar warm-up is now triggered from the frontend to allow UI blocking overlay

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      confirm_quit,
      hide_to_tray,
      update_tray_menu,
      get_supported_browsers,
      is_browser_supported_on_platform,
      download_browser,
      cancel_download,
      delete_profile,
      clone_profile,
      check_browser_exists,
      create_browser_profile_new,
      list_browser_profiles,
      launch_browser_profile,
      fetch_browser_versions_with_count,
      fetch_browser_versions_cached_first,
      fetch_browser_versions_with_count_cached_first,
      get_downloaded_browser_versions,
      get_all_tags,
      get_browser_release_types,
      update_profile_proxy,
      update_profile_vpn,
      update_profile_tags,
      update_profile_note,
      update_profile_window_color,
      set_profile_password,
      change_profile_password,
      remove_profile_password,
      verify_profile_password,
      unlock_profile,
      lock_profile,
      is_profile_locked,
      update_profile_proxy_bypass_rules,
      update_profile_dns_blocklist,
      check_browser_status,
      kill_browser_profile,
      rename_profile,
      get_app_settings,
      save_app_settings,
      read_log_files,
      open_log_directory,
      get_table_sorting_settings,
      save_table_sorting_settings,
      get_system_language,
      get_system_info,
      dismiss_window_resize_warning,
      get_window_resize_warning_dismissed,
      get_onboarding_completed,
      complete_onboarding,
      clear_all_version_cache_and_refetch,
      is_default_browser,
      open_url_with_profile,
      set_as_default_browser,
      trigger_manual_version_update,
      get_version_update_status,
      check_for_browser_updates,
      dismiss_update_notification,
      complete_browser_update_with_auto_update,
      check_for_app_updates,
      check_for_app_updates_manual,
      download_and_prepare_app_update,
      restart_application,
      import_camoufox_profile,
      export_profiles_archive,
      import_profiles_archive,
      inspect_import_source,
      import_data_directory,
      check_missing_binaries,
      check_missing_geoip_database,
      ensure_all_binaries_exist,
      ensure_active_browsers_downloaded,
      create_stored_proxy,
      get_stored_proxies,
      update_stored_proxy,
      delete_stored_proxy,
      check_proxy_validity,
      get_cached_proxy_check,
      export_proxies,
      import_proxies_json,
      parse_txt_proxies,
      import_proxies_from_parsed,
      update_camoufox_config,
      update_browser_config,
      get_profile_groups,
      get_groups_with_profile_counts,
      create_profile_group,
      update_profile_group,
      delete_profile_group,
      assign_profiles_to_group,
      delete_selected_profiles,
      list_extensions,
      get_extension_icon,
      add_extension,
      add_unpacked_extension,
      update_extension,
      update_extension_from_path,
      delete_extension,
      list_extension_groups,
      create_extension_group,
      update_extension_group,
      delete_extension_group,
      add_extension_to_group,
      remove_extension_from_group,
      assign_extension_group_to_profile,
      get_extension_group_for_profile,
      is_geoip_database_available,
      download_geoip_database,
      get_all_traffic_snapshots,
      get_profile_traffic_snapshot,
      clear_all_traffic_stats,
      clear_profile_traffic_stats,
      get_traffic_stats_for_period,
      fingerprint_consistency::match_profile_fingerprint_to_exit,
      launch_gate::get_profile_pre_launch_checks,
      launch_gate::ack_launch_gate,
      window_decorations::get_window_decoration_layout,
      validate_vless_uri,
      fetch_vless_subscription,
      read_profile_cookies,
      get_profile_cookie_stats,
      copy_profile_cookies,
      analyze_pasted_cookies,
      import_pasted_cookies,
      export_profile_cookies,
      // VPN commands
      import_vpn_config,
      list_vpn_configs,
      get_vpn_config,
      delete_vpn_config,
      create_vpn_config_manual,
      update_vpn_config,
      check_vpn_validity,
      connect_vpn,
      disconnect_vpn,
      get_vpn_status,
      list_active_vpn_connections,
      // DNS blocklist commands
      dns_blocklist::get_dns_blocklist_cache_status,
      dns_blocklist::refresh_dns_blocklists,
      dns_blocklist::get_custom_dns_config,
      dns_blocklist::set_custom_dns_config,
      dns_blocklist::import_custom_dns_rules,
      dns_blocklist::export_custom_dns_rules,
    ])
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|_app_handle, _event| {
      #[cfg(target_os = "macos")]
      if let tauri::RunEvent::Reopen { .. } = _event {
        if let Some(window) = _app_handle.get_webview_window("main") {
          let _ = window.show();
          let _ = window.set_focus();
          let _ = window.unminimize();
        }
      }
    });
}

#[cfg(test)]
mod tests {
  use std::fs;

  #[test]
  fn a_command_line_yields_the_links_and_files_it_carries() {
    let exe = "C:/Program Files/Salamander Browser/salamanderbrowser.exe".to_string();

    // The executable's own path leads every command line and is not a target,
    // even on a machine where that path happens to exist.
    assert!(super::urls_from_args([&exe]).is_empty());

    let link = "https://example.com/a?b=c".to_string();
    let insecure = "http://example.com".to_string();
    assert_eq!(
      super::urls_from_args([&exe, &link, &insecure]),
      vec![link.clone(), insecure]
    );

    // Flags and stray words are not links. The old filter took anything
    // starting with "http", which is looser than it looks.
    let flag = "--headless".to_string();
    let near_miss = "httpsomething".to_string();
    assert_eq!(
      super::urls_from_args([&exe, &flag, &near_miss]),
      Vec::<String>::new()
    );

    // A path that is not there is ignored rather than guessed at.
    let missing = "C:/no/such/page.html".to_string();
    assert!(super::urls_from_args([&exe, &missing]).is_empty());

    // Explorer hands a browser a bare path, not a URL, because the registered
    // command passes `%1` straight through. Turning it into a `file://` URL
    // here is what makes the .html association in `default_browser.rs` a real
    // claim rather than an empty one.
    let directory = tempfile::tempdir().expect("temp dir");
    let page = directory.path().join("page.html");
    fs::write(&page, "<html></html>").expect("write the page");
    let page_arg = page.to_string_lossy().to_string();

    let found = super::urls_from_args([&exe, &page_arg]);
    assert_eq!(found.len(), 1, "the file should have produced one URL");
    assert!(
      found[0].starts_with("file:///"),
      "expected a file URL, got {}",
      found[0]
    );
    assert!(
      found[0].ends_with("page.html"),
      "expected the page's own name, got {}",
      found[0]
    );
  }

  #[test]
  fn backend_error_helpers_preserve_codes_and_structure_diagnostics() {
    let coded = super::backend_error("PROFILE_NOT_FOUND");
    assert_eq!(
      serde_json::from_str::<serde_json::Value>(&coded).unwrap()["code"],
      "PROFILE_NOT_FOUND"
    );
    assert_eq!(super::wrap_backend_error(&coded, "ignored"), coded);

    let wrapped = super::wrap_backend_error("disk unavailable", "Failed to save");
    let parsed = serde_json::from_str::<serde_json::Value>(&wrapped).unwrap();
    assert_eq!(parsed["code"], "INTERNAL_ERROR");
    assert_eq!(
      parsed["params"]["detail"],
      "Failed to save: disk unavailable"
    );
  }

  #[test]
  fn test_no_unused_tauri_commands() {
    check_unused_commands(false); // Run in strict mode for CI
  }

  #[test]
  fn test_unused_tauri_commands_detailed() {
    check_unused_commands(true); // Run in verbose mode for development
  }

  fn check_unused_commands(verbose: bool) {
    // Commands that are intentionally not called directly from the frontend.
    // The VPN control commands below let a VPN be connected/disconnected/
    // inspected independent of launching a browser profile (profile launch
    // itself calls vpn_worker_runner directly); no UI exposes standalone VPN
    // control yet, but they're real capabilities exercised by the e2e suite
    // (see e2e/coverage-map.mjs).
    let frontend_exempt_commands = [
      "connect_vpn",
      "disconnect_vpn",
      "get_vpn_status",
      "get_vpn_config",
      "list_active_vpn_connections",
      "verify_profile_password",
      "lock_profile",
      "is_profile_locked",
    ];

    // Extract command names from the generate_handler! macro in this file
    let lib_rs_content = fs::read_to_string("src/lib.rs").expect("Failed to read lib.rs");
    let commands = extract_tauri_commands(&lib_rs_content);

    // Get all frontend files
    let frontend_files = get_frontend_files("../src");

    // Check which commands are actually used
    let mut unused_commands = Vec::new();
    let mut used_commands = Vec::new();

    for command in &commands {
      // Skip commands that are intentionally exempt from the frontend-usage check
      if frontend_exempt_commands.contains(&command.as_str()) {
        used_commands.push(command.clone());
        if verbose {
          println!("✅ {command} (frontend-exempt)");
        }
        continue;
      }

      let mut is_used = false;

      for file_content in &frontend_files {
        // More comprehensive search for command usage
        if is_command_used(file_content, command) {
          is_used = true;
          break;
        }
      }

      if is_used {
        used_commands.push(command.clone());
        if verbose {
          println!("✅ {command}");
        }
      } else {
        unused_commands.push(command.clone());
        if verbose {
          println!("❌ {command} (UNUSED)");
        }
      }
    }

    if verbose {
      println!("\n📊 Summary:");
      println!("  ✅ Used commands: {}", used_commands.len());
      println!("  ❌ Unused commands: {}", unused_commands.len());
    }

    if !unused_commands.is_empty() {
      let message = format!(
        "Found {} unused Tauri commands: {}\n\nThese commands are exported in generate_handler! but not used in the frontend.\nConsider removing them or add them to the allowlist if they're used elsewhere.\n\nRun `pnpm check-unused-commands` for detailed analysis.",
        unused_commands.len(),
        unused_commands.join(", ")
      );

      if verbose {
        println!("\n🚨 {message}");
      } else {
        panic!("{}", message);
      }
    } else if verbose {
      println!("\n🎉 All exported commands are being used!");
    } else {
      println!(
        "✅ All {} exported Tauri commands are being used in the frontend",
        commands.len()
      );
    }
  }

  fn is_command_used(content: &str, command: &str) -> bool {
    // Check various patterns for invoke usage
    let patterns = vec![
      format!("invoke<{}>(\"{}\"", "", command), // invoke<Type>("command"
      format!("invoke(\"{}\"", command),         // invoke("command"
      format!("invoke<{}>(\"{}\",", "", command), // invoke<Type>("command",
      format!("invoke(\"{}\",", command),        // invoke("command",
      format!("\"{}\"", command),                // Just the command name in quotes
    ];

    for pattern in patterns {
      if content.contains(&pattern) {
        return true;
      }
    }

    // Also check for the command name appearing after "invoke" within a reasonable distance
    if let Some(invoke_pos) = content.find("invoke") {
      let after_invoke = &content[invoke_pos..];
      if let Some(cmd_pos) = after_invoke.find(&format!("\"{command}\"")) {
        // If the command appears within 100 characters of "invoke", consider it used
        if cmd_pos < 100 {
          return true;
        }
      }
    }

    false
  }

  fn extract_tauri_commands(content: &str) -> Vec<String> {
    let mut commands = Vec::new();

    // Find the generate_handler! macro
    if let Some(start) = content.find("tauri::generate_handler![") {
      if let Some(end) = content[start..].find("])") {
        let handler_content = &content[start + 25..start + end]; // Skip "tauri::generate_handler!["

        // Extract command names
        for line in handler_content.lines() {
          let line = line.trim();
          if !line.is_empty() && !line.starts_with("//") {
            // Remove trailing comma and whitespace
            let command = line.trim_end_matches(',').trim();
            if !command.is_empty() {
              // Strip module prefix (e.g., "browser_runner::launch_browser_profile" -> "launch_browser_profile")
              let command = command.rsplit("::").next().unwrap_or(command);
              commands.push(command.to_string());
            }
          }
        }
      }
    }

    commands
  }

  fn get_frontend_files(src_dir: &str) -> Vec<String> {
    let mut files_content = Vec::new();

    if let Ok(entries) = fs::read_dir(src_dir) {
      for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
          // Recursively read subdirectories
          let subdir_files = get_frontend_files(&path.to_string_lossy());
          files_content.extend(subdir_files);
        } else if let Some(extension) = path.extension() {
          if matches!(
            extension.to_str(),
            Some("ts") | Some("tsx") | Some("js") | Some("jsx")
          ) {
            if let Ok(content) = fs::read_to_string(&path) {
              files_content.push(content);
            }
          }
        }
      }
    }

    files_content
  }
}
