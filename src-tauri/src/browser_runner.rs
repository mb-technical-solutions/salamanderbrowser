use crate::browser::ProxySettings;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static PROFILE_LAUNCH_LOCKS: LazyLock<
  tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

async fn lock_profile_launch(profile_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
  let lock = {
    let mut locks = PROFILE_LAUNCH_LOCKS.lock().await;
    locks
      .entry(profile_id.to_string())
      .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
      .clone()
  };
  lock.lock_owned().await
}

pub struct BrowserRunner {
  pub profile_manager: &'static ProfileManager,
  pub downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  auto_updater: &'static crate::auto_updater::AutoUpdater,
  camoufox_manager: &'static crate::camoufox_manager::CamoufoxManager,
}

impl BrowserRunner {
  fn new() -> Self {
    Self {
      profile_manager: ProfileManager::instance(),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      auto_updater: crate::auto_updater::AutoUpdater::instance(),
      camoufox_manager: crate::camoufox_manager::CamoufoxManager::instance(),
    }
  }

  pub fn instance() -> &'static BrowserRunner {
    &BROWSER_RUNNER
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    crate::app_dirs::binaries_dir()
  }

  /// Resolve the DNS blocklist level to a cached file path plus whether that
  /// file should be treated as an allowlist. If a level is set but the cache
  /// is missing, fetches/compiles on demand (blocks until done).
  ///
  /// Currently unreachable: DNS-blocklist enforcement was wired only into
  /// Wayfern's local-proxy-worker launch path, and Camoufox (the only engine
  /// left) doesn't route through a local proxy worker yet. Kept rather than
  /// deleted, since wiring blocklist enforcement into Camoufox's launch is a
  /// documented follow-up, not something ruled out.
  #[allow(dead_code)]
  async fn resolve_blocklist_file(
    profile: &crate::profile::BrowserProfile,
  ) -> Result<(Option<String>, bool), String> {
    let Some(ref level_str) = profile.dns_blocklist else {
      return Ok((None, false));
    };
    let Some(level) = crate::dns_blocklist::BlocklistLevel::parse_level(level_str) else {
      return Ok((None, false));
    };
    if level == crate::dns_blocklist::BlocklistLevel::None {
      return Ok((None, false));
    }
    // Only the user's custom list can be an allowlist; the Hagezi tiers are
    // always block lists.
    let allowlist_mode = level == crate::dns_blocklist::BlocklistLevel::Custom
      && crate::dns_blocklist::CustomDnsConfig::load().allowlist_mode;
    let path = crate::dns_blocklist::BlocklistManager::ensure_cached(level)
      .await
      .map_err(|e| format!("Failed to fetch DNS blocklist: {e}"))?;
    Ok((Some(path.to_string_lossy().to_string()), allowlist_mode))
  }

  /// Resolve the proxy settings for a stored proxy ID.
  async fn resolve_proxy_with_refresh(
    &self,
    proxy_id: Option<&String>,
  ) -> Result<Option<ProxySettings>, String> {
    let proxy_id = match proxy_id {
      Some(id) => id,
      None => return Ok(None),
    };
    Ok(PROXY_MANAGER.get_proxy_settings_by_id(proxy_id))
  }

  fn fire_launch_hook(profile: &BrowserProfile) {
    let Some(raw_url) = profile.launch_hook.as_deref() else {
      return;
    };
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
      return;
    }

    let parsed = match url::Url::parse(trimmed) {
      Ok(u) => u,
      Err(e) => {
        log::warn!(
          "Skipping launch hook for profile {} (ID: {}): invalid URL: {e}",
          profile.name,
          profile.id
        );
        return;
      }
    };

    if !matches!(parsed.scheme(), "http" | "https") {
      log::warn!(
        "Skipping launch hook for profile {} (ID: {}): URL must be http or https",
        profile.name,
        profile.id
      );
      return;
    }

    let url = parsed.to_string();
    let url_label = crate::log_redaction::url_label(&url);

    log::info!("Firing launch hook GET {url_label}");

    tokio::spawn(async move {
      let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
      {
        Ok(c) => c,
        Err(e) => {
          log::warn!(
            "Launch hook client build failed: {}",
            crate::log_redaction::text(&e.to_string())
          );
          return;
        }
      };

      match client.get(&url).send().await {
        Ok(resp) => {
          log::info!("Launch hook {url_label} returned status {}", resp.status());
        }
        Err(e) => {
          log::warn!(
            "Launch hook {url_label} failed: {}",
            crate::log_redaction::text(&e.to_string())
          );
        }
      }
    });
  }

  /// Resolve the upstream a launch will use.
  ///
  /// Deliberately does NOT fire the launch hook: that moved below the gate, so
  /// a launch the user blocks and then retries calls the user's webhook once
  /// rather than once per attempt.
  ///
  /// `vpn_id` and `proxy_id` are mutually exclusive on a profile (enforced at
  /// creation), so a VPN-assigned profile is checked first: the userspace
  /// WireGuard worker (`vpn_worker_runner`) exposes the tunnel as a local
  /// SOCKS5 endpoint the same way `check_vpn_validity_core` does for the
  /// pre-creation connectivity check, and that endpoint is what actually has
  /// to reach `CamoufoxConfig.proxy` — otherwise the browser launches with no
  /// proxy configured at all and silently connects directly, defeating the
  /// VPN (and the profile's fingerprint/exit consistency) without any error.
  async fn resolve_launch_proxy(
    &self,
    profile: &BrowserProfile,
  ) -> Result<Option<ProxySettings>, String> {
    if let Some(vpn_id) = profile.vpn_id.as_ref().filter(|id| !id.is_empty()) {
      let vpn_worker = crate::vpn_worker_runner::start_vpn_worker(vpn_id)
        .await
        .map_err(|e| format!("Failed to start VPN worker: {e}"))?;
      let port = vpn_worker
        .local_port
        .ok_or_else(|| "VPN worker did not report a local port".to_string())?;
      return Ok(Some(ProxySettings {
        proxy_type: "socks5".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        username: None,
        password: None,
        vless_uri: None,
      }));
    }

    let Some(stored) = self
      .resolve_proxy_with_refresh(profile.proxy_id.as_ref())
      .await?
    else {
      return Ok(None);
    };

    // Camoufox is driven purely through Firefox `network.proxy.*` prefs
    // (camoufox_manager.rs), which can express only a bare host:port for
    // socks5/http — there is no pref for proxy credentials (Firefox needs an
    // interactive auth prompt or an extension for that, neither of which
    // Camoufox's launch path wires up), and a `vless` scheme means nothing to
    // Firefox at all. Both cases are translated here into a plain,
    // unauthenticated local HTTP endpoint that Camoufox can point at
    // directly: `salamander-proxy` dials the real upstream (through an Xray
    // worker first for `vless`) and re-serves it locally with no auth.
    let needs_wrapping = stored.proxy_type.eq_ignore_ascii_case("vless")
      || (stored.username.is_some() && stored.password.is_some());
    if !needs_wrapping {
      return Ok(Some(stored));
    }

    let upstream_for_wrapper = if stored.proxy_type.eq_ignore_ascii_case("vless") {
      let uri = stored
        .vless_uri
        .as_deref()
        .ok_or_else(|| crate::backend_error("VLESS_CONFIG_INVALID"))?;
      let profile_id_str = profile.id.to_string();
      let worker = crate::xray_worker_runner::start_xray_worker(Some(&profile_id_str), uri)
        .await
        .map_err(|e| format!("Failed to start Xray worker: {e}"))?;
      worker.local_proxy_settings()
    } else {
      stored
    };

    let upstream_url =
      crate::proxy_manager::ProxyManager::build_proxy_url(&upstream_for_wrapper);
    let wrapper = crate::proxy_runner::start_proxy_process_with_profile(
      Some(upstream_url),
      None,
      Some(profile.id.to_string()),
      Vec::new(),
      None,
      false,
      None,
    )
    .await
    .map_err(|e| format!("Failed to start local proxy worker: {e}"))?;
    let port = wrapper
      .local_port
      .ok_or_else(|| "Local proxy worker did not report a local port".to_string())?;

    // Claim the wrapper with this GUI process's own (long-lived) PID rather
    // than Camoufox's — Camoufox's own launch-time PID is unreliable for
    // this: it can exit anywhere from ~100ms to a couple of seconds after
    // spawn depending on its internal process model, which made an earlier
    // version of this tie the wrapper's self-reap watchdog to a PID that
    // died moments into a real, still-active session (the intermittent
    // "proxy server is refusing connections" bug). The app's own PID is
    // stable for the whole session, matching the same pattern
    // `check_proxy_validity` already uses to claim its own short-lived probe
    // worker, and `xray_worker_runner::spawn_xray_worker` already uses by
    // default for the Xray worker itself (left untouched here).
    if !crate::proxy_manager::persist_browser_identity(&wrapper.id, std::process::id()) {
      log::warn!(
        "Failed to claim local proxy wrapper {} with the app's own PID for self-expiry",
        wrapper.id
      );
    }

    Ok(Some(ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port,
      username: None,
      password: None,
      vless_uri: None,
    }))
  }

  /// Get the executable path for a browser profile
  /// This is a common helper to eliminate code duplication across the codebase
  pub fn get_browser_executable_path(
    &self,
    profile: &BrowserProfile,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Create browser instance to get executable path
    let browser_type = crate::browser::BrowserType::from_str(&profile.browser)
      .map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = crate::browser::create_browser(browser_type);

    // Construct browser directory path: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&profile.browser);
    browser_dir.push(&profile.version);

    // Get platform-specific executable path
    browser
      .get_executable_path(&browser_dir)
      .map_err(|e| format!("Failed to get executable path for {}: {e}", profile.browser).into())
  }

  async fn launch_browser_internal(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Handle Camoufox profiles using CamoufoxManager. Proxy is applied
    // directly into the Firefox profile's user.js by CamoufoxManager itself
    // (no local SOCKS5 proxy worker the way Wayfern uses for CDP-based
    // traffic monitoring), so DNS-blocklist enforcement and extension
    // auto-install — both Wayfern/Chromium-specific mechanisms above — do not
    // apply to Camoufox yet.
    if profile.browser == "camoufox" {
      let mut camoufox_config = profile.camoufox_config.clone().unwrap_or_else(|| {
        log::info!(
          "No camoufox config found for profile {}, using default",
          profile.name
        );
        crate::camoufox_manager::CamoufoxConfig::default()
      });

      let upstream_proxy = self
        .resolve_launch_proxy(profile)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

      let gate_result =
        crate::launch_gate::enforce_fingerprint_gate(profile, upstream_proxy.as_ref(), gate).await;
      gate_result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

      Self::fire_launch_hook(profile);

      if let Some(proxy) = &upstream_proxy {
        let proxy_url = if let (Some(username), Some(password)) = (&proxy.username, &proxy.password)
        {
          format!(
            "{}://{}:{}@{}:{}",
            proxy.proxy_type.to_lowercase(),
            username,
            password,
            proxy.host,
            proxy.port
          )
        } else {
          format!(
            "{}://{}:{}",
            proxy.proxy_type.to_lowercase(),
            proxy.host,
            proxy.port
          )
        };
        camoufox_config.proxy = Some(proxy_url);
      } else {
        camoufox_config.proxy = None;
      }

      if profile.password_protected {
        crate::profile::password::prepare_for_launch(profile)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      } else if profile.ephemeral {
        crate::ephemeral_dirs::create_ephemeral_dir(&profile.id.to_string())
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      }

      let mut updated_profile = profile.clone();

      if camoufox_config.fingerprint.is_none() {
        log::info!(
          "No stored device for Camoufox profile {}; generating one",
          profile.name
        );
        let generated = self
          .camoufox_manager
          .generate_fingerprint_config(&app_handle, profile, &camoufox_config)
          .await
          .map_err(|e| {
            crate::backend_error_with_detail("INTERNAL_ERROR", e.to_string())
          })?;
        camoufox_config.fingerprint = Some(generated);
        updated_profile.camoufox_config = Some(camoufox_config.clone());
      }

      log::info!("Launching Camoufox for profile: {}", profile.name);

      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(&updated_profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy().to_string();

      let camoufox_result = self
        .camoufox_manager
        .launch_camoufox_profile(
          app_handle.clone(),
          updated_profile.clone(),
          camoufox_config,
          url.clone(),
          if profile.ephemeral || profile.password_protected {
            Some(profile_data_path.clone())
          } else {
            None
          },
          remote_debugging_port,
          headless,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          crate::wrap_backend_error(e, "Failed to launch Camoufox").into()
        })?;

      let Some(process_id) = camoufox_result.processId.filter(|pid| *pid != 0) else {
        return Err(
          crate::backend_error_with_detail(
            "INTERNAL_ERROR",
            "Camoufox did not report a process identifier",
          )
          .into(),
        );
      };
      log::info!("Camoufox launched successfully with PID: {process_id}");
      let _ = profile_path_str;

      // Proxy-side workers (Xray worker / local HTTP wrapper — see
      // `resolve_launch_proxy`) are deliberately NOT tagged with Camoufox's
      // own PID here. Camoufox's launch-time PID is unreliable for that: it
      // can exit anywhere from ~100ms to a couple of seconds after spawn
      // depending on its internal process model (Firefox's multi-process
      // remoting), which made an earlier version of this tie a worker's
      // self-reap watchdog to a PID that died moments into a real, still-
      // active session — the intermittent "proxy server is refusing
      // connections" bug. Each worker instead claims itself with this GUI
      // process's own (session-long-lived) PID at spawn time — see
      // `resolve_launch_proxy` and `xray_worker_runner::spawn_xray_worker`.

      updated_profile.process_id = Some(process_id);
      updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

      self.save_process_info(&updated_profile)?;
      let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
        let _ = tm.rebuild_from_profiles(&self.profile_manager.list_profiles().unwrap_or_default());
      });

      if let Err(e) = events::emit_empty("profiles-changed") {
        log::warn!("Warning: Failed to emit profiles-changed event: {e}");
      }
      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: updated_profile.process_id.is_some(),
      };
      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      return Ok(updated_profile);
    }

    Err(format!("Unsupported browser type: {}", profile.browser).into())
  }

  pub async fn open_url_in_existing_browser(
    &self,
    _app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: &str,
    _internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if profile.browser != "camoufox" {
      return Err(format!("Unsupported browser type: {}", profile.browser).into());
    }

    // Camoufox (Firefox) has its own remoting protocol: launching the same
    // executable against a profile that already has a running instance,
    // without `-no-remote`, hands the URL to that instance as a new tab and
    // the freshly spawned process exits as soon as the handoff completes —
    // this is exactly why the primary launch path
    // (`camoufox_manager::launch_camoufox_profile`) deliberately omits
    // `-no-remote`. No CDP/remote-debugging port is needed for this.
    let executable_path = self.get_browser_executable_path(profile)?;
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let profile_data_path =
      crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
    let profile_path_str = profile_data_path.to_string_lossy().to_string();

    let mut command = tokio::process::Command::new(&executable_path);
    command
      .args([
        "-profile",
        &crate::camoufox_manager::CamoufoxManager::canonicalize_profile_path(&profile_path_str),
        url,
      ])
      .stdin(std::process::Stdio::null())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null());

    let status = tokio::time::timeout(Duration::from_secs(10), command.status())
      .await
      .map_err(|_| "Timed out handing the URL off to the running Camoufox instance")??;

    if !status.success() {
      return Err(format!("Camoufox exited with {status} while handing off the URL").into());
    }

    Ok(())
  }

  pub async fn launch_browser_with_debugging(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Wayfern starts (and PID-reconciles) its own local proxy
    // inside `launch_browser_internal`, so we hand it None here rather than
    // staging a second, orphaned proxy worker.
    self
      .launch_browser_internal(
        app_handle,
        profile,
        url,
        remote_debugging_port,
        headless,
        gate,
      )
      .await
  }

  pub async fn launch_or_open_url(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    internal_proxy_settings: Option<&ProxySettings>,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    log::info!(
      "launch_or_open_url called for profile: {} (ID: {})",
      profile.name,
      profile.id
    );

    // Get the most up-to-date profile data
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles in launch_or_open_url: {e}"))?;
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone());

    log::info!(
      "Checking browser status for profile: {} (ID: {})",
      updated_profile.name,
      updated_profile.id
    );

    // Check if browser is already running
    let is_running = self
      .check_browser_status(app_handle.clone(), &updated_profile)
      .await
      .map_err(|e| format!("Failed to check browser status: {e}"))?;

    // Get the updated profile again after status check (PID might have been updated)
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles after status check: {e}"))?;
    let final_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| updated_profile.clone());

    log::info!(
      "Browser status check: running={is_running}, URL requested={}, PID present={}",
      url.is_some(),
      final_profile.process_id.is_some()
    );

    if is_running {
      if let Some(url_ref) = url.as_ref() {
        log::info!(
          "Opening {} in existing browser",
          crate::log_redaction::url_label(url_ref)
        );

        match self
          .open_url_in_existing_browser(
            app_handle.clone(),
            &final_profile,
            url_ref,
            internal_proxy_settings,
          )
          .await
        {
          Ok(()) => {
            log::info!("Successfully opened URL in existing browser");
            Ok(final_profile)
          }
          Err(e) => {
            log::info!(
              "Failed to open URL in existing browser: {}",
              crate::log_redaction::text(&e.to_string())
            );
            Err(e)
          }
        }
      } else {
        log::info!("Browser is already running and no URL was requested");
        Ok(final_profile)
      }
    } else {
      log::info!("Launching new browser instance - browser not running");
      self
        .launch_browser_internal(app_handle.clone(), &final_profile, url, None, false, gate)
        .await
    }
  }

  fn save_process_info(
    &self,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use the regular save_profile method which handles the UUID structure
    self.profile_manager.save_profile(profile).map_err(|e| {
      let error_string = e.to_string();
      Box::new(std::io::Error::other(error_string)) as Box<dyn std::error::Error + Send + Sync>
    })
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    self
      .profile_manager
      .check_browser_status(app_handle, profile)
      .await
  }

  pub async fn kill_browser_process(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

    self
      .kill_browser_process_unlocked(app_handle, profile)
      .await
  }

  async fn kill_browser_process_unlocked(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if profile.browser == "camoufox" {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      log::info!(
        "Attempting to kill Camoufox process for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      let mut process_actually_stopped = true;
      if let Ok(Some(camoufox_process)) = self
        .camoufox_manager
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        match self
          .camoufox_manager
          .stop_camoufox(&app_handle, &camoufox_process.id)
          .await
        {
          Ok(stopped) => process_actually_stopped = stopped,
          Err(e) => {
            log::error!("Error stopping Camoufox process {}: {}", camoufox_process.id, e);
            process_actually_stopped = false;
          }
        }
      }

      if !process_actually_stopped {
        return Err(
          format!(
            "Failed to stop Camoufox process for profile {} - process may still be running",
            profile.name
          )
          .into(),
        );
      }

      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;
      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      if let Ok(Some(pending_update)) = self
        .auto_updater
        .get_pending_update(&profile.browser, &profile.version)
      {
        match self.profile_manager.update_profile_version(
          &app_handle,
          &profile.id.to_string(),
          &pending_update.new_version,
        ) {
          Ok(updated_profile_after_update) => {
            updated_profile = updated_profile_after_update;
            if let Err(e) = self
              .auto_updater
              .dismiss_update_notification(&pending_update.id)
            {
              log::warn!("Warning: Failed to dismiss pending update notification: {e}");
            }
          }
          Err(e) => {
            log::error!(
              "Failed to apply pending update for Camoufox profile {}: {}",
              profile.name,
              e
            );
          }
        }
      }

      if updated_profile.version == profile.version {
        if let Some(p) = self
          .auto_updater
          .update_profile_to_latest_installed(&app_handle, &updated_profile)
        {
          updated_profile = p;
        }
      }

      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: false,
      };
      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      if profile.password_protected {
        crate::profile::password::complete_after_quit_and_wait(profile).await;
      } else if profile.ephemeral {
        let id = profile.id.to_string();
        crate::ephemeral_dirs::remove_ephemeral_dir(&id);
        crate::traffic_stats::delete_traffic_stats(&id);
      } else if profile.clear_on_close {
        crate::profile::clear_on_close::clear_profile_browsing_data(profile).await;
      }

      crate::extension_manager::ExtensionManager::cleanup_unpacked_for_profile(
        &profile.id.to_string(),
      );

      log::info!(
        "Camoufox process cleanup completed for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      if let Ok(consolidated) = self
        .downloaded_browsers_registry
        .consolidate_browser_versions(&app_handle)
      {
        if !consolidated.is_empty() {
          log::info!("Post-stop version consolidation results:");
          for action in &consolidated {
            log::info!("  {action}");
          }
        }
      }

      return Ok(());
    }

    Err(
      format!(
        "Unsupported browser '{}' for profile '{}' — only Camoufox is supported",
        profile.browser, profile.name
      )
      .into(),
    )
  }

  pub async fn open_url_with_profile(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: String,
    url: String,
    gate: crate::launch_gate::FingerprintGate,
  ) -> Result<(), String> {
    // Get the profile by name
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let profile = profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;
    let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

    if profile.is_cross_os() {
      return Err(format!(
        "Cannot open URL with profile '{}': this profile was created on {} and cannot be used on a different operating system",
        profile.name,
        profile.host_os.as_deref().unwrap_or("another OS"),
      ));
    }

    log::info!("Opening URL with selected profile");

    // Use launch_or_open_url which handles both launching new instances and opening in existing ones
    if let Err(e) = self
      .launch_or_open_url(app_handle, &profile, Some(url.clone()), None, &gate)
      .await
    {
      log::info!(
        "Failed to open URL with selected profile: {}",
        crate::log_redaction::text(&e.to_string())
      );
      unwind_launch(&profile).await;
      // Pass structured errors through untouched: the gate's block carries the
      // mismatch detail the dialog renders, and wrapping it in English would
      // reach the user as raw JSON.
      return Err(crate::wrap_backend_error(
        e,
        "Failed to open URL with profile",
      ));
    }

    log::info!("Successfully opened URL with selected profile");
    Ok(())
  }
}

#[tauri::command]
pub async fn launch_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  consent_token: Option<String>,
) -> Result<BrowserProfile, String> {
  let options = LaunchOptions {
    gate: match consent_token {
      Some(token) => crate::launch_gate::FingerprintGate::Consented(token),
      None => crate::launch_gate::FingerprintGate::Enforce,
    },
    ..Default::default()
  };
  launch_browser_profile_impl(app_handle, profile, url, options).await
}

/// How one launch should behave.
///
/// A struct rather than four trailing positional arguments: `headless` and
/// `force_new` are already passed adjacently as bare booleans, so a fifth would
/// compile everywhere while silently inverting behavior wherever the order was
/// got wrong.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
  pub remote_debugging_port: Option<u16>,
  pub headless: bool,
  pub force_new: bool,
  pub gate: crate::launch_gate::FingerprintGate,
}

/// Undo the launch bookkeeping a blocked launch attempt took before it failed.
///
/// Several of these error paths are reachable while a browser for the profile
/// is genuinely still running — `PROFILE_RUNNING`, or a failure to open a URL
/// in an existing window. That browser owns the running mark, so this must
/// not clear anything while it's still alive. Hence the liveness check.
async fn unwind_launch(profile: &BrowserProfile) {
  if browser_is_running_for(&profile.id.to_string()) {
    log::debug!(
      "Not unwinding launch state for {}: a browser is still running for it",
      profile.name
    );
  }
}

/// Whether a live browser process is recorded for this profile right now.
/// Re-read from disk: the caller's copy predates the launch attempt.
fn browser_is_running_for(profile_id: &str) -> bool {
  BrowserRunner::instance()
    .profile_manager
    .list_profiles()
    .ok()
    .and_then(|profiles| {
      profiles
        .into_iter()
        .find(|p| p.id.to_string() == profile_id)
        .map(|p| {
          p.process_id
            .is_some_and(crate::proxy_storage::is_process_running)
        })
    })
    .unwrap_or(false)
}

pub async fn launch_browser_profile_impl(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  options: LaunchOptions,
) -> Result<BrowserProfile, String> {
  let LaunchOptions {
    remote_debugging_port,
    headless,
    force_new,
    gate,
  } = options;
  log::info!(
    "Launch request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );
  let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

  if profile.is_cross_os() {
    return Err(format!(
      "Cannot launch profile '{}': this profile was created on {} and cannot be launched on a different operating system",
      profile.name,
      profile.host_os.as_deref().unwrap_or("another OS"),
    ));
  }

  let browser_runner = BrowserRunner::instance();

  // Resolve the most up-to-date profile from disk by ID to avoid using stale proxy_id/browser state
  let profile_for_launch = match browser_runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
  {
    Ok(profiles) => profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone()),
    Err(e) => {
      unwind_launch(&profile).await;
      return Err(e);
    }
  };

  log::info!(
    "Resolved profile for launch: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  log::info!(
    "Starting browser launch for profile: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  if force_new {
    let already_running = match browser_runner
      .check_browser_status(app_handle.clone(), &profile_for_launch)
      .await
    {
      Ok(running) => running,
      Err(error) => {
        unwind_launch(&profile).await;
        return Err(crate::wrap_backend_error(
          error,
          "Failed to check browser status before launch",
        ));
      }
    };
    if already_running {
      unwind_launch(&profile).await;
      return Err(crate::backend_error("PROFILE_RUNNING"));
    }
  }

  // Launch browser or open URL in existing instance. Wayfern starts its
  // own local proxy inside `launch_browser_internal`; other browser types
  // are rejected there, so no proxy needs to be staged here.
  //
  // A `force_new` caller always starts a fresh instance with the requested
  // debug port and headless mode, bypassing the "open URL in the existing
  // window" path which would otherwise ignore both.
  let launch_result = if force_new {
    browser_runner
      .launch_browser_with_debugging(
        app_handle.clone(),
        &profile_for_launch,
        url,
        remote_debugging_port,
        headless,
        &gate,
      )
      .await
  } else {
    browser_runner
      .launch_or_open_url(app_handle.clone(), &profile_for_launch, url, None, &gate)
      .await
  };
  let updated_profile = match launch_result {
    Ok(updated) => updated,
    Err(e) => {
      log::info!(
        "Browser launch failed for profile: {}, error: {}",
        profile_for_launch.name,
        e
      );

      // Emit a failure event to clear loading states in the frontend
      #[derive(serde::Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: profile_for_launch.id.to_string(),
        is_running: false,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      unwind_launch(&profile).await;

      // Check if this is an architecture compatibility issue
      if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
        if io_error.kind() == std::io::ErrorKind::Other
          && io_error.to_string().contains("Exec format error")
        {
          return Err(format!("Failed to launch browser: Executable format error. This browser version is not compatible with your system architecture ({}). Please try a different browser or version that supports your platform.", std::env::consts::ARCH));
        }
      }
      return Err(crate::wrap_backend_error(
        e,
        "Failed to launch browser or open URL",
      ));
    }
  };

  log::info!(
    "Browser launch completed for profile: {} (ID: {})",
    updated_profile.name,
    updated_profile.id
  );

  // The proxy PID mapping was already reconciled inside launch_browser_internal
  // (placeholder → real browser PID); nothing is ever keyed by a constant here.

  Ok(updated_profile)
}

#[tauri::command]
pub fn check_browser_exists(browser_str: String, version: String) -> bool {
  // This is an alias for is_browser_downloaded to provide clearer semantics for auto-updates
  let runner = BrowserRunner::instance();
  runner
    .downloaded_browsers_registry
    .is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub async fn kill_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<(), String> {
  log::info!(
    "Kill request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );
  let browser_runner = BrowserRunner::instance();

  match browser_runner
    .kill_browser_process(app_handle.clone(), &profile)
    .await
  {
    Ok(()) => {
      log::info!(
        "Successfully killed browser profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Auto-update non-running profiles and cleanup unused binaries
      let browser_for_update = profile.browser.clone();
      let app_handle_for_update = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
        let mut versions = registry.get_downloaded_versions(&browser_for_update);
        if !versions.is_empty() {
          versions.sort_by(|a, b| crate::api_client::compare_versions(b, a));
          let latest_version = &versions[0];

          let auto_updater = crate::auto_updater::AutoUpdater::instance();
          match auto_updater
            .auto_update_profile_versions(
              &app_handle_for_update,
              &browser_for_update,
              latest_version,
            )
            .await
          {
            Ok(updated) => {
              if !updated.is_empty() {
                log::info!(
                  "Auto-updated {} profiles after stop: {:?}",
                  updated.len(),
                  updated
                );
              }
            }
            Err(e) => {
              log::error!("Failed to auto-update profile versions after stop: {e}");
            }
          }
        }

        match registry.cleanup_unused_binaries() {
          Ok(cleaned) => {
            if !cleaned.is_empty() {
              log::info!("Cleaned up unused binaries after stop: {:?}", cleaned);
            }
          }
          Err(e) => {
            log::error!("Failed to cleanup unused binaries after stop: {e}");
          }
        }
      });

      Ok(())
    }
    Err(e) => {
      log::info!("Failed to kill browser profile {}: {}", profile.name, e);

      // Emit a failure event to clear loading states in the frontend
      #[derive(serde::Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      // On kill failure, we assume the process is still running
      let payload = RunningChangedPayload {
        id: profile.id.to_string(),
        is_running: true,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      Err(format!("Failed to kill browser: {e}"))
    }
  }
}

#[tauri::command]
pub async fn open_url_with_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
  url: String,
  consent_token: Option<String>,
) -> Result<(), String> {
  let browser_runner = BrowserRunner::instance();
  let gate = match consent_token {
    Some(token) => crate::launch_gate::FingerprintGate::Consented(token),
    None => crate::launch_gate::FingerprintGate::Enforce,
  };
  browser_runner
    .open_url_with_profile(app_handle, profile_id, url, gate)
    .await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn profile_launch_lock_serializes_only_the_same_profile() {
    let profile = format!("launch-lock-{}", uuid::Uuid::new_v4());
    let other_profile = format!("launch-lock-{}", uuid::Uuid::new_v4());
    let first = lock_profile_launch(&profile).await;

    assert!(tokio::time::timeout(
      Duration::from_millis(100),
      lock_profile_launch(&other_profile)
    )
    .await
    .is_ok());
    assert!(
      tokio::time::timeout(Duration::from_millis(100), lock_profile_launch(&profile))
        .await
        .is_err()
    );

    drop(first);
    assert!(
      tokio::time::timeout(Duration::from_millis(100), lock_profile_launch(&profile))
        .await
        .is_ok()
    );
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_RUNNER: BrowserRunner = BrowserRunner::new();
}
