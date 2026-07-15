use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    #[n(0)]
    Integer,
    #[n(1)]
    String,
    #[n(2)]
    Boolean,
    #[n(3)]
    Bytes,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugPlace {
    #[n(0)]
    pub symbol_key: ProtocolBytes,
    #[n(1)]
    pub value_kind: ValueKind,
    #[n(2)]
    pub indices: Vec<u64>,
    #[n(3)]
    pub character: Option<u64>,
    #[n(4)]
    pub fiber_id: Option<u64>,
    #[n(5)]
    pub frame_id: Option<u64>,
    #[n(6)]
    pub generation: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DebugValue {
    #[n(0)]
    Integer(#[n(0)] i64),
    #[n(1)]
    String(#[n(0)] String),
    #[n(2)]
    Boolean(#[n(0)] bool),
    #[n(3)]
    Bytes(#[n(0)] ProtocolBytes),
    #[n(4)]
    Place(#[n(0)] DebugPlace),
}

impl DebugValue {
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Integer(_) => ValueKind::Integer,
            Self::String(_) => ValueKind::String,
            Self::Boolean(_) => ValueKind::Boolean,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Place(place) => place.value_kind,
        }
    }
}
