use serde::{Deserialize, Serialize};

/// What the frontend's auth gate should show. Sent as the return value of
/// `auth_bootstrap`/`auth_handle_callback` and as the payload of the
/// `auth-state-changed` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AuthStatusDto {
  Authenticated {
    username: Option<String>,
    email: Option<String>,
  },
  LoginRequired,
  Error {
    message: String,
  },
}

/// The subset of Authentik's `.well-known/openid-configuration` response this
/// module needs.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcDiscoveryDocument {
  pub authorization_endpoint: String,
  pub token_endpoint: String,
}

/// Only the fields this module actually uses: the long-lived `refresh_token`
/// (what `bootstrap()` always re-redeems — never a locally cached access
/// token, so that deactivating or revoking a user in Authentik takes effect
/// on the very next launch rather than whenever a cached token's lifetime ran
/// out) and `id_token` (read for display claims only, see
/// `oidc_client::decode_id_token_claims`). `access_token` and `expires_in` are
/// part of every OAuth2 token response but are deliberately not captured
/// here, since nothing in this module ever reads them back.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
  #[serde(default)]
  pub refresh_token: Option<String>,
  #[serde(default)]
  pub id_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenErrorResponse {
  pub error: String,
  #[serde(default)]
  pub error_description: Option<String>,
}

/// The claims this module reads out of a decoded (not signature-verified —
/// see the module doc comment in `oidc_client.rs`) `id_token` payload, purely
/// for display.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdTokenClaims {
  #[serde(default)]
  pub preferred_username: Option<String>,
  #[serde(default)]
  pub email: Option<String>,
}
