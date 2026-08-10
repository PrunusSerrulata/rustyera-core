use era_protocol::{ProtocolBytes, ProtocolVersion, SessionId, VersionRange};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{
    ConfigurationClientProfile, ProjectManifest, ServiceKind, SourceLocation, StorageCapabilities,
};

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ServiceCapability {
    #[n(0)]
    pub kind: ServiceKind,
    #[n(1)]
    pub operation: String,
    #[n(2)]
    pub versions: VersionRange,
}

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
    /// The frontend can lay out PRINTC-family semantic column cells.
    #[n(7)]
    pub column_cells: bool,
    /// The frontend can render a semantic separator independently of text width.
    #[n(8)]
    pub separators: bool,
    /// Session-fixed canonical family names used only by CHKFONT. Runtime layout
    /// never depends on frontend measurements.
    #[n(9)]
    pub available_fonts: Vec<String>,
    /// Exact service operations and wire versions supported by the frontend.
    #[n(10)]
    pub services: Vec<ServiceCapability>,
    /// Storage guarantees the frontend can enforce at commit time.
    #[n(11)]
    pub storage: StorageCapabilities,
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

/// A coordinate or extent in the authoritative frontend's device-independent
/// layout space (for example CSS pixels). It is never stored in canonical
/// presentation state.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(transparent)]
pub struct ProjectionLength(#[n(0)] pub i64);

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionSize {
    #[n(0)]
    pub width: ProjectionLength,
    #[n(1)]
    pub height: ProjectionLength,
}

/// Exact affine transform from Era logical milliunits to projection units.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionTransform {
    #[n(0)]
    pub x_numerator: i64,
    #[n(1)]
    pub x_denominator: u64,
    #[n(2)]
    pub y_numerator: i64,
    #[n(3)]
    pub y_denominator: u64,
    #[n(4)]
    pub origin_x: ProjectionLength,
    #[n(5)]
    pub origin_y: ProjectionLength,
}

impl ProjectionTransform {
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.x_denominator != 0 && self.y_denominator != 0
    }
}

/// Authoritative main-frontend observation used by script-visible layout and textbox queries.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionObservation {
    #[n(0)]
    pub environment_revision: u64,
    #[n(1)]
    pub presentation_revision: u64,
    #[n(2)]
    pub client_size: ProjectionSize,
    #[n(3)]
    pub projection_space_revision: u64,
    #[n(4)]
    pub line_columns: u32,
    #[n(5)]
    pub text_box: String,
    #[n(6)]
    pub transform: ProjectionTransform,
}

/// Runtime-owned state that the main frontend projects into platform controls.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectionState {
    #[n(0)]
    pub runtime_revision: u64,
    #[n(1)]
    pub text_box: String,
    #[n(2)]
    pub hotkey_state: Vec<i64>,
    #[n(3)]
    pub button_generation: u64,
    #[n(4)]
    pub text_box_layout: TextBoxLayout,
}

/// Runtime-owned logical `TextBox` placement. The frontend applies its
/// projection transform and platform-specific clipping.
#[derive(Clone, Copy, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct TextBoxLayout {
    #[n(0)]
    pub x: i64,
    #[n(1)]
    pub y: i64,
    /// Zero selects the configured default width.
    #[n(2)]
    pub width: i64,
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
    /// Versioned frontend storage messages are available.
    #[n(11)]
    Storage,
    #[n(12)]
    InputUndo,
    #[n(13)]
    ProjectAnalysis,
    #[n(14)]
    KeyMacros,
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
    /// Maximum size of one logical import or export assembled from chunks.
    #[n(5)]
    pub maximum_transfer_bytes: u64,
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
    /// Ordered BCP-47 preferences sampled by the frontend.
    #[n(5)]
    pub preferred_locales: Vec<String>,
    /// Optional client policy for project configuration defaults and application behavior.
    #[n(6)]
    pub configuration_profile: Option<ConfigurationClientProfile>,
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
    #[n(6)]
    pub selected_locale: String,
    /// Echoed only when the Runtime supports the requested configuration profile.
    #[n(7)]
    pub configuration_profile: Option<ConfigurationClientProfile>,
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
    WaitingExternal,
    #[n(7)]
    DebugPaused,
    #[n(8)]
    Reloading,
    #[n(9)]
    Stopping,
    #[n(10)]
    Stopped,
    #[n(11)]
    Faulted,
    #[n(12)]
    AnalyzingProject,
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
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    #[n(0)]
    Quit,
    #[n(1)]
    Restart,
}

/// Persistent terminal intent. It remains part of resynchronization until the
/// frontend acknowledges it by shutting down the session.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExitRequested {
    #[n(0)]
    pub reason: ExitReason,
    #[n(1)]
    pub force: bool,
    #[n(2)]
    pub runtime_revision: u64,
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
        transfer_id: u64,
    },
    #[n(2)]
    VmSnapshot {
        #[n(0)]
        transfer_id: u64,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StartRequest {
    #[n(0)]
    pub mode: StartMode,
}

/// Discard the active game timeline and enter the title flow without reloading the project.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ReturnToTitleRequest {}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum StateExportKind {
    #[n(0)]
    TraditionalSave,
    #[n(1)]
    VmSnapshot,
    /// Opaque, versioned compiler cache. Frontends persist but never interpret these bytes.
    #[n(2)]
    CompiledProjectCache,
    /// Self-contained project container including all runtime inputs and binary resources.
    #[n(3)]
    FullProjectFile,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FullProjectManifest {
    #[n(0)]
    pub manifest: ProjectManifest,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportCancel {
    #[n(0)]
    pub kind: StateExportKind,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateTransferDescriptor {
    #[n(0)]
    pub transfer_id: u64,
    #[n(1)]
    pub kind: StateExportKind,
    #[n(2)]
    pub total_bytes: u64,
    /// Raw 32-byte BLAKE3 digest of the complete payload.
    #[n(3)]
    pub digest: ProtocolBytes,
    /// Exact bytecode artifact identity when the transfer requires one.
    #[n(4)]
    pub artifact_id: Option<ProtocolBytes>,
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotIneligibleReason {
    #[n(0)]
    StableWaitRequired,
    #[n(1)]
    ExternalOperationPending,
    #[n(2)]
    VmSnapshotUnavailable,
    #[n(3)]
    SnapshotStateUnavailable,
}

/// Declares why a VM snapshot is being captured. Relaxed captures remain subject to the
/// same validation as ordinary snapshots when a frontend later tries to restore them.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotExportPurpose {
    #[n(0)]
    Normal,
    #[n(1)]
    Debug,
    #[n(2)]
    Diagnosis,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportRequest {
    #[n(0)]
    pub kind: StateExportKind,
    #[n(1)]
    pub snapshot_purpose: SnapshotExportPurpose,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StateExportResult {
    #[n(0)]
    Ready {
        #[n(0)]
        transfer: StateTransferDescriptor,
    },
    #[n(1)]
    Ineligible {
        #[n(0)]
        reasons: Vec<SnapshotIneligibleReason>,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateImportBegin {
    #[n(0)]
    pub kind: StateExportKind,
    #[n(1)]
    pub total_bytes: u64,
    #[n(2)]
    pub digest: ProtocolBytes,
    #[n(3)]
    pub artifact_id: Option<ProtocolBytes>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateImportAccepted {
    #[n(0)]
    pub transfer_id: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateImportChunk {
    #[n(0)]
    pub transfer_id: u64,
    #[n(1)]
    pub offset: u64,
    #[n(2)]
    pub data: ProtocolBytes,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateImportCommit {
    #[n(0)]
    pub transfer_id: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateImportReady {
    #[n(0)]
    pub transfer_id: u64,
    #[n(1)]
    pub kind: StateExportKind,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportChunkRequest {
    #[n(0)]
    pub transfer_id: u64,
    #[n(1)]
    pub offset: u64,
    #[n(2)]
    pub maximum_bytes: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateExportChunk {
    #[n(0)]
    pub transfer_id: u64,
    #[n(1)]
    pub offset: u64,
    #[n(2)]
    pub data: ProtocolBytes,
    #[n(3)]
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct StateTransferCancel {
    #[n(0)]
    pub transfer_id: u64,
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
    #[n(7)]
    UnsupportedRuntimeFeature,
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
    pub origin: Option<ExecutionOrigin>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ExecutionOrigin {
    #[n(0)]
    pub command: String,
    #[n(1)]
    pub function: String,
    #[n(2)]
    pub generation: u64,
    #[n(3)]
    pub instruction: u32,
    #[n(4)]
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
