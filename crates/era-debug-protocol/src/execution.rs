use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::DebugValue;

/// A coherent debugger view. Every stopped-state read and mutation carries this
/// token so requests cannot accidentally target a resumed or reloaded VM.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StopToken {
    #[n(0)]
    pub pause_epoch: u64,
    #[n(1)]
    pub program_generation: u64,
    #[n(2)]
    pub runtime_revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugSourceLocation {
    #[n(0)]
    pub relative_path: String,
    #[n(1)]
    pub content_hash: ProtocolBytes,
    #[n(2)]
    pub byte_start: u64,
    #[n(3)]
    pub byte_end: u64,
    #[n(4)]
    pub line: u64,
    #[n(5)]
    pub byte_column: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum FiberState {
    #[n(0)]
    Runnable,
    #[n(1)]
    WaitingHost,
    #[n(2)]
    WaitingResume,
    #[n(3)]
    Completed,
    #[n(4)]
    Faulted,
    #[n(5)]
    Cancelled,
    #[n(6)]
    DebugPaused,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FiberSummary {
    #[n(0)]
    pub fiber_id: u64,
    #[n(1)]
    pub state: FiberState,
    #[n(2)]
    pub primary: bool,
    #[n(3)]
    pub frame_count: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FiberPage {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub fibers: Vec<FiberSummary>,
    #[n(2)]
    pub next_cursor: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FrameSummary {
    #[n(0)]
    pub frame_id: u64,
    #[n(1)]
    pub generation: u64,
    /// Stable 128-bit symbol key encoded as a byte string.
    #[n(2)]
    pub function_key: ProtocolBytes,
    #[n(3)]
    pub function_name: String,
    #[n(4)]
    pub instruction: u32,
    #[n(5)]
    pub source: Option<DebugSourceLocation>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CallStack {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub fiber_id: u64,
    #[n(2)]
    pub frames: Vec<FrameSummary>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct OperandValue {
    #[n(0)]
    pub offset: u64,
    #[n(1)]
    pub value: DebugValue,
}

/// Operand stacks are intentionally represented only by a read response. No command
/// in this protocol accepts an [`OperandValue`] as a mutation target.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct OperandStackPage {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub fiber_id: u64,
    #[n(2)]
    pub frame_id: u64,
    #[n(3)]
    pub values: Vec<OperandValue>,
    #[n(4)]
    pub next_cursor: Option<u64>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    #[n(0)]
    Instruction,
    #[n(1)]
    SourceLine,
    #[n(2)]
    Into,
    #[n(3)]
    Over,
    #[n(4)]
    Out,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StopReason {
    #[n(0)]
    PauseRequested,
    #[n(1)]
    Breakpoint {
        #[n(0)]
        breakpoint_id: u64,
    },
    #[n(2)]
    StepCompleted,
    #[n(3)]
    HostWait,
    #[n(4)]
    FiberCompleted,
    #[n(5)]
    Fault {
        #[n(0)]
        message: String,
    },
    #[n(6)]
    Reload,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugStop {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub reason: StopReason,
    #[n(2)]
    pub selected_fiber: Option<u64>,
    #[n(3)]
    pub source: Option<DebugSourceLocation>,
}
