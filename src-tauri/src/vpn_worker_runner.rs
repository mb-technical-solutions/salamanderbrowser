use crate::proxy_runner::find_sidecar_executable;
use crate::proxy_storage::{
  is_process_running, process_identity_matches, resolve_process_start_time,
};
use crate::vpn_worker_storage::{
  delete_vpn_worker_config, find_vpn_worker_by_vpn_id, generate_vpn_worker_id,
  get_vpn_worker_config, list_vpn_worker_configs, save_vpn_worker_config, vpn_worker_config_path,
  VpnWorkerConfig,
};
use std::process::Stdio;

const VPN_WORKER_POLL_INTERVAL_MS: u64 = 100;
const VPN_WORKER_STARTUP_TIMEOUT_MS: u64 = 30_000;

async fn vpn_worker_accepting_connections(config: &VpnWorkerConfig) -> bool {
  let Some(port) = config.local_port else {
    return false;
  };

  if config
    .local_url
    .as_ref()
    .is_none_or(|local_url| local_url.is_empty())
  {
    return false;
  }

  matches!(
    tokio::time::timeout(
      tokio::time::Duration::from_millis(VPN_WORKER_POLL_INTERVAL_MS),
      tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await,
    Ok(Ok(_))
  )
}

fn worker_log_path(id: &str) -> std::path::PathBuf {
  std::env::temp_dir().join(format!("salamander-vpn-{}.log", id))
}

fn read_worker_log(id: &str) -> String {
  std::fs::read_to_string(worker_log_path(id)).unwrap_or_else(|_| "No log available".to_string())
}

/// The tail of a worker's own log — last-ditch context for callers (like the
/// VPN traffic check) that only see the failure from the outside, as a
/// generic network timeout, and otherwise have no way to tell "the tunnel
/// never even tried a handshake" apart from "the handshake is retrying with
/// no response from the peer". Keeping just the tail avoids multi-megabyte
/// strings for a worker that's been up a long time.
pub(crate) fn read_worker_log_tail(id: &str, max_lines: usize) -> Option<String> {
  let content = std::fs::read_to_string(worker_log_path(id)).ok()?;
  if content.trim().is_empty() {
    return None;
  }
  let lines: Vec<&str> = content.lines().collect();
  let start = lines.len().saturating_sub(max_lines);
  Some(lines[start..].join("\n"))
}

async fn wait_for_vpn_worker_ready(
  id: &str,
) -> Result<VpnWorkerConfig, Box<dyn std::error::Error>> {
  let startup_timeout = tokio::time::Duration::from_millis(VPN_WORKER_STARTUP_TIMEOUT_MS);
  let startup_deadline = tokio::time::Instant::now() + startup_timeout;

  tokio::time::sleep(tokio::time::Duration::from_millis(
    VPN_WORKER_POLL_INTERVAL_MS,
  ))
  .await;

  let mut attempts = 0u32;

  loop {
    tokio::time::sleep(tokio::time::Duration::from_millis(
      VPN_WORKER_POLL_INTERVAL_MS,
    ))
    .await;

    if let Some(updated_config) = get_vpn_worker_config(id) {
      let process_running = updated_config
        .pid
        .is_some_and(|pid| process_identity_matches(pid, updated_config.pid_start_time));

      if !process_running && attempts > 2 {
        let log_output = read_worker_log(id);
        let _ = std::fs::remove_file(&updated_config.config_file_path);
        delete_vpn_worker_config(id);
        return Err(format!("VPN worker process crashed. Log output:\n{}", log_output).into());
      }

      if vpn_worker_accepting_connections(&updated_config).await {
        return Ok(updated_config);
      }
    }

    attempts += 1;
    if tokio::time::Instant::now() >= startup_deadline {
      if let Some(config) = get_vpn_worker_config(id) {
        let process_running = config
          .pid
          .is_some_and(|pid| process_identity_matches(pid, config.pid_start_time));
        let log_output = read_worker_log(id);
        let _ = std::fs::remove_file(&config.config_file_path);
        delete_vpn_worker_config(id);
        return Err(
          format!(
            "VPN worker failed to start within {:.1}s. pid={:?}, process_running={}, local_url={:?}\n\nVPN worker log:\n{}",
            startup_timeout.as_secs_f32(),
            config.pid,
            process_running,
            config.local_url,
            log_output
          )
          .into(),
        );
      }

      delete_vpn_worker_config(id);
      return Err("VPN worker config not found after spawn".into());
    }
  }
}

/// Serializes worker startup for a given VPN, so two concurrent launches cannot
/// both observe "no worker" and both believe they created it. Benign until a
/// launch guard may stop one on failure; then double-ownership means a
/// cancelled launch tears down a tunnel another profile is using. Mirrors
/// `xray_worker_runner::XRAY_START_LOCK`.
static VPN_START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A started VPN worker plus whether *this* call spawned it.
pub struct VpnWorkerStart {
  pub config: VpnWorkerConfig,
  /// False when an already-running worker was adopted. Only the creator may
  /// stop it while unwinding a failed launch.
  pub created: bool,
}

/// Whether any profile with a live browser process is routing through this VPN.
///
/// Extracted from the startup sweep so the launch guard and the sweep agree on
/// what "in use" means instead of each carrying its own copy.
pub fn vpn_id_in_use_by_running_browser(vpn_id: &str) -> bool {
  let Ok(profiles) = crate::profile::ProfileManager::instance().list_profiles() else {
    // Unable to tell — assume in use rather than tear down a live tunnel.
    return true;
  };
  profiles
    .iter()
    .filter(|p| p.process_id.is_some_and(is_process_running))
    .any(|p| p.vpn_id.as_deref() == Some(vpn_id))
}

/// Hold the start lock across an adopt-sensitive section (a launch guard
/// deciding whether to stop a worker it created).
pub async fn lock_vpn_starts() -> tokio::sync::MutexGuard<'static, ()> {
  VPN_START_LOCK.lock().await
}

pub async fn start_vpn_worker(vpn_id: &str) -> Result<VpnWorkerConfig, Box<dyn std::error::Error>> {
  start_vpn_worker_tracked(vpn_id).await.map(|s| s.config)
}

pub async fn start_vpn_worker_tracked(
  vpn_id: &str,
) -> Result<VpnWorkerStart, Box<dyn std::error::Error>> {
  let _start_guard = VPN_START_LOCK.lock().await;
  crate::proxy_runner::ensure_sidecar_version().await?;

  for config in list_vpn_worker_configs() {
    let dead = config
      .pid
      .is_some_and(|pid| !process_identity_matches(pid, config.pid_start_time));
    if dead || config.pid.is_none() {
      let _ = std::fs::remove_file(&config.config_file_path);
      delete_vpn_worker_config(&config.id);
    }
  }

  // Check if a VPN worker for this vpn_id already exists and is running
  if let Some(existing) = find_vpn_worker_by_vpn_id(vpn_id) {
    if let Some(pid) = existing.pid {
      if process_identity_matches(pid, existing.pid_start_time) {
        if vpn_worker_accepting_connections(&existing).await {
          return Ok(VpnWorkerStart {
            config: existing,
            created: false,
          });
        }

        return wait_for_vpn_worker_ready(&existing.id)
          .await
          .map(|config| VpnWorkerStart {
            config,
            created: false,
          });
      }
    }
    // Worker config exists but process is dead, clean up
    delete_vpn_worker_config(&existing.id);
  }

  // Load VPN config from storage to determine type
  let vpn_config = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;
    storage
      .load_config(vpn_id)
      .map_err(|e| format!("Failed to load VPN config: {e}"))?
  };

  let vpn_type_str = match vpn_config.vpn_type {
    crate::vpn::VpnType::WireGuard => "wireguard",
  };

  // Write decrypted config to a temp file
  let config_file_path = std::env::temp_dir()
    .join(format!("salamander_vpn_{}.conf", vpn_id))
    .to_string_lossy()
    .to_string();

  std::fs::write(&config_file_path, &vpn_config.config_data)?;

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&config_file_path, std::fs::Permissions::from_mode(0o600));
  }

  let id = generate_vpn_worker_id();

  // Find an available port
  let local_port = {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.local_addr()?.port()
  };

  let config = VpnWorkerConfig::new(
    id.clone(),
    vpn_id.to_string(),
    vpn_type_str.to_string(),
    config_file_path,
  );
  save_vpn_worker_config(&config)?;

  let config_json_path = vpn_worker_config_path(&id);

  // Spawn detached VPN worker process
  let exe = find_sidecar_executable("salamander-proxy")?;

  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    use std::process::Command as StdCommand;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("vpn-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);
    cmd.arg("--port");
    cmd.arg(local_port.to_string());
    cmd.arg("--config-path");
    cmd.arg(&config_json_path);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    let log_path = std::env::temp_dir().join(format!("salamander-vpn-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("VPN worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    unsafe {
      cmd.pre_exec(|| {
        libc::setsid();
        if libc::setpriority(libc::PRIO_PROCESS, 0, -10) != 0 {
          let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
        }
        Ok(())
      });
    }

    let child = cmd.spawn()?;
    let pid = child.id();

    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    config_with_pid.pid_start_time = resolve_process_start_time(pid);
    config_with_pid.local_port = Some(local_port);
    save_vpn_worker_config(&config_with_pid)?;

    drop(child);
  }

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    use std::process::Command as StdCommand;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("vpn-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);
    cmd.arg("--port");
    cmd.arg(local_port.to_string());
    cmd.arg("--config-path");
    cmd.arg(&config_json_path);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    let log_path = std::env::temp_dir().join(format!("salamander-vpn-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("VPN worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);

    let child = cmd.spawn()?;
    let pid = child.id();

    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    config_with_pid.pid_start_time = resolve_process_start_time(pid);
    config_with_pid.local_port = Some(local_port);
    save_vpn_worker_config(&config_with_pid)?;

    drop(child);
  }

  wait_for_vpn_worker_ready(&id)
    .await
    .map(|config| VpnWorkerStart {
      config,
      created: true,
    })
}

pub async fn stop_vpn_worker(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  let config = get_vpn_worker_config(id);

  if let Some(config) = config {
    if let Some(pid) = config
      .pid
      .filter(|pid| process_identity_matches(*pid, config.pid_start_time))
    {
      #[cfg(unix)]
      {
        use std::process::Command;
        let _ = Command::new("kill")
          .arg("-TERM")
          .arg(pid.to_string())
          .output();
      }
      #[cfg(windows)]
      {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("taskkill")
          .args(["/F", "/PID", &pid.to_string()])
          .creation_flags(CREATE_NO_WINDOW)
          .output();
      }

      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Clean up temp config file
    let _ = std::fs::remove_file(&config.config_file_path);

    delete_vpn_worker_config(id);
    return Ok(true);
  }

  Ok(false)
}

pub async fn stop_vpn_worker_by_vpn_id(vpn_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  if let Some(config) = find_vpn_worker_by_vpn_id(vpn_id) {
    return stop_vpn_worker(&config.id).await;
  }
  Ok(false)
}

pub async fn stop_all_vpn_workers() -> Result<(), Box<dyn std::error::Error>> {
  let configs = list_vpn_worker_configs();
  for config in configs {
    let _ = stop_vpn_worker(&config.id).await;
  }
  Ok(())
}
