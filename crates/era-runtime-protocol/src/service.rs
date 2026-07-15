use era_protocol::{ProtocolBytes, ProtocolVersion};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::FrontendIoError;

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum StorageNamespace {
    #[n(0)]
    Project,
    #[n(1)]
    Save,
    #[n(2)]
    GlobalSave,
    #[n(3)]
    Data,
    #[n(4)]
    Log,
    #[n(5)]
    Resource,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StorageOperation {
    #[n(0)]
    Read,
    #[n(1)]
    Write {
        #[n(0)]
        data: ProtocolBytes,
        #[n(1)]
        atomic_replace: bool,
        #[n(2)]
        expected_revision: Option<String>,
    },
    #[n(2)]
    List {
        #[n(0)]
        pattern: Option<String>,
    },
    #[n(3)]
    Delete {
        #[n(0)]
        expected_revision: Option<String>,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StorageRequest {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub namespace: StorageNamespace,
    #[n(2)]
    pub relative_path: String,
    #[n(3)]
    pub operation: StorageOperation,
    #[n(4)]
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StorageEntry {
    #[n(0)]
    pub relative_path: String,
    #[n(1)]
    pub byte_length: u64,
    #[n(2)]
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StorageResult {
    #[n(0)]
    Read {
        #[n(0)]
        data: ProtocolBytes,
        #[n(1)]
        revision: Option<String>,
    },
    #[n(1)]
    Written {
        #[n(0)]
        revision: Option<String>,
    },
    #[n(2)]
    Listed {
        #[n(0)]
        entries: Vec<StorageEntry>,
    },
    #[n(3)]
    Deleted,
    #[n(4)]
    Error {
        #[n(0)]
        error: FrontendIoError,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StorageResponse {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub result: StorageResult,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    #[n(0)]
    FontMetrics,
    #[n(1)]
    Image,
    #[n(2)]
    Canvas,
    #[n(3)]
    Audio,
    #[n(4)]
    Network,
    #[n(5)]
    OpenUrl,
    #[n(6)]
    Extension,
    /// Fresh frontend-owned keyboard state used by GETKEY-family functions.
    #[n(7)]
    InputState,
    /// Frontend-owned local calendar time. Timed waits never use this service.
    #[n(8)]
    Clock,
    /// Frontend-owned nondeterminism used only when the caller omitted a seed.
    #[n(9)]
    Entropy,
}

pub const LOCAL_DATE_TIME_OPERATION: &str = "local_date_time";
pub const LOCAL_DATE_TIME_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const RANDOM_SEED_OPERATION: &str = "random_seed";
pub const RANDOM_SEED_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct LocalDateTimeRequest {}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct LocalDateTimeResponse {
    #[n(0)]
    pub year: i32,
    #[n(1)]
    pub month: u8,
    #[n(2)]
    pub day: u8,
    #[n(3)]
    pub hour: u8,
    #[n(4)]
    pub minute: u8,
    #[n(5)]
    pub second: u8,
    #[n(6)]
    pub millisecond: u16,
    #[n(7)]
    pub utc_offset_minutes: i16,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RandomSeedRequest {}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RandomSeedResponse {
    #[n(0)]
    pub seed: u64,
}

pub const GET_KEY_STATE_OPERATION: &str = "get_key_state";
pub const GET_KEY_STATE_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

/// Typed payload for a fresh GETKEY query. The `EraBasic` range check happens in
/// the runtime before constructing this value, so an out-of-range key never
/// creates a transient frontend request.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GetKeyStateRequest {
    #[n(0)]
    pub key_code: u8,
}

/// Platform-independent projection of Win32 `GetKeyState`'s observable bits.
/// `toggle_state` is retained because GETKEY and GETKEYTRIGGERED share the
/// reference implementation's per-key observation state.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
#[allow(clippy::struct_excessive_bools)]
pub struct GetKeyStateResponse {
    #[n(0)]
    pub frontend_active: bool,
    #[n(1)]
    pub pressed: bool,
    #[n(2)]
    pub toggle_state: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ServiceRequest {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub kind: ServiceKind,
    #[n(2)]
    pub operation: String,
    #[n(3)]
    pub operation_version: ProtocolVersion,
    #[n(4)]
    pub payload: ProtocolBytes,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ServiceError {
    #[n(0)]
    pub code: String,
    #[n(1)]
    pub message: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceResult {
    #[n(0)]
    Ready {
        #[n(0)]
        payload: ProtocolBytes,
    },
    #[n(1)]
    Error {
        #[n(0)]
        error: ServiceError,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ServiceResponse {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub result: ServiceResult,
}
