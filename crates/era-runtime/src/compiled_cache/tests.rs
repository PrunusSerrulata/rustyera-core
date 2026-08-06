use std::fmt::Write as _;

use era_runtime_protocol::{ConfigurationClientProfile, FileCategory, FilePayload, SubmittedFile};

use super::*;

fn manifest(source: &str, revision: u64) -> ProjectManifest {
    ProjectManifest {
        project_revision: revision,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(source.into()),
            content_hash: None,
        }],
    }
}

#[test]
fn compiled_project_cache_round_trips_and_keys_source_content() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let bytes = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();
    let decoded_file = decode_project_file(&bytes, 64 * 1024 * 1024).unwrap();

    assert_eq!(&bytes[..8], b"RERAPROJ");
    assert_eq!(bytes[8], 4);
    assert_eq!(
        decoded.key,
        project_key(
            &project_identity(&project),
            &[],
            ConfigurationClientProfile::Reference,
        )
    );
    assert_eq!(decoded_file.identity, project_identity(&project));
    assert_eq!(decoded_file.manifest, project);
    assert_eq!(decoded.diagnostics, build.report.diagnostics);
    assert_eq!(decoded.incremental, build.incremental);
    assert_eq!(
        decoded.artifact.artifact(),
        build.artifact.as_ref().unwrap().artifact()
    );
    assert_eq!(
        decoded.artifact.artifact().manifest.artifact_id,
        build.artifact.unwrap().artifact().manifest.artifact_id
    );
    assert!(
        decoded
            .artifact
            .artifact()
            .source_map
            .statement_fingerprints
            .iter()
            .all(|fingerprint| fingerprint.0[16..] == [0; 16])
    );
    assert_eq!(
        project_key(
            &project_identity(&project),
            &[],
            ConfigurationClientProfile::Reference,
        ),
        project_key(
            &project_identity(&manifest("@SYSTEM_TITLE\nRETURN\n", 9)),
            &[],
            ConfigurationClientProfile::Reference,
        )
    );
    assert_ne!(
        project_key(
            &project_identity(&project),
            &[],
            ConfigurationClientProfile::Reference,
        ),
        project_key(
            &project_identity(&manifest("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n", 1)),
            &[],
            ConfigurationClientProfile::Reference,
        )
    );
    assert_ne!(
        project_key(
            &project_identity(&project),
            &[],
            ConfigurationClientProfile::Reference,
        ),
        project_key(
            &project_identity(&project),
            &[],
            ConfigurationClientProfile::Tui,
        )
    );
}

#[test]
fn project_file_stores_manifest_payload_once_and_extracts_it_exactly() {
    let resource = (0..=u8::MAX).cycle().take(128 * 1024).collect::<Vec<_>>();
    let resource_hash = ProtocolBytes::new(blake3::hash(&resource).as_bytes().to_vec());
    let source_file = manifest("@SYSTEM_TITLE\nRETURN\n", 7)
        .files
        .into_iter()
        .next()
        .unwrap();
    let project = ProjectManifest {
        project_revision: 7,
        files: vec![
            source_file,
            SubmittedFile {
                relative_path: "resources/payload.bin".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(resource.clone())),
                content_hash: Some(resource_hash),
            },
        ],
    };
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let bytes = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
    let extracted = decode_project_file(&bytes, bytes.len()).unwrap();
    let frontend = decode_project_file_frontend_manifest(&bytes, bytes.len()).unwrap();

    assert_eq!(extracted.manifest, project);
    assert!(
        matches!(&frontend.manifest.files[0].payload, FilePayload::Utf8(text) if text.is_empty())
    );
    assert_eq!(frontend.manifest.files[1].payload, project.files[1].payload);
    assert_eq!(frontend.identity, extracted.identity);
    assert!(sections.snapshot.decoded_length < resource.len() as u64);
    let resource_length = u64::try_from(resource.len()).unwrap();
    assert!(sections.manifest.decoded_length > resource_length);
    assert!(sections.manifest.decoded_length < resource_length + 1024);
}

#[test]
fn frontend_manifest_keeps_resources_and_diagnostic_sources_only() {
    let mut project = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@MAIN\nRETURN\n".into()),
                content_hash: Some(ProtocolBytes::new(vec![1; 32])),
            },
            SubmittedFile {
                relative_path: "other.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@OTHER\nRETURN\n".into()),
                content_hash: Some(ProtocolBytes::new(vec![2; 32])),
            },
            SubmittedFile {
                relative_path: "resources/a.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![3, 4, 5])),
                content_hash: Some(ProtocolBytes::new(vec![3; 32])),
            },
            SubmittedFile {
                relative_path: "emuera.config".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8("FontSize:18\n".into()),
                content_hash: None,
            },
        ],
    };
    let diagnostics = vec![ProtocolDiagnostic {
        code: "test".into(),
        level: era_runtime_protocol::RuntimeLogLevel::Warning,
        message: "warning".into(),
        source: Some(era_runtime_protocol::SourceLocation {
            relative_path: "MAIN.ERB".into(),
            byte_start: 0,
            byte_end: 1,
            line: Some(1),
            byte_column: Some(1),
        }),
    }];

    compact_frontend_manifest(&mut project, &diagnostics);

    assert!(matches!(&project.files[0].payload, FilePayload::Utf8(text) if !text.is_empty()));
    assert!(matches!(&project.files[1].payload, FilePayload::Utf8(text) if text.is_empty()));
    assert!(
        matches!(&project.files[2].payload, FilePayload::Bytes(bytes) if bytes.as_slice() == [3, 4, 5])
    );
    assert!(matches!(&project.files[3].payload, FilePayload::Utf8(text) if text.is_empty()));
    assert_eq!(
        project.files[1].content_hash.as_ref().unwrap().as_slice(),
        [2; 32]
    );
    assert_eq!(
        project.files[3].content_hash.as_ref().unwrap().as_slice(),
        blake3::hash(b"FontSize:18\n").as_bytes()
    );
}

#[test]
fn compiled_project_cache_rejects_corruption() {
    assert!(decode(b"not a compiled cache", 1024).is_err());
    assert!(decode_project_file(b"not a project file", 1024).is_err());
    assert!(decode_project_file_frontend_manifest(b"not a project file", 1024).is_err());
    let mut obsolete = vec![0_u8; 512];
    obsolete[..8].copy_from_slice(b"RERACACH");
    assert!(decode_project_file(&obsolete, obsolete.len()).is_err());
    assert!(decode_project_file_frontend_manifest(&obsolete, obsolete.len()).is_err());
}

#[test]
fn project_file_projection_honors_limits_and_version() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let mut bytes = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    assert!(decode_project_file(&bytes, bytes.len() - 1).is_err());
    assert!(decode_project_file_frontend_manifest(&bytes, bytes.len() - 1).is_err());
    assert!(decode_project_file_frontend_manifest(&bytes[..bytes.len() - 1], bytes.len()).is_err());
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(decode_project_file_frontend_manifest(&corrupt, corrupt.len()).is_err());
    bytes[8] = 2;
    let digest_offset = bytes.len() - 32;
    let digest = blake3::hash(&bytes[..digest_offset]);
    bytes[digest_offset..].copy_from_slice(digest.as_bytes());
    assert!(
        decode_project_file(&bytes, 64 * 1024 * 1024)
            .unwrap_err()
            .to_string()
            .contains("unsupported project file version 02")
    );
    assert!(
        decode_project_file_frontend_manifest(&bytes, 64 * 1024 * 1024)
            .unwrap_err()
            .to_string()
            .contains("unsupported project file version 02")
    );
}

#[test]
fn compiled_project_cache_encoding_honors_cancellation() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let cancelled = AtomicBool::new(true);
    let cache_keys = build
        .incremental
        .compact_cache_keys(build.artifact.as_ref().unwrap().artifact())
        .unwrap();

    let snapshot = CompiledSnapshotMetadata::from(build.snapshot.as_ref().unwrap());
    let error = encode_cancellable(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &cache_keys,
        &snapshot,
        &build.report.diagnostics,
        &cancelled,
    )
    .unwrap_err();

    assert_eq!(error, "compiled cache build cancelled");
}

#[test]
fn compact_source_section_preserves_none_and_empty_origin_chains() {
    let function = SymbolKey::derive("compiled-cache-source-test", b"function");
    let entries = vec![
        SourceMapEntry {
            function,
            code_start: 0,
            code_end: 1,
            byte_start: 4,
            byte_end: 5,
            statement_fingerprint: 2,
            origin_chain: None,
            source_index: 3,
        },
        SourceMapEntry {
            function,
            code_start: 1,
            code_end: 2,
            byte_start: 6,
            byte_end: 7,
            statement_fingerprint: 8,
            origin_chain: Some(Box::default()),
            source_index: 9,
        },
    ];
    let function_indices = [(function, 0)].into_iter().collect();
    let functions = vec![BytecodeFunction {
        key: function,
        name: "test".into(),
        kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
        parameters: Vec::new(),
        result: None,
        labels: Vec::new(),
        imports: Vec::new(),
        code: Vec::new(),
        max_stack: 0,
    }];
    let encoded = encode_source_section(&entries, &function_indices, None).unwrap();
    let mut cursor = 0;
    let section = read_section(&encoded, &mut cursor, encoded.len()).unwrap();

    assert_eq!(
        decode_source_section(&section, &functions).unwrap(),
        entries
    );
    assert_eq!(cursor, encoded.len());
}

#[test]
fn sharded_binary_cache_is_deterministic() {
    let mut source = "@SYSTEM_TITLE\nRETURN\n".to_owned();
    for index in 0..256 {
        write!(
            source,
            "@CACHE_SIZE_{index}\nPRINTL repeated compiled cache payload\nRETURN\n"
        )
        .unwrap();
    }
    let project = manifest(&source, 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let first = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    let second = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    assert_eq!(first, second);
}
