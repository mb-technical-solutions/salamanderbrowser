//! Persistence for the auth module. The only thing that needs to survive a
//! restart is the refresh token, a long-lived bearer credential, so it goes
//! in the OS credential store (Windows Credential Manager / macOS Keychain /
//! Linux Secret Service) via `keyring` instead of a file on disk. Everything
//! else (the access token, who's signed in) lives only in memory for the
//! current process, because `AuthManager::bootstrap` always re-redeems the
//! refresh token on launch rather than trusting anything cached from a prior
//! run — see `StoredAuthState`'s doc comment.

const KEYRING_SERVICE: &str = "salamanderbrowser";
const KEYRING_ACCOUNT: &str = "oidc-refresh-token";

fn keyring_entry() -> Result<keyring::Entry, String> {
  keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
    .map_err(|e| format!("Failed to access OS credential store: {e}"))
}

pub fn save_refresh_token(token: &str) -> Result<(), String> {
  keyring_entry()?
    .set_password(token)
    .map_err(|e| format!("Failed to store refresh token: {e}"))
}

/// `None` covers both "never logged in" and "OS credential store unavailable"
/// — either way the caller's answer is the same: fall through to interactive
/// login.
pub fn load_refresh_token() -> Option<String> {
  keyring_entry().ok()?.get_password().ok()
}

pub fn clear_refresh_token() {
  if let Ok(entry) = keyring_entry() {
    let _ = entry.delete_credential();
  }
}
