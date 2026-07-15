use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::ProtocolValue;

/// A transient frontend effect. Recoverable scene and audio state stay in the
/// presentation snapshot; an effect is replayed only while it remains journaled.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EffectKind {
    #[n(0)]
    PlaySound(#[n(0)] String),
    #[n(1)]
    StartAnimation(#[n(0)] String),
    #[n(2)]
    PlayVideo(#[n(0)] String),
    #[n(3)]
    Extension(#[n(0)] String, #[n(1)] ProtocolValue),
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

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct EffectAcknowledgement {
    #[n(0)]
    pub through_effect_id: u64,
}
