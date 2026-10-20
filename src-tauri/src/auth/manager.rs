use super::oidc_client::{self, OidcError};
use super::pkce;
use super::storage;
use super::types::{AuthStatusDto, TokenResponse};
use chrono::Utc;
use std::sync::Mutex;

/// A pending login (its `state`/PKCE verifier live only in this process's
/// memory) older than this is treated as abandoned rather than matched
/// against a late or replayed callback.
const PENDING_LOGIN_TTL_SECS: i64 = 600;

struct PendingLogin {
  state: String,
  verifier: String,
  created_at: i64,
}

pub struct AuthManager {
  pending: Mutex<Option<PendingLogin>>,
}

impl AuthManager {
  fn new() -> Self {
    Self {
      pending: Mutex::new(None),
    }
  }

  pub fn instance() -> &'static AuthManager {
    &AUTH_MANAGER
  }

  /// Run once at launch. Always redeems the stored refresh token against
  /// Authentik — never trusts a locally cached token as still good — so that
  /// a user deactivated or a session revoked in Authentik loses access on the
  /// very next launch rather than whenever a cached token happened to expire.
  /// Still no password prompt: the refresh token itself is what's silent.
  /// Interactive login is only reached when there's nothing left to redeem.
  pub async fn bootstrap(&self) -> AuthStatusDto {
    let Some(refresh_token) = storage::load_refresh_token() else {
      return AuthStatusDto::LoginRequired;
    };

    match oidc_client::refresh(&refresh_token).await {
      Ok(tokens) => self.store_tokens(tokens),
      Err(OidcError::InvalidGrant(_)) => {
        self.clear_session();
        AuthStatusDto::LoginRequired
      }
      Err(e) => AuthStatusDto::Error {
        message: e.to_string(),
      },
    }
  }

  /// Opens the system browser to Authentik's authorize page for a fresh PKCE
  /// pair, remembering the verifier/state in memory until the deep-link
  /// callback arrives.
  pub async fn start_login(&self, app: &tauri::AppHandle) -> Result<(), String> {
    log::info!("[auth] start_login: building authorize URL");
    let pair = pkce::generate_pair();
    let state = pkce::generate_state();
    let url = oidc_client::authorize_url(&pair.challenge, &state)
      .await
      .map_err(|e| {
        log::error!("[auth] start_login: failed to build authorize URL: {e}");
        e.to_string()
      })?;
    log::info!("[auth] start_login: authorize URL built, opening system browser: {url}");

    *self.pending.lock().unwrap() = Some(PendingLogin {
      state,
      verifier: pair.verifier,
      created_at: Utc::now().timestamp(),
    });

    // If Salamander itself is registered as the OS default browser, asking
    // the opener to use "the default browser" hands the authorize URL right
    // back to Salamander (it's an http(s) URL, and Salamander's own deep-link
    // handling treats those as "open this in a profile" — see
    // `handle_url_open` in `lib.rs`) instead of a real browser, which looks
    // like the browser never opened at all. Name an explicit, near-always-
    // installed browser in that case so the login page actually gets a place
    // to render.
    let is_default_browser = crate::settings_manager::SettingsManager::instance()
      .load_settings()
      .map(|s| s.set_as_default_browser)
      .unwrap_or(false);
    let with_browser = is_default_browser.then(fallback_browser_program);
    if let Some(browser) = with_browser {
      log::info!(
        "[auth] start_login: Salamander is the default browser, opening explicitly with {browser}"
      );
    }

    use tauri_plugin_opener::OpenerExt;
    match app.opener().open_url(url, with_browser) {
      Ok(()) => {
        log::info!("[auth] start_login: opener reported success");
        Ok(())
      }
      Err(e) => {
        log::error!("[auth] start_login: opener failed: {e}");
        Err(format!("Failed to open system browser: {e}"))
      }
    }
  }

  /// Handles the `salamander-auth://callback?...` URL from `on_open_url`.
  pub async fn handle_callback(&self, callback_url: &str) -> AuthStatusDto {
    match self.handle_callback_inner(callback_url).await {
      Ok(status) => status,
      Err(message) => AuthStatusDto::Error { message },
    }
  }

  async fn handle_callback_inner(&self, callback_url: &str) -> Result<AuthStatusDto, String> {
    let url = url::Url::parse(callback_url).map_err(|e| format!("Invalid callback URL: {e}"))?;
    let params: std::collections::HashMap<String, String> =
      url.query_pairs().into_owned().collect();

    if let Some(error) = params.get("error") {
      return Err(
        params
          .get("error_description")
          .cloned()
          .unwrap_or_else(|| error.clone()),
      );
    }

    let code = params
      .get("code")
      .ok_or("Sign-in callback had no authorization code")?;
    let returned_state = params
      .get("state")
      .ok_or("Sign-in callback had no state parameter")?;

    let pending = self
      .pending
      .lock()
      .unwrap()
      .take()
      .ok_or("No sign-in attempt was in progress for this callback")?;

    if Utc::now().timestamp() - pending.created_at > PENDING_LOGIN_TTL_SECS {
      return Err("Sign-in attempt expired, please try again".to_string());
    }
    if pending.state != *returned_state {
      return Err("Sign-in callback did not match the request that started it".to_string());
    }

    let tokens = oidc_client::exchange_code(code, &pending.verifier)
      .await
      .map_err(|e| e.to_string())?;
    Ok(self.store_tokens(tokens))
  }

  pub fn logout(&self) -> Result<(), String> {
    self.clear_session();
    Ok(())
  }

  fn clear_session(&self) {
    storage::clear_refresh_token();
  }

  fn store_tokens(&self, tokens: TokenResponse) -> AuthStatusDto {
    let claims = tokens
      .id_token
      .as_deref()
      .and_then(oidc_client::decode_id_token_claims);

    if let Some(refresh_token) = &tokens.refresh_token {
      if let Err(e) = storage::save_refresh_token(refresh_token) {
        log::warn!("Failed to persist refresh token: {e}");
      }
    }

    AuthStatusDto::Authenticated {
      username: claims.as_ref().and_then(|c| c.preferred_username.clone()),
      email: claims.as_ref().and_then(|c| c.email.clone()),
    }
  }
}

/// A browser Salamander could never itself be registered as — used in place
/// of "the OS default browser" specifically when Salamander IS that default,
/// to break the self-referential loop described in `start_login`.
#[cfg(target_os = "windows")]
fn fallback_browser_program() -> &'static str {
  "msedge"
}

#[cfg(target_os = "macos")]
fn fallback_browser_program() -> &'static str {
  "Safari"
}

#[cfg(target_os = "linux")]
fn fallback_browser_program() -> &'static str {
  ["firefox", "google-chrome", "chromium", "chromium-browser"]
    .into_iter()
    .find(|bin| {
      std::process::Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    })
    .unwrap_or("firefox")
}

lazy_static::lazy_static! {
  static ref AUTH_MANAGER: AuthManager = AuthManager::new();
}

#[tauri::command]
pub async fn auth_bootstrap() -> Result<AuthStatusDto, String> {
  Ok(AuthManager::instance().bootstrap().await)
}

#[tauri::command]
pub async fn auth_start_login(app: tauri::AppHandle) -> Result<(), String> {
  AuthManager::instance().start_login(&app).await
}

#[tauri::command]
pub async fn auth_logout() -> Result<(), String> {
  AuthManager::instance().logout()?;
  // The gate (`AuthGateProvider`) only runs its own check once, at mount —
  // this is what tells it to show the login screen again for a logout
  // triggered from deep inside the already-authenticated app (Settings).
  let _ = crate::events::emit("auth-state-changed", AuthStatusDto::LoginRequired);
  Ok(())
}
