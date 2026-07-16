use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::ProtocolValue;

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum AudioEffectAction {
    #[n(0)]
    Play,
    #[n(1)]
    Stop,
    #[n(2)]
    SetVolume,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AudioEffect {
    #[n(0)]
    pub channel_id: u64,
    #[n(1)]
    pub action: AudioEffectAction,
    #[n(2)]
    pub resource_id: Option<String>,
    #[n(3)]
    pub repeat_count: i64,
    #[n(4)]
    pub volume_millionths: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VideoEffect {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub skippable: bool,
}

/// A transient frontend effect. Recoverable scene and audio state stay in the
/// presentation snapshot; an effect is replayed only while it remains journaled.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EffectKind {
    #[n(0)]
    Audio(#[n(0)] AudioEffect),
    #[n(1)]
    StartAnimation(#[n(0)] String),
    #[n(2)]
    Video(#[n(0)] VideoEffect),
    #[n(3)]
    Extension(#[n(0)] String, #[n(1)] ProtocolValue),
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum EffectOutcomeStatus {
    #[n(0)]
    Completed,
    #[n(1)]
    Failed,
    #[n(2)]
    Cancelled,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EffectOutcome {
    #[n(0)]
    pub effect_id: u64,
    #[n(1)]
    pub status: EffectOutcomeStatus,
    #[n(2)]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EffectEvent {
    #[n(0)]
    pub effect_id: u64,
    #[n(1)]
    pub kind: EffectKind,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EffectBatch {
    #[n(0)]
    pub effects: Vec<EffectEvent>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EffectAcknowledgement {
    #[n(0)]
    pub outcomes: Vec<EffectOutcome>,
}
