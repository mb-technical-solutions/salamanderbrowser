//! Best-effort secure deletion.
//!
//! "Best-effort" is load-bearing and is not a hedge. On copy-on-write
//! filesystems (APFS, Btrfs, ZFS, ReFS) and on any SSD with wear levelling, the
//! blocks holding the old contents may survive an overwrite entirely, because
//! the write lands somewhere else. RAM-backed storage can also be paged out,
//! and zeroing a file cannot reach the swap slot that held it. Treat these
//! helpers as raising the cost of recovery, never as a guarantee of erasure.
//!
//! These helpers raise the cost of recovering deleted data where it is written
//! to disk at all; they are not a substitute for not writing it.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Zero a file's bytes and flush before unlinking, so the contents are not
/// trivially recoverable from the freed blocks.
///
/// The overwrite must never gate the unlink. A write that fails part-way
/// (ENOSPC on a copy-on-write volume, EIO) would otherwise leave the file both
/// un-wiped and un-deleted, which is strictly worse than the plain remove this
/// replaces, because callers report success either way and the data would
/// silently survive.
pub fn secure_remove_file(path: &Path) -> std::io::Result<()> {
  if let Ok(meta) = fs::metadata(path) {
    let len = meta.len();
    if len > 0 {
      if let Ok(mut f) = fs::OpenOptions::new().write(true).open(path) {
        let zeros = vec![0u8; 64 * 1024];
        let mut remaining = len;
        while remaining > 0 {
          let chunk = remaining.min(zeros.len() as u64) as usize;
          if f.write_all(&zeros[..chunk]).is_err() {
            break;
          }
          remaining -= chunk as u64;
        }
        // One flush per file, not per chunk: syncing every 64 KiB turns a
        // profile teardown into thousands of barriers for no extra safety.
        let _ = f.flush();
        let _ = f.sync_all();
      }
    }
  }
  fs::remove_file(path)
}

/// Remove a directory tree, optionally zeroing regular files first.
pub fn secure_remove_dir_all(root: &Path, zero: bool) -> std::io::Result<u64> {
  if !root.exists() {
    return Ok(0);
  }
  let mut removed = 0;
  for entry in fs::read_dir(root)? {
    let path = entry?.path();
    if path.is_dir() {
      removed += secure_remove_dir_all(&path, zero)?;
    } else {
      if zero {
        secure_remove_file(&path)?;
      } else {
        fs::remove_file(&path)?;
      }
      removed += 1;
    }
  }
  fs::remove_dir(root)?;
  Ok(removed)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn zeroing_erase_removes_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let cookies = tmp.path().join("Cookies");
    fs::write(&cookies, b"session=supersecretvalue").unwrap();

    secure_remove_file(&cookies).unwrap();
    assert!(!cookies.exists());
  }

  #[test]
  fn missing_file_reports_the_usual_not_found_error() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("never-existed");
    assert_eq!(
      secure_remove_file(&absent).unwrap_err().kind(),
      std::io::ErrorKind::NotFound
    );
  }
}
