//! Read-only migration support around the interoperable Emuera 1808 wire format.

use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};

use crate::{SaveCodecError, SaveCodecLimits, SaveMetadataInspection, inspect_metadata};

const MAGIC: &[u8; 8] = b"RERASAV\0";
const VERSION: u32 = 2;
const HEADER_BYTES: usize = 64;
const MAXIMUM_IDENTITY_BYTES: usize = 4096;
const CHECKSUM_DOMAIN: &[u8] = b"rustyera.rerasav.envelope.v2\0";

fn invalid(message: impl Into<String>) -> SaveCodecError {
    SaveCodecError::InvalidFormat(message.into())
}

fn envelope_checksum(bytes: &[u8]) -> Result<blake3::Hash, SaveCodecError> {
    if bytes.len() < HEADER_BYTES {
        return Err(invalid("truncated save identity header"));
    }
    let mut checksum = blake3::Hasher::new();
    checksum.update(CHECKSUM_DOMAIN);
    checksum.update(&bytes[..32]);
    checksum.update(&bytes[HEADER_BYTES..]);
    Ok(checksum.finalize())
}

struct EnvelopePrefix<'a> {
    identity: CompatibilityIdentity,
    state: &'a [u8],
    payload: &'a [u8],
    expected_state_bytes: usize,
    expected_payload_bytes: usize,
    checksum: &'a [u8],
}

fn envelope_prefix(
    bytes: &[u8],
    complete: bool,
    limits: SaveCodecLimits,
) -> Result<Option<EnvelopePrefix<'_>>, SaveCodecError> {
    if bytes.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    if bytes.len() < HEADER_BYTES {
        return if complete {
            Err(invalid("truncated save identity header"))
        } else {
            Ok(None)
        };
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed header"));
    if version != VERSION {
        return Err(invalid(format!(
            "unsupported save envelope version {version}"
        )));
    }
    let identity_bytes =
        u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header")) as usize;
    let state_bytes = usize::try_from(u64::from_le_bytes(
        bytes[16..24].try_into().expect("fixed header"),
    ))
    .map_err(|_| SaveCodecError::LimitExceeded("maximum bytes"))?;
    let payload_bytes = usize::try_from(u64::from_le_bytes(
        bytes[24..32].try_into().expect("fixed header"),
    ))
    .map_err(|_| SaveCodecError::LimitExceeded("maximum bytes"))?;
    let start = HEADER_BYTES
        .checked_add(identity_bytes)
        .ok_or(SaveCodecError::LimitExceeded("maximum bytes"))?;
    let payload_start = start
        .checked_add(state_bytes)
        .ok_or(SaveCodecError::LimitExceeded("maximum bytes"))?;
    let total = payload_start
        .checked_add(payload_bytes)
        .ok_or(SaveCodecError::LimitExceeded("maximum bytes"))?;
    if identity_bytes > MAXIMUM_IDENTITY_BYTES || total > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    if bytes.len() > total || (complete && bytes.len() != total) {
        return Err(invalid("save envelope length differs"));
    }
    if bytes.len() < start {
        return if complete {
            Err(invalid("truncated save identity"))
        } else {
            Ok(None)
        };
    }
    let mut decoder = minicbor::Decoder::new(&bytes[HEADER_BYTES..start]);
    let received: CompatibilityIdentity = decoder
        .decode()
        .map_err(|error| invalid(error.to_string()))?;
    if decoder.position() != identity_bytes {
        return Err(invalid("trailing data in save identity"));
    }
    let canonical = minicbor::to_vec(&received).map_err(|error| invalid(error.to_string()))?;
    if canonical.as_slice() != &bytes[HEADER_BYTES..start] {
        return Err(invalid("save identity is not encoded canonically"));
    }
    Ok(Some(EnvelopePrefix {
        identity: received,
        state: &bytes[start..payload_start.min(bytes.len())],
        payload: &bytes[payload_start.min(bytes.len())..],
        expected_state_bytes: state_bytes,
        expected_payload_bytes: payload_bytes,
        checksum: &bytes[32..HEADER_BYTES],
    }))
}

/// Validated interoperable bytes or exact read-only legacy migration envelope contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibleSaveEnvelope<'a> {
    pub state: &'a [u8],
    pub payload: &'a [u8],
    pub source: CompatibleSaveSource,
}

/// Origin of bytes accepted by the compatibility boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibleSaveSource {
    /// Bare Emuera 1808 bytes. The envelope boundary is shared; profile-specific Text layouts
    /// are selected later by the runtime adapter.
    Interoperable1808,
    /// Exact snake identity 11 `RERASAV` v2 input retained for one-way migration.
    LegacySnakeOwnedV11,
}

/// Validate ownership and checksum before exposing both runtime state and legacy payload.
///
/// # Errors
/// Accepts bare Emuera 1808 bytes for current profiles and the exact snake-v11 migration
/// envelope in a current snake session. Every other envelope identity is rejected.
pub fn unwrap_compatible_envelope<'a>(
    bytes: &'a [u8],
    identity: &CompatibilityIdentity,
    limits: SaveCodecLimits,
) -> Result<CompatibleSaveEnvelope<'a>, SaveCodecError> {
    identity
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    if !bytes.starts_with(MAGIC) {
        return Ok(CompatibleSaveEnvelope {
            state: &[],
            payload: bytes,
            source: CompatibleSaveSource::Interoperable1808,
        });
    }
    let prefix =
        envelope_prefix(bytes, true, limits)?.ok_or_else(|| invalid("truncated save envelope"))?;
    if identity.profile != CompatibilityProfileId::EmueraSkiaSnake
        || !prefix.identity.is_legacy_snake_owned_save_v11()
    {
        return Err(invalid(format!(
            "save compatibility differs: current profile is {identity:?}, envelope is {:?}",
            prefix.identity
        )));
    }
    if envelope_checksum(bytes)?.as_bytes() != prefix.checksum {
        return Err(invalid("save envelope checksum differs"));
    }
    Ok(CompatibleSaveEnvelope {
        state: prefix.state,
        payload: prefix.payload,
        source: CompatibleSaveSource::LegacySnakeOwnedV11,
    })
}

/// Validate identity and checksum before exposing the original codec bytes.
///
/// # Errors
/// Rejects unsupported envelopes, corruption, and limits.
pub fn unwrap_compatible_save<'a>(
    bytes: &'a [u8],
    identity: &CompatibilityIdentity,
    limits: SaveCodecLimits,
) -> Result<&'a [u8], SaveCodecError> {
    Ok(unwrap_compatible_envelope(bytes, identity, limits)?.payload)
}

/// Inspect a bounded prefix while enforcing the session's save ownership.
///
/// The complete checksum is verified once all bytes are present and again before restoration.
/// # Errors
/// Rejects unsupported identity, malformed metadata, or size limits.
pub fn inspect_compatible_metadata(
    bytes: &[u8],
    complete: bool,
    identity: &CompatibilityIdentity,
    limits: SaveCodecLimits,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    identity
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if bytes.len() < MAGIC.len() && !complete {
        return Ok(SaveMetadataInspection::NeedMore);
    }
    if !bytes.starts_with(MAGIC) {
        let raw = unwrap_compatible_save(bytes, identity, limits)?;
        return inspect_metadata(raw, complete, limits);
    }
    let Some(prefix) = envelope_prefix(bytes, complete, limits)? else {
        return Ok(SaveMetadataInspection::NeedMore);
    };
    if identity.profile != CompatibilityProfileId::EmueraSkiaSnake
        || !prefix.identity.is_legacy_snake_owned_save_v11()
    {
        return Err(invalid(format!(
            "save compatibility differs: current profile is {identity:?}, envelope is {:?}",
            prefix.identity
        )));
    }
    if prefix.state.len() != prefix.expected_state_bytes
        || prefix.payload.len() != prefix.expected_payload_bytes
    {
        return Ok(SaveMetadataInspection::NeedMore);
    }
    if envelope_checksum(bytes)?.as_bytes() != prefix.checksum {
        return Err(invalid("save envelope checksum differs"));
    }
    inspect_metadata(prefix.payload, true, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope_for(identity: &CompatibilityIdentity, payload: &[u8], state: &[u8]) -> Vec<u8> {
        let encoded = minicbor::to_vec(identity).unwrap();
        let mut result = Vec::new();
        result.extend_from_slice(MAGIC);
        result.extend_from_slice(&VERSION.to_le_bytes());
        result.extend_from_slice(&u32::try_from(encoded.len()).unwrap().to_le_bytes());
        result.extend_from_slice(&u64::try_from(state.len()).unwrap().to_le_bytes());
        result.extend_from_slice(&u64::try_from(payload.len()).unwrap().to_le_bytes());
        result.extend_from_slice(&[0; 32]);
        result.extend_from_slice(&encoded);
        result.extend_from_slice(state);
        result.extend_from_slice(payload);
        let checksum = envelope_checksum(&result).unwrap();
        result[32..HEADER_BYTES].copy_from_slice(checksum.as_bytes());
        result
    }

    fn legacy_envelope(payload: &[u8], state: &[u8]) -> Vec<u8> {
        envelope_for(
            &CompatibilityIdentity::legacy_snake_owned_save_v11(),
            payload,
            state,
        )
    }

    #[test]
    fn unknown_envelope_and_policy_versions_reject_before_payload_use() {
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let limits = SaveCodecLimits::default();
        let encoded = legacy_envelope(b"1\n2\nfixture\n", b"state");
        let mut unknown_version = encoded.clone();
        unknown_version[8..12].copy_from_slice(&1_u32.to_le_bytes());
        for complete in [false, true] {
            let error = inspect_compatible_metadata(&unknown_version, complete, &snake, limits)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("unsupported save envelope version"),
                "{error}"
            );
        }
        assert!(unwrap_compatible_save(&unknown_version, &snake, limits).is_err());

        let legacy_identity = minicbor::to_vec(&snake).unwrap();
        let legacy_payload = b"1\n2\nlegacy-v1\n";
        let mut legacy_checksum = blake3::Hasher::new();
        legacy_checksum.update(&legacy_identity);
        legacy_checksum.update(legacy_payload);
        let mut legacy_v1 = Vec::new();
        legacy_v1.extend_from_slice(MAGIC);
        legacy_v1.extend_from_slice(&1_u32.to_le_bytes());
        legacy_v1.extend_from_slice(&u32::try_from(legacy_identity.len()).unwrap().to_le_bytes());
        legacy_v1.extend_from_slice(&u64::try_from(legacy_payload.len()).unwrap().to_le_bytes());
        legacy_v1.extend_from_slice(legacy_checksum.finalize().as_bytes());
        legacy_v1.extend_from_slice(&legacy_identity);
        legacy_v1.extend_from_slice(legacy_payload);
        assert!(unwrap_compatible_envelope(&legacy_v1, &snake, limits).is_err());
        assert!(inspect_compatible_metadata(&legacy_v1, true, &snake, limits).is_err());

        let mut unsupported = CompatibilityIdentity::legacy_snake_owned_save_v11();
        unsupported.policy_version += 1;
        let identity_bytes = minicbor::to_vec(&unsupported).unwrap();
        let previous_len = u32::from_le_bytes(encoded[12..16].try_into().unwrap()) as usize;
        let mut unknown_policy = encoded[..HEADER_BYTES].to_vec();
        unknown_policy[12..16]
            .copy_from_slice(&u32::try_from(identity_bytes.len()).unwrap().to_le_bytes());
        unknown_policy.extend_from_slice(&identity_bytes);
        unknown_policy.extend_from_slice(&encoded[HEADER_BYTES + previous_len..]);
        let checksum = envelope_checksum(&unknown_policy).unwrap();
        unknown_policy[32..HEADER_BYTES].copy_from_slice(checksum.as_bytes());
        // A valid checksum cannot make an unsupported policy safe to restore.
        assert!(unwrap_compatible_save(&unknown_policy, &snake, limits).is_err());
        for complete in [false, true] {
            assert!(
                inspect_compatible_metadata(&unknown_policy, complete, &snake, limits).is_err()
            );
        }

        let current_identity = envelope_for(&snake, b"1\n2\ncurrent\n", b"state");
        assert!(unwrap_compatible_envelope(&current_identity, &snake, limits).is_err());
    }

    #[test]
    fn profiles_preserve_reference_bytes_and_enforce_snake_ownership() {
        let reference = CompatibilityIdentity::default();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let limits = SaveCodecLimits::default();
        let raw = b"1\n2\nfixture\n".to_vec();
        assert_eq!(
            unwrap_compatible_save(&raw, &reference, limits).unwrap(),
            raw
        );
        assert_eq!(unwrap_compatible_save(&raw, &snake, limits).unwrap(), raw);
        let encoded = legacy_envelope(&raw, b"canonical-owned-state");
        assert_eq!(
            unwrap_compatible_save(&encoded, &snake, limits).unwrap(),
            raw
        );
        assert_eq!(
            unwrap_compatible_envelope(&encoded, &snake, limits)
                .unwrap()
                .source,
            CompatibleSaveSource::LegacySnakeOwnedV11
        );
        assert!(unwrap_compatible_save(&encoded, &reference, limits).is_err());
        for length in 0..encoded.len() {
            assert_eq!(
                inspect_compatible_metadata(&encoded[..length], false, &snake, limits).unwrap(),
                SaveMetadataInspection::NeedMore
            );
        }
        let mut damaged = encoded.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(unwrap_compatible_save(&damaged, &snake, limits).is_err());
        assert!(inspect_compatible_metadata(&damaged, true, &snake, limits).is_err());
        assert!(unwrap_compatible_save(&encoded[..encoded.len() - 1], &snake, limits).is_err());
        assert!(matches!(
            inspect_compatible_metadata(&encoded, true, &snake, limits).unwrap(),
            SaveMetadataInspection::Complete { .. }
        ));

        let owned = encoded;
        let decoded = unwrap_compatible_envelope(&owned, &snake, limits).unwrap();
        assert_eq!(decoded.state, b"canonical-owned-state");
        assert_eq!(decoded.payload, b"1\n2\nfixture\n");
        let identity_bytes = u32::from_le_bytes(owned[12..16].try_into().unwrap()) as usize;
        let mut damaged_state = owned.clone();
        damaged_state[HEADER_BYTES + identity_bytes] ^= 1;
        assert!(unwrap_compatible_envelope(&damaged_state, &snake, limits).is_err());
        assert!(inspect_compatible_metadata(&damaged_state, true, &snake, limits).is_err());

        for offset in [12, 16, 24, 32] {
            let mut damaged_boundary = owned.clone();
            damaged_boundary[offset] ^= 1;
            assert!(unwrap_compatible_envelope(&damaged_boundary, &snake, limits).is_err());
        }

        let canonical_identity =
            minicbor::to_vec(CompatibilityIdentity::legacy_snake_owned_save_v11()).unwrap();
        assert!(canonical_identity[0] & 0xe0 == 0xa0);
        let mut noncanonical_identity = canonical_identity;
        noncanonical_identity[0] = 0xbf;
        noncanonical_identity.push(0xff);
        let mut noncanonical = Vec::new();
        noncanonical.extend_from_slice(MAGIC);
        noncanonical.extend_from_slice(&VERSION.to_le_bytes());
        noncanonical.extend_from_slice(
            &u32::try_from(noncanonical_identity.len())
                .unwrap()
                .to_le_bytes(),
        );
        noncanonical.extend_from_slice(&0_u64.to_le_bytes());
        noncanonical.extend_from_slice(
            &u64::try_from(b"1\n2\nfixture\n".len())
                .unwrap()
                .to_le_bytes(),
        );
        noncanonical.extend_from_slice(&[0; 32]);
        noncanonical.extend_from_slice(&noncanonical_identity);
        noncanonical.extend_from_slice(b"1\n2\nfixture\n");
        let checksum = envelope_checksum(&noncanonical).unwrap();
        noncanonical[32..HEADER_BYTES].copy_from_slice(checksum.as_bytes());
        assert!(unwrap_compatible_envelope(&noncanonical, &snake, limits).is_err());
    }
}
