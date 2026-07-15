//! Transport-neutral primitives shared by the runtime and debugger protocols.
//!
//! This crate is an interface contract, not a runtime implementation. Wire messages
//! use deterministic CBOR so the same envelope can cross the in-process C ABI today
//! and a framed client/server transport in the future.

mod bytes;
mod codec;
mod envelope;
mod error;
mod version;

pub use bytes::ProtocolBytes;
pub use codec::{decode_canonical, encode_canonical};
pub use envelope::{
    Channel, Envelope, EnvelopeJsonProjection, SessionId, WireLimits, decode_envelope,
    encode_envelope,
};
pub use error::{ProtocolError, ProtocolErrorCode};
pub use version::{ProtocolVersion, VersionRange, negotiate_version};

/// Version of the common envelope layout.
pub const WIRE_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);
