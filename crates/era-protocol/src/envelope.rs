use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{ProtocolBytes, ProtocolError, ProtocolErrorCode, ProtocolVersion, WIRE_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireLimits {
    pub maximum_envelope_bytes: usize,
    pub maximum_payload_bytes: usize,
}

impl Default for WireLimits {
    fn default() -> Self {
        Self {
            maximum_envelope_bytes: 16 * 1024 * 1024,
            maximum_payload_bytes: 15 * 1024 * 1024,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    #[n(0)]
    Runtime,
    #[n(1)]
    Debug,
}

/// Encode an envelope after applying negotiated transport limits.
///
/// # Errors
///
/// Returns an error for invalid fields, an oversized payload or an oversized result.
pub fn encode_envelope(envelope: &Envelope, limits: WireLimits) -> Result<Vec<u8>, ProtocolError> {
    envelope.validate()?;
    if envelope.payload.0.len() > limits.maximum_payload_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorCode::PayloadTooLarge,
            "envelope payload exceeds the negotiated limit",
        ));
    }
    let bytes = crate::encode_canonical(envelope)?;
    if bytes.len() > limits.maximum_envelope_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorCode::EnvelopeTooLarge,
            "encoded envelope exceeds the negotiated limit",
        ));
    }
    Ok(bytes)
}

/// Decode a deterministic envelope without allocating a payload larger than the
/// caller-advertised envelope limit.
///
/// # Errors
///
/// Returns an error for limits, malformed CBOR or invalid envelope invariants.
pub fn decode_envelope(bytes: &[u8], limits: WireLimits) -> Result<Envelope, ProtocolError> {
    if bytes.len() > limits.maximum_envelope_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorCode::EnvelopeTooLarge,
            "encoded envelope exceeds the negotiated limit",
        ));
    }
    let envelope: Envelope = crate::decode_canonical(bytes)?;
    envelope.validate()?;
    if envelope.payload.0.len() > limits.maximum_payload_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorCode::PayloadTooLarge,
            "envelope payload exceeds the negotiated limit",
        ));
    }
    Ok(envelope)
}

/// A stable 128-bit session identity represented without platform-sized integers.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Decode,
    Encode,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
)]
#[cbor(map)]
pub struct SessionId {
    #[n(0)]
    pub high: u64,
    #[n(1)]
    pub low: u64,
}

/// Identifies one authoritative game timeline within a session.
///
/// Restores, new games and committed code replacement advance this value so a
/// delayed input or service response can never affect the replacement timeline.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Decode,
    Encode,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
)]
#[cbor(transparent)]
pub struct SessionEpoch(#[n(0)] pub u64);

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct Envelope {
    #[n(0)]
    pub wire_version: ProtocolVersion,
    #[n(1)]
    pub channel_version: ProtocolVersion,
    #[n(2)]
    pub channel: Channel,
    #[n(3)]
    pub session: Option<SessionId>,
    #[n(4)]
    pub sequence: u64,
    #[n(5)]
    pub message_id: u64,
    #[n(6)]
    pub correlation_id: Option<u64>,
    #[n(7)]
    pub payload_tag: u32,
    #[n(8)]
    pub payload: ProtocolBytes,
    #[n(9)]
    pub session_epoch: Option<SessionEpoch>,
}

impl Envelope {
    #[must_use]
    pub fn new(
        channel: Channel,
        channel_version: ProtocolVersion,
        sequence: u64,
        message_id: u64,
        payload_tag: u32,
        payload: ProtocolBytes,
    ) -> Self {
        Self {
            wire_version: WIRE_VERSION,
            channel_version,
            channel,
            session: None,
            sequence,
            message_id,
            correlation_id: None,
            payload_tag,
            payload,
            session_epoch: None,
        }
    }

    /// Validate invariants that do not depend on a session state machine.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported wire versions or zero message identities.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.wire_version.major != WIRE_VERSION.major {
            return Err(ProtocolError::new(
                ProtocolErrorCode::VersionMismatch,
                "unsupported common envelope major version",
            ));
        }
        if self.message_id == 0 {
            return Err(ProtocolError::new(
                ProtocolErrorCode::InvalidIdentifier,
                "message_id zero is reserved",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn json_projection(&self) -> EnvelopeJsonProjection {
        EnvelopeJsonProjection {
            wire_version: self.wire_version,
            channel_version: self.channel_version,
            channel: self.channel,
            session: self.session,
            sequence: self.sequence,
            message_id: self.message_id,
            correlation_id: self.correlation_id,
            payload_tag: self.payload_tag,
            payload_hex: hex(&self.payload.0),
            session_epoch: self.session_epoch,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeJsonProjection {
    pub wire_version: ProtocolVersion,
    pub channel_version: ProtocolVersion,
    pub channel: Channel,
    pub session: Option<SessionId>,
    pub sequence: u64,
    pub message_id: u64,
    pub correlation_id: Option<u64>,
    pub payload_tag: u32,
    pub payload_hex: String,
    pub session_epoch: Option<SessionEpoch>,
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}
