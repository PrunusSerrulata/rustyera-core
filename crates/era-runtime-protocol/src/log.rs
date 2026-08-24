use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Backend-authoritative severity for runtime logs and diagnostics.
#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLogLevel {
    #[n(0)]
    Debug,
    #[n(1)]
    Info,
    #[n(2)]
    Warning,
    #[n(3)]
    Error,
}

/// Frontend notification guidance carried by a structured diagnostic.
#[derive(Clone, Copy, Debug, Decode, Default, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticNotification {
    #[default]
    #[n(0)]
    Default,
    #[n(1)]
    LogOnly,
}

/// A presentation-neutral log record emitted by the runtime.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RuntimeLog {
    #[n(0)]
    pub level: RuntimeLogLevel,
    #[n(1)]
    pub message: String,
}
