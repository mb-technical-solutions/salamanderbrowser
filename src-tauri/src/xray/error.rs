use thiserror::Error;

pub type XrayResult<T> = Result<T, XrayError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XrayError {
  #[error("invalid VLESS URI")]
  InvalidUri,
  #[error("URI scheme must be vless")]
  UnsupportedScheme,
  #[error("missing required field: {0}")]
  MissingField(&'static str),
  #[error("invalid field: {field} ({reason})")]
  InvalidField {
    field: &'static str,
    reason: &'static str,
  },
  #[error("unsupported query parameter: {0}")]
  UnsupportedParameter(String),
  #[error("duplicate query parameter: {0}")]
  DuplicateParameter(String),
  #[error("unsupported value for {field}; expected {expected}")]
  UnsupportedValue {
    field: &'static str,
    expected: &'static str,
  },
  #[error("failed to serialize Xray client configuration")]
  Serialization,
  #[error("failed to fetch the subscription URL")]
  SubscriptionFetchFailed,
  #[error("subscription response was too large")]
  SubscriptionTooLarge,
}

impl XrayError {
  /// A stable, translatable identifier for *why* a URI was rejected.
  ///
  /// Salamander supports a specific, checked set of VLESS shapes (TCP/WS/gRPC/H2
  /// transports, over none/TLS/REALITY security), so most rejections are
  /// "your setup is a kind we do not support", not "you made a typo". The
  /// frontend turns these into a sentence naming the unsupported part;
  /// without them every rejection reads as a malformed URI and a user with a
  /// working QUIC or mKCP server has no idea why it failed.
  pub fn reason_code(&self) -> &'static str {
    match self {
      Self::UnsupportedScheme => "scheme",
      Self::UnsupportedValue { field, .. } | Self::InvalidField { field, .. } => match *field {
        "security" => "security",
        "flow" => "flow",
        "type" => "transport",
        "encryption" => "encryption",
        "headerType" => "headerType",
        "fp" => "fingerprint",
        // A malformed sni/public key is the same user-facing problem as a
        // missing one, so it earns the same specific help rather than the
        // generic "invalid URI".
        "sni" | "server_name" => "sni",
        "pbk" | "public_key" => "publicKey",
        "alpn" => "alpn",
        // path/host/serviceName are all transport-specific fields (WS, H2,
        // gRPC); a value problem there is still a transport-shape problem
        // from the user's perspective.
        "path" | "host" | "serviceName" => "transport",
        _ => "malformed",
      },
      Self::MissingField(field) => match *field {
        "sni" => "sni",
        "pbk" => "publicKey",
        "security" => "security",
        "flow" => "flow",
        "path" | "host" | "serviceName" => "transport",
        _ => "malformed",
      },
      Self::UnsupportedParameter(_) => "parameter",
      Self::DuplicateParameter(_) => "malformed",
      Self::InvalidUri | Self::Serialization => "malformed",
      Self::SubscriptionFetchFailed | Self::SubscriptionTooLarge => "network",
    }
  }
}
