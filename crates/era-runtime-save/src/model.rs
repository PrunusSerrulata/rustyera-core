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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Text1808ValueType {
    Integer,
    String,
}

/// One project-defined position in the current text save layout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Text1808Variable {
    pub name: String,
    pub value_type: Text1808ValueType,
    pub dimensions: Vec<u32>,
}

/// Schema-neutral description of the order supplied by the active project.
///
/// Extended groups are explicit because the reference format uses separators,
/// not tags, to distinguish scalar/1D/2D/3D and integer/string dictionaries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Text1808Layout {
    pub kind: SaveFileKind,
    pub base_variables: Vec<Text1808Variable>,
    pub base_character_variables: Vec<Text1808Variable>,
    pub extended_groups: Vec<Vec<Text1808Variable>>,
    pub extended_character_groups: Vec<Vec<Text1808Variable>>,
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

/// Structured representation of the three extension records written by Emuera 1808.
/// Map entries remain ordered because `MAP_GETKEYS` and `MAP_TOXML` make that order observable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SaveExtension {
    Map {
        key: String,
        entries: Vec<(String, String)>,
    },
    Xml {
        key: String,
        document: String,
    },
    DataTable {
        key: String,
        schema: String,
        data: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveDocument {
    pub format: SaveFormat,
    pub kind: SaveFileKind,
    pub metadata: SaveMetadata,
    pub characters: Vec<Vec<SaveEntry>>,
    /// Entry offset of the binary 1813 user-defined section in each character.
    ///
    /// Emuera writes a separator even when the section is empty. Keeping the offset separate
    /// from the entries preserves that case as well as exact binary round trips.
    pub character_user_defined_starts: Vec<Option<usize>>,
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
