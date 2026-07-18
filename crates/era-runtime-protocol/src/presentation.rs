use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{InputWait, InteractionToken};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SystemTextKey {
    #[n(0)]
    InvalidValue,
    #[n(1)]
    SaveQuestion,
    #[n(2)]
    LoadQuestion,
    #[n(3)]
    OverwriteQuestion,
    #[n(4)]
    NotEnoughMoney,
    #[n(5)]
    OutOfStock,
    #[n(6)]
    AutoSaveFailed,
    #[n(7)]
    AutoSaveSkipped,
    #[n(8)]
    PressAnyKey,
    #[n(9)]
    SaveSlot,
    #[n(10)]
    Back,
    #[n(11)]
    NewGame,
    #[n(12)]
    LoadGame,
    #[n(13)]
    ContinuousTrainProgress,
    #[n(14)]
    ContinuousTrainCommandFailed,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SystemTextArgument {
    #[n(0)]
    Integer(#[n(0)] i64),
    #[n(1)]
    String(#[n(0)] String),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SystemTextRef {
    #[n(0)]
    pub key: SystemTextKey,
    #[n(1)]
    pub arguments: Vec<SystemTextArgument>,
}

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
    /// Font size in 1/1000 point units; the wire contract contains no floats.
    #[n(7)]
    pub font_millipoints: u32,
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
    pub opacity_millionths: u32,
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
    },
    #[n(7)]
    Space {
        #[n(0)]
        width: PresentationLength,
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
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SpriteFrameReplay {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub source_rectangle: [i32; 4],
    #[n(2)]
    pub offset: [i32; 2],
    #[n(3)]
    pub delay_ms: u32,
    #[n(4)]
    pub destination_size: Option<[u32; 2]>,
    /// Runtime-created animation frames can reference a replay canvas instead of a file resource.
    #[n(5)]
    pub canvas_id: Option<i64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SpriteReplay {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub size: [u32; 2],
    #[n(2)]
    pub position: [i32; 2],
    #[n(3)]
    pub frames: Vec<SpriteFrameReplay>,
    #[n(4)]
    pub canvas_id: Option<i64>,
    #[n(5)]
    pub canvas_rectangle: Option<CanvasRect>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanvasReplayCommand {
    #[n(0)]
    Clear {
        #[n(0)]
        argb: u32,
        #[n(1)]
        rectangle: Option<CanvasRect>,
    },
    #[n(1)]
    DrawSprite {
        #[n(0)]
        name: String,
        #[n(1)]
        destination: CanvasRect,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasReplay {
    #[n(0)]
    pub canvas_id: i64,
    #[n(1)]
    pub size: CanvasSize,
    #[n(2)]
    pub commands: Vec<CanvasReplayCommand>,
    #[n(3)]
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResourceReplay {
    #[n(0)]
    pub sprites: Vec<SpriteReplay>,
    #[n(1)]
    pub canvases: Vec<CanvasReplay>,
    /// Canonical redraw cadence selected by SETANIMETIMER. Frontends schedule rendering from
    /// this value but never advance game time or choose animation frames for the runtime.
    #[n(2)]
    pub animation_timer_ms: i32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PresentationSnapshot {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub lines: Vec<DisplayLine>,
    #[n(3)]
    pub backgrounds: Vec<MediaPlacement>,
    #[n(4)]
    pub audio: Vec<AudioState>,
    #[n(5)]
    pub input_wait: Option<InputWait>,
    #[n(6)]
    pub settings: PresentationSettings,
    #[n(7)]
    pub tooltip: TooltipSettings,
    #[n(8)]
    pub resources: ResourceReplay,
    /// Independent top-layer HTML documents, in script insertion order.
    #[n(9)]
    pub html_island: Vec<erabasic_html::HtmlDocument>,
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
