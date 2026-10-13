use std::collections::HashMap;

use std::net::IpAddr;

use url::{Host, Url};
use uuid::Uuid;

use super::{
  model::validate_display_name, GrpcTransportSettings, H2TransportSettings, ParsedVlessUri,
  RealityFingerprint, RealitySettings, TlsSettings, VlessConfig, VlessFlow, VlessSecurity,
  VlessTransport, WsTransportSettings, XrayError, XrayResult,
};

const SUPPORTED_PARAMETERS: &[&str] = &[
  "encryption",
  "flow",
  "security",
  "sni",
  "fp",
  "pbk",
  "sid",
  "spx",
  "alpn",
  "type",
  "headerType",
  "path",
  "host",
  "serviceName",
];

pub fn parse_vless_uri(input: &str) -> XrayResult<ParsedVlessUri> {
  if input.trim() != input {
    return Err(XrayError::InvalidUri);
  }
  let url = Url::parse(input).map_err(|_| XrayError::InvalidUri)?;
  if url.scheme() != "vless" {
    return Err(XrayError::UnsupportedScheme);
  }
  if url.password().is_some() {
    return Err(XrayError::InvalidField {
      field: "id",
      reason: "password-style user information is not supported",
    });
  }
  if !matches!(url.path(), "" | "/") {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "the URI path must be empty; a transport path belongs in the ?path= parameter",
    });
  }

  let raw_id = url.username();
  if raw_id.is_empty() {
    return Err(XrayError::MissingField("id"));
  }
  let id = Uuid::parse_str(raw_id)
    .map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })?
    .to_string();
  let address = match url.host().ok_or(XrayError::MissingField("address"))? {
    Host::Domain(value) => value.to_string(),
    Host::Ipv4(value) => value.to_string(),
    Host::Ipv6(value) => value.to_string(),
  };
  let port = url.port().ok_or(XrayError::MissingField("port"))?;

  let (parameters, unsupported) = parse_parameters(&url)?;

  // Transport and security first: together they decide which of the
  // remaining parameters are even meaningful, and reporting an unsupported
  // shape ("QUIC", "mKCP") beats reporting a stray parameter it happens to
  // carry.
  let transport = parse_transport(&parameters)?;
  let security = parse_security(&parameters)?;
  optional_value(&parameters, "encryption", "none")?;
  optional_value(&parameters, "headerType", "none")?;

  let flow = parse_flow(&parameters)?;

  // Only once the shape is known-good does an unrecognized parameter become
  // the most useful thing to report.
  if let Some(name) = unsupported.into_iter().next() {
    return Err(XrayError::UnsupportedParameter(name));
  }

  let name = url
    .fragment()
    .filter(|fragment| !fragment.is_empty())
    .map(|fragment| {
      urlencoding::decode(fragment)
        .map(|value| value.into_owned())
        .map_err(|_| XrayError::InvalidField {
          field: "name",
          reason: "must use valid percent encoding",
        })
    })
    .transpose()?;

  let parsed = ParsedVlessUri {
    name,
    config: VlessConfig {
      address,
      port,
      id,
      flow,
      security,
      transport,
    },
  };
  parsed.validate()?;
  Ok(parsed)
}

fn parse_transport(parameters: &HashMap<String, String>) -> XrayResult<VlessTransport> {
  match parameters.get("type").map(String::as_str) {
    None | Some("tcp" | "raw") => Ok(VlessTransport::Tcp),
    Some("ws") => {
      let path = required_parameter(parameters, "path")?.to_string();
      let host = parameters.get("host").cloned();
      Ok(VlessTransport::Ws(WsTransportSettings { path, host }))
    }
    Some("grpc") => {
      let service_name = required_parameter(parameters, "serviceName")?.to_string();
      Ok(VlessTransport::Grpc(GrpcTransportSettings { service_name }))
    }
    Some("http" | "h2") => {
      let path = required_parameter(parameters, "path")?.to_string();
      let host = required_parameter(parameters, "host")?.to_string();
      Ok(VlessTransport::H2(H2TransportSettings { path, host }))
    }
    Some(_) => Err(XrayError::UnsupportedValue {
      field: "type",
      expected: "tcp, raw, ws, grpc, or http",
    }),
  }
}

fn parse_security(parameters: &HashMap<String, String>) -> XrayResult<VlessSecurity> {
  match parameters.get("security").map(String::as_str) {
    None | Some("" | "none") => Ok(VlessSecurity::None),
    Some("tls") => {
      let server_name = required_parameter(parameters, "sni")?.to_string();
      let fingerprint = parameters
        .get("fp")
        .map(|value| RealityFingerprint::parse(value))
        .transpose()?
        .unwrap_or_default();
      let alpn = parse_alpn(parameters)?;
      Ok(VlessSecurity::Tls(TlsSettings {
        server_name,
        fingerprint,
        alpn,
      }))
    }
    Some("reality") => {
      let server_name = required_parameter(parameters, "sni")?.to_string();
      let public_key = required_parameter(parameters, "pbk")?.to_string();
      let short_id = parameters.get("sid").cloned().unwrap_or_default();
      let spider_x = parameters
        .get("spx")
        .cloned()
        .unwrap_or_else(|| "/".to_string());
      let fingerprint = parameters
        .get("fp")
        .map(|value| RealityFingerprint::parse(value))
        .transpose()?
        .unwrap_or_default();
      Ok(VlessSecurity::Reality(RealitySettings {
        server_name,
        public_key,
        short_id,
        fingerprint,
        spider_x,
      }))
    }
    Some(_) => Err(XrayError::UnsupportedValue {
      field: "security",
      expected: "none, tls, or reality",
    }),
  }
}

fn parse_flow(parameters: &HashMap<String, String>) -> XrayResult<VlessFlow> {
  match parameters.get("flow").map(String::as_str) {
    None | Some("" | "none") => Ok(VlessFlow::None),
    Some(value) if value == VlessFlow::Vision.as_str() => Ok(VlessFlow::Vision),
    Some(_) => Err(XrayError::UnsupportedValue {
      field: "flow",
      expected: "xtls-rprx-vision or none",
    }),
  }
}

fn parse_alpn(parameters: &HashMap<String, String>) -> XrayResult<Vec<String>> {
  match parameters.get("alpn") {
    None => Ok(Vec::new()),
    Some(raw) => Ok(
      raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect(),
    ),
  }
}

pub fn export_vless_uri(config: &VlessConfig, name: Option<&str>) -> XrayResult<String> {
  config.validate()?;
  if let Some(name) = name {
    validate_display_name(name)?;
  }

  let mut url = Url::parse("vless://placeholder@127.0.0.1").expect("static VLESS URL is valid");
  url
    .set_username(&config.id)
    .map_err(|_| XrayError::InvalidUri)?;
  let uri_host = match config.address.parse::<IpAddr>() {
    Ok(IpAddr::V6(address)) => format!("[{address}]"),
    _ => config.address.clone(),
  };
  url
    .set_host(Some(&uri_host))
    .map_err(|_| XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    })?;
  url
    .set_port(Some(config.port))
    .map_err(|_| XrayError::InvalidField {
      field: "port",
      reason: "must be between 1 and 65535",
    })?;

  {
    let mut query = url.query_pairs_mut();
    query.append_pair("encryption", "none");
    query.append_pair("flow", config.flow.as_str());
    query.append_pair("security", config.security.as_wire_str());
    match &config.security {
      VlessSecurity::None => {}
      VlessSecurity::Tls(settings) => {
        query.append_pair("sni", &settings.server_name);
        query.append_pair("fp", settings.fingerprint.as_str());
        if !settings.alpn.is_empty() {
          query.append_pair("alpn", &settings.alpn.join(","));
        }
      }
      VlessSecurity::Reality(settings) => {
        query.append_pair("sni", &settings.server_name);
        query.append_pair("fp", settings.fingerprint.as_str());
        query.append_pair("pbk", &settings.public_key);
        query.append_pair("sid", &settings.short_id);
        query.append_pair("spx", &settings.spider_x);
      }
    }
    match &config.transport {
      VlessTransport::Tcp => {
        query.append_pair("type", "tcp");
      }
      VlessTransport::Ws(settings) => {
        query.append_pair("type", "ws");
        query.append_pair("path", &settings.path);
        if let Some(host) = &settings.host {
          query.append_pair("host", host);
        }
      }
      VlessTransport::Grpc(settings) => {
        query.append_pair("type", "grpc");
        query.append_pair("serviceName", &settings.service_name);
      }
      VlessTransport::H2(settings) => {
        query.append_pair("type", "http");
        query.append_pair("path", &settings.path);
        query.append_pair("host", &settings.host);
      }
    }
    query.append_pair("headerType", "none");
  }
  // The parser percent-DECODES the fragment, so the exporter must encode it or
  // a name containing `%` (or `#`) comes back different every time the URI is
  // canonicalized — the name mutates a little more on each save.
  let encoded_name = name.map(|value| urlencoding::encode(value).into_owned());
  url.set_fragment(encoded_name.as_deref());
  Ok(url.into())
}

/// Split the query into recognized parameters and the names of the rest.
///
/// Unrecognized names are returned rather than rejected on the spot so the
/// caller can report the *shape* problem first — a QUIC or mKCP link carries
/// parameters no supported shape uses at all, and naming the actual
/// unsupported transport is more useful than naming one of its side fields.
fn parse_parameters(url: &Url) -> XrayResult<(HashMap<String, String>, Vec<String>)> {
  let mut parameters = HashMap::new();
  let mut unsupported = Vec::new();
  for (name, value) in url.query_pairs() {
    if !SUPPORTED_PARAMETERS.contains(&name.as_ref()) {
      unsupported.push(name.into_owned());
      continue;
    }
    if parameters
      .insert(name.to_string(), value.into_owned())
      .is_some()
    {
      return Err(XrayError::DuplicateParameter(name.into_owned()));
    }
  }
  Ok((parameters, unsupported))
}

fn required_parameter<'a>(
  parameters: &'a HashMap<String, String>,
  name: &'static str,
) -> XrayResult<&'a str> {
  parameters
    .get(name)
    .filter(|value| !value.is_empty())
    .map(String::as_str)
    .ok_or(XrayError::MissingField(name))
}

fn optional_value(
  parameters: &HashMap<String, String>,
  name: &'static str,
  expected: &'static str,
) -> XrayResult<()> {
  if parameters.get(name).is_some_and(|value| value != expected) {
    return Err(XrayError::UnsupportedValue {
      field: name,
      expected,
    });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

  use super::*;

  const ID: &str = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";

  #[test]
  fn unsupported_setups_report_which_part_is_unsupported() {
    let good = format!(
      "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&sid=00&fp=chrome"
    );
    assert!(parse_vless_uri(&good).is_ok(), "baseline URI must parse");

    let reason = |uri: &str| parse_vless_uri(uri).unwrap_err().reason_code();

    assert_eq!(
      reason(&good.replace("flow=xtls-rprx-vision", "flow=made-up")),
      "flow"
    );
    // A quic/kcp transport still names itself, not a stray field.
    assert_eq!(reason(&good.replace("type=tcp", "type=quic")), "transport");
    assert_eq!(reason(&good.replace("type=tcp", "type=kcp")), "transport");
    assert_eq!(reason(&good.replace("&sni=a.com", "")), "sni");
    assert_eq!(
      reason(&good.replace("&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4", "")),
      "publicKey"
    );
    assert_eq!(reason(&good.replace("vless://", "vmess://")), "scheme");
    assert_eq!(reason("not a uri"), "malformed");
  }

  #[test]
  fn a_display_name_survives_an_export_parse_round_trip() {
    for name in ["50% off", "a#b", "spaced name", "100%25", "üñî"] {
      let parsed = parse_vless_uri(&format!(
        "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4"
      ))
      .expect("baseline parses");

      let exported = export_vless_uri(&parsed.config, Some(name)).expect("exports");
      let reparsed = parse_vless_uri(&exported).expect("re-parses");
      assert_eq!(
        reparsed.name.as_deref(),
        Some(name),
        "display name mutated across a round trip: {exported}"
      );

      let exported_again =
        export_vless_uri(&reparsed.config, reparsed.name.as_deref()).expect("re-exports");
      assert_eq!(exported, exported_again);
    }
  }

  #[test]
  fn real_world_websocket_link_parses_with_tls_and_ws() {
    let ws = format!(
      "vless://{ID}@cdn.example.com:443?encryption=none&security=tls&type=ws\
&path=%2Fray&host=cdn.example.com&sni=cdn.example.com#WS%20node"
    );
    let parsed = parse_vless_uri(&ws).expect("real-world ws link must parse");
    assert_eq!(parsed.name.as_deref(), Some("WS node"));
    match &parsed.config.security {
      VlessSecurity::Tls(settings) => assert_eq!(settings.server_name, "cdn.example.com"),
      other => panic!("expected tls security, got {other:?}"),
    }
    match &parsed.config.transport {
      VlessTransport::Ws(settings) => {
        assert_eq!(settings.path, "/ray");
        assert_eq!(settings.host.as_deref(), Some("cdn.example.com"));
      }
      other => panic!("expected ws transport, got {other:?}"),
    }
    assert_eq!(parsed.config.flow, VlessFlow::None);
  }

  #[test]
  fn real_world_grpc_link_parses_with_reality_and_grpc() {
    let grpc = format!(
      "vless://{ID}@grpc.example.com:443?encryption=none&security=reality&type=grpc\
&serviceName=gun&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4"
    );
    let parsed = parse_vless_uri(&grpc).expect("real-world grpc link must parse");
    match &parsed.config.transport {
      VlessTransport::Grpc(settings) => assert_eq!(settings.service_name, "gun"),
      other => panic!("expected grpc transport, got {other:?}"),
    }
    assert_eq!(parsed.config.flow, VlessFlow::None);

    // A genuinely unknown option on an otherwise-supported URI still reports
    // as a parameter problem, which is the accurate answer there.
    let odd = format!(
      "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&madeUpKey=1"
    );
    assert_eq!(
      parse_vless_uri(&odd).unwrap_err().reason_code(),
      "parameter"
    );
  }

  #[test]
  fn h2_transport_requires_both_path_and_host() {
    let missing_host =
      format!("vless://{ID}@example.com:443?security=tls&type=http&path=%2Fray&sni=example.com");
    assert!(parse_vless_uri(&missing_host).is_err());

    let full = format!(
      "vless://{ID}@example.com:443?security=tls&type=http&path=%2Fray&host=example.com&sni=example.com"
    );
    let parsed = parse_vless_uri(&full).expect("h2 link with both fields must parse");
    match &parsed.config.transport {
      VlessTransport::H2(settings) => {
        assert_eq!(settings.path, "/ray");
        assert_eq!(settings.host, "example.com");
      }
      other => panic!("expected h2 transport, got {other:?}"),
    }
  }

  #[test]
  fn plain_tls_with_alpn_and_no_reality_fields_parses() {
    let uri = format!(
      "vless://{ID}@example.com:443?security=tls&sni=example.com&alpn=h2%2Chttp%2F1.1&flow="
    );
    let parsed = parse_vless_uri(&uri).expect("plain TLS link must parse");
    match &parsed.config.security {
      VlessSecurity::Tls(settings) => {
        assert_eq!(settings.server_name, "example.com");
        assert_eq!(
          settings.alpn,
          vec!["h2".to_string(), "http/1.1".to_string()]
        );
      }
      other => panic!("expected tls security, got {other:?}"),
    }
  }

  #[test]
  fn security_none_requires_no_tls_or_reality_fields() {
    let uri = format!("vless://{ID}@example.com:443?security=none");
    let parsed = parse_vless_uri(&uri).expect("plaintext link must parse");
    assert_eq!(parsed.config.security, VlessSecurity::None);

    // Omitting security entirely is the same as security=none.
    let bare = format!("vless://{ID}@example.com:443");
    let parsed_bare = parse_vless_uri(&bare).expect("bare link must parse");
    assert_eq!(parsed_bare.config.security, VlessSecurity::None);
  }

  #[test]
  fn vision_flow_over_a_non_tcp_transport_is_rejected_by_final_validation() {
    let uri = format!(
      "vless://{ID}@cdn.example.com:443?security=tls&type=ws&path=%2Fray&sni=cdn.example.com&flow=xtls-rprx-vision"
    );
    assert!(matches!(
      parse_vless_uri(&uri),
      Err(XrayError::InvalidField { field: "flow", .. })
    ));
  }

  fn public_key() -> String {
    URL_SAFE_NO_PAD.encode([7_u8; 32])
  }

  fn uri(overrides: &[(&str, &str)]) -> String {
    let key = public_key();
    let mut parameters = vec![
      ("encryption", "none"),
      ("flow", "xtls-rprx-vision"),
      ("security", "reality"),
      ("sni", "www.example.com"),
      ("fp", "chrome"),
      ("pbk", key.as_str()),
      ("sid", "0123456789abcdef"),
      ("spx", "/"),
      ("type", "tcp"),
      ("headerType", "none"),
    ];
    for (name, value) in overrides {
      if let Some(parameter) = parameters.iter_mut().find(|(key, _)| key == name) {
        parameter.1 = value;
      } else {
        parameters.push((name, value));
      }
    }
    let query = parameters
      .into_iter()
      .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
      .collect::<Vec<_>>()
      .join("&");
    format!("vless://{ID}@vpn.example.com:443?{query}#Primary")
  }

  #[test]
  fn parses_supported_reality_vision_uri() {
    let parsed = parse_vless_uri(&uri(&[])).unwrap();
    assert_eq!(parsed.name.as_deref(), Some("Primary"));
    assert_eq!(parsed.config.address, "vpn.example.com");
    assert_eq!(parsed.config.port, 443);
    assert_eq!(parsed.config.id, ID);
    assert_eq!(parsed.config.flow, VlessFlow::Vision);
    match &parsed.config.security {
      VlessSecurity::Reality(settings) => {
        assert_eq!(settings.server_name, "www.example.com");
        assert_eq!(settings.public_key, public_key());
        assert_eq!(settings.short_id, "0123456789abcdef");
        assert_eq!(settings.fingerprint, RealityFingerprint::Chrome);
        assert_eq!(settings.spider_x, "/");
      }
      other => panic!("expected reality security, got {other:?}"),
    }
  }

  #[test]
  fn parses_ipv6_and_percent_encoded_metadata() {
    let input = uri(&[("spx", "/search?q=hello world")])
      .replace("vpn.example.com", "[2001:db8::1]")
      .replace("#Primary", "#Home%20server");
    let parsed = parse_vless_uri(&input).unwrap();
    assert_eq!(parsed.config.address, "2001:db8::1");
    match &parsed.config.security {
      VlessSecurity::Reality(settings) => {
        assert_eq!(settings.spider_x, "/search?q=hello world");
      }
      other => panic!("expected reality security, got {other:?}"),
    }
    assert_eq!(parsed.name.as_deref(), Some("Home server"));
  }

  #[test]
  fn applies_only_safe_optional_defaults() {
    let key = public_key();
    let input = format!(
      "vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com&pbk={key}"
    );
    let parsed = parse_vless_uri(&input).unwrap();
    match &parsed.config.security {
      VlessSecurity::Reality(settings) => {
        assert_eq!(settings.fingerprint, RealityFingerprint::Chrome);
        assert_eq!(settings.short_id, "");
        assert_eq!(settings.spider_x, "/");
      }
      other => panic!("expected reality security, got {other:?}"),
    }
  }

  #[test]
  fn accepts_raw_as_tcp_alias() {
    assert!(parse_vless_uri(&uri(&[("type", "raw")])).is_ok());
  }

  #[test]
  fn rejects_wrong_scheme_credentials_path_and_missing_port() {
    let valid = uri(&[]);
    let cases = [
      valid.replacen("vless://", "https://", 1),
      valid.replacen(ID, &format!("{ID}:password"), 1),
      valid.replacen(":443?", ":443/path?", 1),
      valid.replacen(":443?", "?", 1),
    ];
    for input in cases {
      assert!(parse_vless_uri(&input).is_err(), "{input}");
    }
  }

  #[test]
  fn rejects_missing_required_reality_values() {
    let key = public_key();
    // Omitting `flow` entirely is no longer an error — it means "no flow
    // control" (`VlessFlow::None`), a legitimate REALITY shape — so only a
    // missing `sni` or `pbk` remains a rejection here.
    let cases = [
      format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&pbk={key}"),
      format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com"),
    ];
    for input in cases {
      assert!(parse_vless_uri(&input).is_err(), "{input}");
    }
  }

  #[test]
  fn rejects_unsupported_security_transport_flow_and_encryption() {
    for (name, value) in [
      ("security", "ssl"),
      ("type", "kcp"),
      ("flow", "made-up"),
      ("encryption", "auto"),
      ("headerType", "http"),
      ("fp", "unsafe"),
    ] {
      assert!(
        matches!(
          parse_vless_uri(&uri(&[(name, value)])),
          Err(XrayError::UnsupportedValue { .. } | XrayError::MissingField(_))
        ),
        "{name}={value}"
      );
    }
  }

  #[test]
  fn rejects_unknown_and_duplicate_parameters() {
    assert_eq!(
      parse_vless_uri(&uri(&[("madeUpKey", "unsupported")])),
      Err(XrayError::UnsupportedParameter("madeUpKey".to_string()))
    );
    let input = uri(&[]).replace("#Primary", "&sni=duplicate.example.com#Primary");
    assert_eq!(
      parse_vless_uri(&input),
      Err(XrayError::DuplicateParameter("sni".to_string()))
    );
  }

  #[test]
  fn rejects_invalid_uuid_key_short_id_and_spider_x_without_echoing_secrets() {
    let invalid_key = "private-value-that-must-not-be-echoed";
    let cases = [
      uri(&[]).replacen(ID, "not-a-uuid", 1),
      uri(&[("pbk", invalid_key)]),
      uri(&[("sid", "xyz")]),
      uri(&[("spx", "relative")]),
    ];
    for input in cases {
      let error = parse_vless_uri(&input).unwrap_err();
      assert!(!error.to_string().contains(invalid_key));
      assert!(!error.to_string().contains(ID));
    }
  }

  #[test]
  fn export_is_canonical_and_round_trips() {
    let parsed = parse_vless_uri(&uri(&[("fp", "firefox")])).unwrap();
    let exported = export_vless_uri(&parsed.config, Some("Home server")).unwrap();
    assert!(exported.starts_with(&format!("vless://{ID}@vpn.example.com:443?")));
    assert!(exported.contains("type=tcp"));
    assert!(exported.contains("flow=xtls-rprx-vision"));
    assert!(exported.ends_with("#Home%20server"));

    let reparsed = parse_vless_uri(&exported).unwrap();
    assert_eq!(
      reparsed,
      ParsedVlessUri {
        name: Some("Home server".to_string()),
        config: parsed.config,
      }
    );
  }

  #[test]
  fn export_handles_ipv6_host() {
    let mut parsed = parse_vless_uri(&uri(&[])).unwrap();
    parsed.config.address = "2001:db8::1".to_string();
    let exported = export_vless_uri(&parsed.config, None).unwrap();
    assert!(exported.starts_with(&format!("vless://{ID}@[2001:db8::1]:443?")));
    assert_eq!(
      parse_vless_uri(&exported).unwrap().config.address,
      "2001:db8::1"
    );
  }

  #[test]
  fn export_and_reparse_round_trips_every_new_shape() {
    let cases: Vec<VlessConfig> = vec![
      VlessConfig {
        address: "cdn.example.com".to_string(),
        port: 443,
        id: ID.to_string(),
        flow: VlessFlow::None,
        security: VlessSecurity::Tls(TlsSettings {
          server_name: "cdn.example.com".to_string(),
          fingerprint: RealityFingerprint::Firefox,
          alpn: vec!["h2".to_string(), "http/1.1".to_string()],
        }),
        transport: VlessTransport::Ws(WsTransportSettings {
          path: "/ray".to_string(),
          host: Some("cdn.example.com".to_string()),
        }),
      },
      VlessConfig {
        address: "grpc.example.com".to_string(),
        port: 443,
        id: ID.to_string(),
        flow: VlessFlow::None,
        security: VlessSecurity::Reality(RealitySettings {
          server_name: "www.example.com".to_string(),
          public_key: public_key(),
          short_id: "ab".to_string(),
          fingerprint: RealityFingerprint::Chrome,
          spider_x: "/".to_string(),
        }),
        transport: VlessTransport::Grpc(GrpcTransportSettings {
          service_name: "gun".to_string(),
        }),
      },
      VlessConfig {
        address: "h2.example.com".to_string(),
        port: 443,
        id: ID.to_string(),
        flow: VlessFlow::None,
        security: VlessSecurity::None,
        transport: VlessTransport::H2(H2TransportSettings {
          path: "/ray".to_string(),
          host: "h2.example.com".to_string(),
        }),
      },
    ];

    for config in cases {
      let exported = export_vless_uri(&config, Some("Node")).expect("export succeeds");
      let reparsed = parse_vless_uri(&exported).expect("reparse succeeds");
      assert_eq!(reparsed.config, config, "round trip mismatch: {exported}");
    }
  }
}
