use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::ProtocolValue;

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    #[n(0)]
    EnterKey,
    #[n(1)]
    AnyKey,
    #[n(2)]
    IntegerValue,
    #[n(3)]
    StringValue,
    #[n(4)]
    Void,
    #[n(5)]
    AnyValue,
    #[n(6)]
    IntegerButton,
    #[n(7)]
    StringButton,
    #[n(8)]
    PrimitiveMouseKey,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum WaitStability {
    #[n(0)]
    StableInput,
    /// A snapshot-ineligible Host wait. This includes deadlines, frontend
    /// queries and non-resumable Void waits; the name does not promise that the
    /// wait will eventually finish.
    #[n(1)]
    Transient,
}

impl WaitStability {
    /// Classify a reference `InputRequest` after any message-skip shortcut has
    /// already been applied. Only deadline-free, user-resumable waits are
    /// stable exact-snapshot points.
    #[must_use]
    pub const fn for_reference_wait(kind: WaitKind, timelimit_ms: i64) -> Self {
        if timelimit_ms > 0 || matches!(kind, WaitKind::Void) {
            Self::Transient
        } else {
            Self::StableInput
        }
    }
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
// These flags are independent fields in the Emuera InputRequest contract. Combining
// them into states would reject valid reference combinations.
#[allow(clippy::struct_excessive_bools)]
pub struct InputWait {
    #[n(0)]
    pub wait_id: u64,
    #[n(1)]
    pub kind: WaitKind,
    #[n(2)]
    pub stability: WaitStability,
    #[n(3)]
    pub one_input: bool,
    #[n(4)]
    pub stop_message_skip: bool,
    #[n(5)]
    pub system_input: bool,
    #[n(6)]
    pub mouse_input: bool,
    #[n(7)]
    pub default_value: Option<ProtocolValue>,
    #[n(8)]
    pub deadline_ns: Option<u64>,
    #[n(9)]
    pub display_time: bool,
    #[n(10)]
    pub timeout_message: Option<String>,
    #[n(11)]
    pub button_generation: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PrimitiveInput {
    #[n(0)]
    pub input_type: i32,
    #[n(1)]
    pub result_1: i32,
    #[n(2)]
    pub result_2: i32,
    #[n(3)]
    pub result_3: i32,
    #[n(4)]
    pub result_4: i64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum InputValue {
    #[n(0)]
    Enter,
    #[n(1)]
    AnyKey(#[n(0)] String),
    #[n(2)]
    Integer(#[n(0)] i64),
    #[n(3)]
    String(#[n(0)] String),
    #[n(4)]
    IntegerButton(#[n(0)] i64),
    #[n(5)]
    StringButton(#[n(0)] String),
    #[n(6)]
    Primitive(#[n(0)] PrimitiveInput),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FrontendInput {
    #[n(0)]
    pub wait_id: u64,
    #[n(1)]
    pub button_generation: u64,
    #[n(2)]
    pub monotonic_time_ns: u64,
    #[n(3)]
    pub value: InputValue,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AdvanceTime {
    #[n(0)]
    pub monotonic_time_ns: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum WaitChange {
    #[n(0)]
    Opened(#[n(0)] InputWait),
    #[n(1)]
    Updated(#[n(0)] InputWait),
    #[n(2)]
    Closed(#[n(0)] u64),
}
