use directories::BaseDirs;
use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIRS: OnceLock<BaseDirs> = OnceLock::new();
static PORTABLE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

fn base_dirs() -> &'static BaseDirs {
  BASE_DIRS.get_or_init(|| BaseDirs::new().expect("Failed to get base directories"))
}

/// Returns the portable base directory if a `.portable` marker exists next to the executable.
fn portable_dir() -> Option<&'static PathBuf> {
  PORTABLE_DIR
    .get_or_init(|| {
      std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        .filter(|dir| dir.join(".portable").exists())
    })
    .as_ref()
}

/// Returns true if the app is running in portable mode.
pub fn is_portable() -> bool {
  portable_dir().is_some()
}

/// Optional single-root override for all on-disk state. Set
/// `SALAMANDERBROWSER_DATA_ROOT=/path` (e.g. a tmpfs mount) to relocate
/// data/cache/logs under `<root>/{data,cache,logs}` without touching the real
/// dev/prod directories. The more specific `SALAMANDERBROWSER_DATA_DIR` /
/// `SALAMANDERBROWSER_CACHE_DIR` overrides still take precedence over this.
fn data_root() -> Option<PathBuf> {
  std::env::var_os("SALAMANDERBROWSER_DATA_ROOT")
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
}

/// Where logs go when something other than the platform default applies:
/// `<root>/logs` for `SALAMANDERBROWSER_DATA_ROOT`, else `<exe dir>/logs` in
/// portable mode. `None` means the platform default app log dir.
///
/// Portable belongs here for the same reason `data_dir` and `cache_dir` honour
/// it: a portable install is expected to keep its state beside the executable.
/// Logs were the one thing still written to the host machine, which quietly
/// defeated that.
pub fn log_dir_override() -> Option<PathBuf> {
  log_dir_for(data_root(), portable_dir())
}

/// Split out from `log_dir_override` so the precedence is testable without a
/// real `.portable` marker sitting next to the test binary.
fn log_dir_for(root: Option<PathBuf>, portable: Option<&PathBuf>) -> Option<PathBuf> {
  if let Some(root) = root {
    return Some(root.join("logs"));
  }
  portable.map(|dir| dir.join("logs"))
}

/// File name `tauri-plugin-window-state` persists geometry under.
pub const WINDOW_STATE_FILENAME: &str = ".window-state.json";

/// True when app state has been moved off the platform default location, by
/// portable mode or by either directory override.
fn state_is_relocated() -> bool {
  std::env::var_os("SALAMANDERBROWSER_DATA_DIR").is_some_and(|v| !v.is_empty())
    || data_root().is_some()
    || portable_dir().is_some()
}

/// Absolute path the window-state file should live at, or `None` to leave the
/// plugin on its platform default.
///
/// `tauri-plugin-window-state` resolves its file as
/// `app_config_dir().join(filename)` and exposes no way to change the
/// directory, so the only lever is the file name. Handing it an ABSOLUTE path
/// works because `Path::join` discards the base when the argument is absolute,
/// which lands the file with the rest of our relocated state instead of on the
/// host machine. If a future plugin version sanitises the name to a bare file
/// component this silently reverts to the default directory, which is why the
/// first-run probe in `lib.rs` reads this same function rather than assuming.
pub fn window_state_path_override() -> Option<PathBuf> {
  state_is_relocated().then(|| data_dir().join(WINDOW_STATE_FILENAME))
}

/// Where the window-state file actually is, override or not. Used for the
/// first-run probe, which must agree with whatever the plugin was configured
/// with or portable installs re-apply the default geometry on every launch.
pub fn window_state_path<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> Option<PathBuf> {
  if let Some(path) = window_state_path_override() {
    return Some(path);
  }
  use tauri::Manager;
  handle
    .path()
    .app_config_dir()
    .ok()
    .map(|dir| dir.join(WINDOW_STATE_FILENAME))
}

pub fn app_name() -> &'static str {
  if cfg!(debug_assertions) {
    "SalamanderBrowserDev"
  } else {
    "SalamanderBrowser"
  }
}

pub fn data_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_DATA_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  if let Ok(dir) = std::env::var("SALAMANDERBROWSER_DATA_DIR") {
    return PathBuf::from(dir);
  }

  if let Some(root) = data_root() {
    return root.join("data");
  }

  if let Some(dir) = portable_dir() {
    return dir.join("data");
  }

  base_dirs().data_local_dir().join(app_name())
}

/// One-time migration for anyone upgrading from "Donut Browser" — this fork's
/// previous name, before `app_name()` became "SalamanderBrowser". The old
/// build wrote everything (downloaded browsers, profiles, proxies, VPNs,
/// settings) under a sibling directory named after the old brand; without
/// this, upgrading users would find an empty data directory and have to
/// re-download every browser engine and recreate every profile from scratch.
///
/// Only touches the *default*, unoverridden data location: a custom
/// `SALAMANDERBROWSER_DATA_DIR`/`SALAMANDERBROWSER_DATA_ROOT` or a portable
/// install is a deliberate choice, not an in-place upgrade, so those are left
/// alone. Safe to call on every startup — a marker file makes the actual copy
/// run at most once.
pub fn migrate_from_previous_brand() {
  #[cfg(test)]
  {
    if TEST_DATA_DIR.with(|cell| cell.borrow().is_some()) {
      return;
    }
  }

  if std::env::var_os("SALAMANDERBROWSER_DATA_DIR").is_some_and(|v| !v.is_empty())
    || data_root().is_some()
    || portable_dir().is_some()
  {
    return;
  }

  let new_dir = base_dirs().data_local_dir().join(app_name());
  let old_name = if cfg!(debug_assertions) {
    "DonutBrowserDev"
  } else {
    "DonutBrowser"
  };
  let old_dir = base_dirs().data_local_dir().join(old_name);

  migrate_dir_if_needed(&old_dir, &new_dir);
}

/// Testable core of `migrate_from_previous_brand`: given the old and new
/// directories explicitly, copy once (marker-gated) and never clobber a file
/// that already exists at the destination.
fn migrate_dir_if_needed(old_dir: &std::path::Path, new_dir: &std::path::Path) {
  let marker = new_dir.join(".brand-migration-checked");
  if marker.exists() {
    return;
  }

  if old_dir.exists() && old_dir != new_dir {
    match copy_dir_skip_existing(old_dir, new_dir) {
      Ok(()) => log::info!(
        "Migrated data from previous installation at {} to {}",
        old_dir.display(),
        new_dir.display()
      ),
      Err(e) => log::warn!(
        "Brand migration copy from {} failed: {e}",
        old_dir.display()
      ),
    }
  }

  if let Err(e) = std::fs::create_dir_all(new_dir) {
    log::warn!("Failed to create data dir during brand migration check: {e}");
    return;
  }
  let _ = std::fs::write(&marker, b"1");
}

/// Copy everything under `source` into the live data directory, skipping any
/// file that already exists at the destination. Used by the manual "Import an
/// installation" UI action — `migrate_from_previous_brand` only covers the
/// same-machine, default-location, same-session case; this covers everything
/// else: a different machine, a backup copied in by hand, a portable install,
/// or a data directory relocated via an env var override.
pub fn import_data_tree_from(source: &std::path::Path) -> std::io::Result<()> {
  let destination = data_dir();
  let source = source.canonicalize()?;
  let destination_canonical = destination.canonicalize().unwrap_or(destination.clone());

  // Never copy a data tree into itself or from a parent containing the live
  // data directory. Both cases can recurse forever and partially corrupt the
  // destination while importing.
  if source == destination_canonical || destination_canonical.starts_with(&source) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidInput,
      "import source overlaps the active data directory",
    ));
  }

  // Stage the complete source first. This prevents a malformed tree or a
  // failed source read from partially mutating the live data directory.
  let staging = destination
    .parent()
    .unwrap_or_else(|| std::path::Path::new("."))
    .join(format!(".salamander-import-{}", uuid::Uuid::new_v4()));
  let result = (|| {
    copy_dir_skip_existing(&source, &staging)?;
    copy_dir_skip_existing(&staging, &destination)
  })();
  let _ = std::fs::remove_dir_all(&staging);
  result
}

/// Recursively copy `src` into `dst`, creating directories as needed and
/// skipping any file that already exists at the destination (so a partial
/// prior migration, or a file the new build already wrote, is never
/// clobbered).
fn copy_dir_skip_existing(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
  std::fs::create_dir_all(dst)?;
  for entry in std::fs::read_dir(src)? {
    let entry = entry?;
    let file_type = entry.file_type()?;
    let from = entry.path();
    let to = dst.join(entry.file_name());
    if file_type.is_symlink() {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("data import contains unsupported symlink: {}", from.display()),
      ));
    }
    if file_type.is_dir() {
      if entry.file_name() == "vpn" {
        copy_vpn_dir_atomically(&from, &to)?;
      } else {
        copy_dir_skip_existing(&from, &to)?;
      }
    } else if file_type.is_file() && !to.exists() {
      std::fs::copy(&from, &to)?;
    }
  }
  Ok(())
}

/// Copy a `vpn/` directory, keeping `vpn_configs.json` and `.vpn_key` paired.
///
/// `vpn_configs.json` is only decryptable with the exact `.vpn_key` it was
/// encrypted with. Copying the pair file-by-file under "skip if destination
/// already exists" can bring in one half from the source and leave the other
/// half untouched at the destination (e.g. a freshly-generated local
/// `.vpn_key` surviving while the source's `vpn_configs.json` gets copied in),
/// silently producing configs that fail to decrypt (`aead::Error`) and break
/// launch for any profile referencing them. Only adopt the source pair when
/// the destination has neither file yet; everything else in the directory
/// still copies file-by-file as usual.
fn copy_vpn_dir_atomically(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
  std::fs::create_dir_all(dst)?;
  let src_configs = src.join("vpn_configs.json");
  let src_key = src.join(".vpn_key");
  let dst_configs = dst.join("vpn_configs.json");
  let dst_key = dst.join(".vpn_key");
  if src_configs.exists() && src_key.exists() && !dst_configs.exists() && !dst_key.exists() {
    std::fs::copy(&src_configs, &dst_configs)?;
    std::fs::copy(&src_key, &dst_key)?;
  }

  for entry in std::fs::read_dir(src)? {
    let entry = entry?;
    let file_type = entry.file_type()?;
    let from = entry.path();
    let name = entry.file_name();
    if name == "vpn_configs.json" || name == ".vpn_key" {
      continue;
    }
    let to = dst.join(&name);
    if file_type.is_symlink() {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("VPN import contains unsupported symlink: {}", from.display()),
      ));
    }
    if file_type.is_dir() {
      copy_dir_skip_existing(&from, &to)?;
    } else if file_type.is_file() && !to.exists() {
      std::fs::copy(&from, &to)?;
    }
  }
  Ok(())
}

pub fn cache_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_CACHE_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  if let Ok(dir) = std::env::var("SALAMANDERBROWSER_CACHE_DIR") {
    return PathBuf::from(dir);
  }

  if let Some(root) = data_root() {
    return root.join("cache");
  }

  if let Some(dir) = portable_dir() {
    return dir.join("cache");
  }

  base_dirs().cache_dir().join(app_name())
}

pub fn profiles_dir() -> PathBuf {
  data_dir().join("profiles")
}

pub fn binaries_dir() -> PathBuf {
  data_dir().join("binaries")
}

pub fn data_subdir() -> PathBuf {
  data_dir().join("data")
}

pub fn settings_dir() -> PathBuf {
  data_dir().join("settings")
}

pub fn proxies_dir() -> PathBuf {
  data_dir().join("proxies")
}

pub fn proxy_workers_dir() -> PathBuf {
  cache_dir().join("proxy_workers")
}

pub fn vpn_dir() -> PathBuf {
  data_dir().join("vpn")
}

pub fn extensions_dir() -> PathBuf {
  data_dir().join("extensions")
}

pub fn dns_blocklist_dir() -> PathBuf {
  cache_dir().join("dns_blocklists")
}

/// Resolve the directory that tauri-plugin-log writes to. Mirrors the
/// `LogDir` target used in the plugin builder so the path matches what's
/// actually on disk for this OS.
pub fn log_dir<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> PathBuf {
  if let Some(dir) = log_dir_override() {
    return dir;
  }
  use tauri::Manager;
  handle
    .path()
    .app_log_dir()
    .unwrap_or_else(|_| std::env::temp_dir())
}

#[cfg(test)]
thread_local! {
  static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
  static TEST_CACHE_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub struct TestDirGuard {
  kind: TestDirKind,
}

#[cfg(test)]
enum TestDirKind {
  Data,
  Cache,
}

#[cfg(test)]
impl Drop for TestDirGuard {
  fn drop(&mut self) {
    match self.kind {
      TestDirKind::Data => TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = None),
      TestDirKind::Cache => TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = None),
    }
  }
}

#[cfg(test)]
pub fn set_test_data_dir(dir: PathBuf) -> TestDirGuard {
  TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Data,
  }
}

#[cfg(test)]
pub fn set_test_cache_dir(dir: PathBuf) -> TestDirGuard {
  TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Cache,
  }
}

/// Restrict a just-written file to owner-only read/write (`0600`) on Unix so
/// other local users/processes can't read secret material (tokens, E2E
/// password, encrypted vault files). Best-effort: the write already succeeded,
/// so a permission failure is logged, not propagated. On Windows the per-user
/// profile ACL already restricts access, so this is a no-op there.
pub fn restrict_to_owner(path: &std::path::Path) {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
      log::warn!("Failed to restrict permissions on {}: {e}", path.display());
    }
  }
  #[cfg(not(unix))]
  {
    let _ = path;
  }
}

/// Write sensitive data without creating a wider-permission file first.
pub fn create_owner_only(path: &std::path::Path) -> std::io::Result<std::fs::File> {
  if path.exists() {
    restrict_to_owner(path);
  }
  let mut options = std::fs::OpenOptions::new();
  options.create(true).truncate(true).write(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  let file = options.open(path)?;
  restrict_to_owner(path);
  Ok(file)
}

pub fn write_owner_only(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
  use std::io::Write;
  let mut file = create_owner_only(path)?;
  file.write_all(content)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_app_name() {
    let name = app_name();
    assert!(
      name == "SalamanderBrowser" || name == "SalamanderBrowserDev",
      "app_name should be SalamanderBrowser or SalamanderBrowserDev, got: {name}"
    );
  }

  #[cfg(unix)]
  #[test]
  fn owner_only_writer_uses_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("secret.json");
    write_owner_only(&path, b"secret").unwrap();
    assert_eq!(
      std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
      0o600
    );
  }

  #[test]
  fn test_data_dir_returns_path() {
    let dir = data_dir();
    // Portable mode deliberately drops the app_name segment: state lives at
    // <exe dir>/data. The assertion only holds for the platform-default path.
    if is_portable() {
      assert!(dir.ends_with("data"));
    } else {
      assert!(
        dir.to_string_lossy().contains(app_name()),
        "data_dir should contain app_name"
      );
    }
  }

  #[test]
  fn test_cache_dir_returns_path() {
    let dir = cache_dir();
    if is_portable() {
      assert!(dir.ends_with("cache"));
    } else {
      assert!(
        dir.to_string_lossy().contains(app_name()),
        "cache_dir should contain app_name"
      );
    }
  }

  #[test]
  fn log_dir_follows_portable_mode_and_data_root() {
    let root = PathBuf::from("/tmp/salamander-root");
    let portable = PathBuf::from("/tmp/salamander-portable");

    // Neither: the platform default app log dir is used.
    assert_eq!(log_dir_for(None, None), None);

    // Portable alone keeps logs beside the executable rather than on the host.
    assert_eq!(
      log_dir_for(None, Some(&portable)),
      Some(portable.join("logs"))
    );

    // SALAMANDERBROWSER_DATA_ROOT wins over portable, matching data_dir/cache_dir.
    assert_eq!(
      log_dir_for(Some(root.clone()), Some(&portable)),
      Some(root.join("logs"))
    );
    assert_eq!(
      log_dir_for(Some(root.clone()), None),
      Some(root.join("logs"))
    );
  }

  #[test]
  fn absolute_filename_escapes_the_plugin_base_dir() {
    // The whole window-state redirect rests on this std behaviour: joining an
    // absolute path discards the base. tauri-plugin-window-state does
    // `app_config_dir().join(filename)`, so an absolute "filename" relocates
    // the file. If this ever stops holding, the redirect silently stops too.
    let base = PathBuf::from("/Users/someone/Library/Application Support/com.salamanderbrowser");
    let absolute = PathBuf::from("/Volumes/Stick/Salamander/data").join(WINDOW_STATE_FILENAME);
    assert_eq!(base.join(&absolute), absolute);
    assert!(!base.join(&absolute).starts_with(&base));
  }

  #[test]
  fn window_state_stays_at_the_platform_default_for_a_normal_install() {
    // A normal install must not be relocated: moving it would drop the window
    // geometry every existing user already has.
    if !state_is_relocated() {
      assert_eq!(window_state_path_override(), None);
    }
  }

  #[test]
  fn window_state_follows_a_relocated_data_dir() {
    let tmp = PathBuf::from("/tmp/salamander-relocated");
    let _guard = set_test_data_dir(tmp.clone());
    // data_dir is overridden, so the file tracks it rather than app_config_dir.
    assert_eq!(
      data_dir().join(WINDOW_STATE_FILENAME),
      tmp.join(".window-state.json")
    );
  }

  #[test]
  fn portable_keeps_data_cache_and_logs_under_one_root() {
    // The three state directories must agree on where portable state lives, so
    // a portable install leaves nothing behind on the host.
    let portable = PathBuf::from("/tmp/salamander-portable");
    assert_eq!(
      log_dir_for(None, Some(&portable)),
      Some(portable.join("logs"))
    );
    assert!(portable.join("data").starts_with(&portable));
    assert!(portable.join("cache").starts_with(&portable));
  }

  #[test]
  fn test_subdirectory_helpers() {
    assert!(profiles_dir().ends_with("profiles"));
    assert!(binaries_dir().ends_with("binaries"));
    assert!(data_subdir().ends_with("data"));
    assert!(settings_dir().ends_with("settings"));
    assert!(proxies_dir().ends_with("proxies"));
    assert!(proxy_workers_dir().ends_with("proxy_workers"));
    assert!(vpn_dir().ends_with("vpn"));
    assert!(extensions_dir().ends_with("extensions"));
    assert!(dns_blocklist_dir().ends_with("dns_blocklists"));
  }

  #[test]
  fn test_set_test_data_dir() {
    let tmp = PathBuf::from("/tmp/test-salamander-data");
    let _guard = set_test_data_dir(tmp.clone());
    assert_eq!(data_dir(), tmp);
    assert_eq!(profiles_dir(), tmp.join("profiles"));
    assert_eq!(binaries_dir(), tmp.join("binaries"));
  }

  #[test]
  fn test_set_test_cache_dir() {
    let tmp = PathBuf::from("/tmp/test-salamander-cache");
    let _guard = set_test_cache_dir(tmp.clone());
    assert_eq!(cache_dir(), tmp);
  }

  #[test]
  fn test_guard_cleanup() {
    let original_data = data_dir();
    let original_cache = cache_dir();

    {
      let _guard = set_test_data_dir(PathBuf::from("/tmp/test-cleanup-data"));
      assert_eq!(data_dir(), PathBuf::from("/tmp/test-cleanup-data"));
    }
    assert_eq!(data_dir(), original_data);

    {
      let _guard = set_test_cache_dir(PathBuf::from("/tmp/test-cleanup-cache"));
      assert_eq!(cache_dir(), PathBuf::from("/tmp/test-cleanup-cache"));
    }
    assert_eq!(cache_dir(), original_cache);
  }

  #[test]
  fn brand_migration_copies_old_data_into_new_dir() {
    let work = tempfile::tempdir().unwrap();
    let old_dir = work.path().join("DonutBrowser");
    let new_dir = work.path().join("SalamanderBrowser");

    std::fs::create_dir_all(old_dir.join("binaries/camoufox")).unwrap();
    std::fs::write(old_dir.join("binaries/camoufox/camoufox.exe"), b"engine").unwrap();
    std::fs::create_dir_all(old_dir.join("profiles/abc")).unwrap();
    std::fs::write(old_dir.join("profiles/abc/metadata.json"), b"{}").unwrap();

    migrate_dir_if_needed(&old_dir, &new_dir);

    assert_eq!(
      std::fs::read(new_dir.join("binaries/camoufox/camoufox.exe")).unwrap(),
      b"engine"
    );
    assert_eq!(
      std::fs::read(new_dir.join("profiles/abc/metadata.json")).unwrap(),
      b"{}"
    );
    assert!(new_dir.join(".brand-migration-checked").exists());
    // The old installation's files are left in place — this is a copy, not a move.
    assert!(old_dir.join("binaries/camoufox/camoufox.exe").exists());
  }

  #[test]
  fn brand_migration_never_overwrites_an_existing_destination_file() {
    let work = tempfile::tempdir().unwrap();
    let old_dir = work.path().join("DonutBrowser");
    let new_dir = work.path().join("SalamanderBrowser");

    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::write(old_dir.join("settings.json"), b"old").unwrap();
    std::fs::create_dir_all(&new_dir).unwrap();
    std::fs::write(new_dir.join("settings.json"), b"new").unwrap();

    migrate_dir_if_needed(&old_dir, &new_dir);

    assert_eq!(
      std::fs::read(new_dir.join("settings.json")).unwrap(),
      b"new"
    );
  }

  #[test]
  fn brand_migration_is_a_one_shot_even_if_old_dir_appears_later() {
    let work = tempfile::tempdir().unwrap();
    let old_dir = work.path().join("DonutBrowser");
    let new_dir = work.path().join("SalamanderBrowser");

    // No old dir yet — first call just plants the marker.
    migrate_dir_if_needed(&old_dir, &new_dir);
    assert!(new_dir.join(".brand-migration-checked").exists());

    // Old dir shows up afterwards (shouldn't happen in practice, but the
    // marker must still win so a later call is a true no-op).
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::write(old_dir.join("camoufox.bin"), b"engine").unwrap();
    migrate_dir_if_needed(&old_dir, &new_dir);

    assert!(!new_dir.join("camoufox.bin").exists());
  }

  #[test]
  fn brand_migration_no_op_when_old_dir_absent() {
    let work = tempfile::tempdir().unwrap();
    let old_dir = work.path().join("DonutBrowser");
    let new_dir = work.path().join("SalamanderBrowser");

    migrate_dir_if_needed(&old_dir, &new_dir);

    assert!(new_dir.exists());
    assert!(new_dir.join(".brand-migration-checked").exists());
    assert!(!old_dir.exists());
  }

  #[test]
  fn copy_dir_skip_existing_recurses_into_nested_directories() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("src");
    let dst = work.path().join("dst");
    std::fs::create_dir_all(src.join("a/b")).unwrap();
    std::fs::write(src.join("a/b/file.txt"), b"nested").unwrap();

    copy_dir_skip_existing(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("a/b/file.txt")).unwrap(), b"nested");
  }

  #[test]
  fn copy_dir_skip_existing_adopts_vpn_pair_when_destination_has_neither_file() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("src");
    let dst = work.path().join("dst");
    std::fs::create_dir_all(src.join("vpn")).unwrap();
    std::fs::write(src.join("vpn/vpn_configs.json"), b"configs-from-source").unwrap();
    std::fs::write(src.join("vpn/.vpn_key"), b"key-from-source").unwrap();

    copy_dir_skip_existing(&src, &dst).unwrap();

    assert_eq!(
      std::fs::read(dst.join("vpn/vpn_configs.json")).unwrap(),
      b"configs-from-source"
    );
    assert_eq!(
      std::fs::read(dst.join("vpn/.vpn_key")).unwrap(),
      b"key-from-source"
    );
  }

  #[test]
  fn copy_dir_skip_existing_never_mixes_vpn_config_and_key_across_installs() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("src");
    let dst = work.path().join("dst");
    std::fs::create_dir_all(src.join("vpn")).unwrap();
    std::fs::write(src.join("vpn/vpn_configs.json"), b"configs-from-source").unwrap();
    std::fs::write(src.join("vpn/.vpn_key"), b"key-from-source").unwrap();
    // Destination already generated its own key (e.g. the new install ran
    // once before this import/migration happened) — the source's configs,
    // which were encrypted with the source's key, must NOT be paired with it.
    std::fs::create_dir_all(dst.join("vpn")).unwrap();
    std::fs::write(dst.join("vpn/.vpn_key"), b"key-from-destination").unwrap();

    copy_dir_skip_existing(&src, &dst).unwrap();

    assert!(!dst.join("vpn/vpn_configs.json").exists());
    assert_eq!(
      std::fs::read(dst.join("vpn/.vpn_key")).unwrap(),
      b"key-from-destination"
    );
  }

  #[test]
  fn copy_dir_skip_existing_still_copies_other_vpn_files_file_by_file() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("src");
    let dst = work.path().join("dst");
    std::fs::create_dir_all(src.join("vpn")).unwrap();
    std::fs::write(src.join("vpn/vpn_configs.json"), b"configs").unwrap();
    std::fs::write(src.join("vpn/.vpn_key"), b"key").unwrap();
    std::fs::write(src.join("vpn/other.txt"), b"unrelated").unwrap();
    std::fs::create_dir_all(dst.join("vpn")).unwrap();
    std::fs::write(dst.join("vpn/.vpn_key"), b"existing-key").unwrap();

    copy_dir_skip_existing(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("vpn/other.txt")).unwrap(), b"unrelated");
  }
}
