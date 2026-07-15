use era_protocol::{ProtocolBytes, ProtocolVersion, SessionId, VersionRange};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::SourceLocation;

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    #[n(0)]
    Keyboard,
    #[n(1)]
    Mouse,
    #[n(2)]
    Touch,
    #[n(3)]
    Gamepad,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
#[allow(clippy::struct_excessive_bools)]
pub struct ClientCapabilities {
    #[n(0)]
    pub input_modalities: Vec<InputModality>,
    #[n(1)]
    pub rich_text: bool,
    #[n(2)]
    pub html: bool,
    #[n(3)]
    pub graphics: bool,
    #[n(4)]
    pub audio: bool,
    #[n(5)]
    pub video: bool,
    #[n(6)]
    pub font_metrics: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
#[allow(clippy::struct_excessive_bools)]
pub struct ClientStateChanged {
    #[n(0)]
    pub focused: bool,
    #[n(1)]
    pub visible: bool,
    #[n(2)]
    pub audio_available: bool,
    #[n(3)]
    pub reduce_motion: bool,
    #[n(4)]
    pub high_contrast: bool,
    #[n(5)]
    pub screen_reader: bool,
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFeature {
    #[n(0)]
    ProjectReload,
    #[n(1)]
    TraditionalSave,
    #[n(2)]
    VmSnapshot,
    #[n(3)]
    TimedInput,
    #[n(4)]
    RichText,
    #[n(5)]
    Html,
    #[n(6)]
    Graphics,
    #[n(7)]
    Audio,
    #[n(8)]
    MouseInput,
    #[n(9)]
    ExternalServices,
    #[n(10)]
    StateResynchronization,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RuntimeLimits {
    #[n(0)]
    pub maximum_envelope_bytes: u64,
    #[n(1)]
    pub maximum_payload_bytes: u64,
    #[n(2)]
    pub maximum_pending_requests: u32,
    #[n(3)]
    pub maximum_journal_entries: u32,
    #[n(4)]
    pub maximum_drive_instructions: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ClientHello {
    #[n(0)]
    pub runtime_versions: VersionRange,
    #[n(1)]
    pub client_name: String,
    #[n(2)]
    pub features: Vec<RuntimeFeature>,
    #[n(3)]
    pub requested_limits: RuntimeLimits,
    #[n(4)]
    pub capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ServerHello {
    #[n(0)]
    pub selected_version: ProtocolVersion,
    #[n(1)]
    pub session: SessionId,
    #[n(2)]
    pub features: Vec<RuntimeFeature>,
    #[n(3)]
    pub limits: RuntimeLimits,
    #[n(4)]
    pub epoch: u64,
    #[n(5)]
    pub selected_capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VersionRejected {
    #[n(0)]
    pub supported: VersionRange,
    #[n(1)]
    pub message: String,
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase {
    #[n(0)]
    Negotiating,
    #[n(1)]
    LoadingProject,
    #[n(2)]
    Ready,
    #[n(3)]
    Starting,
    #[n(4)]
    Running,
    #[n(5)]
    WaitingInput,
    #[n(6)]
    Paused,
    #[n(7)]
    Reloading,
    #[n(8)]
    Stopping,
    #[n(9)]
    Stopped,
    #[n(10)]
    Faulted,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RuntimeStateChanged {
    #[n(0)]
    pub phase: RuntimePhase,
    #[n(1)]
    pub revision: u64,
    #[n(2)]
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SequenceAcknowledgement {
    #[n(0)]
    pub through_sequence: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResynchronizeRequest {
    #[n(0)]
    pub after_sequence: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StartMode {
    #[n(0)]
    NewGame {
        #[n(0)]
        seed: Option<u64>,
    },
    #[n(1)]
    TraditionalSave {
        #[n(0)]
        data: ProtocolBytes,
    },
    #[n(2)]
    VmSnapshot {
        #[n(0)]
        artifact_id: ProtocolBytes,
        #[n(1)]
        data: ProtocolBytes,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StartRequest {
    #[n(0)]
    pub mode: StartMode,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum StateExportKind {
    #[n(0)]
    TraditionalSave,
    #[n(1)]
    VmSnapshot,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportRequest {
    #[n(0)]
    pub kind: StateExportKind,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StateExportResult {
    #[n(0)]
    Ready {
        #[n(0)]
        data: ProtocolBytes,
        #[n(1)]
        artifact_id: Option<ProtocolBytes>,
    },
    #[n(1)]
    Ineligible {
        #[n(0)]
        reasons: Vec<String>,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportReady {
    #[n(0)]
    pub kind: StateExportKind,
    #[n(1)]
    pub result: StateExportResult,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum FaultCode {
    #[n(0)]
    InvalidState,
    #[n(1)]
    InvalidMessage,
    #[n(2)]
    ProjectLoad,
    #[n(3)]
    VmFault,
    #[n(4)]
    ServiceFailure,
    #[n(5)]
    ResourceLimit,
    #[n(6)]
    Internal,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorCode {
    #[n(0)]
    InvalidState,
    #[n(1)]
    InvalidValue,
    #[n(2)]
    StaleRequest,
    #[n(3)]
    VersionMismatch,
    #[n(4)]
    PermissionDenied,
    #[n(5)]
    FeatureUnavailable,
    #[n(6)]
    ResourceLimit,
}

/// A semantic command rejection does not fault the runtime session.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CommandRejected {
    #[n(0)]
    pub code: CommandErrorCode,
    #[n(1)]
    pub message: String,
    #[n(2)]
    pub recoverable: bool,
    #[n(3)]
    pub source: Option<SourceLocation>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RuntimeFault {
    #[n(0)]
    pub code: FaultCode,
    #[n(1)]
    pub message: String,
    #[n(2)]
    pub source: Option<SourceLocation>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ShutdownRequest {
    #[n(0)]
    pub graceful: bool,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ShutdownReady {
    #[n(0)]
    pub final_runtime_revision: u64,
    #[n(1)]
    pub pending_operations_cancelled: u32,
}
