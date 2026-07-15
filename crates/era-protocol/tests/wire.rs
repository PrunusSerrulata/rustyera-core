use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, VersionRange, WireLimits,
    decode_canonical, decode_envelope, encode_canonical, encode_envelope, negotiate_version,
};

#[test]
fn envelope_round_trip_is_byte_stable() {
    let envelope = Envelope::new(
        Channel::Runtime,
        ProtocolVersion::new(1, 0),
        3,
        4,
        10,
        ProtocolBytes::new([0x01, 0x02]),
    );
    let bytes = encode_canonical(&envelope).expect("encode envelope");
    let decoded: Envelope = decode_canonical(&bytes).expect("decode envelope");
    assert_eq!(decoded, envelope);
    assert_eq!(decoded.json_projection().payload_hex, "0102");
}

#[test]
fn envelope_and_payload_limits_are_enforced() {
    let envelope = Envelope::new(
        Channel::Runtime,
        ProtocolVersion::new(1, 0),
        1,
        1,
        1,
        ProtocolBytes::new([1, 2, 3]),
    );
    let limits = WireLimits {
        maximum_envelope_bytes: 1024,
        maximum_payload_bytes: 2,
    };
    assert_eq!(
        encode_envelope(&envelope, limits)
            .expect_err("payload must be limited")
            .code,
        ProtocolErrorCode::PayloadTooLarge
    );

    let bytes = encode_canonical(&envelope).expect("encode unrestricted envelope");
    assert_eq!(
        decode_envelope(
            &bytes,
            WireLimits {
                maximum_envelope_bytes: bytes.len() - 1,
                maximum_payload_bytes: 10,
            }
        )
        .expect_err("envelope must be limited")
        .code,
        ProtocolErrorCode::EnvelopeTooLarge
    );
}

#[test]
fn alternate_integer_width_is_rejected() {
    let version = ProtocolVersion::new(1, 0);
    let canonical = encode_canonical(&version).expect("encode version");
    assert_eq!(canonical, [0xa2, 0x00, 0x01, 0x01, 0x00]);

    let non_canonical = [0xa2, 0x18, 0x00, 0x01, 0x01, 0x00];
    let error = decode_canonical::<ProtocolVersion>(&non_canonical)
        .expect_err("wide map key must be rejected");
    assert_eq!(error.code, ProtocolErrorCode::NonCanonicalCbor);
}

#[test]
fn canonical_unknown_minor_fields_are_ignored() {
    let with_unknown_field = [0xa3, 0x00, 0x01, 0x01, 0x00, 0x02, 0x09];
    assert_eq!(
        decode_canonical::<ProtocolVersion>(&with_unknown_field),
        Ok(ProtocolVersion::new(1, 0))
    );
}

#[test]
fn duplicate_and_reordered_map_keys_are_rejected() {
    let duplicate = [0xa3, 0x00, 0x01, 0x01, 0x00, 0x01, 0x09];
    let reordered = [0xa2, 0x01, 0x00, 0x00, 0x01];
    for bytes in [&duplicate[..], &reordered[..]] {
        assert_eq!(
            decode_canonical::<ProtocolVersion>(bytes)
                .expect_err("map order must be deterministic")
                .code,
            ProtocolErrorCode::NonCanonicalCbor
        );
    }
}

#[test]
fn negotiation_selects_highest_shared_minor() {
    let selected = negotiate_version(
        VersionRange {
            minimum: ProtocolVersion::new(1, 1),
            maximum: ProtocolVersion::new(1, 4),
        },
        VersionRange {
            minimum: ProtocolVersion::new(1, 2),
            maximum: ProtocolVersion::new(1, 3),
        },
    );
    assert_eq!(selected, Some(ProtocolVersion::new(1, 3)));
}
