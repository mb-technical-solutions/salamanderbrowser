//! Gates app access behind the user's self-hosted Authentik instance via
//! OAuth2 Authorization Code + PKCE. See `manager::AuthManager` for the
//! actual state machine; everything else here is a supporting piece
//! (PKCE generation, the Authentik HTTP client, persistence).

mod manager;
mod oidc_client;
mod pkce;
mod storage;
mod types;

pub use manager::{auth_bootstrap, auth_logout, auth_start_login, AuthManager};
pub use types::AuthStatusDto;
