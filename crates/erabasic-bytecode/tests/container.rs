use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, BytecodeFunctionKind, DecodeError,
    DecodeLimits, Digest, PatchError, SourceMap, SourceMapEntry, SourceRecord, SymbolKey,
    apply_patch, create_patch, decode_artifact, encode_artifact, opcode,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

fn artifact() -> BytecodeArtifact {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        runtime_builtins: Vec::new(),
        runtime_variables: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        project_data,
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        event_groups: Vec::new(),
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
fn symbol_keys_keep_the_canonical_lowercase_hex_json_shape() {
    let key = SymbolKey([
        0x00, 0x01, 0x0f, 0x10, 0x2a, 0x3b, 0x4c, 0x5d, 0x6e, 0x7f, 0x80, 0x91, 0xa2, 0xb3, 0xc4,
        0xff,
    ]);
    let encoded = serde_json::to_string(&key).expect("symbol key should serialize");
    assert_eq!(encoded, "\"00010f102a3b4c5d6e7f8091a2b3c4ff\"");
    assert_eq!(serde_json::from_str::<SymbolKey>(&encoded).unwrap(), key);
}

#[test]
fn binary_identity_covers_instruction_and_exact_source_origin_contents() {
    let mut with_instruction = artifact();
    let function = SymbolKey::derive("binary-identity-test", b"function");
    with_instruction.functions.push(BytecodeFunction {
        key: function,
        name: "IDENTITY".into(),
        kind: BytecodeFunctionKind::Normal,
        parameters: Vec::new(),
        result: None,
        labels: Vec::new(),
        imports: Vec::new(),
        code: vec![opcode::push_integer(1)],
        max_stack: 1,
    });
    with_instruction.refresh_ids().unwrap();
    let mut changed_instruction = with_instruction.clone();
    changed_instruction.functions[0].code[0] = opcode::push_integer(2);
    changed_instruction.refresh_ids().unwrap();
    assert_ne!(
        changed_instruction.manifest.program_version.execution_id,
        with_instruction.manifest.program_version.execution_id
    );

    let mut no_origin = with_instruction;
    no_origin.source_map.entries.push(SourceMapEntry {
        function,
        code_start: 0,
        code_end: 1,
        byte_start: 2,
        byte_end: 3,
        statement_fingerprint: 0,
        origin_chain: None,
        source_index: 0,
    });
    no_origin.refresh_ids().unwrap();
    let mut empty_origin = no_origin.clone();
    empty_origin.source_map.entries[0].origin_chain = Some(Box::default());
    empty_origin.refresh_ids().unwrap();
    assert_ne!(
        empty_origin.manifest.artifact_id,
        no_origin.manifest.artifact_id
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

#[test]
fn finite_native_authorization_survives_container_patch_and_changes_execution_identity() {
    use erabasic_bytecode::{
        BytecodeType, RuntimeArgumentConstraint as C, RuntimeBuiltinSymbol, RuntimeCallableShape,
        RuntimeExpressionShape, RuntimeNativeAuthorization, canonical_native_contract,
    };
    let mut base = artifact();
    let symbol = RuntimeBuiltinSymbol {
        name: "MAX".into(),
        result: BytecodeType::Integer,
        shapes: vec![RuntimeCallableShape {
            minimum: 1,
            maximum: None,
            omitted_from: 1,
            arguments: vec![C::Integer],
            allow_omitted: false,
        }],
    };
    base.runtime_builtins.push(symbol.clone());
    base.refresh_ids().unwrap();
    let mut target = base.clone();
    let family = RuntimeNativeAuthorization::new(&symbol, canonical_native_contract("max"));
    let integer = Some(RuntimeExpressionShape {
        value_type: BytecodeType::Integer,
        variable: false,
        mutable: false,
    });
    let one = family.bind(&[integer]).unwrap();
    let many = family.bind(&[integer; 8]).unwrap();
    assert_eq!(one.service_key, many.service_key);
    assert_ne!(one.import.key, many.import.key);
    assert!(family.bind(&[]).is_none());
    target.runtime_native_authorizations.push(family);
    target.refresh_ids().unwrap();
    assert_ne!(
        base.manifest.program_version.execution_id,
        target.manifest.program_version.execution_id
    );
    let patch = create_patch(&base, &target);
    assert_eq!(apply_patch(&base, &patch).unwrap(), target);
    let decoded =
        decode_artifact(&encode_artifact(&target).unwrap(), &DecodeLimits::default()).unwrap();
    assert_eq!(decoded.into_inner(), target);
}
