use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

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
