use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

use super::{parse_vless_uri, ParsedVlessUri, XrayError, XrayResult};

/// Subscription responses are typically a few KB to a few hundred KB (a list
/// of server lines). Capping well above that stops a misbehaving or
/// malicious endpoint from streaming an unbounded body into memory.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 15;

/// One server line the subscription carried, and what came of parsing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEntry {
  /// The display name to offer the user, preferring the `#fragment` the
  /// server line carried, falling back to a stable ordinal.
  pub name: String,
  /// The exact `vless://` line as the subscription sent it, so importing it
  /// is a byte-for-byte match against what the operator's panel shows.
  pub uri: String,
}

/// A line the subscription carried that this build could not use, and why —
/// surfaced so a user with a mixed subscription (some VLESS, some other
/// protocols, some shapes we don't support) can see what was skipped instead
/// of silently losing servers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionSkip {
  /// A truncated, non-secret preview of the line (scheme + host only) —
  /// never the full URI, which may carry a UUID or a REALITY public key.
  pub preview: String,
  pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionImportResult {
  pub entries: Vec<SubscriptionEntry>,
  pub skipped: Vec<SubscriptionSkip>,
}

/// Fetch a subscription URL and parse every VLESS line it carries.
///
/// Subscription bodies come in two shapes in the wild: the whole body
/// base64-encoded (the common case, originating from V2Ray/Xray panels), or
/// a plain newline-separated list of URIs (some self-hosted generators skip
/// the encoding step). Both are accepted; base64 is tried first and a
/// failure falls back to treating the body as already-plain text, since a
/// real subscription body is never valid base64 AND a valid line list at
/// once.
pub async fn fetch_vless_subscription(url: &str) -> XrayResult<SubscriptionImportResult> {
  let parsed_url = url::Url::parse(url).map_err(|_| XrayError::InvalidField {
    field: "url",
    reason: "must be a valid http or https URL",
  })?;
  if !matches!(parsed_url.scheme(), "http" | "https") {
    return Err(XrayError::InvalidField {
      field: "url",
      reason: "must be a valid http or https URL",
    });
  }

  let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
    .build()
    .map_err(|_| XrayError::SubscriptionFetchFailed)?;

  let response = client
    .get(parsed_url)
    .send()
    .await
    .map_err(|_| XrayError::SubscriptionFetchFailed)?;
  if !response.status().is_success() {
    return Err(XrayError::SubscriptionFetchFailed);
  }
  let body = response
    .text()
    .await
    .map_err(|_| XrayError::SubscriptionFetchFailed)?;
  if body.len() > MAX_BODY_BYTES {
    return Err(XrayError::SubscriptionTooLarge);
  }

  Ok(parse_subscription_body(&body))
}

fn parse_subscription_body(body: &str) -> SubscriptionImportResult {
  let trimmed = body.trim();
  let decoded = STANDARD
    .decode(trimmed)
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
    .unwrap_or_else(|| trimmed.to_string());

  let mut entries = Vec::new();
  let mut skipped = Vec::new();
  let mut ordinal = 0usize;

  for line in decoded.lines() {
    let line = line.trim();
    if line.is_empty() {
      continue;
    }
    if !line.starts_with("vless://") {
      // Other subscription protocols (vmess, trojan, shadowsocks, ...) are
      // out of scope here — skip quietly rather than reporting every one as
      // a parse failure, since a mixed subscription is the common case.
      continue;
    }
    ordinal += 1;
    match parse_vless_uri(line) {
      Ok(ParsedVlessUri { name, config: _ }) => {
        entries.push(SubscriptionEntry {
          name: name.unwrap_or_else(|| format!("Server {ordinal}")),
          uri: line.to_string(),
        });
      }
      Err(error) => {
        skipped.push(SubscriptionSkip {
          preview: line_preview(line),
          reason: error.reason_code().to_string(),
        });
      }
    }
  }

  SubscriptionImportResult { entries, skipped }
}

/// `vless://<uuid>@host:port?...#name` -> `vless://<host>:<port>`. Enough to
/// place a skipped line in a list without echoing the id, any REALITY
/// secret, or any other query parameter.
fn line_preview(line: &str) -> String {
  url::Url::parse(line)
    .ok()
    .map(|url| {
      let host = url.host_str().unwrap_or("?");
      match url.port() {
        Some(port) => format!("vless://{host}:{port}"),
        None => format!("vless://{host}"),
      }
    })
    .unwrap_or_else(|| "vless://<unparsable>".to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use base64::engine::general_purpose::URL_SAFE_NO_PAD;
  use wiremock::matchers::method;
  use wiremock::{Mock, MockServer, ResponseTemplate};

  const ID: &str = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";

  fn public_key() -> String {
    URL_SAFE_NO_PAD.encode([7_u8; 32])
  }

  fn reality_line(fragment: &str) -> String {
    let key = public_key();
    format!(
      "vless://{ID}@vpn.example.com:443?security=reality&flow=xtls-rprx-vision&sni=a.com&pbk={key}&sid=00#{fragment}"
    )
  }

  fn ws_line(fragment: &str) -> String {
    format!(
      "vless://{ID}@cdn.example.com:443?security=tls&type=ws&path=%2Fray&sni=cdn.example.com#{fragment}"
    )
  }

  #[test]
  fn parses_a_plain_newline_separated_body() {
    let body = format!(
      "{}\n{}\n# a comment line, not a URI\n\nvmess://unsupported-protocol\n",
      reality_line("Primary"),
      ws_line("WS%20node")
    );
    let result = parse_subscription_body(&body);
    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.entries[0].name, "Primary");
    assert_eq!(result.entries[1].name, "WS node");
    assert!(result.skipped.is_empty());
  }

  #[test]
  fn parses_a_base64_encoded_body() {
    let plain = format!("{}\n{}", reality_line("A"), reality_line("B"));
    let body = STANDARD.encode(&plain);
    let result = parse_subscription_body(&body);
    assert_eq!(result.entries.len(), 2);
  }

  #[test]
  fn unsupported_or_malformed_vless_lines_are_reported_without_leaking_secrets() {
    let key = public_key();
    let bad = format!("vless://{ID}@vpn.example.com:443?security=made-up&sni=a.com&pbk={key}");
    let body = format!("{}\n{bad}", reality_line("Good"));
    let result = parse_subscription_body(&body);
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].preview, "vless://vpn.example.com:443");
    assert_eq!(result.skipped[0].reason, "security");
    assert!(!result.skipped[0].preview.contains(ID));
    assert!(!result.skipped[0].preview.contains(&key));
  }

  #[test]
  fn falls_back_to_a_stable_ordinal_when_no_name_is_present() {
    let key = public_key();
    let bare = format!(
      "vless://{ID}@vpn.example.com:443?security=reality&flow=xtls-rprx-vision&sni=a.com&pbk={key}&sid=00"
    );
    let result = parse_subscription_body(&bare);
    assert_eq!(result.entries[0].name, "Server 1");
  }

  #[tokio::test]
  async fn fetches_and_parses_a_remote_subscription() {
    let mock_server = MockServer::start().await;
    let body = reality_line("Remote");
    Mock::given(method("GET"))
      .respond_with(ResponseTemplate::new(200).set_body_string(&body))
      .mount(&mock_server)
      .await;

    let result = fetch_vless_subscription(&mock_server.uri()).await.unwrap();
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].name, "Remote");
  }

  #[tokio::test]
  async fn a_non_success_status_is_reported_as_a_fetch_failure() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
      .respond_with(ResponseTemplate::new(404))
      .mount(&mock_server)
      .await;

    let error = fetch_vless_subscription(&mock_server.uri())
      .await
      .unwrap_err();
    assert_eq!(error, XrayError::SubscriptionFetchFailed);
  }

  #[tokio::test]
  async fn non_http_urls_are_rejected_before_any_network_call() {
    let error = fetch_vless_subscription("ftp://example.com/list")
      .await
      .unwrap_err();
    assert!(matches!(
      error,
      XrayError::InvalidField { field: "url", .. }
    ));
  }
}
