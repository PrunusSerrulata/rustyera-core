use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// A major/minor protocol version. Major changes are incompatible; minor changes
/// may only add optional fields or capabilities negotiated by both peers.
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
pub struct ProtocolVersion {
    #[n(0)]
    pub major: u16,
    #[n(1)]
    pub minor: u16,
}

impl ProtocolVersion {
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct VersionRange {
    #[n(0)]
    pub minimum: ProtocolVersion,
    #[n(1)]
    pub maximum: ProtocolVersion,
}

impl VersionRange {
    #[must_use]
    pub const fn exact(version: ProtocolVersion) -> Self {
        Self {
            minimum: version,
            maximum: version,
        }
    }
}

/// Select the highest mutually supported minor version within one major line.
#[must_use]
pub fn negotiate_version(left: VersionRange, right: VersionRange) -> Option<ProtocolVersion> {
    if left.minimum.major != left.maximum.major
        || right.minimum.major != right.maximum.major
        || left.minimum.major != right.minimum.major
    {
        return None;
    }
    let minimum = left.minimum.minor.max(right.minimum.minor);
    let maximum = left.maximum.minor.min(right.maximum.minor);
    (minimum <= maximum).then(|| ProtocolVersion::new(left.minimum.major, maximum))
}
