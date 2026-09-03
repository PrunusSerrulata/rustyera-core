use super::*;
use era_protocol::{ProtocolBytes, decode_canonical, encode_canonical};
use era_runtime_protocol::{FileCategory, FilePayload};

fn manifest(count: usize, size: usize) -> ProjectManifest {
    ProjectManifest {
        project_revision: 7,
        compatibility: CompatibilityIdentity::default(),
        files: (0..count)
            .map(|index| SubmittedFile {
                relative_path: format!("resources/{index}.bin"),
                category: FileCategory::Resource,
                payload: if index % 2 == 0 {
                    FilePayload::Bytes(ProtocolBytes::new(vec![42; size]))
                } else {
                    FilePayload::Utf8("汉字".repeat(size / 6))
                },
                content_hash: None,
            })
            .collect(),
    }
}

#[test]
fn manifest_import_decodes_every_chunk_boundary_without_retaining_the_wire_payload() {
    let expected = manifest(4, 384);
    let bytes = encode_canonical(&expected).unwrap();
    for chunk_size in [1, 2, 7, 31, 256, bytes.len()] {
        let mut decoder = ManifestImportDecoder::default();
        for chunk in bytes.chunks(chunk_size) {
            decoder.push(chunk).unwrap();
        }
        assert_eq!(decoder.received, bytes.len() as u64);
        assert!(decoder.pending.is_empty());
        assert_eq!(decoder.finish().unwrap(), expected);
    }
}

#[test]
fn manifest_import_retains_only_the_incomplete_file_between_chunks() {
    let expected = manifest(24, 128 * 1024);
    let bytes = encode_canonical(&expected).unwrap();
    let mut decoder = ManifestImportDecoder::default();
    for chunk in bytes.chunks(4096) {
        decoder.push(chunk).unwrap();
        assert!(decoder.pending.len() < 132 * 1024);
    }
    assert_eq!(decoder.finish().unwrap(), expected);
}

fn with_unknown_field(value: &[u8]) -> Vec<u8> {
    let mut bytes = encode_canonical(&manifest(0, 0)).unwrap();
    assert_eq!(bytes[0], 0xa3);
    bytes[0] = 0xa4;
    bytes.push(3);
    bytes.extend_from_slice(value);
    bytes
}

#[test]
fn manifest_import_preserves_canonical_and_forward_compatibility_rules() {
    let valid = encode_canonical(&manifest(2, 48)).unwrap();
    let mut nonminimal_map = vec![0xb8, 3];
    nonminimal_map.extend_from_slice(&valid[1..]);
    let mut nonminimal_key = valid.clone();
    nonminimal_key.splice(1..2, [0x18, 0]);
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut duplicate = with_unknown_field(&[0]);
    let key = duplicate.len() - 2;
    duplicate[key] = 2;
    let mut too_deep = vec![0x81; 128];
    too_deep.push(0);
    let mut allowed_depth = vec![0x81; 127];
    allowed_depth.push(0);
    let cases = [
        valid,
        with_unknown_field(&[0xa1, 0, 0]),
        with_unknown_field(&allowed_depth),
        with_unknown_field(&too_deep),
        with_unknown_field(&[0xa2, 1, 0, 0, 0]),
        with_unknown_field(&[0x61, 0xff]),
        with_unknown_field(&[0x9f, 0xff]),
        with_unknown_field(&[0xf9, 0, 0]),
        nonminimal_map,
        nonminimal_key,
        duplicate,
        trailing,
        vec![0xa0],
        vec![0xbf, 0xff],
        vec![0xff],
    ];
    for bytes in cases {
        let expected = decode_canonical::<ProjectManifest>(&bytes);
        for chunk_size in [1, 7, bytes.len()] {
            let mut decoder = ManifestImportDecoder::default();
            for chunk in bytes.chunks(chunk_size) {
                decoder.push(chunk).unwrap();
            }
            assert_eq!(
                decoder.finish().ok(),
                expected.as_ref().ok().cloned(),
                "{bytes:?}"
            );
        }
    }
}

#[test]
fn manifest_import_rejects_truncation_at_commit_and_discards_invalid_payloads() {
    let bytes = encode_canonical(&manifest(2, 48)).unwrap();
    for end in 0..bytes.len() {
        let mut decoder = ManifestImportDecoder::default();
        decoder.push(&bytes[..end]).unwrap();
        assert!(decoder.finish().is_err(), "truncation at {end}");
    }
    let mut decoder = ManifestImportDecoder::default();
    decoder.push(&[0xff]).unwrap();
    decoder.push(&vec![0; 1024 * 1024]).unwrap();
    assert!(decoder.pending.is_empty());
    assert!(decoder.files.is_none());
    assert!(decoder.finish().is_err());
}
