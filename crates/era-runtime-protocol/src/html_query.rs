//! HTML query v2 measures core-authored trees; script-visible slicing remains in core.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{HtmlDocument, PresentationSettings, ProjectionQueryContext, ServiceError, TextStyle};

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlMeasureRequestV2 {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub style: HtmlQueryStyleV2,
    #[n(2)]
    pub probes: Vec<HtmlMeasureProbeV2>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlQueryStyleV2 {
    #[n(0)]
    pub current: TextStyle,
    #[n(1)]
    pub base: TextStyle,
    #[n(2)]
    pub settings: PresentationSettings,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlMeasureProbeV2 {
    #[n(0)]
    pub id: u32,
    #[n(1)]
    pub document: HtmlDocument,
    #[n(2)]
    pub mode: HtmlProbeModeV2,
    #[n(3)]
    pub cuts: Vec<HtmlProbeCutV2>,
    /// Core-authored reference `AltText`, used only when the snapshot has no image sprite.
    /// Required for `ImageSlot`, absent for other modes.
    #[n(4)]
    pub missing_document: Option<HtmlDocument>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlProbeModeV2 {
    /// One styled text part, independently shaped at every requested prefix.
    #[n(0)]
    TextPart,
    /// Resolve a declared sprite's destination-base size, or measure core's missing fallback.
    #[n(1)]
    ImageSlot,
    /// Validate renderer readiness only. Core computes the shape/division layout slot.
    #[n(2)]
    FixedSlot,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlProbeCutV2 {
    #[n(0)]
    pub id: u32,
    /// Indices from document.nodes through Element.children to a Text node.
    #[n(1)]
    pub text_node_path: Vec<u32>,
    #[n(2)]
    pub decoded_utf8_offset: u32,
    #[n(3)]
    pub decoded_utf16_offset: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlMeasureResponseV2 {
    #[n(0)]
    pub context: ProjectionQueryContext,
    #[n(1)]
    pub probes: Vec<HtmlProbeResponseV2>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlProbeResponseV2 {
    #[n(0)]
    pub id: u32,
    #[n(1)]
    pub result: HtmlProbeResultV2,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HtmlProbeResultV2 {
    #[n(0)]
    TextMeasured {
        /// Thousandths of a CSS logical pixel. This does not change `ProjectionLength`.
        #[n(0)]
        advance_millipixels: i64,
        #[n(1)]
        cuts: Vec<HtmlCutAdvanceV2>,
    },
    /// A later probe failure must not invalidate a cut already chosen by core.
    #[n(1)]
    Error {
        #[n(0)]
        error: ServiceError,
    },
    #[n(2)]
    ImageLoaded {
        /// Sprite destination-base dimensions, not atlas source dimensions or DOM bounds.
        #[n(0)]
        natural_width: u32,
        #[n(1)]
        natural_height: u32,
    },
    #[n(3)]
    ImageMissing {
        /// Only a sprite absent from the request's resource snapshot can use this fallback.
        /// Permission/hash/decode failures for declared resources remain errors.
        #[n(0)]
        fallback_advance_millipixels: i64,
    },
    #[n(4)]
    FixedReady,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlCutAdvanceV2 {
    #[n(0)]
    pub id: u32,
    /// Thousandths of a CSS logical pixel, before core's reference rounding.
    #[n(1)]
    pub advance_millipixels: i64,
}
