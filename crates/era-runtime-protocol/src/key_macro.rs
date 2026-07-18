use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::FilePayload;

pub const KEY_MACRO_GROUPS: usize = 10;
pub const KEY_MACRO_SLOTS: usize = 12;

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct KeyMacroProfileSubmit {
    #[n(0)]
    pub relative_path: String,
    #[n(1)]
    pub payload: FilePayload,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum KeyMacroCommand {
    #[n(0)]
    SelectGroup(#[n(0)] u8),
    #[n(1)]
    Store {
        #[n(0)]
        group: u8,
        #[n(1)]
        slot: u8,
        #[n(2)]
        text: String,
    },
    #[n(2)]
    Clear {
        #[n(0)]
        group: u8,
        #[n(1)]
        slot: u8,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct KeyMacroState {
    #[n(0)]
    pub enabled: bool,
    #[n(1)]
    pub selected_group: u8,
    #[n(2)]
    pub group_names: Vec<String>,
    /// Group-major, exactly 120 entries.
    #[n(3)]
    pub entries: Vec<String>,
    /// Canonical UTF-8 Japanese-format macro.txt content for frontend persistence.
    #[n(4)]
    pub serialized: String,
}
