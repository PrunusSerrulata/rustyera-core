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
fn project_identity_matches_the_cross_host_fixed_vector() {
    let file = |path: &str, category: FileCategory, digest: Vec<u8>| SubmittedFile {
        relative_path: path.into(),
        category,
        payload: FilePayload::Utf8("@TEST\nRETURN".into()),
        content_hash: Some(ProtocolBytes::new(digest)),
    };
    let left = ProjectManifest {
        project_revision: 1,
        files: vec![
            file("ERB/a.erb", FileCategory::Erb, vec![1; 32]),
            file("ERB/A.erb", FileCategory::Erh, vec![2; 32]),
            file("CSV/config.csv", FileCategory::Csv, (0_u8..32).collect()),
            file("resources/icon.png", FileCategory::Resource, vec![255; 32]),
        ],
    };
    let right = ProjectManifest {
        project_revision: 1,
        files: vec![
            file("resources/icon.png", FileCategory::Resource, vec![255; 32]),
            file("CSV/config.csv", FileCategory::Csv, (0_u8..32).collect()),
            file("ERB/A.erb", FileCategory::Erh, vec![2; 32]),
            file("ERB/a.erb", FileCategory::Erb, vec![1; 32]),
        ],
    };

    assert_eq!(project_identity(&left), project_identity(&right));
    assert_eq!(
        project_identity(&left).source_digest.as_slice(),
        &[
            0x15, 0xd7, 0x21, 0x99, 0xf2, 0xe3, 0x3c, 0x42, 0x9e, 0x0b, 0xd4, 0x18, 0x5e, 0x34,
            0x41, 0xa2, 0x3c, 0x06, 0x50, 0xc1, 0x42, 0x78, 0xd5, 0x76, 0x0c, 0x51, 0x27, 0xd1,
            0xa7, 0x0e, 0x07, 0xec,
        ]
    );
}

#[test]
fn cooperative_planning_bounds_manifest_and_function_traversal() {
    let files = (0..=COOPERATIVE_ITEM_QUANTUM * 2)
        .map(|index| SubmittedFile {
            relative_path: format!("resources/{index:04}.bin"),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(index.to_le_bytes().to_vec())),
            content_hash: Some(ProtocolBytes::new(
                blake3::hash(&index.to_le_bytes()).as_bytes().to_vec(),
            )),
        })
        .collect();
    let resource_manifest = ProjectManifest {
        project_revision: 7,
        files,
    };
    let mut identity = ProjectIdentityPlanner::new();
    assert!(identity.step(&resource_manifest).is_none());
    assert_eq!(identity.cursor, COOPERATIVE_ITEM_QUANTUM);
    assert!(identity.step(&resource_manifest).is_none());
    assert_eq!(identity.cursor, COOPERATIVE_ITEM_QUANTUM * 2);
    let planned_identity = loop {
        if let Some(value) = identity.step(&resource_manifest) {
            break value;
        }
    };
    assert_eq!(planned_identity, project_identity(&resource_manifest));

    let mut source = "@SYSTEM_TITLE\nRETURN\n".to_owned();
    for index in 0..=COOPERATIVE_ITEM_QUANTUM * 2 {
        writeln!(source, "@CACHE_PLAN_{index}\nRETURN").unwrap();
    }
    let project = manifest(&source, 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let artifact = build.artifact.unwrap();
    let mut planner = CacheLayoutPlanner::new(CacheKeyPlanner::Incremental {
        state: Arc::new(build.incremental),
        keys: Vec::new(),
    });
    while planner.identity.is_none() {
        assert!(planner.step(&project, &artifact).unwrap().is_none());
    }
    assert!(planner.step(&project, &artifact).unwrap().is_none());
    assert_eq!(planner.cursor, COOPERATIVE_ITEM_QUANTUM);
    let CacheKeyPlanner::Incremental { keys, .. } = &planner.cache_keys else {
        panic!("incremental cache-key planner changed variant");
    };
    assert_eq!(keys.len(), COOPERATIVE_ITEM_QUANTUM);
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
    assert_eq!(bytes[8], 5);
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
fn project_configuration_updates_append_without_rebuilding_the_cache() {
    let mut project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let original_source = "[audio]\nvolume = 100\n";
    project.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(original_source.into()),
        content_hash: None,
    });
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let base = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let original_identity = project_identity(&project);
    let expected = blake3::hash(original_source.as_bytes());
    assert!(
        prepare_project_configuration_update(
            &base,
            base.len(),
            expected.as_bytes(),
            "[audio]\nvolume = 42\n",
        )
        .unwrap_err()
        .to_string()
        .contains("transfer limit")
    );
    let first = prepare_project_configuration_update(
        &base,
        usize::MAX,
        expected.as_bytes(),
        "[audio]\r\nvolume = 42\r\n",
    )
    .unwrap();
    assert_eq!(first.truncate_to, base.len() as u64);
    assert!(first.append.len() < base.len());
    assert_ne!(first.identity, original_identity);

    let mut updated = base.clone();
    updated.extend_from_slice(&first.append);
    let decoded = decode_project_file(&updated, updated.len()).unwrap();
    assert_eq!(decoded.identity, first.identity);
    assert!(matches!(
        &decoded.manifest.files.last().unwrap().payload,
        FilePayload::Utf8(source) if source == "[audio]\nvolume = 42\n"
    ));
    let cached = decode(&updated, updated.len()).unwrap();
    assert!(cached.snapshot.manifest.files.iter().any(|file| matches!(
        &file.payload,
        FilePayload::Utf8(source) if source == "[audio]\nvolume = 42\n"
    )));
    assert_eq!(
        cached.key,
        project_key(
            &original_identity,
            &[],
            ConfigurationClientProfile::Reference
        )
    );

    let current = blake3::hash(b"[audio]\nvolume = 42\n");
    let unchanged = prepare_project_configuration_update(
        &updated,
        usize::MAX,
        current.as_bytes(),
        "[audio]\r\nvolume = 42\r\n",
    )
    .unwrap();
    assert!(unchanged.append.is_empty());
    assert!(
        prepare_project_configuration_update(
            &updated,
            usize::MAX,
            expected.as_bytes(),
            "[audio]\nvolume = 80\n",
        )
        .unwrap_err()
        .to_string()
        .contains("modified by another process")
    );
    let second = prepare_project_configuration_update(
        &updated,
        usize::MAX,
        current.as_bytes(),
        "[audio]\nvolume = 42\n[text]\nreplace_full_width_spaces = true\n",
    )
    .unwrap();
    updated.extend_from_slice(&second.append);
    let decoded = decode_project_file(&updated, updated.len()).unwrap();
    assert_eq!(decoded.identity, second.identity);
    assert!(decoded.manifest.files.iter().any(|file| matches!(
        &file.payload,
        FilePayload::Utf8(source) if source.contains("replace_full_width_spaces = true")
    )));
    let compact = decode_project_file_frontend_manifest(&updated, updated.len()).unwrap();
    assert_eq!(compact.identity, second.identity);
}

#[test]
fn project_configuration_journal_recovers_only_an_incomplete_tail() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let base = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let update = prepare_project_configuration_update(
        &base,
        usize::MAX,
        &[],
        "[text]\nreplace_full_width_spaces = true\n",
    )
    .unwrap();
    let mut interrupted = base.clone();
    interrupted.extend_from_slice(&update.append[..update.append.len() / 2]);
    assert_eq!(
        decode_project_file(&interrupted, interrupted.len())
            .unwrap()
            .manifest,
        project
    );
    let recovered = prepare_project_configuration_update(
        &interrupted,
        usize::MAX,
        &[],
        "[audio]\nvolume = 80\n",
    )
    .unwrap();
    assert_eq!(recovered.truncate_to, base.len() as u64);

    let mut corrupt = base;
    corrupt.extend_from_slice(&update.append);
    let checksum = corrupt.len() - 44;
    corrupt[checksum] ^= 1;
    assert!(decode_project_file(&corrupt, corrupt.len()).is_err());
    assert!(
        prepare_project_configuration_update(&corrupt, usize::MAX, &[], "[audio]\nvolume = 80\n")
            .is_err()
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
#[cfg(not(target_arch = "wasm32"))]
fn compiled_project_cache_encoding_honors_cancellation() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let cancelled = AtomicBool::new(true);
    let snapshot = CompiledSnapshotMetadata::from(build.snapshot.as_ref().unwrap());
    let error = encode_cancellable(
        Arc::new(project),
        Vec::new(),
        build.artifact.unwrap(),
        Arc::new(build.incremental),
        snapshot,
        build.report.diagnostics,
        Arc::new(cancelled),
    )
    .unwrap_err();

    assert_eq!(error, "compiled cache build cancelled");
}

#[test]
fn cooperative_cache_encoding_yields_between_sections_and_manifest_chunks() {
    let resource = vec![0x5a; COOPERATIVE_MANIFEST_CHUNK_BYTES * 2 + 1];
    let project = ProjectManifest {
        project_revision: 1,
        files: vec![
            manifest("@SYSTEM_TITLE\nRETURN\n", 1)
                .files
                .into_iter()
                .next()
                .unwrap(),
            SubmittedFile {
                relative_path: "resources/large.bin".into(),
                category: FileCategory::Resource,
                content_hash: Some(ProtocolBytes::new(
                    blake3::hash(&resource).as_bytes().to_vec(),
                )),
                payload: FilePayload::Bytes(ProtocolBytes::new(resource)),
            },
        ],
    };
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let artifact = build.artifact.clone().unwrap();
    let snapshot = build.snapshot.as_ref().unwrap();
    let cache_keys = build
        .incremental
        .compact_cache_keys(artifact.artifact())
        .unwrap();
    let canonical = encode(
        &snapshot.manifest,
        &[],
        &artifact,
        &build.incremental,
        snapshot,
        &build.report.diagnostics,
    )
    .unwrap();
    let mut encoder = CooperativeCompiledCacheEncoder::new(
        Arc::clone(&snapshot.manifest),
        Vec::new(),
        artifact,
        cache_keys,
        CompiledSnapshotMetadata::from(snapshot),
        build.report.diagnostics.clone(),
        None,
    );

    assert!(encoder.step().unwrap().is_none());
    let mut steps = 1;
    let bytes = loop {
        steps += 1;
        if let Some(bytes) = encoder.step().unwrap() {
            break bytes;
        }
        assert!(steps < 256, "cooperative cache encoder did not finish");
    };

    assert!(steps > 16, "cache encoding should span multiple host pumps");
    assert_eq!(bytes, canonical);
    let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();
    assert_eq!(
        decoded.snapshot.manifest.as_ref(),
        snapshot.manifest.as_ref()
    );
    assert_eq!(
        decoded.artifact.artifact(),
        build.artifact.unwrap().artifact()
    );
}

#[test]
fn cooperative_manifest_encoding_preserves_empty_payloads_and_reports_file_errors() {
    let project = ProjectManifest {
        project_revision: 1,
        files: vec![
            manifest("@SYSTEM_TITLE\nRETURN\n", 1)
                .files
                .into_iter()
                .next()
                .unwrap(),
            SubmittedFile {
                relative_path: "resources/empty.bin".into(),
                category: FileCategory::Resource,
                content_hash: Some(ProtocolBytes::new(blake3::hash(&[]).as_bytes().to_vec())),
                payload: FilePayload::Bytes(ProtocolBytes::new(Vec::new())),
            },
        ],
    };
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let artifact = build.artifact.clone().unwrap();
    let snapshot = build.snapshot.as_ref().unwrap();
    let cache_keys = build
        .incremental
        .compact_cache_keys(artifact.artifact())
        .unwrap();
    let run = |manifest: ProjectManifest| {
        let mut encoder = CooperativeCompiledCacheEncoder::new(
            Arc::new(manifest),
            Vec::new(),
            artifact.clone(),
            cache_keys.clone(),
            CompiledSnapshotMetadata::from(snapshot),
            build.report.diagnostics.clone(),
            None,
        );
        loop {
            match encoder.step() {
                Ok(Some(bytes)) => break Ok(bytes),
                Ok(None) => {}
                Err(error) => break Err(error),
            }
        }
    };

    let bytes = run(project.clone()).unwrap();
    assert_eq!(
        decode_project_file(&bytes, bytes.len())
            .unwrap()
            .manifest
            .files[1]
            .payload,
        FilePayload::Bytes(ProtocolBytes::new(Vec::new()))
    );

    let mut mismatched = project.clone();
    mismatched.files[1].content_hash = Some(ProtocolBytes::new(vec![1; 32]));
    assert_eq!(
        run(mismatched).unwrap_err(),
        "project manifest content hash differs from its payload"
    );

    let mut unreadable = project;
    unreadable.files[1].payload = FilePayload::IoError(era_runtime_protocol::FrontendIoError {
        kind: era_runtime_protocol::FrontendIoErrorKind::Other,
        message: "fixture".into(),
        platform_code: None,
    });
    assert_eq!(
        run(unreadable).unwrap_err(),
        "project files with I/O errors cannot be cached"
    );
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
