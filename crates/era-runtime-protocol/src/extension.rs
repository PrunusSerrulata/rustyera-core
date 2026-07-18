use era_protocol::ProtocolVersion;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::ProtocolValue;

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionValueType {
    #[n(0)]
    Integer,
    #[n(1)]
    String,
    #[n(2)]
    Void,
    #[n(3)]
    Any,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCallableKind {
    #[n(0)]
    Instruction,
    #[n(1)]
    Function,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionArgumentStyle {
    #[n(0)]
    Normal,
    #[n(1)]
    Formatted,
    #[n(2)]
    Raw,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionArgument {
    #[n(0)]
    pub value_type: ExtensionValueType,
    #[n(1)]
    pub mutable: bool,
    #[n(2)]
    pub optional: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionDeclaration {
    #[n(0)]
    pub id: String,
    #[n(1)]
    pub era_name: String,
    #[n(2)]
    pub kind: ExtensionCallableKind,
    #[n(3)]
    pub arguments: Vec<ExtensionArgument>,
    #[n(4)]
    pub variadic: bool,
    #[n(5)]
    pub return_type: ExtensionValueType,
    #[n(6)]
    pub argument_style: ExtensionArgumentStyle,
    #[n(7)]
    pub operation: String,
    #[n(8)]
    pub operation_version: ProtocolVersion,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionRegistrySubmit {
    #[n(0)]
    pub declarations: Vec<ExtensionDeclaration>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionInvocation {
    #[n(0)]
    pub extension_id: String,
    #[n(1)]
    pub arguments: Vec<ProtocolValue>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionWrite {
    #[n(0)]
    pub argument_ordinal: u32,
    #[n(1)]
    pub value: ProtocolValue,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExtensionResult {
    #[n(0)]
    pub value: Option<ProtocolValue>,
    #[n(1)]
    pub writes: Vec<ExtensionWrite>,
}
