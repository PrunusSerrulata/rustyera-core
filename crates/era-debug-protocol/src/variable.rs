use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{DebugValue, StopToken, ValueKind};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum VariableStorage {
    #[n(0)]
    Global,
    #[n(1)]
    FunctionStatic,
    #[n(2)]
    Character,
    #[n(3)]
    Local,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariableReference {
    #[n(0)]
    pub symbol_key: ProtocolBytes,
    #[n(1)]
    pub storage: VariableStorage,
    #[n(2)]
    pub fiber_id: Option<u64>,
    #[n(3)]
    pub frame_id: Option<u64>,
    #[n(4)]
    pub generation: u64,
    #[n(5)]
    pub character: Option<u64>,
    #[n(6)]
    pub indices: Vec<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariableDescriptor {
    #[n(0)]
    pub symbol_key: ProtocolBytes,
    #[n(1)]
    pub name: String,
    #[n(2)]
    pub storage: VariableStorage,
    #[n(3)]
    pub value_kind: ValueKind,
    #[n(4)]
    pub dimensions: Vec<u64>,
    #[n(5)]
    pub mutable: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariablePage {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub variables: Vec<VariableDescriptor>,
    #[n(2)]
    pub next_cursor: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariableValue {
    #[n(0)]
    pub reference: VariableReference,
    #[n(1)]
    pub value: DebugValue,
    #[n(2)]
    pub revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariableWrite {
    #[n(0)]
    pub reference: VariableReference,
    #[n(1)]
    pub value: DebugValue,
    #[n(2)]
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VariableWriteOutcome {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub values: Vec<VariableValue>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum FieldMutability {
    #[n(0)]
    ReadOnly,
    #[n(1)]
    DebugWritable,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GameFieldDescriptor {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub value_kind: ValueKind,
    #[n(2)]
    pub mutability: FieldMutability,
    #[n(3)]
    pub description: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GameFieldPage {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub fields: Vec<GameFieldDescriptor>,
    #[n(2)]
    pub next_cursor: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GameFieldValue {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub value: DebugValue,
    #[n(2)]
    pub revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GameFieldWrite {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub value: DebugValue,
    #[n(2)]
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GameFieldWriteOutcome {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub values: Vec<GameFieldValue>,
}
