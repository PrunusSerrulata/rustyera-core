mod replay;
mod text;

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{InputWait, InteractionToken};

pub use replay::{
    CanvasReplay, CanvasReplayCommand, PresentationSnapshot, ResourceReplay, SpriteFrameReplay,
    SpriteReplay,
};
pub use text::{SystemTextArgument, SystemTextKey, SystemTextRef};

#[derive(Clone, Copy, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct Color {
    #[n(0)]
    pub red: u8,
    #[n(1)]
    pub green: u8,
    #[n(2)]
    pub blue: u8,
    #[n(3)]
    pub alpha: u8,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
// These are independent font attributes in Emuera's observable console state.
#[allow(clippy::struct_excessive_bools)]
pub struct TextStyle {
    #[n(0)]
    pub foreground: Color,
    #[n(1)]
    pub background: Option<Color>,
    #[n(2)]
    pub bold: bool,
    #[n(3)]
    pub italic: bool,
    #[n(4)]
    pub underline: bool,
    #[n(5)]
    pub strikeout: bool,
    #[n(6)]
    pub font_family: Option<String>,
    /// Font size in 1/1000 logical pixel units; the wire contract contains no floats.
    #[n(7)]
    pub font_millipixels: u32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            foreground: Color {
                red: 192,
                green: 192,
                blue: 192,
                alpha: 255,
            },
            background: None,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            font_family: Some("ＭＳ ゴシック".into()),
            font_millipixels: 18_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum LineAlignment {
    #[n(0)]
    Left,
    #[n(1)]
    Center,
    #[n(2)]
    Right,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum CellAlignment {
    #[n(0)]
    Left,
    #[n(1)]
    Right,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SeparatorRole {
    #[n(0)]
    Rule,
}

/// A signed distance in the runtime-owned Era presentation coordinate space.
///
/// One script-visible logical unit is represented by 1,000 milliunits. This is
/// deliberately not a device pixel; the authoritative frontend applies the
/// negotiated projection transform when rendering it.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(transparent)]
pub struct LogicalLength(#[n(0)] pub i64);

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct LogicalRect {
    #[n(0)]
    pub x: LogicalLength,
    #[n(1)]
    pub y: LogicalLength,
    #[n(2)]
    pub width: LogicalLength,
    #[n(3)]
    pub height: LogicalLength,
}

/// A script shape length before frontend projection.
///
/// The reference syntax treats a value with the `px` suffix as absolute and a
/// suffix-less value as a percentage of the configured font height. Retaining
/// that distinction prevents the canonical model from baking in font metrics.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum PresentationLength {
    #[n(0)]
    Logical(#[n(0)] LogicalLength),
    #[n(1)]
    FontHeightHundredths(#[n(0)] i64),
}

/// Integer coordinates in a runtime-created raster canvas.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasPoint {
    #[n(0)]
    pub x: i32,
    #[n(1)]
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasSize {
    #[n(0)]
    pub width: u32,
    #[n(1)]
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasRect {
    #[n(0)]
    pub x: i32,
    #[n(1)]
    pub y: i32,
    #[n(2)]
    pub width: i32,
    #[n(3)]
    pub height: i32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct MediaPlacement {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub x: LogicalLength,
    #[n(2)]
    pub y: LogicalLength,
    #[n(3)]
    pub width: LogicalLength,
    #[n(4)]
    pub height: LogicalLength,
    #[n(5)]
    pub depth: i64,
    #[n(6)]
    pub opacity: RationalOpacity,
    #[n(7)]
    pub revision: u64,
    #[n(8)]
    pub hover_resource_id: Option<String>,
    #[n(9)]
    pub mask_resource_id: Option<String>,
    #[n(10)]
    pub requested_width: Option<PresentationLength>,
    #[n(11)]
    pub requested_height: Option<PresentationLength>,
    #[n(12)]
    pub requested_y: Option<PresentationLength>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RationalOpacity {
    #[n(0)]
    pub numerator: i64,
    #[n(1)]
    pub denominator: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct Shape {
    #[n(0)]
    pub kind: String,
    #[n(1)]
    pub parameters: Vec<PresentationLength>,
    #[n(2)]
    pub foreground: Option<Color>,
    #[n(3)]
    pub background: Option<Color>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DisplayRun {
    #[n(0)]
    Text {
        #[n(0)]
        text: String,
        #[n(1)]
        style: TextStyle,
        #[n(2)]
        system_text: Option<SystemTextRef>,
    },
    #[n(1)]
    Button {
        #[n(0)]
        runs: Vec<DisplayRun>,
        #[n(1)]
        token: InteractionToken,
        #[n(2)]
        title: Option<String>,
        #[n(3)]
        hover_style: Option<TextStyle>,
        /// Original `EraBasic` value; the interaction token is not serialized as HTML.
        #[n(4)]
        value: crate::ProtocolValue,
        /// BREAKBUTTON generation captured when the button was created.
        #[n(5)]
        generation: u64,
        /// Runtime-owned semantic availability. A frontend may style disabled
        /// buttons, but must never submit them as interactions.
        #[n(6)]
        enabled: bool,
    },
    #[n(2)]
    HtmlDocument {
        #[n(0)]
        document: erabasic_html::HtmlDocument,
    },
    #[n(3)]
    Image {
        #[n(0)]
        placement: MediaPlacement,
        #[n(1)]
        alt_text: Option<String>,
    },
    #[n(4)]
    Shape {
        #[n(0)]
        shape: Shape,
    },
    /// A PRINTC-family layout intent. Runtime state contains no font-dependent padding.
    #[n(5)]
    ColumnCell {
        #[n(0)]
        content: Vec<DisplayRun>,
        #[n(1)]
        alignment: CellAlignment,
        #[n(2)]
        preferred_columns: u32,
    },
    /// A width-independent DRAWLINE intent.
    #[n(6)]
    Separator {
        #[n(0)]
        pattern: String,
        #[n(1)]
        role: SeparatorRole,
        /// The active text style with Emuera's DRAWLINE regular-font normalization.
        #[n(2)]
        #[serde(default)]
        style: TextStyle,
    },
    #[n(7)]
    Space {
        #[n(0)]
        width: PresentationLength,
    },
    /// Frontend projection of canonical text with a runtime-owned project column advance.
    /// This variant is never stored in a runtime snapshot.
    #[n(8)]
    TextLayout {
        #[n(0)]
        text: String,
        #[n(1)]
        style: TextStyle,
        #[n(2)]
        system_text: Option<SystemTextRef>,
        #[n(3)]
        columns: u32,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DisplayLine {
    #[n(0)]
    pub line_id: u64,
    #[n(1)]
    pub temporary: bool,
    #[n(2)]
    pub logical_line_start: bool,
    #[n(3)]
    pub line_end: bool,
    #[n(4)]
    pub alignment: LineAlignment,
    #[n(5)]
    pub runs: Vec<DisplayRun>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PresentationSettings {
    #[n(0)]
    pub drawable_width: LogicalLength,
    #[n(1)]
    pub line_height: LogicalLength,
    #[n(2)]
    pub background: Color,
    #[n(3)]
    pub button_focus_foreground: Color,
    /// Runtime-owned physical history limit. Trimming old rows never changes LINECOUNT.
    #[n(4)]
    pub maximum_physical_lines: u32,
    #[n(5)]
    pub prevent_button_wrap: bool,
    #[n(6)]
    pub legacy_nonbutton_wrap: bool,
    #[n(7)]
    pub drawable_height: LogicalLength,
}

/// Ordered semantic edits from which a frontend derives physical console rows.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PresentationHistoryOperation {
    #[n(0)]
    Append {
        #[n(0)]
        line: DisplayLine,
    },
    #[n(1)]
    DeletePhysical {
        #[n(0)]
        count: u32,
    },
    #[n(2)]
    ReplaceTemporary {
        #[n(0)]
        line: DisplayLine,
    },
    #[n(3)]
    Clear,
    #[n(4)]
    SetButtonGeneration {
        #[n(0)]
        generation: u64,
    },
    /// Discard physical history from the oldest edge after reaching `MaxLog`.
    #[n(5)]
    TrimPhysical {
        #[n(0)]
        count: u32,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PresentationHistory {
    #[n(0)]
    pub logical_lines: Vec<DisplayLine>,
    /// Self-contained replay baseline for the currently retained physical rows. A snapshot may
    /// normalize prior edits to `Append`; this is not an unbounded audit journal.
    #[n(1)]
    pub operations: Vec<PresentationHistoryOperation>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RedrawState {
    #[n(0)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AudioState {
    #[n(0)]
    pub channel_id: u64,
    #[n(1)]
    pub resource_id: String,
    #[n(2)]
    pub repeat_count: i64,
    #[n(3)]
    pub volume_millionths: u32,
    #[n(4)]
    pub playing: bool,
    #[n(5)]
    pub revision: u64,
}

/// Canonical tooltip policy. A frontend may project the font and timing to its
/// platform, but it must not invent different game-visible tooltip contents.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct TooltipSettings {
    #[n(0)]
    pub foreground: Color,
    #[n(1)]
    pub background: Color,
    #[n(2)]
    pub delay_ms: u32,
    #[n(3)]
    pub duration_ms: u32,
    #[n(4)]
    pub font_family: Option<String>,
    #[n(5)]
    pub font_millipoints: u32,
    #[n(6)]
    pub custom: bool,
    #[n(7)]
    pub format: i64,
    #[n(8)]
    pub images: bool,
    /// Portable normalized layout selected by `TOOLTIP_FORMAT`.
    #[n(9)]
    pub normalized_format: TooltipFormat,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct TooltipFormat {
    #[n(0)]
    pub flags: Vec<TooltipFormatFlag>,
    /// Bits not defined by the pinned `TextFormatFlags` contract.
    #[n(1)]
    pub unknown_bits: u64,
}

impl TooltipFormat {
    #[must_use]
    pub fn from_raw(raw: i64) -> Self {
        let bits = u64::from_ne_bytes(raw.to_ne_bytes());
        let definitions = [
            (0x0000_0001_u64, TooltipFormatFlag::HorizontalCenter),
            (0x0000_0002, TooltipFormatFlag::Right),
            (0x0000_0004, TooltipFormatFlag::VerticalCenter),
            (0x0000_0008, TooltipFormatFlag::Bottom),
            (0x0000_0010, TooltipFormatFlag::WordBreak),
            (0x0000_0020, TooltipFormatFlag::SingleLine),
            (0x0000_0040, TooltipFormatFlag::ExpandTabs),
            (0x0000_0100, TooltipFormatFlag::NoClipping),
            (0x0000_0200, TooltipFormatFlag::ExternalLeading),
            (0x0000_0800, TooltipFormatFlag::NoPrefix),
            (0x0000_1000, TooltipFormatFlag::Internal),
            (0x0000_2000, TooltipFormatFlag::TextBoxControl),
            (0x0000_4000, TooltipFormatFlag::PathEllipsis),
            (0x0000_8000, TooltipFormatFlag::EndEllipsis),
            (0x0001_0000, TooltipFormatFlag::ModifyString),
            (0x0002_0000, TooltipFormatFlag::RightToLeft),
            (0x0004_0000, TooltipFormatFlag::WordEllipsis),
            (0x0008_0000, TooltipFormatFlag::NoFullWidthCharacterBreak),
            (0x0010_0000, TooltipFormatFlag::HidePrefix),
            (0x0020_0000, TooltipFormatFlag::PrefixOnly),
            (0x0100_0000, TooltipFormatFlag::PreserveGraphicsClipping),
            (
                0x0200_0000,
                TooltipFormatFlag::PreserveGraphicsTranslateTransform,
            ),
            (0x1000_0000, TooltipFormatFlag::NoPadding),
            (0x2000_0000, TooltipFormatFlag::LeftAndRightPadding),
        ];
        let known_bits = definitions.iter().fold(0_u64, |mask, (bit, _)| mask | bit);
        Self {
            flags: definitions
                .into_iter()
                .filter_map(|(bit, flag)| (bits & bit != 0).then_some(flag))
                .collect(),
            unknown_bits: bits & !known_bits,
        }
    }
}

impl Default for TooltipFormat {
    fn default() -> Self {
        Self::from_raw(0)
    }
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum TooltipFormatFlag {
    #[n(0)]
    HorizontalCenter,
    #[n(1)]
    Right,
    #[n(2)]
    VerticalCenter,
    #[n(3)]
    Bottom,
    #[n(4)]
    WordBreak,
    #[n(5)]
    SingleLine,
    #[n(6)]
    ExpandTabs,
    #[n(7)]
    NoClipping,
    #[n(8)]
    ExternalLeading,
    #[n(9)]
    NoPrefix,
    #[n(10)]
    Internal,
    #[n(11)]
    TextBoxControl,
    #[n(12)]
    PathEllipsis,
    #[n(13)]
    EndEllipsis,
    #[n(14)]
    ModifyString,
    #[n(15)]
    RightToLeft,
    #[n(16)]
    WordEllipsis,
    #[n(17)]
    NoFullWidthCharacterBreak,
    #[n(18)]
    HidePrefix,
    #[n(19)]
    PrefixOnly,
    #[n(20)]
    PreserveGraphicsClipping,
    #[n(21)]
    PreserveGraphicsTranslateTransform,
    #[n(22)]
    NoPadding,
    #[n(23)]
    LeftAndRightPadding,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PresentationOperation {
    #[n(0)]
    AppendLine {
        #[n(0)]
        line: DisplayLine,
    },
    #[n(1)]
    DeleteLines {
        #[n(0)]
        count: u32,
    },
    #[n(2)]
    Clear,
    #[n(3)]
    SetTitle {
        #[n(0)]
        title: String,
    },
    #[n(4)]
    SetBackgrounds {
        #[n(0)]
        backgrounds: Vec<MediaPlacement>,
    },
    #[n(5)]
    SetAudio {
        #[n(0)]
        audio: Vec<AudioState>,
    },
    #[n(6)]
    SetInputWait {
        #[n(0)]
        input_wait: Option<InputWait>,
    },
    #[n(7)]
    ReplaceLine {
        #[n(0)]
        line_id: u64,
        #[n(1)]
        line: DisplayLine,
    },
    #[n(8)]
    SetSettings {
        #[n(0)]
        settings: PresentationSettings,
    },
    #[n(9)]
    SetTooltip {
        #[n(0)]
        tooltip: TooltipSettings,
    },
    #[n(10)]
    SetResources {
        #[n(0)]
        resources: ResourceReplay,
    },
    #[n(11)]
    SetHtmlIsland {
        #[n(0)]
        html_island: Vec<erabasic_html::HtmlDocument>,
    },
    #[n(12)]
    SetRedraw {
        #[n(0)]
        redraw: RedrawState,
    },
    #[n(13)]
    SetButtonGeneration {
        #[n(0)]
        generation: u64,
    },
    /// Discard the oldest projected lines while retaining the logical line counter.
    #[n(14)]
    TrimLines {
        #[n(0)]
        count: u32,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PresentationDelta {
    #[n(0)]
    pub base_revision: u64,
    #[n(1)]
    pub new_revision: u64,
    #[n(2)]
    pub operations: Vec<PresentationOperation>,
}
