use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{VlessConfig, VlessSecurity, VlessTransport, XrayError, XrayResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XrayClientRuntime {
  pub listen_port: u16,
  pub username: String,
  pub password: String,
}

impl XrayClientRuntime {
  pub fn validate(&self) -> XrayResult<()> {
    if self.listen_port == 0 {
      return Err(XrayError::InvalidField {
        field: "listen_port",
        reason: "must be between 1 and 65535",
      });
    }
    validate_socks_credential("username", &self.username)?;
    validate_socks_credential("password", &self.password)
  }
}

fn security_settings(security: &VlessSecurity) -> Value {
  match security {
    VlessSecurity::None => Value::Null,
    VlessSecurity::Tls(settings) => {
      let mut value = json!({
        "serverName": settings.server_name,
        "fingerprint": settings.fingerprint.as_str(),
      });
      if !settings.alpn.is_empty() {
        value["alpn"] = json!(settings.alpn);
      }
      value
    }
    VlessSecurity::Reality(settings) => json!({
      "show": false,
      "fingerprint": settings.fingerprint.as_str(),
      "serverName": settings.server_name,
      "publicKey": settings.public_key,
      "shortId": settings.short_id,
      "spiderX": settings.spider_x
    }),
  }
}

fn transport_settings(transport: &VlessTransport) -> Option<(&'static str, Value)> {
  match transport {
    VlessTransport::Tcp => None,
    VlessTransport::Ws(settings) => {
      let mut headers = serde_json::Map::new();
      if let Some(host) = &settings.host {
        headers.insert("Host".to_string(), json!(host));
      }
      Some((
        "wsSettings",
        json!({
          "path": settings.path,
          "headers": headers,
        }),
      ))
    }
    VlessTransport::Grpc(settings) => Some((
      "grpcSettings",
      json!({
        "serviceName": settings.service_name,
        "multiMode": false,
      }),
    )),
    VlessTransport::H2(settings) => Some((
      "httpSettings",
      json!({
        "path": settings.path,
        "host": [settings.host],
      }),
    )),
  }
}

pub fn build_client_config(config: &VlessConfig, runtime: &XrayClientRuntime) -> XrayResult<Value> {
  config.validate()?;
  runtime.validate()?;

  let mut stream_settings = json!({
    "network": config.transport.as_wire_str(),
    "security": config.security.as_wire_str(),
    "sockopt": {
      "tcpKeepAliveIdle": 30,
      "tcpKeepAliveInterval": 15
    }
  });

  let security_key = match &config.security {
    VlessSecurity::None => None,
    VlessSecurity::Tls(_) => Some("tlsSettings"),
    VlessSecurity::Reality(_) => Some("realitySettings"),
  };
  if let Some(key) = security_key {
    stream_settings[key] = security_settings(&config.security);
  }
  if let Some((key, value)) = transport_settings(&config.transport) {
    stream_settings[key] = value;
  }

  Ok(json!({
    "log": {
      "loglevel": "warning"
    },
    "inbounds": [{
      "tag": "local-socks",
      "listen": "127.0.0.1",
      "port": runtime.listen_port,
      "protocol": "socks",
      "settings": {
        "auth": "password",
        "accounts": [{
          "user": runtime.username,
          "pass": runtime.password
        }],
        "udp": true,
        "ip": "127.0.0.1"
      }
    }],
    "outbounds": [{
      "tag": "proxy",
      "protocol": "vless",
      "settings": {
        "vnext": [{
          "address": config.address,
          "port": config.port,
          "users": [{
            "id": config.id,
            "encryption": "none",
            "flow": config.flow.as_str()
          }]
        }]
      },
      "streamSettings": stream_settings
    }]
  }))
}

pub fn build_client_config_json(
  config: &VlessConfig,
  runtime: &XrayClientRuntime,
) -> XrayResult<String> {
  serde_json::to_string_pretty(&build_client_config(config, runtime)?)
    .map_err(|_| XrayError::Serialization)
}

fn validate_socks_credential(field: &'static str, value: &str) -> XrayResult<()> {
  if value.is_empty() || value.len() > 255 {
    return Err(XrayError::InvalidField {
      field,
      reason: "must contain between 1 and 255 URL-safe ASCII characters",
    });
  }
  if !value
    .bytes()
    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
  {
    return Err(XrayError::InvalidField {
      field,
      reason: "must contain only URL-safe ASCII characters",
    });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

  use super::super::{
    GrpcTransportSettings, H2TransportSettings, RealityFingerprint, RealitySettings, TlsSettings,
    VlessFlow, WsTransportSettings,
  };
  use super::*;

  fn valid_config() -> VlessConfig {
    VlessConfig {
      address: "vpn.example.com".to_string(),
      port: 443,
      id: "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4".to_string(),
      flow: VlessFlow::Vision,
      security: VlessSecurity::Reality(RealitySettings {
        server_name: "www.example.com".to_string(),
        public_key: URL_SAFE_NO_PAD.encode([7_u8; 32]),
        short_id: "0123456789abcdef".to_string(),
        fingerprint: RealityFingerprint::Chrome,
        spider_x: "/".to_string(),
      }),
      transport: VlessTransport::Tcp,
    }
  }

  fn runtime() -> XrayClientRuntime {
    XrayClientRuntime {
      listen_port: 41_321,
      username: "local_user-1".to_string(),
      password: "local_password-1".to_string(),
    }
  }

  #[test]
  fn generates_minimal_authenticated_tcp_reality_config() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();

    assert_eq!(value["log"]["loglevel"], "warning");
    assert_eq!(value["inbounds"].as_array().unwrap().len(), 1);
    assert_eq!(value["inbounds"][0]["listen"], "127.0.0.1");
    assert_eq!(value["inbounds"][0]["port"], 41_321);
    assert_eq!(value["inbounds"][0]["protocol"], "socks");
    assert_eq!(value["inbounds"][0]["settings"]["auth"], "password");
    assert_eq!(
      value["inbounds"][0]["settings"]["accounts"][0]["user"],
      "local_user-1"
    );
    assert_eq!(
      value["inbounds"][0]["settings"]["accounts"][0]["pass"],
      "local_password-1"
    );
    assert_eq!(value["inbounds"][0]["settings"]["udp"], true);
    assert_eq!(value["inbounds"][0]["settings"]["ip"], "127.0.0.1");

    assert_eq!(value["outbounds"].as_array().unwrap().len(), 1);
    assert_eq!(value["outbounds"][0]["protocol"], "vless");
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["address"],
      "vpn.example.com"
    );
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["users"][0]["encryption"],
      "none"
    );
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"],
      "xtls-rprx-vision"
    );
    assert_eq!(value["outbounds"][0]["streamSettings"]["network"], "tcp");
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["security"],
      "reality"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["fingerprint"],
      "chrome"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["serverName"],
      "www.example.com"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["shortId"],
      "0123456789abcdef"
    );
    assert!(value["outbounds"][0]["streamSettings"]
      .get("wsSettings")
      .is_none());
  }

  #[test]
  fn does_not_add_bypass_or_observability_surfaces() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();
    for absent in ["api", "dns", "policy", "routing", "stats"] {
      assert!(value.get(absent).is_none(), "{absent}");
    }
    assert!(value["outbounds"][0].get("mux").is_none());
    assert!(!value.to_string().contains("freedom"));
    assert!(!value.to_string().contains("blackhole"));
  }

  #[test]
  fn json_output_round_trips_without_shape_changes() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();
    let json = build_client_config_json(&valid_config(), &runtime()).unwrap();
    let reparsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(reparsed, value);
  }

  #[test]
  fn runtime_requires_nonzero_port_and_url_safe_credentials() {
    let cases = [
      XrayClientRuntime {
        listen_port: 0,
        ..runtime()
      },
      XrayClientRuntime {
        username: String::new(),
        ..runtime()
      },
      XrayClientRuntime {
        password: "contains:@".to_string(),
        ..runtime()
      },
      XrayClientRuntime {
        username: "a".repeat(256),
        ..runtime()
      },
    ];
    for runtime in cases {
      assert!(runtime.validate().is_err());
    }
  }

  #[test]
  fn invalid_model_is_rejected_before_generation() {
    let mut config = valid_config();
    config.security = VlessSecurity::Reality(RealitySettings {
      server_name: "www.example.com".to_string(),
      public_key: "secret-invalid-key".to_string(),
      short_id: String::new(),
      fingerprint: RealityFingerprint::Chrome,
      spider_x: "/".to_string(),
    });
    let error = build_client_config(&config, &runtime()).unwrap_err();
    assert!(matches!(
      error,
      XrayError::InvalidField { field: "pbk", .. }
    ));
    assert!(!error.to_string().contains("secret-invalid-key"));
  }

  #[test]
  fn ws_transport_over_plain_tls_sets_path_and_host_header() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.security = VlessSecurity::Tls(TlsSettings {
      server_name: "cdn.example.com".to_string(),
      fingerprint: RealityFingerprint::Chrome,
      alpn: vec!["h2".to_string(), "http/1.1".to_string()],
    });
    config.transport = VlessTransport::Ws(WsTransportSettings {
      path: "/ray".to_string(),
      host: Some("cdn.example.com".to_string()),
    });

    let value = build_client_config(&config, &runtime()).unwrap();
    let stream = &value["outbounds"][0]["streamSettings"];
    assert_eq!(stream["network"], "ws");
    assert_eq!(stream["security"], "tls");
    assert_eq!(stream["tlsSettings"]["serverName"], "cdn.example.com");
    assert_eq!(stream["tlsSettings"]["alpn"][0], "h2");
    assert_eq!(stream["wsSettings"]["path"], "/ray");
    assert_eq!(stream["wsSettings"]["headers"]["Host"], "cdn.example.com");
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"],
      ""
    );
  }

  #[test]
  fn grpc_transport_sets_service_name() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.transport = VlessTransport::Grpc(GrpcTransportSettings {
      service_name: "gun".to_string(),
    });

    let value = build_client_config(&config, &runtime()).unwrap();
    let stream = &value["outbounds"][0]["streamSettings"];
    assert_eq!(stream["network"], "grpc");
    assert_eq!(stream["grpcSettings"]["serviceName"], "gun");
  }

  #[test]
  fn h2_transport_sets_path_and_host_array() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.transport = VlessTransport::H2(H2TransportSettings {
      path: "/ray".to_string(),
      host: "cdn.example.com".to_string(),
    });

    let value = build_client_config(&config, &runtime()).unwrap();
    let stream = &value["outbounds"][0]["streamSettings"];
    assert_eq!(stream["network"], "http");
    assert_eq!(stream["httpSettings"]["path"], "/ray");
    assert_eq!(stream["httpSettings"]["host"][0], "cdn.example.com");
  }

  #[test]
  fn none_security_omits_a_security_settings_block() {
    let mut config = valid_config();
    config.flow = VlessFlow::None;
    config.security = VlessSecurity::None;

    let value = build_client_config(&config, &runtime()).unwrap();
    let stream = &value["outbounds"][0]["streamSettings"];
    assert_eq!(stream["security"], "none");
    assert!(stream.get("tlsSettings").is_none());
    assert!(stream.get("realitySettings").is_none());
  }

  #[test]
  fn vision_flow_is_rejected_over_ws() {
    let mut config = valid_config();
    config.transport = VlessTransport::Ws(WsTransportSettings {
      path: "/ray".to_string(),
      host: None,
    });
    assert!(matches!(
      build_client_config(&config, &runtime()),
      Err(XrayError::InvalidField { field: "flow", .. })
    ));
  }
}
