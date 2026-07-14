use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, DecodeError, DecodeLimits, Digest, PatchError, SourceMap,
    SourceRecord, apply_patch, create_patch, decode_artifact, encode_artifact,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

fn artifact() -> BytecodeArtifact {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        project_data,
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();
    artifact
}

#[test]
fn patch_rejects_a_source_different_base_with_the_same_execution_id() {
    let base = artifact();
    let patch = create_patch(&base, &base);
    let mut source_different = base.clone();
    source_different.source_map.sources.push(SourceRecord {
        relative_path: "different.erb".into(),
        content_hash: Digest::default(),
        byte_len: 0,
        line_starts: vec![0],
    });
    source_different.refresh_ids().unwrap();
    assert_eq!(
        source_different.manifest.program_version.execution_id,
        base.manifest.program_version.execution_id
    );
    assert_ne!(
        source_different.manifest.artifact_id,
        base.manifest.artifact_id
    );
    assert_eq!(
        apply_patch(&source_different, &patch),
        Err(PatchError::BaseMismatch)
    );
}

#[test]
fn canonical_container_round_trips_and_detects_corruption() {
    let artifact = artifact();
    let bytes = encode_artifact(&artifact).unwrap();
    let decoded = decode_artifact(&bytes, &DecodeLimits::default()).unwrap();
    assert_eq!(decoded.into_inner(), artifact);

    let mut corrupt = bytes;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        decode_artifact(&corrupt, &DecodeLimits::default()),
        Err(DecodeError::CorruptSection(_))
    ));
}

#[test]
fn skips_unknown_optional_sections_but_rejects_required_ones() {
    let bytes = encode_artifact(&artifact()).unwrap();
    let optional = append_unknown(bytes.clone(), false);
    assert!(decode_artifact(&optional, &DecodeLimits::default()).is_ok());

    let required = append_unknown(bytes, true);
    assert_eq!(
        decode_artifact(&required, &DecodeLimits::default()),
        Err(DecodeError::UnknownRequiredSection(0x8000))
    );
}

fn append_unknown(mut bytes: Vec<u8>, required: bool) -> Vec<u8> {
    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) + 1;
    bytes[12..16].copy_from_slice(&count.to_le_bytes());
    let payload = b"optional extension data";
    bytes.extend_from_slice(&0x8000u16.to_le_bytes());
    bytes.push(u8::from(required));
    bytes.push(0);
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(blake3::hash(payload).as_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
