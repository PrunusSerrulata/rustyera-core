//! Negotiated portable capabilities. Names do not describe the host operating system.
use era_protocol::{ProtocolVersion, VersionRange};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub const INPUT_TIMED_VIEWPORT_CAPABILITY: &str = "input.timed_viewport";
pub const INPUT_DEVICE_LATCH_CAPABILITY: &str = "input.device_latch";
pub const INPUT_DEVICE_PUMP_CAPABILITY: &str = "input.device_pump";
pub const INPUT_SEQUENCE_CAPABILITY: &str = "input.sequence";
pub const INPUT_MACROS_CAPABILITY: &str = "input.macros";
pub const INPUT_ENVIRONMENT_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const DEVICE_PUMP_OPERATION: &str = "device_pump";
pub const DEVICE_PUMP_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EnvironmentCapability {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub versions: VersionRange,
}

#[derive(Clone, Copy, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum InputViewportPolicy {
    #[default]
    #[n(0)]
    FollowOutput,
    /// Keep explicit NF user-scroll intent through timeout, clear and redraw;
    /// the next `FollowOutput` wait or session change ends this policy.
    #[n(1)]
    PreserveUserViewport,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DevicePumpRequest {
    #[n(0)]
    pub epoch: u64,
    /// Last accepted physical device event. Frontend must flush subsequent real
    /// events in order before replying; an empty event-loop pump is valid.
    #[n(1)]
    pub after_event_sequence: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DevicePumpResponse {
    #[n(0)]
    pub epoch: u64,
    /// Exact final event sequence delivered before this acknowledgement.
    #[n(1)]
    pub through_event_sequence: u64,
}
