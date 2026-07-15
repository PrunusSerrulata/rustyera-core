use std::fmt;

use minicbor::{Decode, Decoder, Encode, Encoder, decode, encode};
use serde::{Deserialize, Serialize};

/// Owned bytes encoded as one CBOR byte string rather than an array of integers.
#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProtocolBytes(pub Vec<u8>);

impl ProtocolBytes {
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for ProtocolBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ProtocolBytes(len={})", self.0.len())
    }
}

impl<C> Encode<C> for ProtocolBytes {
    fn encode<W: encode::Write>(
        &self,
        encoder: &mut Encoder<W>,
        _context: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        encoder.bytes(&self.0)?;
        Ok(())
    }
}

impl<'bytes, C> Decode<'bytes, C> for ProtocolBytes {
    fn decode(decoder: &mut Decoder<'bytes>, _context: &mut C) -> Result<Self, decode::Error> {
        Ok(Self(decoder.bytes()?.to_vec()))
    }
}
