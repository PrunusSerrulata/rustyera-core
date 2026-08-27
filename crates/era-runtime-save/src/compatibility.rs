//! Profile ownership around existing codecs, without altering the Emuera 1808 wire format.

use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};

use crate::{SaveCodecError, SaveCodecLimits, SaveMetadataInspection, inspect_metadata};

const MAGIC: &[u8; 8] = b"RERASAV\0";
const VERSION: u32 = 1;
const HEADER_BYTES: usize = 56;
const MAXIMUM_IDENTITY_BYTES: usize = 4096;

fn invalid(message: impl Into<String>) -> SaveCodecError {
    SaveCodecError::InvalidFormat(message.into())
}

/// Preserve reference interoperability or wrap a snake save with its exact policy identity.
///
/// # Errors
/// Returns an error for unsupported identities, encoding failures, or size limits.
pub fn wrap_compatible_save(
    payload: Vec<u8>,
    identity: &CompatibilityIdentity,
    limits: SaveCodecLimits,
) -> Result<Vec<u8>, SaveCodecError> {
    identity
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if payload.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    if identity.profile == CompatibilityProfileId::EmueraEm {
        return Ok(payload);
    }
    let encoded = minicbor::to_vec(identity).map_err(|error| invalid(error.to_string()))?;
    let total = HEADER_BYTES
        .checked_add(encoded.len())
        .and_then(|size| size.checked_add(payload.len()))
        .ok_or(SaveCodecError::LimitExceeded("maximum bytes"))?;
    if encoded.len() > MAXIMUM_IDENTITY_BYTES || total > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let mut checksum = blake3::Hasher::new();
    checksum.update(&encoded);
    checksum.update(&payload);
    let mut result = Vec::with_capacity(total);
    result.extend_from_slice(MAGIC);
    result.extend_from_slice(&VERSION.to_le_bytes());
    result.extend_from_slice(
        &u32::try_from(encoded.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    result.extend_from_slice(
        &u64::try_from(payload.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    result.extend_from_slice(checksum.finalize().as_bytes());
    result.extend_from_slice(&encoded);
    result.extend_from_slice(&payload);
    Ok(result)
}

struct EnvelopePrefix<'a> {
    payload: &'a [u8],
    expected_payload_bytes: usize,
    checksummed: &'a [u8],
    checksum: &'a [u8],
}

fn envelope_prefix<'a>(
    bytes: &'a [u8],
    identity: &CompatibilityIdentity,
    complete: bool,
    limits: SaveCodecLimits,
) -> Result<Option<EnvelopePrefix<'a>>, SaveCodecError> {
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
    let payload_bytes = usize::try_from(u64::from_le_bytes(
        bytes[16..24].try_into().expect("fixed header"),
    ))
    .map_err(|_| SaveCodecError::LimitExceeded("maximum bytes"))?;
    let start = HEADER_BYTES
        .checked_add(identity_bytes)
        .ok_or(SaveCodecError::LimitExceeded("maximum bytes"))?;
    let total = start
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
    received
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if &received != identity {
        return Err(invalid(format!(
            "save compatibility differs: expected {identity:?}, received {received:?}"
        )));
    }
    Ok(Some(EnvelopePrefix {
        payload: &bytes[start..],
        expected_payload_bytes: payload_bytes,
        checksummed: &bytes[HEADER_BYTES..],
        checksum: &bytes[24..HEADER_BYTES],
    }))
}

/// Validate identity and checksum before exposing the original codec bytes.
///
/// # Errors
/// Rejects wrong profiles, legacy saves in snake sessions, malformed envelopes, and limits.
pub fn unwrap_compatible_save<'a>(
    bytes: &'a [u8],
    identity: &CompatibilityIdentity,
    limits: SaveCodecLimits,
) -> Result<&'a [u8], SaveCodecError> {
    identity
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    if !bytes.starts_with(MAGIC) {
        return if identity.profile == CompatibilityProfileId::EmueraEm {
            Ok(bytes)
        } else {
            Err(invalid(
                "emuera.skia.snake requires a profile-owned save envelope; legacy import is unsupported",
            ))
        };
    }
    let prefix = envelope_prefix(bytes, identity, true, limits)?
        .ok_or_else(|| invalid("truncated save envelope"))?;
    if blake3::hash(prefix.checksummed).as_bytes() != prefix.checksum {
        return Err(invalid("save envelope checksum differs"));
    }
    Ok(prefix.payload)
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
    let Some(prefix) = envelope_prefix(bytes, identity, complete, limits)? else {
        return Ok(SaveMetadataInspection::NeedMore);
    };
    if prefix.payload.len() == prefix.expected_payload_bytes
        && blake3::hash(prefix.checksummed).as_bytes() != prefix.checksum
    {
        return Err(invalid("save envelope checksum differs"));
    }
    inspect_metadata(prefix.payload, complete, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_envelope_and_policy_versions_reject_before_payload_use() {
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let limits = SaveCodecLimits::default();
        let encoded = wrap_compatible_save(b"1\n2\nfixture\n".to_vec(), &snake, limits).unwrap();
        let mut unknown_version = encoded.clone();
        unknown_version[8..12].copy_from_slice(&2_u32.to_le_bytes());
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

        let mut unsupported = snake.clone();
        unsupported.policy_version += 1;
        assert!(wrap_compatible_save(Vec::new(), &unsupported, limits).is_err());
        let identity_bytes = minicbor::to_vec(&unsupported).unwrap();
        let previous_len = u32::from_le_bytes(encoded[12..16].try_into().unwrap()) as usize;
        let mut unknown_policy = encoded[..HEADER_BYTES].to_vec();
        unknown_policy[12..16]
            .copy_from_slice(&u32::try_from(identity_bytes.len()).unwrap().to_le_bytes());
        unknown_policy.extend_from_slice(&identity_bytes);
        unknown_policy.extend_from_slice(&encoded[HEADER_BYTES + previous_len..]);
        let checksum = blake3::hash(&unknown_policy[HEADER_BYTES..]);
        unknown_policy[24..HEADER_BYTES].copy_from_slice(checksum.as_bytes());
        // A valid checksum cannot make an unsupported policy safe to restore.
        assert!(unwrap_compatible_save(&unknown_policy, &snake, limits).is_err());
        for complete in [false, true] {
            assert!(
                inspect_compatible_metadata(&unknown_policy, complete, &snake, limits).is_err()
            );
        }
    }

    #[test]
    fn profiles_preserve_reference_bytes_and_enforce_snake_ownership() {
        let reference = CompatibilityIdentity::default();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let limits = SaveCodecLimits::default();
        let raw = b"1\n2\nfixture\n".to_vec();
        assert_eq!(
            wrap_compatible_save(raw.clone(), &reference, limits).unwrap(),
            raw
        );
        assert!(unwrap_compatible_save(&raw, &snake, limits).is_err());
        let encoded = wrap_compatible_save(raw.clone(), &snake, limits).unwrap();
        assert_eq!(
            unwrap_compatible_save(&encoded, &snake, limits).unwrap(),
            raw
        );
        assert!(unwrap_compatible_save(&encoded, &reference, limits).is_err());
        for length in 0..HEADER_BYTES {
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
    }
}
