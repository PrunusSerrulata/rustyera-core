use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// The fixed sound-channel count exposed by the snake Emuera audio contract.
pub const AUDIO_SOUND_CHANNEL_COUNT: u8 = 10;

/// A stable audio target. Sound channels are numbered 0 through 9; BGM has its own target.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "channel", rename_all = "snake_case")]
pub enum AudioChannelV1 {
    #[n(0)]
    Sound(#[n(0)] u8),
    #[n(1)]
    Bgm,
}

impl AudioChannelV1 {
    /// Construct a sound target while enforcing the public 0..=9 channel range.
    #[must_use]
    pub const fn sound(channel: u8) -> Option<Self> {
        if channel < AUDIO_SOUND_CHANNEL_COUNT {
            Some(Self::Sound(channel))
        } else {
            None
        }
    }

    /// Whether this decoded value is within the protocol's fixed channel range.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        match self {
            Self::Sound(channel) => channel < AUDIO_SOUND_CHANNEL_COUNT,
            Self::Bgm => true,
        }
    }
}

/// Actual state observed from a frontend-owned audio provider.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum AudioPlaybackStateV1 {
    #[n(0)]
    Stopped,
    #[n(1)]
    Playing,
    #[n(2)]
    Paused,
}

/// Query one exact provider channel at the revision expected by the runtime.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AudioObservationRequestV1 {
    #[n(0)]
    pub channel: AudioChannelV1,
    #[n(1)]
    pub expected_revision: u64,
}

/// A revision-bound observation of frontend-owned playback state.
///
/// Duration and position use integer milliseconds, matching the snake ERB API. Volume and rate
/// use millionths so the wire format remains deterministic and does not depend on floating-point
/// serialization. The runtime rejects a response whose channel or revision differs from the
/// request; `frontend_monotonic_time_ns` only orders observations and never advances game time.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AudioObservationResponseV1 {
    #[n(0)]
    pub channel: AudioChannelV1,
    #[n(1)]
    pub revision: u64,
    #[n(2)]
    pub duration_ms: u64,
    #[n(3)]
    pub position_ms: u64,
    #[n(4)]
    pub state: AudioPlaybackStateV1,
    #[n(5)]
    pub volume_millionths: u32,
    #[n(6)]
    pub rate_millionths: u32,
    #[n(7)]
    pub preserve_pitch: bool,
    #[n(8)]
    pub frontend_monotonic_time_ns: u64,
}

impl AudioObservationResponseV1 {
    /// Whether this response belongs to the exact request generation.
    #[must_use]
    pub fn is_fresh_for(self, request: AudioObservationRequestV1) -> bool {
        self.channel.is_valid()
            && request.channel.is_valid()
            && self.channel == request.channel
            && self.revision == request.expected_revision
    }
}
