use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SaveFormat {
    Text1808,
    Binary1808,
    Binary1808Gzip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SaveFileKind {
    Normal = 0,
    Global = 1,
    Variable = 2,
    Character = 3,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveMetadata {
    pub unique_code: i64,
    pub version: i64,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SaveValue {
    Integer(i64),
    String(String),
    Integers {
        dimensions: Vec<u32>,
        values: Vec<i64>,
    },
    Strings {
        dimensions: Vec<u32>,
        values: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveEntry {
    pub name: String,
    pub value: SaveValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpaqueSaveExtension {
    pub type_tag: u8,
    pub key: String,
    /// Exact encoded value bytes, excluding the type tag and key.
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveDocument {
    pub format: SaveFormat,
    pub kind: SaveFileKind,
    pub metadata: SaveMetadata,
    pub characters: Vec<Vec<SaveEntry>>,
    pub variables: Vec<SaveEntry>,
    pub opaque_extensions: Vec<OpaqueSaveExtension>,
    /// Current text saves retain their positional eramaker prefix losslessly.
    pub text_payload: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveCodecLimits {
    pub maximum_bytes: usize,
    pub maximum_entries: usize,
    pub maximum_characters: usize,
    pub maximum_elements: usize,
    pub maximum_string_bytes: usize,
}

impl Default for SaveCodecLimits {
    fn default() -> Self {
        Self {
            maximum_bytes: 16 * 1024 * 1024,
            maximum_entries: 100_000,
            maximum_characters: 10_000,
            maximum_elements: 10_000_000,
            maximum_string_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveCodecError {
    InvalidHeader,
    UnsupportedVersion(u32),
    InvalidFormat(String),
    LimitExceeded(&'static str),
    Compression(String),
}

impl std::fmt::Display for SaveCodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader => formatter.write_str("invalid save header"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported save version {version}")
            }
            Self::InvalidFormat(message) | Self::Compression(message) => {
                formatter.write_str(message)
            }
            Self::LimitExceeded(limit) => write!(formatter, "save exceeds {limit}"),
        }
    }
}

impl std::error::Error for SaveCodecError {}
