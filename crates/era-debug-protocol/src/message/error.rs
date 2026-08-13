use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum DebugErrorCode {
    #[n(0)]
    PermissionDenied,
    #[n(1)]
    InvalidState,
    #[n(2)]
    StaleStop,
    #[n(3)]
    StaleRevision,
    #[n(4)]
    UnknownTarget,
    #[n(5)]
    TypeMismatch,
    #[n(6)]
    UnsafeConsoleStatement,
    #[n(7)]
    ResourceLimit,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugError {
    #[n(0)]
    pub code: DebugErrorCode,
    #[n(1)]
    pub message: String,
}
