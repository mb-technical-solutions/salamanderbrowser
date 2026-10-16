use crate::proxy_storage::get_storage_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const TRANSIENT_IO_RETRY_ATTEMPTS: u32 = 25;
const TRANSIENT_IO_RETRY_DELAY: Duration = Duration::from_millis(10);

// Windows refuses to replace or open a file another handle holds without
// FILE_SHARE_DELETE, and virus scanners open files exactly that way; the
// detached worker process reads this config right after the parent writes
// it, so both sides can hit that race. Those are worth retrying; every
// other error is real and must surface. Mirrors `xray_worker_storage`'s
// identical helper.
#[cfg(windows)]
fn io_error_is_transient(error: &std::io::Error) -> bool {
  const ERROR_ACCESS_DENIED: i32 = 5;
  const ERROR_SHARING_VIOLATION: i32 = 32;
  const ERROR_LOCK_VIOLATION: i32 = 33;
  matches!(
    error.raw_os_error(),
    Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
  )
}

#[cfg(not(windows))]
fn io_error_is_transient(_error: &std::io::Error) -> bool {
  false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnWorkerConfig {
  pub id: String,
  pub vpn_id: String,
  pub vpn_type: String,
  pub config_file_path: String,
  pub local_port: Option<u16>,
  pub local_url: Option<String>,
  pub pid: Option<u32>,
  /// Start time of `pid`, pinning a liveness check to one exact process.
  /// Without it a recycled PID reads as "my worker is still running" once
  /// the real worker has died — see the identical field on
  /// `XrayWorkerConfig`/`ProxyConfig`. `None` on configs written before this
  /// field existed; those fall back to a bare existence check.
  #[serde(default)]
  pub pid_start_time: Option<u64>,
}

impl VpnWorkerConfig {
  pub fn new(id: String, vpn_id: String, vpn_type: String, config_file_path: String) -> Self {
    Self {
      id,
      vpn_id,
      vpn_type,
      config_file_path,
      local_port: None,
      local_url: None,
      pid: None,
      pid_start_time: None,
    }
  }
}

pub fn vpn_worker_config_path(id: &str) -> std::path::PathBuf {
  get_storage_dir().join(format!("vpn_worker_{}.json", id))
}

pub fn save_vpn_worker_config(config: &VpnWorkerConfig) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_storage_dir();
  fs::create_dir_all(&storage_dir)?;

  let file_path = storage_dir.join(format!("vpn_worker_{}.json", config.id));
  save_vpn_worker_config_to_path(config, &file_path)
}

/// Write a worker config to a specific path. Used by detached worker
/// processes that already know their config file path (passed via
/// `--config-path`) and must write back to the same location regardless of
/// how `get_storage_dir()` resolves in the worker process — which can
/// differ from the parent on Linux distros that sandbox the GUI (Qubes,
/// flatpak, etc.).
pub fn save_vpn_worker_config_to_path(
  config: &VpnWorkerConfig,
  path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
  let parent = path.parent().ok_or("config path has no parent directory")?;
  fs::create_dir_all(parent)?;
  let content = serde_json::to_vec_pretty(config)?;

  // The worker process is spawned, then the parent immediately writes its
  // config back to this same path (to record its pid/port); the detached
  // child reads that same path with no retry of its own. A plain `fs::write`
  // truncates the file before writing the new bytes, so a child reading in
  // that exact window sees 0 bytes and crashes on a JSON EOF error — this is
  // what produced "EOF while parsing a value at line 1 column 0". Write to a
  // sibling temp file and persist (rename) over the target instead: that's a
  // single atomic directory-entry swap on both Windows and POSIX, so a
  // concurrent reader always sees either the old or the new content in full,
  // never a partial file. Mirrors `xray_worker_storage`'s identical helper.
  let mut temporary = tempfile::Builder::new()
    .prefix(".vpn-worker-config-")
    .tempfile_in(parent)?;
  temporary.write_all(&content)?;
  temporary.flush()?;
  temporary.as_file().sync_all()?;

  let mut attempt = 0;
  loop {
    match temporary.persist(path) {
      Ok(_) => break,
      Err(error)
        if attempt < TRANSIENT_IO_RETRY_ATTEMPTS && io_error_is_transient(&error.error) =>
      {
        temporary = error.file;
        attempt += 1;
        std::thread::sleep(TRANSIENT_IO_RETRY_DELAY);
      }
      Err(error) => return Err(error.error.into()),
    }
  }

  Ok(())
}

/// Read a worker config from a specific path. Counterpart to
/// `save_vpn_worker_config_to_path`.
pub fn get_vpn_worker_config_from_path(path: &Path) -> Option<VpnWorkerConfig> {
  let content = fs::read_to_string(path).ok()?;
  serde_json::from_str(&content).ok()
}

pub fn get_vpn_worker_config(id: &str) -> Option<VpnWorkerConfig> {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("vpn_worker_{}.json", id));

  if !file_path.exists() {
    return None;
  }

  match fs::read_to_string(&file_path) {
    Ok(content) => serde_json::from_str(&content).ok(),
    Err(_) => None,
  }
}

pub fn delete_vpn_worker_config(id: &str) -> bool {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("vpn_worker_{}.json", id));

  if !file_path.exists() {
    return false;
  }

  fs::remove_file(&file_path).is_ok()
}

pub fn list_vpn_worker_configs() -> Vec<VpnWorkerConfig> {
  let storage_dir = get_storage_dir();

  if !storage_dir.exists() {
    return Vec::new();
  }

  let mut configs = Vec::new();
  if let Ok(entries) = fs::read_dir(&storage_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with("vpn_worker_") && name.ends_with(".json") {
          if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<VpnWorkerConfig>(&content) {
              configs.push(config);
            }
          }
        }
      }
    }
  }

  configs
}

pub fn find_vpn_worker_by_vpn_id(vpn_id: &str) -> Option<VpnWorkerConfig> {
  list_vpn_worker_configs()
    .into_iter()
    .find(|c| c.vpn_id == vpn_id)
}

pub fn generate_vpn_worker_id() -> String {
  format!(
    "vpnw_{}_{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs(),
    rand::random::<u32>()
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample_config(id: &str) -> VpnWorkerConfig {
    VpnWorkerConfig::new(
      id.to_string(),
      "vpn-1".to_string(),
      "wireguard".to_string(),
      "/tmp/does-not-matter.conf".to_string(),
    )
  }

  #[test]
  fn save_and_load_round_trips_through_a_specific_path() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("vpn_worker_test.json");
    let mut config = sample_config("test-1");
    config.pid = Some(1234);

    save_vpn_worker_config_to_path(&config, &path).unwrap();
    let loaded = get_vpn_worker_config_from_path(&path).unwrap();

    assert_eq!(loaded.id, config.id);
    assert_eq!(loaded.pid, Some(1234));
  }

  // Regression test for the exact crash a detached worker hit in the wild:
  // the parent writes the config once before spawning the worker, then again
  // right after (to record its pid) while the worker is already reading the
  // same path with no retry of its own. A plain truncate-then-write left a
  // window where that read saw 0 bytes and failed with a JSON EOF error
  // ("Failed to parse VPN worker config ... EOF while parsing a value at
  // line 1 column 0"). The fix makes every write an atomic rename, so a
  // concurrent reader must always see a complete, valid file.
  #[test]
  fn concurrent_writes_never_expose_a_partial_or_empty_config_to_a_reader() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("vpn_worker_race.json");
    save_vpn_worker_config_to_path(&sample_config("race-1"), &path).unwrap();

    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
      for pid in 1..=500u32 {
        let mut config = sample_config("race-1");
        config.pid = Some(pid);
        save_vpn_worker_config_to_path(&config, &writer_path).unwrap();
      }
    });

    // Bound the reader on the writer's own lifetime, same reasoning as
    // `xray_worker_storage`'s identical test: a completion flag set last by
    // the writer is never set if it panics, which would strand this loop.
    while !writer.is_finished() {
      let content = fs::read_to_string(&path).expect("config file stays readable while replaced");
      let config: VpnWorkerConfig =
        serde_json::from_str(&content).expect("config file is always fully-formed JSON");
      assert_eq!(config.id, "race-1");
      std::thread::yield_now();
    }
    writer.join().unwrap();
  }
}
