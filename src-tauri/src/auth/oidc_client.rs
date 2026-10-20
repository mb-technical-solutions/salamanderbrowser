//! Talks to the self-hosted Authentik instance over OAuth2 Authorization Code
//! with PKCE. All network calls happen here, in Rust, over `reqwest` — not
//! from the frontend — so no HTTP capability needs to be exposed to the
//! webview.

use super::types::{IdTokenClaims, OidcDiscoveryDocument, TokenErrorResponse, TokenResponse};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use tokio::sync::OnceCell;

/// Authentik issuer for the "salamander" application. Not a secret: visible
/// to anyone who inspects network traffic or the Authentik admin UI.
const ISSUER: &str = "https://auth.clickops.pro/application/o/salamander/";
/// Public (no client secret) OAuth2 client registered in Authentik for this
/// app. A public client has no secret to protect — PKCE (RFC 7636) is what
/// proves this app, not an attacker, is the one redeeming the authorization
/// code, which is the standard native-app pattern (RFC 8252) since a secret
/// embedded in a distributable binary can't actually stay secret.
pub const CLIENT_ID: &str = "KQwl3cqokgiIlLxjVCteu4aSghwbHImzuKns5FaO";
/// Must exactly match the Redirect URI configured on the Authentik provider.
pub const REDIRECT_URI: &str = "salamander-auth://callback";
/// `offline_access` is what makes Authentik return a `refresh_token`, which is
/// what lets every launch after the first re-check silently instead of
/// reopening a browser.
pub const SCOPES: &str = "openid profile email offline_access";

/// Distinguishes *why* a token-endpoint call failed, since the auth gate
/// needs to react differently: a rejected (revoked/expired) grant means
/// "sign in again", while a network failure means "retry", not "sign in
/// again" — the latter would just fail the same way.
#[derive(Debug)]
pub enum OidcError {
  InvalidGrant(String),
  Network(String),
  Other(String),
}

impl std::fmt::Display for OidcError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      OidcError::InvalidGrant(m) | OidcError::Network(m) | OidcError::Other(m) => {
        write!(f, "{m}")
      }
    }
  }
}

static DISCOVERY: OnceCell<OidcDiscoveryDocument> = OnceCell::const_new();

fn http_client() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(15))
    .build()
    .expect("reqwest client builder should not fail with no custom TLS config")
}

async fn discovery() -> Result<&'static OidcDiscoveryDocument, OidcError> {
  DISCOVERY
    .get_or_try_init(|| async {
      let url = format!("{ISSUER}.well-known/openid-configuration");
      log::info!("[auth] fetching Authentik discovery document from {url}");
      let response = http_client().get(&url).send().await.map_err(|e| {
        log::error!("[auth] discovery request failed: {e}");
        OidcError::Network(format!("Failed to reach Authentik discovery endpoint: {e}"))
      })?;
      if !response.status().is_success() {
        log::error!(
          "[auth] discovery endpoint returned HTTP {}",
          response.status()
        );
        return Err(OidcError::Other(format!(
          "Authentik discovery endpoint returned HTTP {}",
          response.status()
        )));
      }
      let doc = response.json::<OidcDiscoveryDocument>().await.map_err(|e| {
        log::error!("[auth] failed to parse discovery document: {e}");
        OidcError::Other(format!("Failed to parse Authentik discovery document: {e}"))
      })?;
      log::info!(
        "[auth] discovery ok: authorization_endpoint={}, token_endpoint={}",
        doc.authorization_endpoint,
        doc.token_endpoint
      );
      Ok(doc)
    })
    .await
}

/// Builds the browser-facing authorize URL for one login attempt.
pub async fn authorize_url(challenge: &str, state: &str) -> Result<String, OidcError> {
  let doc = discovery().await?;
  let url = url::Url::parse_with_params(
    &doc.authorization_endpoint,
    &[
      ("response_type", "code"),
      ("client_id", CLIENT_ID),
      ("redirect_uri", REDIRECT_URI),
      ("scope", SCOPES),
      ("state", state),
      ("code_challenge", challenge),
      ("code_challenge_method", "S256"),
    ],
  )
  .map_err(|e| OidcError::Other(format!("Failed to build authorize URL: {e}")))?;
  Ok(url.to_string())
}

async fn post_token_request(params: &[(&str, &str)]) -> Result<TokenResponse, OidcError> {
  let doc = discovery().await?;
  let response = http_client()
    .post(&doc.token_endpoint)
    .form(params)
    .send()
    .await
    .map_err(|e| OidcError::Network(format!("Failed to reach Authentik token endpoint: {e}")))?;

  let status = response.status();
  let body = response
    .text()
    .await
    .map_err(|e| OidcError::Network(format!("Failed to read token endpoint response: {e}")))?;

  if !status.is_success() {
    if let Ok(err) = serde_json::from_str::<TokenErrorResponse>(&body) {
      let message = err.error_description.unwrap_or_else(|| err.error.clone());
      return Err(if err.error == "invalid_grant" {
        OidcError::InvalidGrant(message)
      } else {
        OidcError::Other(message)
      });
    }
    return Err(OidcError::Other(format!(
      "Token endpoint returned HTTP {status}"
    )));
  }

  serde_json::from_str::<TokenResponse>(&body)
    .map_err(|e| OidcError::Other(format!("Failed to parse token endpoint response: {e}")))
}

/// Authorization Code grant: trades a freshly-issued `code` (plus the PKCE
/// `verifier` that proves this process started the login) for tokens.
pub async fn exchange_code(code: &str, verifier: &str) -> Result<TokenResponse, OidcError> {
  post_token_request(&[
    ("grant_type", "authorization_code"),
    ("client_id", CLIENT_ID),
    ("redirect_uri", REDIRECT_URI),
    ("code", code),
    ("code_verifier", verifier),
  ])
  .await
}

/// Refresh Token grant: the silent re-check run on every ordinary launch.
pub async fn refresh(refresh_token: &str) -> Result<TokenResponse, OidcError> {
  post_token_request(&[
    ("grant_type", "refresh_token"),
    ("client_id", CLIENT_ID),
    ("refresh_token", refresh_token),
  ])
  .await
}

/// Reads display claims out of an `id_token` payload without verifying its
/// signature. Safe here because the token arrived in a direct, TLS-protected
/// response from `token_endpoint` (an Authorization Code exchange, not a
/// front-channel/implicit redirect) — the channel itself already authenticates
/// the issuer. Only used to show a username/email in the UI, never to make an
/// access-control decision.
pub fn decode_id_token_claims(id_token: &str) -> Option<IdTokenClaims> {
  let payload = id_token.split('.').nth(1)?;
  let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
  serde_json::from_slice(&decoded).ok()
}
