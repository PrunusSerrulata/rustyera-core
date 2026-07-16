use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::DebugSourceLocation;

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BreakpointLocation {
    #[n(0)]
    Source {
        #[n(0)]
        relative_path: String,
        #[n(1)]
        content_hash: ProtocolBytes,
        #[n(2)]
        byte_offset: u64,
    },
    #[n(1)]
    Function {
        #[n(0)]
        symbol_key: ProtocolBytes,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct Breakpoint {
    #[n(0)]
    pub breakpoint_id: u64,
    #[n(1)]
    pub enabled: bool,
    #[n(2)]
    pub location: BreakpointLocation,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum BreakpointBinding {
    #[n(0)]
    Verified,
    #[n(1)]
    Moved,
    #[n(2)]
    Unbound,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResolvedBreakpoint {
    #[n(0)]
    pub breakpoint_id: u64,
    #[n(1)]
    pub generation: u64,
    #[n(2)]
    pub binding: BreakpointBinding,
    #[n(3)]
    pub source: Option<DebugSourceLocation>,
    #[n(4)]
    pub message: Option<String>,
    #[n(5)]
    pub hit_count: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct BreakpointUpdate {
    #[n(0)]
    pub requested: Vec<Breakpoint>,
    #[n(1)]
    pub remove: Vec<u64>,
}
