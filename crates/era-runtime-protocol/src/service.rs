use era_protocol::{ProtocolBytes, ProtocolVersion};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{CanvasPoint, FrontendIoError, ProjectionLength};

/// Filesystem semantics negotiated independently from rendering capabilities.
/// Candidate saves require every field; ordinary script Host operations can
/// continue to use their reference-compatible unconditional behavior.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
#[allow(clippy::struct_excessive_bools)]
pub struct StorageCapabilities {
    #[n(0)]
    pub revisions: bool,
    #[n(1)]
    pub atomic_replace: bool,
    #[n(2)]
    pub missing_precondition: bool,
    #[n(3)]
    pub delete: bool,
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
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
        precondition: StoragePrecondition,
    },
    #[n(2)]
    List {
        #[n(0)]
        pattern: Option<String>,
        /// Whether entries below child directories are included.
        #[n(1)]
        recursive: bool,
    },
    #[n(3)]
    Delete {
        #[n(0)]
        precondition: StoragePrecondition,
    },
    /// Read metadata without transferring the file contents.
    #[n(4)]
    Stat,
}

/// Optimistic concurrency condition applied by the frontend at commit time.
///
/// `Missing` is required for a new slot and prevents an unnoticed overwrite;
/// `Revision` protects an overwrite/delete selected from an earlier listing.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "revision", rename_all = "snake_case")]
pub enum StoragePrecondition {
    #[n(0)]
    Any,
    #[n(1)]
    Missing,
    #[n(2)]
    Revision(#[n(0)] String),
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
    #[n(5)]
    pub deadline_ns: Option<u64>,
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
#[cbor(map)]
pub struct StorageMetadata {
    #[n(0)]
    pub byte_length: u64,
    #[n(1)]
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
    #[n(5)]
    Metadata(#[n(0)] StorageMetadata),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StorageResponse {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub result: StorageResult,
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
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
    /// Queries of physical lines and HTML as realized by the authoritative frontend.
    #[n(10)]
    PresentationQuery,
}

pub const LOCAL_DATE_TIME_OPERATION: &str = "local_date_time";
pub const LOCAL_DATE_TIME_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const RANDOM_SEED_OPERATION: &str = "random_seed";
pub const RANDOM_SEED_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const IMAGE_METADATA_OPERATION: &str = "image_metadata";
pub const IMAGE_METADATA_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const IMAGE_PIXEL_OPERATION: &str = "image_pixel";
pub const IMAGE_PIXEL_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const UPDATE_CHECK_OPERATION: &str = "update_check";
pub const UPDATE_CHECK_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const OPEN_URL_OPERATION: &str = "open_url";
pub const OPEN_URL_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const POINTER_STATE_OPERATION: &str = "pointer_state";
pub const POINTER_STATE_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const GET_DISPLAY_LINE_OPERATION: &str = "get_display_line";
pub const GET_DISPLAY_LINE_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const HTML_GET_PRINTED_STR_OPERATION: &str = "html_get_printed_str";
pub const HTML_GET_PRINTED_STR_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const HTML_STRING_LEN_OPERATION: &str = "html_string_len";
pub const HTML_STRING_LEN_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const HTML_SUBSTRING_OPERATION: &str = "html_substring";
pub const HTML_SUBSTRING_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const HTML_STRING_LINES_OPERATION: &str = "html_string_lines";
pub const HTML_STRING_LINES_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const GGET_TEXT_SIZE_OPERATION: &str = "gget_text_size";
pub const GGET_TEXT_SIZE_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const SAMPLE_CANVAS_PIXEL_OPERATION: &str = "sample_canvas_pixel";
pub const SAMPLE_CANVAS_PIXEL_OPERATION_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

/// Causal identity shared by every query of realized frontend presentation.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionQueryContext {
    #[n(0)]
    pub presentation_revision: u64,
    #[n(1)]
    pub environment_revision: u64,
    #[n(2)]
    pub projection_space_revision: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionStringIndexRequest {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub index: i64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionStringResponse {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlMeasureRequest {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub markup: String,
    #[n(2)]
    pub argument: i64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionIntegerResponse {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub value: i64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlSubstringResponse {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub head: String,
    #[n(2)]
    pub tail: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct TextExtentRequest {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub text: String,
    #[n(2)]
    pub font_family: String,
    #[n(3)]
    pub font_size: i64,
    #[n(4)]
    pub style_bits: u8,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct TextExtentResponse {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub width: ProjectionLength,
    #[n(2)]
    pub height: ProjectionLength,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasPixelRequest {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub canvas_id: i64,
    #[n(2)]
    pub canvas_revision: u64,
    #[n(3)]
    pub point: CanvasPoint,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasPixelResponse {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub canvas_revision: u64,
    #[n(2)]
    pub argb: u32,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PointerStateRequest {
    /// Presentation revision whose interaction projection is being observed.
    #[n(0)]
    pub presentation_revision: u64,
    #[n(1)]
    pub environment_revision: u64,
    #[n(2)]
    pub projection_space_revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PointerStateResponse {
    /// Coordinates are frontend-independent logical (CSS) pixels.
    #[n(0)]
    pub x: ProjectionLength,
    #[n(1)]
    pub y: ProjectionLength,
    /// `EraBasic` value of the hovered button, or the empty string.
    #[n(2)]
    pub button_value: String,
    #[n(3)]
    pub presentation_revision: u64,
    #[n(4)]
    pub environment_revision: u64,
    #[n(5)]
    pub projection_space_revision: u64,
}

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

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ImageMetadataRequest {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub content_digest: ProtocolBytes,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ImageMetadataResponse {
    #[n(0)]
    pub width: u32,
    #[n(1)]
    pub height: u32,
    #[n(2)]
    pub format: String,
    #[n(3)]
    pub animated: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ImagePixelRequest {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub content_digest: ProtocolBytes,
    #[n(2)]
    pub x: u32,
    #[n(3)]
    pub y: u32,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ImagePixelResponse {
    /// ARGB in the same unsigned 32-bit layout exposed by Emuera.
    #[n(0)]
    pub argb: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct UpdateCheckRequest {
    #[n(0)]
    pub url: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct UpdateCheckResponse {
    #[n(0)]
    pub remote_version: String,
    #[n(1)]
    pub download_url: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct OpenUrlRequest {
    #[n(0)]
    pub url: String,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct OpenUrlResponse {
    #[n(0)]
    pub opened: bool,
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
    #[n(5)]
    pub deadline_ns: Option<u64>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ExternalRequestKind {
    #[n(0)]
    Storage,
    #[n(1)]
    Service,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CancelExternalRequest {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub kind: ExternalRequestKind,
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
