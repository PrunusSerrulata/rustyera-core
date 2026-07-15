use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{InputWait, ProtocolValue};

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
    pub font_family: Option<String>,
    /// Font size in 1/1000 point units; the wire contract contains no floats.
    #[n(6)]
    pub font_millipoints: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct MediaPlacement {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub x_millipixels: i64,
    #[n(2)]
    pub y_millipixels: i64,
    #[n(3)]
    pub width_millipixels: i64,
    #[n(4)]
    pub height_millipixels: i64,
    #[n(5)]
    pub depth: i64,
    #[n(6)]
    pub opacity_millionths: u32,
    #[n(7)]
    pub revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct Shape {
    #[n(0)]
    pub kind: String,
    #[n(1)]
    pub parameters: Vec<i64>,
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
    },
    #[n(1)]
    Button {
        #[n(0)]
        text: String,
        #[n(1)]
        value: ProtocolValue,
        #[n(2)]
        generation: u64,
        #[n(3)]
        style: TextStyle,
    },
    #[n(2)]
    Html {
        #[n(0)]
        markup: String,
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
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DisplayLine {
    #[n(0)]
    pub line_id: u64,
    #[n(1)]
    pub temporary: bool,
    #[n(2)]
    pub runs: Vec<DisplayRun>,
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
