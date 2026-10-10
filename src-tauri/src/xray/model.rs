use std::net::IpAddr;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use url::Host;
use uuid::Uuid;

use super::{XrayError, XrayResult};

const MAX_SPIDER_X_BYTES: usize = 2048;
const MAX_PATH_BYTES: usize = 2048;
const MAX_ALPN_ENTRIES: usize = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VlessFlow {
  #[default]
  #[serde(rename = "xtls-rprx-vision")]
  Vision,
  /// No flow control. The only flow value valid outside plain TCP+TLS/REALITY
  /// — WebSocket, gRPC, HTTP/2, and QUIC all terminate TLS themselves, so
  /// XTLS Vision (which needs to see the raw TLS record layer) cannot apply.
  #[serde(rename = "")]
  None,
}

impl VlessFlow {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Vision => "xtls-rprx-vision",
      Self::None => "",
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RealityFingerprint {
  #[default]
  Chrome,
  Firefox,
  Safari,
  Edge,
  Ios,
  Android,
}

impl RealityFingerprint {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Chrome => "chrome",
      Self::Firefox => "firefox",
      Self::Safari => "safari",
      Self::Edge => "edge",
      Self::Ios => "ios",
      Self::Android => "android",
    }
  }

  pub(crate) fn parse(value: &str) -> XrayResult<Self> {
    match value {
      "chrome" => Ok(Self::Chrome),
      "firefox" => Ok(Self::Firefox),
      "safari" => Ok(Self::Safari),
      "edge" => Ok(Self::Edge),
      "ios" => Ok(Self::Ios),
      "android" => Ok(Self::Android),
      _ => Err(XrayError::UnsupportedValue {
        field: "fp",
        expected: "chrome, firefox, safari, edge, ios, or android",
      }),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealitySettings {
  pub server_name: String,
  pub public_key: String,
  #[serde(default)]
  pub short_id: String,
  #[serde(default)]
  pub fingerprint: RealityFingerprint,
  #[serde(default = "default_spider_x")]
  pub spider_x: String,
}

impl RealitySettings {
  pub fn validate(&self) -> XrayResult<()> {
    validate_server_name(&self.server_name)?;
    validate_public_key(&self.public_key)?;
    validate_short_id(&self.short_id)?;
    validate_spider_x(&self.spider_x)
  }
}

/// Plain TLS (no REALITY). The common shape behind a CDN or a certificate the
/// server terminates itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSettings {
  pub server_name: String,
  #[serde(default)]
  pub fingerprint: RealityFingerprint,
  /// ALPN protocol IDs, offered in order (e.g. `["h2", "http/1.1"]`). Empty
  /// means let xray-core pick its own default.
  #[serde(default)]
  pub alpn: Vec<String>,
}

impl TlsSettings {
  pub fn validate(&self) -> XrayResult<()> {
    validate_server_name(&self.server_name)?;
    validate_alpn(&self.alpn)
  }
}

/// The transport-layer security a VLESS outbound negotiates. `None` (plain
/// TCP, no TLS at all) only makes sense chained behind something else that
/// already provides encryption, but real deployments do run it — usually
/// over an already-encrypted tunnel — so it is supported rather than
/// rejected outright.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VlessSecurity {
  None,
  Tls(TlsSettings),
  Reality(RealitySettings),
}

impl VlessSecurity {
  pub fn validate(&self) -> XrayResult<()> {
    match self {
      Self::None => Ok(()),
      Self::Tls(settings) => settings.validate(),
      Self::Reality(settings) => settings.validate(),
    }
  }

  pub(crate) const fn as_wire_str(&self) -> &'static str {
    match self {
      Self::None => "none",
      Self::Tls(_) => "tls",
      Self::Reality(_) => "reality",
    }
  }
}

/// WebSocket transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsTransportSettings {
  pub path: String,
  /// The `Host` header sent to the server — usually the CDN-facing domain,
  /// separate from `sni` when the two differ.
  #[serde(default)]
  pub host: Option<String>,
}

impl WsTransportSettings {
  fn validate(&self) -> XrayResult<()> {
    validate_transport_path(&self.path)?;
    if let Some(host) = &self.host {
      validate_transport_host(host)?;
    }
    Ok(())
  }
}

/// gRPC transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrpcTransportSettings {
  pub service_name: String,
}

impl GrpcTransportSettings {
  fn validate(&self) -> XrayResult<()> {
    if self.service_name.is_empty()
      || self.service_name.chars().count() > 200
      || self.service_name.chars().any(char::is_control)
    {
      return Err(XrayError::InvalidField {
        field: "serviceName",
        reason: "must be a non-empty string of at most 200 characters",
      });
    }
    Ok(())
  }
}

/// HTTP/2 transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct H2TransportSettings {
  pub path: String,
  pub host: String,
}

impl H2TransportSettings {
  fn validate(&self) -> XrayResult<()> {
    validate_transport_path(&self.path)?;
    validate_transport_host(&self.host)
  }
}

/// The wire transport a VLESS outbound rides on. `Tcp` is the only one
/// compatible with `VlessFlow::Vision`; the others all terminate their own
/// framing (and, for QUIC, their own TLS) before VLESS ever sees a byte.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VlessTransport {
  #[default]
  Tcp,
  Ws(WsTransportSettings),
  Grpc(GrpcTransportSettings),
  H2(H2TransportSettings),
}

impl VlessTransport {
  pub fn validate(&self) -> XrayResult<()> {
    match self {
      Self::Tcp => Ok(()),
      Self::Ws(settings) => settings.validate(),
      Self::Grpc(settings) => settings.validate(),
      Self::H2(settings) => settings.validate(),
    }
  }

  pub(crate) const fn as_wire_str(&self) -> &'static str {
    match self {
      Self::Tcp => "tcp",
      Self::Ws(_) => "ws",
      Self::Grpc(_) => "grpc",
      Self::H2(_) => "http",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VlessConfig {
  pub address: String,
  pub port: u16,
  pub id: String,
  #[serde(default)]
  pub flow: VlessFlow,
  pub security: VlessSecurity,
  #[serde(default)]
  pub transport: VlessTransport,
}

impl VlessConfig {
  pub fn validate(&self) -> XrayResult<()> {
    validate_endpoint_address(&self.address)?;
    if self.port == 0 {
      return Err(XrayError::InvalidField {
        field: "port",
        reason: "must be between 1 and 65535",
      });
    }
    Uuid::parse_str(&self.id).map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })?;
    self.security.validate()?;
    self.transport.validate()?;

    // XTLS Vision needs to observe the raw TLS record layer to splice
    // upload/download directly; WS/gRPC/H2 all wrap VLESS in their own
    // framing first, so Vision can never reach it there. xray-core itself
    // refuses this combination at runtime — reject it up front instead of
    // producing a config that fails silently on first connect.
    if self.flow == VlessFlow::Vision && !matches!(self.transport, VlessTransport::Tcp) {
      return Err(XrayError::InvalidField {
        field: "flow",
        reason: "xtls-rprx-vision requires the tcp transport",
      });
    }
    Ok(())
  }
}

/// Backward-compatible alias for the pre-expansion REALITY-only shape. Kept
/// so call sites that only ever dealt with REALITY (the xray worker runner,
/// its storage layer) don't need to match on `VlessSecurity` themselves.
pub type VlessRealityConfig = VlessConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsedVlessUri {
  pub name: Option<String>,
  pub config: VlessConfig,
}

impl ParsedVlessUri {
  pub fn validate(&self) -> XrayResult<()> {
    if let Some(name) = &self.name {
      validate_display_name(name)?;
    }
    self.config.validate()
  }
}

pub(crate) fn default_spider_x() -> String {
  "/".to_string()
}

pub(crate) fn validate_display_name(name: &str) -> XrayResult<()> {
  if name.is_empty() {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not be empty",
    });
  }
  if name.chars().count() > 200 {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not exceed 200 characters",
    });
  }
  if name.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not contain control characters",
    });
  }
  Ok(())
}

fn validate_endpoint_address(address: &str) -> XrayResult<()> {
  if address.is_empty()
    || address.trim() != address
    || address.starts_with('[')
    || address.ends_with(']')
  {
    return Err(XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    });
  }
  if address.parse::<IpAddr>().is_ok() {
    return Ok(());
  }
  Host::parse(address).map_err(|_| XrayError::InvalidField {
    field: "address",
    reason: "must be a valid hostname or IP address",
  })?;
  Ok(())
}

fn validate_server_name(server_name: &str) -> XrayResult<()> {
  if server_name.is_empty() || server_name.trim() != server_name {
    return Err(XrayError::InvalidField {
      field: "sni",
      reason: "must be a valid DNS name",
    });
  }
  match Host::parse(server_name) {
    Ok(Host::Domain(_)) => Ok(()),
    _ => Err(XrayError::InvalidField {
      field: "sni",
      reason: "must be a valid DNS name",
    }),
  }
}

fn validate_public_key(public_key: &str) -> XrayResult<()> {
  let decoded = URL_SAFE_NO_PAD
    .decode(public_key)
    .map_err(|_| XrayError::InvalidField {
      field: "pbk",
      reason: "must be an unpadded base64url-encoded 32-byte key",
    })?;
  if decoded.len() != 32 || URL_SAFE_NO_PAD.encode(decoded) != public_key {
    return Err(XrayError::InvalidField {
      field: "pbk",
      reason: "must be an unpadded base64url-encoded 32-byte key",
    });
  }
  Ok(())
}

fn validate_short_id(short_id: &str) -> XrayResult<()> {
  if short_id.len() > 16 || !short_id.len().is_multiple_of(2) {
    return Err(XrayError::InvalidField {
      field: "sid",
      reason: "must be empty or contain up to 16 even-length hexadecimal characters",
    });
  }
  if !short_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return Err(XrayError::InvalidField {
      field: "sid",
      reason: "must be empty or contain up to 16 even-length hexadecimal characters",
    });
  }
  Ok(())
}

fn validate_spider_x(spider_x: &str) -> XrayResult<()> {
  if !spider_x.starts_with('/') {
    return Err(XrayError::InvalidField {
      field: "spx",
      reason: "must start with /",
    });
  }
  if spider_x.len() > MAX_SPIDER_X_BYTES || spider_x.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field: "spx",
      reason: "must be a valid relative path no longer than 2048 bytes",
    });
  }
  Ok(())
}

fn validate_transport_path(path: &str) -> XrayResult<()> {
  if !path.starts_with('/') {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "must start with /",
    });
  }
  if path.len() > MAX_PATH_BYTES || path.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "must be a valid relative path no longer than 2048 bytes",
    });
  }
  Ok(())
}

fn validate_transport_host(host: &str) -> XrayResult<()> {
  if host.is_empty() || host.trim() != host {
    return Err(XrayError::InvalidField {
      field: "host",
      reason: "must be a valid hostname",
    });
  }
  Host::parse(host).map_err(|_| XrayError::InvalidField {
    field: "host",
    reason: "must be a valid hostname",
  })?;
  Ok(())
}

fn validate_alpn(alpn: &[String]) -> XrayResult<()> {
  if alpn.len() > MAX_ALPN_ENTRIES {
    return Err(XrayError::InvalidField {
      field: "alpn",
      reason: "must contain at most 8 protocol identifiers",
    });
  }
  for id in alpn {
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_graphic()) {
      return Err(XrayError::InvalidField {
        field: "alpn",
        reason: "each protocol identifier must be 1-32 printable ASCII characters",
      });
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn public_key() -> String {
    URL_SAFE_NO_PAD.encode([7_u8; 32])
  }

  fn reality_settings() -> RealitySettings {
    RealitySettings {
      server_name: "www.example.com".to_string(),
      public_key: public_key(),
      short_id: "0123456789abcdef".to_string(),
      fingerprint: RealityFingerprint::Chrome,
      spider_x: "/".to_string(),
    }
  }

  fn valid_config() -> VlessConfig {
    VlessConfig {
      address: "vpn.example.com".to_string(),
      port: 443,
      id: "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4".to_string(),
      flow: VlessFlow::Vision,
      security: VlessSecurity::Reality(reality_settings()),
      transport: VlessTransport::Tcp,
    }
  }

  #[test]
  fn valid_model_passes_validation() {
    assert_eq!(valid_config().validate(), Ok(()));
  }

  #[test]
  fn endpoint_accepts_ipv4_ipv6_and_dns() {
    for address in ["198.51.100.4", "2001:db8::1", "vpn.example.com"] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert_eq!(config.validate(), Ok(()), "{address}");
    }
  }

  #[test]
  fn endpoint_rejects_empty_whitespace_and_invalid_hosts() {
    for address in [
      "",
      " vpn.example.com",
      "vpn example.com",
      "vpn.example.com:443",
      "[2001:db8::1]",
    ] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField {
          field: "address",
          ..
        })
      ));
    }
  }

  #[test]
  fn id_must_be_a_uuid() {
    let mut config = valid_config();
    config.id = "not-a-uuid".to_string();
    assert_eq!(
      config.validate(),
      Err(XrayError::InvalidField {
        field: "id",
        reason: "must be a UUID",
      })
    );
  }

  #[test]
  fn server_name_must_be_dns_name() {
    for server_name in ["", "203.0.113.5", "bad server"] {
      let mut config = valid_config();
      config.security = VlessSecurity::Reality(RealitySettings {
        server_name: server_name.to_string(),
        ..reality_settings()
      });
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sni", .. })
      ));
    }
  }

  #[test]
  fn public_key_must_be_canonical_base64url_and_32_bytes() {
    let short_key = URL_SAFE_NO_PAD.encode([1_u8; 31]);
    for public_key in [
      "not-base64!",
      short_key.as_str(),
      "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=",
    ] {
      let mut config = valid_config();
      config.security = VlessSecurity::Reality(RealitySettings {
        public_key: public_key.to_string(),
        ..reality_settings()
      });
      let error = config.validate().unwrap_err();
      assert!(matches!(
        error,
        XrayError::InvalidField { field: "pbk", .. }
      ));
      assert!(!error.to_string().contains(public_key));
    }
  }

  #[test]
  fn short_id_accepts_empty_or_even_hex_up_to_sixteen_chars() {
    for short_id in ["", "ab", "0123456789abcdef", "ABCDEF"] {
      let mut config = valid_config();
      config.security = VlessSecurity::Reality(RealitySettings {
        short_id: short_id.to_string(),
        ..reality_settings()
      });
      assert_eq!(config.validate(), Ok(()), "{short_id}");
    }
  }

  #[test]
  fn short_id_rejects_odd_non_hex_and_overlong_values() {
    for short_id in ["a", "xz", "0123456789abcdef00"] {
      let mut config = valid_config();
      config.security = VlessSecurity::Reality(RealitySettings {
        short_id: short_id.to_string(),
        ..reality_settings()
      });
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sid", .. })
      ));
    }
  }

  #[test]
  fn spider_x_must_be_safe_relative_path() {
    for spider_x in ["relative", "/line\nbreak"] {
      let mut config = valid_config();
      config.security = VlessSecurity::Reality(RealitySettings {
        spider_x: spider_x.to_string(),
        ..reality_settings()
      });
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "spx", .. })
      ));
    }
  }

  #[test]
  fn vision_flow_is_rejected_outside_tcp_transport() {
    let bad_transports = [
      VlessTransport::Ws(WsTransportSettings {
        path: "/ray".to_string(),
        host: None,
      }),
      VlessTransport::Grpc(GrpcTransportSettings {
        service_name: "gun".to_string(),
      }),
      VlessTransport::H2(H2TransportSettings {
        path: "/ray".to_string(),
        host: "cdn.example.com".to_string(),
      }),
    ];
    for transport in bad_transports {
      let mut config = valid_config();
      config.transport = transport.clone();
      assert!(
        matches!(
          config.validate(),
          Err(XrayError::InvalidField { field: "flow", .. })
        ),
        "{transport:?}"
      );
    }
  }

  #[test]
  fn none_flow_is_allowed_on_every_transport() {
    let transports = [
      VlessTransport::Tcp,
      VlessTransport::Ws(WsTransportSettings {
        path: "/ray".to_string(),
        host: None,
      }),
      VlessTransport::Grpc(GrpcTransportSettings {
        service_name: "gun".to_string(),
      }),
      VlessTransport::H2(H2TransportSettings {
        path: "/ray".to_string(),
        host: "cdn.example.com".to_string(),
      }),
    ];
    for transport in transports {
      let mut config = valid_config();
      config.flow = VlessFlow::None;
      config.transport = transport.clone();
      assert_eq!(config.validate(), Ok(()), "{transport:?}");
    }
  }

  #[test]
  fn none_security_and_plain_tls_pass_validation() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.security = VlessSecurity::None;
    assert_eq!(config.validate(), Ok(()));

    config.security = VlessSecurity::Tls(TlsSettings {
      server_name: "cdn.example.com".to_string(),
      fingerprint: RealityFingerprint::Chrome,
      alpn: vec!["h2".to_string(), "http/1.1".to_string()],
    });
    assert_eq!(config.validate(), Ok(()));
  }

  #[test]
  fn tls_server_name_and_alpn_are_validated() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.security = VlessSecurity::Tls(TlsSettings {
      server_name: "bad server".to_string(),
      fingerprint: RealityFingerprint::Chrome,
      alpn: vec![],
    });
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField { field: "sni", .. })
    ));

    config.security = VlessSecurity::Tls(TlsSettings {
      server_name: "cdn.example.com".to_string(),
      fingerprint: RealityFingerprint::Chrome,
      alpn: vec!["not printable\u{0}".to_string()],
    });
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField { field: "alpn", .. })
    ));
  }

  #[test]
  fn ws_grpc_and_h2_transport_fields_are_validated() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;

    config.transport = VlessTransport::Ws(WsTransportSettings {
      path: "no-leading-slash".to_string(),
      host: None,
    });
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField { field: "path", .. })
    ));

    config.transport = VlessTransport::Grpc(GrpcTransportSettings {
      service_name: String::new(),
    });
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField {
        field: "serviceName",
        ..
      })
    ));

    config.transport = VlessTransport::H2(H2TransportSettings {
      path: "/ray".to_string(),
      host: "bad host".to_string(),
    });
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField { field: "host", .. })
    ));
  }

  #[test]
  fn serde_defaults_preserve_the_supported_profile() {
    let value = serde_json::json!({
      "address": "vpn.example.com",
      "port": 443,
      "id": "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4",
      "security": {
        "type": "reality",
        "server_name": "www.example.com",
        "public_key": public_key()
      }
    });
    let config: VlessConfig = serde_json::from_value(value).unwrap();
    assert_eq!(config.flow, VlessFlow::Vision);
    assert_eq!(config.transport, VlessTransport::Tcp);
    match &config.security {
      VlessSecurity::Reality(settings) => {
        assert_eq!(settings.fingerprint, RealityFingerprint::Chrome);
        assert_eq!(settings.short_id, "");
        assert_eq!(settings.spider_x, "/");
      }
      other => panic!("expected reality security, got {other:?}"),
    }
  }

  #[test]
  fn serde_rejects_unknown_configuration_fields() {
    let value = serde_json::json!({
      "address": "vpn.example.com",
      "port": 443,
      "id": "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4",
      "made_up_field": "websocket",
      "security": {
        "type": "reality",
        "server_name": "www.example.com",
        "public_key": public_key()
      }
    });
    assert!(serde_json::from_value::<VlessConfig>(value).is_err());
  }
}
