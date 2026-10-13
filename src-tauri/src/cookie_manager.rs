use crate::profile::manager::ProfileManager;
use crate::profile::BrowserProfile;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// Unified cookie representation that works across both browser types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCookie {
  pub name: String,
  pub value: String,
  pub domain: String,
  pub path: String,
  pub expires: i64,
  pub is_secure: bool,
  pub is_http_only: bool,
  pub same_site: i32,
  pub creation_time: i64,
  pub last_accessed: i64,
}

/// Cookies grouped by domain for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainCookies {
  pub domain: String,
  pub cookies: Vec<UnifiedCookie>,
  pub cookie_count: usize,
}

/// Result of reading cookies from a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieReadResult {
  pub profile_id: String,
  pub browser_type: String,
  pub domains: Vec<DomainCookies>,
  pub total_count: usize,
}

/// Lightweight cookie metadata for the profile-info dialog. Computed without
/// decrypting any cookie values, so it stays cheap even for multi-MB Chromium
/// cookie stores and never blocks the runtime for noticeable time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieStats {
  pub profile_id: String,
  pub browser_type: String,
  pub total_count: usize,
  /// Every domain the profile has cookies for, sorted by cookie count desc.
  pub domains: Vec<DomainCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainCount {
  pub domain: String,
  pub count: usize,
}

/// Request to copy specific cookies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieCopyRequest {
  pub source_profile_id: String,
  pub target_profile_ids: Vec<String>,
  pub selected_cookies: Vec<SelectedCookie>,
}

/// Identifies a specific cookie to copy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedCookie {
  pub domain: String,
  pub name: String,
}

/// Result of a copy operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieCopyResult {
  pub target_profile_id: String,
  pub cookies_copied: usize,
  pub cookies_replaced: usize,
  pub errors: Vec<String>,
}

/// What a write does to cookies the profile already has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CookieWriteMode {
  /// Update the rows the incoming cookies match, insert the rest, delete
  /// nothing.
  #[default]
  Merge,
  /// Clear every cookie the profile holds for the sites named in this write
  /// before applying it, so the result is exactly what was pasted. Subdomains
  /// and every other site are left alone.
  ReplaceMatchingSites,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieWriteCounts {
  pub added: usize,
  pub overwritten: usize,
  pub deleted: usize,
}

/// One accepted cookie, as the paste preview shows it.
///
/// Deliberately has no `value`: the value IS the credential, and a preview that
/// carries it puts every pasted session token into the frontend's state, its
/// logs and any screenshot of the dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastedCookiePreview {
  pub name: String,
  pub domain: String,
  pub path: String,
  pub expires: i64,
  pub is_secure: bool,
  pub is_http_only: bool,
  pub same_site: i32,
}

impl From<&UnifiedCookie> for PastedCookiePreview {
  fn from(cookie: &UnifiedCookie) -> Self {
    Self {
      name: cookie.name.clone(),
      domain: cookie.domain.clone(),
      path: cookie.path.clone(),
      expires: cookie.expires,
      is_secure: cookie.is_secure,
      is_http_only: cookie.is_http_only,
      same_site: cookie.same_site,
    }
  }
}

/// Everything the paste dialog needs to describe a paste before writing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookiePasteAnalysis {
  pub format: Option<crate::cookie_paste::PasteFormat>,
  pub cookies: Vec<PastedCookiePreview>,
  pub issues: Vec<crate::cookie_paste::CookieIssue>,
  pub site_required: bool,
  pub expired_count: usize,
  /// How many stored rows [`CookieWriteMode::ReplaceMatchingSites`] would
  /// delete for the sites this paste names. `None` when the store cannot be
  /// read — a profile that has never launched, a locked database, a browser
  /// with no supported cookie store.
  pub replace_delete_count: Option<usize>,
  /// The profile wipes its browsing data when the browser exits, so an import
  /// here lives exactly one session. Not an error, but silently losing a pasted
  /// login is the worst thing this feature can do.
  pub clears_on_close: bool,
  /// The `{"code":…}` string explaining why an import would be refused right
  /// now, ready for `translateBackendError`. `None` when the import can run.
  pub blocked_by: Option<String>,
}

/// Result of writing a paste into a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookiePasteImportResult {
  pub added: usize,
  pub overwritten: usize,
  pub deleted: usize,
  /// Cookies that parsed cleanly but were not written, i.e. expired ones the
  /// caller chose to leave out.
  pub skipped: usize,
  pub issues: Vec<crate::cookie_paste::CookieIssue>,
}

pub struct CookieManager;

impl CookieManager {
  /// Get the cookie database path for a profile (read-side: errors if missing).
  fn get_cookie_db_path(profile: &BrowserProfile, profiles_dir: &Path) -> Result<PathBuf, String> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    if profile.browser.as_str() != "camoufox" {
      return Err(format!(
        "Unsupported browser type for cookie operations: {}",
        profile.browser
      ));
    }

    let path = profile_data_path.join("cookies.sqlite");
    if path.exists() {
      Ok(path)
    } else {
      Err(format!("Cookie database not found at: {}", path.display()))
    }
  }

  /// Get the cookie database path for a profile, creating an empty
  /// browser-compatible database if it doesn't exist yet. Use this for write
  /// paths (copy / import) so we can populate the cookie store of a profile
  /// that has never been launched.
  fn ensure_cookie_db_path(
    profile: &BrowserProfile,
    profiles_dir: &Path,
  ) -> Result<PathBuf, String> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    if profile.browser.as_str() != "camoufox" {
      return Err(format!(
        "Unsupported browser type for cookie operations: {}",
        profile.browser
      ));
    }

    let path = profile_data_path.join("cookies.sqlite");
    if !path.exists() {
      Self::create_empty_firefox_cookies_db(&path)?;
    }
    Ok(path)
  }

  /// Create an empty Firefox-format cookies.sqlite database at `path`.
  fn create_empty_firefox_cookies_db(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create cookie directory: {e}"))?;
    }
    let conn =
      Connection::open(path).map_err(|e| format!("Failed to create cookie database: {e}"))?;
    conn
      .execute_batch(
        "CREATE TABLE moz_cookies (
          id INTEGER PRIMARY KEY,
          originAttributes TEXT NOT NULL DEFAULT '',
          name TEXT,
          value TEXT,
          host TEXT,
          path TEXT,
          expiry INTEGER,
          lastAccessed INTEGER,
          creationTime INTEGER,
          isSecure INTEGER,
          isHttpOnly INTEGER,
          inBrowserElement INTEGER DEFAULT 0,
          sameSite INTEGER DEFAULT 0,
          rawSameSite INTEGER DEFAULT 0,
          schemeMap INTEGER DEFAULT 0,
          CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes)
        );",
      )
      .map_err(|e| format!("Failed to initialize cookie database schema: {e}"))?;
    Ok(())
  }

  /// Read cookies from a Camoufox (Firefox-format) profile.
  fn read_firefox_cookies(db_path: &Path) -> Result<Vec<UnifiedCookie>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut stmt = conn
      .prepare(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly,
                sameSite, creationTime, lastAccessed
         FROM moz_cookies",
      )
      .map_err(|e| format!("Failed to prepare statement: {e}"))?;

    let cookies = stmt
      .query_map([], |row| {
        Ok(UnifiedCookie {
          name: row.get(0)?,
          value: row.get(1)?,
          domain: row.get(2)?,
          path: row.get(3)?,
          expires: row.get(4)?,
          is_secure: row.get::<_, i32>(5)? != 0,
          is_http_only: row.get::<_, i32>(6)? != 0,
          same_site: row.get(7)?,
          creation_time: row.get::<_, i64>(8)? / 1_000_000,
          last_accessed: row.get::<_, i64>(9)? / 1_000_000,
        })
      })
      .map_err(|e| format!("Failed to query cookies: {e}"))?
      .collect::<Result<Vec<_>, _>>()
      .map_err(|e| format!("Failed to collect cookies: {e}"))?;

    Ok(cookies)
  }

  /// Write cookies to a Camoufox (Firefox-format) profile.
  ///
  /// Firefox's `moz_cookies.expiry` is "seconds since Unix epoch", so
  /// `expiry = 0` is interpreted as 1970-01-01 and purged on read. To let
  /// imported session cookies survive browser restart, rewrite them to a
  /// far-future expiry.
  ///
  /// `schemeMap` is a bitfield (1 = HTTP, 2 = HTTPS, 3 = both). Setting it
  /// based on `is_secure` preserves Firefox's scheme-bound cookie enforcement.
  fn write_firefox_cookies(
    db_path: &Path,
    cookies: &[UnifiedCookie],
    mode: CookieWriteMode,
  ) -> Result<CookieWriteCounts, String> {
    let mut conn =
      Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;
    let tx = conn
      .transaction()
      .map_err(|e| format!("Failed to start cookie transaction: {e}"))?;

    let mut counts = CookieWriteCounts::default();

    if mode == CookieWriteMode::ReplaceMatchingSites {
      for host in Self::replace_host_keys(cookies) {
        counts.deleted += tx
          .execute("DELETE FROM moz_cookies WHERE host = ?1", params![host])
          .map_err(|e| format!("Failed to clear existing cookies: {e}"))?;
      }
    }

    let now = Self::now_secs();
    // Session cookies get 30 days of persistence so they survive restart.
    let session_cookie_expiry = now + 30 * 86400;

    for cookie in cookies {
      let expiry = if cookie.expires > 0 {
        cookie.expires
      } else {
        session_cookie_expiry
      };
      // schemeMap bitfield: 1 = HTTP, 2 = HTTPS
      let scheme_map: i32 = if cookie.is_secure { 2 } else { 1 };

      let existing: Option<i64> = tx
        .query_row(
          "SELECT id FROM moz_cookies WHERE host = ?1 AND name = ?2 AND path = ?3",
          params![&cookie.domain, &cookie.name, &cookie.path],
          |row| row.get(0),
        )
        .ok();

      if let Some(id) = existing {
        tx.execute(
          "UPDATE moz_cookies SET value = ?1, expiry = ?2, isSecure = ?3,
                   isHttpOnly = ?4, sameSite = ?5, rawSameSite = ?5,
                   lastAccessed = ?6, schemeMap = ?7
                   WHERE id = ?8",
          params![
            &cookie.value,
            expiry,
            cookie.is_secure as i32,
            cookie.is_http_only as i32,
            cookie.same_site,
            cookie.last_accessed * 1_000_000,
            scheme_map,
            id,
          ],
        )
        .map_err(|e| format!("Failed to update cookie: {e}"))?;
        counts.overwritten += 1;
      } else {
        tx.execute(
          "INSERT INTO moz_cookies
                   (originAttributes, name, value, host, path, expiry, lastAccessed,
                    creationTime, isSecure, isHttpOnly, sameSite, rawSameSite, schemeMap)
                   VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11)",
          params![
            &cookie.name,
            &cookie.value,
            &cookie.domain,
            &cookie.path,
            expiry,
            cookie.last_accessed * 1_000_000,
            cookie.creation_time * 1_000_000,
            cookie.is_secure as i32,
            cookie.is_http_only as i32,
            cookie.same_site,
            scheme_map,
          ],
        )
        .map_err(|e| format!("Failed to insert cookie: {e}"))?;
        counts.added += 1;
      }
    }

    tx.commit()
      .map_err(|e| format!("Failed to commit cookies: {e}"))?;

    Ok(counts)
  }

  /// Which `host_key` values a replace-mode write clears out first.
  ///
  /// `.example.com` and `example.com` are one site to the person pasting and
  /// two distinct host keys to Chromium, so replacing one without the other
  /// leaves the stale half of the login behind. Subdomains stay untouched:
  /// nothing in the paste says anything about them.
  fn replace_host_keys(cookies: &[UnifiedCookie]) -> Vec<String> {
    let mut hosts = std::collections::BTreeSet::new();
    for cookie in cookies {
      let bare = cookie.domain.trim_start_matches('.');
      if bare.is_empty() {
        continue;
      }
      hosts.insert(bare.to_string());
      hosts.insert(format!(".{bare}"));
    }
    hosts.into_iter().collect()
  }

  /// How many stored rows a replace-mode write of `cookies` would delete.
  ///
  /// Reads the snapshot view so a running browser's lock does not turn the
  /// preview into an error.
  fn count_replaceable_rows(db_path: &Path, cookies: &[UnifiedCookie]) -> Result<usize, String> {
    let conn = Self::open_cookie_db_readonly(db_path)?;
    let mut total = 0usize;
    for host in Self::replace_host_keys(cookies) {
      let count: i64 = conn
        .query_row(
          "SELECT COUNT(*) FROM moz_cookies WHERE host = ?1",
          params![host],
          |row| row.get(0),
        )
        .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?;
      total += count as usize;
    }
    Ok(total)
  }

  /// Public API: Read cookies from a profile
  pub fn read_cookies(profile_id: &str) -> Result<CookieReadResult, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    let db_path = Self::get_cookie_db_path(profile, &profiles_dir)?;

    let cookies = match profile.browser.as_str() {
      "camoufox" => Self::read_firefox_cookies(&db_path)?,
      _ => return Err(format!("Unsupported browser type: {}", profile.browser)),
    };

    let mut domain_map: HashMap<String, Vec<UnifiedCookie>> = HashMap::new();

    for cookie in cookies {
      domain_map
        .entry(cookie.domain.clone())
        .or_default()
        .push(cookie);
    }

    let mut domains: Vec<DomainCookies> = domain_map
      .into_iter()
      .map(|(domain, cookies)| DomainCookies {
        domain,
        cookie_count: cookies.len(),
        cookies,
      })
      .collect();

    domains.sort_by(|a, b| a.domain.cmp(&b.domain));

    let total_count = domains.iter().map(|d| d.cookie_count).sum();

    Ok(CookieReadResult {
      profile_id: profile_id.to_string(),
      browser_type: profile.browser.clone(),
      domains,
      total_count,
    })
  }

  /// Open the cookie SQLite database read-only without acquiring any lock.
  ///
  /// `immutable=1` tells SQLite the file will not change during the read,
  /// which causes it to skip all locking. That lets us read metadata even
  /// while the browser holds an exclusive lock on the cookies database —
  /// the trade-off is that we may see a slightly stale snapshot, which is
  /// acceptable for the badge/preview use cases this powers.
  fn open_cookie_db_readonly(db_path: &Path) -> Result<Connection, String> {
    let path_str = db_path.to_string_lossy();
    if path_str.contains('?') || path_str.contains('#') {
      return Err(
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": "profile path contains a reserved URI character" }
        })
        .to_string(),
      );
    }
    let uri = format!("file:{path_str}?mode=ro&immutable=1");
    Connection::open_with_flags(
      &uri,
      OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
      let code = if e.to_string().to_lowercase().contains("locked") {
        "COOKIE_DB_LOCKED"
      } else {
        "COOKIE_DB_UNAVAILABLE"
      };
      serde_json::json!({
        "code": code,
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })
  }

  /// Public API: read lightweight stats (total count + top 5 domains) for a
  /// profile's cookie store. Reads from a snapshot view of the SQLite file
  /// without holding a lock, so this works while the browser is running.
  pub fn read_stats(profile_id: &str) -> Result<CookieStats, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager.list_profiles().map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| serde_json::json!({ "code": "PROFILE_NOT_FOUND" }).to_string())?;

    let db_path = Self::get_cookie_db_path(profile, &profiles_dir).map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e }
      })
      .to_string()
    })?;

    let conn = Self::open_cookie_db_readonly(&db_path)?;

    let (count_sql, domain_sql) = match profile.browser.as_str() {
      "camoufox" => (
        "SELECT COUNT(*) FROM moz_cookies",
        "SELECT host, COUNT(*) FROM moz_cookies GROUP BY host ORDER BY COUNT(*) DESC, host ASC",
      ),
      _ => {
        return Err(
          serde_json::json!({
            "code": "COOKIE_DB_UNAVAILABLE",
            "params": { "detail": format!("unsupported browser: {}", profile.browser) }
          })
          .to_string(),
        )
      }
    };

    let total_count: usize = conn
      .query_row(count_sql, [], |row| row.get::<_, i64>(0))
      .map_err(|e| {
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": e.to_string() }
        })
        .to_string()
      })? as usize;

    let mut stmt = conn.prepare(domain_sql).map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })?;
    let domains: Vec<DomainCount> = stmt
      .query_map([], |row| {
        Ok(DomainCount {
          domain: row.get::<_, String>(0)?,
          count: row.get::<_, i64>(1)? as usize,
        })
      })
      .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
      .map_err(|e| {
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": e.to_string() }
        })
        .to_string()
      })?;

    Ok(CookieStats {
      profile_id: profile_id.to_string(),
      browser_type: profile.browser.clone(),
      total_count,
      domains,
    })
  }

  /// Public API: Copy cookies between profiles
  pub async fn copy_cookies(
    app_handle: &AppHandle,
    request: CookieCopyRequest,
  ) -> Result<Vec<CookieCopyResult>, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let source = profiles
      .iter()
      .find(|p| p.id.to_string() == request.source_profile_id)
      .ok_or_else(|| format!("Source profile not found: {}", request.source_profile_id))?;

    let source_db_path = Self::get_cookie_db_path(source, &profiles_dir)?;
    let all_cookies = match source.browser.as_str() {
      "camoufox" => Self::read_firefox_cookies(&source_db_path)?,
      _ => return Err(format!("Unsupported browser type: {}", source.browser)),
    };

    let cookies_to_copy: Vec<UnifiedCookie> = if request.selected_cookies.is_empty() {
      all_cookies
    } else {
      all_cookies
        .into_iter()
        .filter(|c| {
          request.selected_cookies.iter().any(|s| {
            if s.name.is_empty() {
              c.domain == s.domain
            } else {
              c.domain == s.domain && c.name == s.name
            }
          })
        })
        .collect()
    };

    let mut results = Vec::new();

    for target_id in &request.target_profile_ids {
      let target = match profiles.iter().find(|p| p.id.to_string() == *target_id) {
        Some(p) => p,
        None => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![format!("Profile not found: {target_id}")],
          });
          continue;
        }
      };

      let is_running = profile_manager
        .check_browser_status(app_handle.clone(), target)
        .await
        .unwrap_or(false);

      if is_running {
        results.push(CookieCopyResult {
          target_profile_id: target_id.clone(),
          cookies_copied: 0,
          cookies_replaced: 0,
          errors: vec![format!("Browser is running for profile: {}", target.name)],
        });
        continue;
      }

      // Target may be a brand-new profile that has never been launched, so
      // its Cookies DB file doesn't exist yet. Create an empty one on demand.
      let target_db_path = match Self::ensure_cookie_db_path(target, &profiles_dir) {
        Ok(p) => p,
        Err(e) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![e],
          });
          continue;
        }
      };

      let write_result = match target.browser.as_str() {
        "camoufox" => {
          Self::write_firefox_cookies(&target_db_path, &cookies_to_copy, CookieWriteMode::Merge)
        }
        _ => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![format!("Unsupported browser: {}", target.browser)],
          });
          continue;
        }
      };

      match write_result {
        Ok(counts) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: counts.added,
            cookies_replaced: counts.overwritten,
            errors: vec![],
          });
        }
        Err(e) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![e],
          });
        }
      }
    }

    Ok(results)
  }

  /// Format cookies as Netscape TXT
  ///
  /// http-only cookies carry the `#HttpOnly_` host prefix curl and the
  /// cookies.txt extensions use. The plain format has no field for the flag, so
  /// without the prefix an export of a login cookie re-imports as a cookie any
  /// page script can read.
  pub fn format_netscape_cookies(cookies: &[UnifiedCookie]) -> String {
    let mut lines = Vec::new();
    lines.push("# Netscape HTTP Cookie File".to_string());
    for cookie in cookies {
      let flag = if cookie.domain.starts_with('.') {
        "TRUE"
      } else {
        "FALSE"
      };
      let secure = if cookie.is_secure { "TRUE" } else { "FALSE" };
      let host = if cookie.is_http_only {
        format!("#HttpOnly_{}", cookie.domain)
      } else {
        cookie.domain.clone()
      };
      lines.push(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        host, flag, cookie.path, secure, cookie.expires, cookie.name, cookie.value
      ));
    }
    lines.join("\n")
  }

  /// Format cookies as JSON
  pub fn format_json_cookies(cookies: &[UnifiedCookie]) -> String {
    let arr: Vec<Value> = cookies
      .iter()
      .map(|c| {
        // -1 is Chromium's "unspecified", which is what every cookie that never
        // carried a SameSite attribute stores. Exporting it as
        // "no_restriction" turned it into an explicit SameSite=None on the way
        // back in, and Chromium refuses to send those unless they are also
        // Secure — so a re-imported login simply stopped being sent.
        let same_site_str = match c.same_site {
          0 => "no_restriction",
          1 => "lax",
          2 => "strict",
          _ => "unspecified",
        };
        serde_json::json!({
          "name": c.name,
          "value": c.value,
          "domain": c.domain,
          "path": c.path,
          "secure": c.is_secure,
          "httpOnly": c.is_http_only,
          "sameSite": same_site_str,
          "expirationDate": c.expires,
          "session": c.expires == 0,
          "hostOnly": !c.domain.starts_with('.'),
        })
      })
      .collect();
    serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string())
  }

  /// The profile a cookie write targets, or the code saying it does not exist.
  fn paste_target(profile_id: &str) -> Result<BrowserProfile, String> {
    ProfileManager::instance()
      .list_profiles()
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))
  }

  /// Why writing cookies into this profile would be refused right now, as the
  /// `{"code":…}` string the frontend translates. `None` means go ahead.
  async fn paste_blocker(app_handle: &AppHandle, profile: &BrowserProfile) -> Option<String> {
    let is_running = ProfileManager::instance()
      .check_browser_status(app_handle.clone(), profile)
      .await
      .unwrap_or(false);
    if is_running {
      return Some(crate::backend_error("COOKIE_IMPORT_BROWSER_RUNNING"));
    }
    // The profile directory on disk is ciphertext while locked; a plaintext
    // SQLite write into it is not a cookie, it is corruption.
    if profile.password_protected {
      return Some(crate::backend_error("COOKIE_IMPORT_PROFILE_PROTECTED"));
    }
    None
  }

  /// Whether a successful import into this profile would be wiped on browser
  /// exit. Mirrors the condition `browser_runner` actually applies, which lets
  /// ephemeral and password-protected profiles take their own paths first.
  fn clears_cookies_on_close(profile: &BrowserProfile) -> bool {
    profile.clear_on_close && !profile.ephemeral && !profile.password_protected
  }

  /// Public API: describe a pasted blob without writing anything.
  pub async fn analyze_paste(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
    site: Option<&str>,
  ) -> Result<CookiePasteAnalysis, String> {
    let profile = Self::paste_target(profile_id)?;
    let parsed = crate::cookie_paste::parse_paste(content, site);
    let blocked_by = Self::paste_blocker(app_handle, &profile).await;

    // Counted over every accepted cookie, so this is the number of rows the
    // sites in the paste currently hold — the ceiling on what replace mode
    // removes, whatever the caller later decides about expired cookies.
    let replace_delete_count = if parsed.cookies.is_empty() {
      Some(0)
    } else {
      Self::get_cookie_db_path(&profile, &ProfileManager::instance().get_profiles_dir())
        .ok()
        .and_then(|db_path| Self::count_replaceable_rows(&db_path, &parsed.cookies).ok())
    };

    Ok(CookiePasteAnalysis {
      format: parsed.format,
      cookies: parsed
        .cookies
        .iter()
        .map(PastedCookiePreview::from)
        .collect(),
      issues: parsed.issues,
      site_required: parsed.site_required,
      expired_count: parsed.expired_count,
      replace_delete_count,
      clears_on_close: Self::clears_cookies_on_close(&profile),
      blocked_by,
    })
  }

  /// Public API: write a pasted blob into a profile's cookie store.
  pub async fn import_paste(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
    site: Option<&str>,
    mode: CookieWriteMode,
    include_expired: bool,
  ) -> Result<CookiePasteImportResult, String> {
    let profile = Self::paste_target(profile_id)?;
    if let Some(blocker) = Self::paste_blocker(app_handle, &profile).await {
      return Err(blocker);
    }

    let parsed = crate::cookie_paste::parse_paste(content, site);
    if parsed.cookies.is_empty() {
      return Err(crate::backend_error("COOKIE_IMPORT_NO_COOKIES"));
    }

    let now = Self::now_secs();
    let total = parsed.cookies.len();
    let to_write: Vec<UnifiedCookie> = if include_expired {
      parsed.cookies
    } else {
      parsed
        .cookies
        .into_iter()
        .filter(|c| c.expires == 0 || c.expires > now)
        .collect()
    };
    let skipped = total - to_write.len();

    let counts = Self::write_paste(&profile, &to_write, mode).await?;

    Ok(CookiePasteImportResult {
      added: counts.added,
      overwritten: counts.overwritten,
      deleted: counts.deleted,
      skipped,
      issues: parsed.issues,
    })
  }

  /// The SQLite half of a paste write, off the async runtime.
  ///
  /// `check_browser_status` is async and every caller must await it before
  /// getting here; only this synchronous tail belongs on a blocking thread.
  async fn write_paste(
    profile: &BrowserProfile,
    cookies: &[UnifiedCookie],
    mode: CookieWriteMode,
  ) -> Result<CookieWriteCounts, String> {
    let browser = profile.browser.clone();
    if browser != "camoufox" {
      return Err(crate::backend_error_with_detail(
        "COOKIE_DB_UNAVAILABLE",
        format!("unsupported browser: {}", profile.browser),
      ));
    }

    // A profile that has never launched has no Cookies file yet.
    let db_path =
      Self::ensure_cookie_db_path(profile, &ProfileManager::instance().get_profiles_dir())
        .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?;
    let owned: Vec<UnifiedCookie> = cookies.to_vec();

    tokio::task::spawn_blocking(move || Self::write_firefox_cookies(&db_path, &owned, mode))
      .await
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))
  }

  fn now_secs() -> i64 {
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs() as i64)
      .unwrap_or(0)
  }

  /// Public API: Export cookies from a profile in the specified format
  pub fn export_cookies(profile_id: &str, format: &str) -> Result<String, String> {
    let result = Self::read_cookies(profile_id)?;
    let all_cookies: Vec<UnifiedCookie> =
      result.domains.into_iter().flat_map(|d| d.cookies).collect();

    match format {
      "json" => Ok(Self::format_json_cookies(&all_cookies)),
      "netscape" => Ok(Self::format_netscape_cookies(&all_cookies)),
      _ => Err(format!("Unsupported export format: {format}")),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[cfg(target_os = "macos")]
  const SYNTHETIC_COOKIE_HOST: &str = ".example.test";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_COOKIE_VALUE: &str = "synthetic-cookie-value";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_OS_CRYPT_PASSWORD: &[u8] = b"salamander-synthetic-cookie-key";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_ENCRYPTED_COOKIE_HEX: &str = "763130d83b9fd3e6d1b1c793769f55251f5e9d1193be72c0c08ea32e2cf068a85d9d0b97d8b2e6deca93a2b3c290e98e1a851f83d5566f9aa9314befe56dc6bdbd423d";

  #[cfg(target_os = "macos")]
  fn synthetic_encrypted_cookie() -> Vec<u8> {
    (0..SYNTHETIC_ENCRYPTED_COOKIE_HEX.len())
      .step_by(2)
      .map(|i| u8::from_str_radix(&SYNTHETIC_ENCRYPTED_COOKIE_HEX[i..i + 2], 16).unwrap())
      .collect()
  }

  #[test]
  fn test_format_netscape_cookies() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: false,
      same_site: 0,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_netscape_cookies(&cookies);
    assert!(output.contains("# Netscape HTTP Cookie File"));
    assert!(output.contains(".example.com\tTRUE\t/\tTRUE\t1700000000\tsid\tabc"));
  }

  #[test]
  fn test_format_netscape_cookies_marks_http_only() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: true,
      same_site: -1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_netscape_cookies(&cookies);
    assert!(output.contains("#HttpOnly_.example.com\tTRUE\t/\tTRUE\t1700000000\tsid\tabc"));

    let parsed = crate::cookie_paste::parse_paste(&output, None);
    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(parsed.cookies[0].domain, ".example.com");
    assert!(parsed.cookies[0].is_http_only);
  }

  #[test]
  fn test_format_json_cookies() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: true,
      same_site: 1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_json_cookies(&cookies);
    let parsed: Vec<Value> = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["name"], "sid");
    assert_eq!(parsed[0]["sameSite"], "lax");
    assert_eq!(parsed[0]["session"], false);
    assert_eq!(parsed[0]["hostOnly"], false);
  }

  #[test]
  fn test_netscape_roundtrip() {
    let cookies = vec![
      UnifiedCookie {
        name: "a".to_string(),
        value: "1".to_string(),
        domain: ".d.com".to_string(),
        path: "/".to_string(),
        expires: 1700000000,
        is_secure: true,
        is_http_only: false,
        same_site: 0,
        creation_time: 0,
        last_accessed: 0,
      },
      UnifiedCookie {
        name: "b".to_string(),
        value: "2".to_string(),
        domain: "d.com".to_string(),
        path: "/p".to_string(),
        expires: 0,
        is_secure: false,
        is_http_only: false,
        same_site: 0,
        creation_time: 0,
        last_accessed: 0,
      },
    ];
    let formatted = CookieManager::format_netscape_cookies(&cookies);
    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "a");
    assert_eq!(parsed[0].domain, ".d.com");
    assert!(parsed[0].is_secure);
    assert_eq!(parsed[1].name, "b");
    assert_eq!(parsed[1].domain, "d.com");
  }

  #[test]
  fn test_json_roundtrip() {
    let cookies = vec![UnifiedCookie {
      name: "tok".to_string(),
      value: "xyz".to_string(),
      domain: ".site.org".to_string(),
      path: "/app".to_string(),
      expires: 1700000000,
      is_secure: false,
      is_http_only: true,
      same_site: 2,
      creation_time: 0,
      last_accessed: 0,
    }];
    let formatted = CookieManager::format_json_cookies(&cookies);
    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "tok");
    assert_eq!(parsed[0].domain, ".site.org");
    assert_eq!(parsed[0].path, "/app");
    assert!(!parsed[0].is_secure);
    assert!(parsed[0].is_http_only);
    assert_eq!(parsed[0].same_site, 2);
    assert_eq!(parsed[0].expires, 1700000000);
  }

  /// Every Netscape import now lands as -1 (unspecified), which used to export
  /// as "no_restriction" and come back as an explicit SameSite=None — a cookie
  /// Chromium then refuses to send unless it is also Secure.
  #[test]
  fn test_json_roundtrip_preserves_unspecified_same_site() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "v".to_string(),
      domain: ".site.org".to_string(),
      path: "/".to_string(),
      expires: 1900000000,
      is_secure: false,
      is_http_only: false,
      same_site: -1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let formatted = CookieManager::format_json_cookies(&cookies);
    assert_eq!(
      serde_json::from_str::<Vec<Value>>(&formatted).unwrap()[0]["sameSite"],
      "unspecified"
    );

    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].same_site, -1);
  }
}
