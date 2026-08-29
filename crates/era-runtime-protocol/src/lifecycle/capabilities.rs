//! Frontend capability and client-state wire records.

use era_protocol::VersionRange;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{ServiceKind, StorageCapabilities};

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
    #[n(12)]
    pub environment: Vec<crate::EnvironmentCapability>,
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
