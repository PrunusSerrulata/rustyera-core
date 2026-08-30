use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlElementKind {
    #[n(0)]
    Bold,
    #[n(1)]
    Italic,
    #[n(2)]
    Underline,
    #[n(3)]
    Strike,
    #[n(4)]
    Font,
    #[n(5)]
    Paragraph,
    #[n(6)]
    NoBreak,
    #[n(7)]
    Button,
    #[n(8)]
    NonButton,
    #[n(9)]
    ClearButton,
    #[n(10)]
    Image,
    #[n(11)]
    Shape,
    #[n(12)]
    Division,
    #[n(13)]
    Break,
}

impl HtmlElementKind {
    pub(super) fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "b" => Self::Bold,
            "i" => Self::Italic,
            "u" => Self::Underline,
            "s" => Self::Strike,
            "font" => Self::Font,
            "p" => Self::Paragraph,
            "nobr" => Self::NoBreak,
            "button" => Self::Button,
            "nonbutton" => Self::NonButton,
            "clearbutton" => Self::ClearButton,
            "img" => Self::Image,
            "shape" => Self::Shape,
            "div" => Self::Division,
            "br" => Self::Break,
            _ => return None,
        })
    }

    pub(super) const fn is_void(self) -> bool {
        matches!(self, Self::Break | Self::Image | Self::Shape)
    }

    /// Return the source-level tag name used by the Emuera console dialect.
    #[must_use]
    pub const fn tag_name(self) -> &'static str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strike => "s",
            Self::Font => "font",
            Self::Paragraph => "p",
            Self::NoBreak => "nobr",
            Self::Button => "button",
            Self::NonButton => "nonbutton",
            Self::ClearButton => "clearbutton",
            Self::Image => "img",
            Self::Shape => "shape",
            Self::Division => "div",
            Self::Break => "br",
        }
    }
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlAttribute {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlInteraction {
    #[n(0)]
    pub epoch: u64,
    #[n(1)]
    pub id: u64,
    #[n(2)]
    pub integer_value: Option<i64>,
    #[n(3)]
    pub string_value: Option<String>,
    #[n(4)]
    pub generation: u64,
    #[n(5)]
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum HtmlLength {
    #[n(0)]
    Pixels(#[n(0)] i32),
    #[n(1)]
    FontHeightHundredths(#[n(0)] i32),
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlAlignment {
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
pub enum HtmlDisplayMode {
    #[n(0)]
    Relative,
    #[n(1)]
    Absolute,
    #[n(2)]
    AbsoluteLeftTop,
    #[n(3)]
    AbsoluteLeftBottom,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlVerticalAlignment {
    #[n(0)]
    Top,
    #[n(1)]
    Middle,
    #[n(2)]
    Bottom,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlTextRenderer {
    #[n(0)]
    Gdi,
    #[n(1)]
    Skia,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlFontEdging {
    #[n(0)]
    Alias,
    #[n(1)]
    AntiAlias,
    #[n(2)]
    SubpixelAntiAlias,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlFontHinting {
    #[n(0)]
    None,
    #[n(1)]
    Slight,
    #[n(2)]
    Normal,
    #[n(3)]
    Full,
}

#[derive(Clone, Copy, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlTextRenderIntent {
    #[n(0)]
    pub renderer: Option<HtmlTextRenderer>,
    #[n(1)]
    pub edging: Option<HtmlFontEdging>,
    #[n(2)]
    pub hinting: Option<HtmlFontHinting>,
}

/// Canonical color-matrix intent carried by an image node.
///
/// Parsing produces a validated variable address. The runtime replaces it with
/// fixed 1/256 values before the document crosses the presentation boundary.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HtmlColorMatrix {
    #[n(0)]
    Variable {
        #[n(0)]
        name: String,
        #[n(1)]
        indices: [u64; 3],
    },
    #[n(1)]
    Fixed(#[n(0)] Box<[i64; 25]>),
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlBoxModel {
    #[n(0)]
    pub border: Option<[HtmlLength; 4]>,
    #[n(1)]
    pub radius: Option<[HtmlLength; 4]>,
    #[n(2)]
    pub margin: Option<[HtmlLength; 4]>,
    #[n(3)]
    pub padding: Option<[HtmlLength; 4]>,
    #[n(4)]
    pub border_colors: Option<[u32; 4]>,
}

/// Typed, renderer-independent meaning of every accepted Emuera tag.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HtmlElementSemantic {
    #[n(0)]
    Style,
    #[n(1)]
    Font {
        #[n(0)]
        face: Option<String>,
        #[n(1)]
        color: Option<u32>,
        #[n(2)]
        button_color: Option<u32>,
        /// Requested size in 1/1000 logical pixel units.
        #[n(3)]
        size_millipixels: Option<u32>,
        #[n(4)]
        vertical_alignment: Option<HtmlVerticalAlignment>,
        #[n(5)]
        render_intent: HtmlTextRenderIntent,
    },
    #[n(2)]
    Paragraph {
        #[n(0)]
        alignment: HtmlAlignment,
    },
    #[n(3)]
    NoBreak,
    #[n(4)]
    Button {
        #[n(0)]
        value: Option<String>,
        #[n(1)]
        title: Option<String>,
        #[n(2)]
        position: Option<i32>,
    },
    #[n(5)]
    NonButton {
        #[n(0)]
        title: Option<String>,
        #[n(1)]
        position: Option<i32>,
    },
    #[n(6)]
    ClearButton {
        #[n(0)]
        suppress_tooltip: bool,
    },
    #[n(7)]
    Image {
        #[n(0)]
        source: String,
        #[n(1)]
        hover_source: Option<String>,
        #[n(2)]
        mask_source: Option<String>,
        #[n(3)]
        height: Option<HtmlLength>,
        #[n(4)]
        width: Option<HtmlLength>,
        #[n(5)]
        y: Option<HtmlLength>,
        #[n(6)]
        x: Option<HtmlLength>,
        #[n(7)]
        display: HtmlDisplayMode,
        /// Canonical variable address or runtime-resolved fixed-point matrix.
        #[n(8)]
        color_matrix: Option<HtmlColorMatrix>,
    },
    #[n(8)]
    Shape {
        #[n(0)]
        kind: String,
        #[n(1)]
        parameters: Vec<HtmlLength>,
        #[n(2)]
        color: Option<u32>,
        #[n(3)]
        button_color: Option<u32>,
    },
    #[n(9)]
    Division {
        #[n(0)]
        x: Option<HtmlLength>,
        #[n(1)]
        y: Option<HtmlLength>,
        #[n(2)]
        width: HtmlLength,
        #[n(3)]
        height: Option<HtmlLength>,
        #[n(4)]
        depth: i32,
        #[n(5)]
        color: Option<u32>,
        #[n(6)]
        display: HtmlDisplayMode,
        #[n(7)]
        box_model: HtmlBoxModel,
    },
    #[n(10)]
    Break,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// Keeping the normalized semantic value inline makes the public AST ergonomic
// and avoids exposing an allocation detail in the runtime protocol.
#[allow(clippy::large_enum_variant)]
pub enum HtmlNode {
    #[n(0)]
    Text {
        #[n(0)]
        text: String,
        #[n(1)]
        start: u64,
        #[n(2)]
        end: u64,
    },
    #[n(1)]
    Element {
        #[n(0)]
        kind: HtmlElementKind,
        #[n(1)]
        attributes: Vec<HtmlAttribute>,
        #[n(2)]
        children: Vec<HtmlNode>,
        #[n(3)]
        interaction: Option<HtmlInteraction>,
        #[n(4)]
        start: u64,
        #[n(5)]
        end: u64,
        #[n(6)]
        semantic: HtmlElementSemantic,
    },
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlDocument {
    #[n(0)]
    pub nodes: Vec<HtmlNode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlErrorKind {
    UnterminatedTag,
    UnknownTag,
    UnexpectedClosingTag,
    MismatchedClosingTag,
    InvalidAttribute,
    DuplicateAttribute,
    InvalidEntity,
    MissingAttribute,
    InvalidAttributeValue,
    InvalidNesting,
}

#[derive(Clone, Eq, PartialEq)]
pub struct HtmlError {
    pub kind: HtmlErrorKind,
    pub start: usize,
    pub end: usize,
    pub(crate) origin: super::query::HtmlQueryErrorOrigin,
}

// Provenance is routing metadata, not a change to existing debug diagnostics.
#[allow(clippy::missing_fields_in_debug)] // Preserve existing public diagnostic text.
impl std::fmt::Debug for HtmlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HtmlError")
            .field("kind", &self.kind)
            .field("start", &self.start)
            .field("end", &self.end)
            .finish()
    }
}

impl HtmlError {
    /// Unclassified callers cannot assert trusted source-input provenance.
    #[must_use]
    pub const fn new(kind: HtmlErrorKind, start: usize, end: usize) -> Self {
        Self {
            kind,
            start,
            end,
            origin: super::query::HtmlQueryErrorOrigin::NonScript,
        }
    }

    #[must_use]
    pub const fn origin(&self) -> super::query::HtmlQueryErrorOrigin {
        self.origin
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlWarningKind {
    CrossedClosingTag,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlWarning {
    pub kind: HtmlWarningKind,
    /// Start of the non-standard closing tag in the input markup, as a UTF-8 byte offset.
    pub start: usize,
    /// End of the non-standard closing tag in the input markup, as a UTF-8 byte offset.
    pub end: usize,
    pub closing: HtmlElementKind,
    pub crossed: Vec<HtmlElementKind>,
}
