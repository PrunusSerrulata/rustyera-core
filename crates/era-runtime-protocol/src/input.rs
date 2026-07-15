use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::ProtocolValue;

/// Opaque, epoch-scoped capability authorizing one interaction.
///
/// Frontends must return the token they received and must not derive game values
/// from it. The runtime additionally checks that the token is currently active.
#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(map)]
pub struct InteractionToken {
    #[n(0)]
    pub epoch: u64,
    #[n(1)]
    pub id: u64,
}

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
    pub submission_token: InteractionToken,
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
    pub result_4: i32,
    /// Optional runtime-issued selection token. The frontend never supplies RESULT[5].
    #[n(5)]
    pub selection_token: Option<InteractionToken>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum InputIntent {
    #[n(0)]
    Enter,
    #[n(1)]
    AnyKey(#[n(0)] String),
    #[n(2)]
    CommitText(#[n(0)] String),
    #[n(3)]
    Activate(#[n(0)] InteractionToken),
    #[n(4)]
    Continue,
    #[n(5)]
    Cancel,
    #[n(6)]
    Primitive(#[n(0)] PrimitiveInput),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FrontendInput {
    #[n(0)]
    pub wait_id: u64,
    #[n(1)]
    pub token: InteractionToken,
    #[n(2)]
    pub monotonic_time_ns: u64,
    #[n(3)]
    pub intent: InputIntent,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum InputDeviceKind {
    #[n(0)]
    Keyboard,
    #[n(1)]
    Mouse,
    #[n(2)]
    Touch,
    #[n(3)]
    Gamepad,
}

/// Ordered frontend-owned device state. `EraBasic` interpretation remains in the runtime.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DeviceStateChanged {
    #[n(0)]
    pub device: InputDeviceKind,
    #[n(1)]
    pub code: u32,
    #[n(2)]
    pub pressed: bool,
    #[n(3)]
    pub x: i32,
    #[n(4)]
    pub y: i32,
    #[n(5)]
    pub monotonic_time_ns: u64,
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
