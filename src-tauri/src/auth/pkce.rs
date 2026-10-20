use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngExt;
use sha2::{Digest, Sha256};

/// RFC 7636 PKCE pair. `verifier` is kept in memory only for the lifetime of
/// one pending login; `challenge` (its S256 hash) is sent to the authorize
/// endpoint.
pub struct PkcePair {
  pub verifier: String,
  pub challenge: String,
}

/// Generates a `code_verifier` (32 random bytes, base64url-no-pad -> 43
/// chars, all within RFC 7636's unreserved charset) and its S256
/// `code_challenge`.
pub fn generate_pair() -> PkcePair {
  let verifier_bytes: [u8; 32] = rand::rng().random();
  let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

  let mut hasher = Sha256::new();
  hasher.update(verifier.as_bytes());
  let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

  PkcePair { verifier, challenge }
}

/// A random CSRF `state` value for one authorize round-trip.
pub fn generate_state() -> String {
  let bytes: [u8; 16] = rand::rng().random();
  URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn verifier_is_rfc7636_length() {
    let pair = generate_pair();
    assert!(pair.verifier.len() >= 43 && pair.verifier.len() <= 128);
    assert!(pair
      .verifier
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
  }

  #[test]
  fn challenge_matches_verifier() {
    let pair = generate_pair();
    let mut hasher = Sha256::new();
    hasher.update(pair.verifier.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
    assert_eq!(pair.challenge, expected);
  }

  #[test]
  fn pairs_are_unique() {
    let a = generate_pair();
    let b = generate_pair();
    assert_ne!(a.verifier, b.verifier);
    assert_ne!(a.challenge, b.challenge);
    assert_ne!(generate_state(), generate_state());
  }
}
