use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Scalar values that may cross the frontend boundary without exposing Rust layouts.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ProtocolValue {
    #[n(0)]
    Integer(#[n(0)] i64),
    #[n(1)]
    String(#[n(0)] String),
    #[n(2)]
    Boolean(#[n(0)] bool),
    #[n(3)]
    Bytes(#[n(0)] ProtocolBytes),
}
