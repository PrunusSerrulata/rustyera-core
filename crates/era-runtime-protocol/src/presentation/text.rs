//! Stable localized-system-text protocol records.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

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
