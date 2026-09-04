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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    let canonical = encode_compiled_cache_for_test(
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

    let creations_before = super::cooperative::MANIFEST_ENCODER_CREATIONS.get();
    assert!(encoder.step().unwrap().is_none());
    let mut steps = 1;
    let bytes = loop {
        steps += 1;
        if let Some(bytes) = encoder.step().unwrap() {
            break bytes.into_vec();
        }
        assert!(steps < 256, "cooperative cache encoder did not finish");
    };

    assert!(steps > 16, "cache encoding should span multiple host pumps");
    assert_eq!(
        super::cooperative::MANIFEST_ENCODER_CREATIONS.get() - creations_before,
        1,
        "manifest compression must allocate one encoder across all host pumps"
    );
    assert_eq!(bytes, canonical);
    let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();
    let decoded_files = &decoded.snapshot.manifest.files;
    assert_eq!(decoded_files.len(), snapshot.manifest.files.len());
    for (decoded, original) in decoded_files.iter().zip(&snapshot.manifest.files) {
        assert_eq!(decoded.relative_path, original.relative_path);
        assert_eq!(decoded.category, original.category);
        let original_payload = match &original.payload {
            FilePayload::Utf8(value) => value.as_bytes(),
            FilePayload::Bytes(value) => value.as_slice(),
            FilePayload::IoError(_) | FilePayload::ExternalResource(_) => unreachable!(),
        };
        assert_eq!(
            decoded.content_hash.as_ref().map(ProtocolBytes::as_slice),
            Some(blake3::hash(original_payload).as_bytes().as_slice())
        );
        assert!(
            matches!(
                &decoded.payload,
                FilePayload::Utf8(value) if value.is_empty()
            ) || matches!(
                &decoded.payload,
                FilePayload::Bytes(value) if value.as_slice().is_empty()
            )
        );
    }
    assert_eq!(
        decoded.artifact.artifact(),
        build.artifact.unwrap().artifact()
    );
}

#[test]
fn cooperative_manifest_encoding_preserves_empty_payloads_and_reports_file_errors() {
    let resource: Vec<u8> = (0..163_840_u64)
        .flat_map(|index| blake3::hash(&index.to_le_bytes()).as_bytes().to_vec())
        .collect();
    let project = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            SubmittedFile {
                relative_path: "resources/large.bin".into(),
                category: FileCategory::Resource,
                content_hash: Some(ProtocolBytes::new(blake3::hash(&resource).as_bytes().to_vec())),
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
    let run = |manifest: ProjectManifest| {
        let mut encoder = CooperativeCompiledCacheEncoder::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::FullProject,
            manifest: Arc::new(manifest),
            extensions: Vec::new(),
            artifact: artifact.clone(),
            cache_keys: CacheKeyPlanner::Ready(Some(cache_keys.clone())),
            snapshot: CompiledSnapshotMetadata::from(snapshot),
            diagnostics: build.report.diagnostics.clone(),
            cancelled: None,
            progress: None,
            trailing_data: Vec::new(),
        });
        loop {
            match encoder.step() {
                Ok(Some(bytes)) => {
                    assert!(encoder.manifest.files.iter().all(|file| matches!(&file.payload,
                        FilePayload::Bytes(value) if value.as_slice().is_empty()
                    )), "completed export files must release their payloads");
                    break Ok(bytes.into_vec());
                }
                Ok(None) => {}
                Err(error) => break Err(error),
            }
        }
    };

    let creations_before = super::cooperative::MANIFEST_ENCODER_CREATIONS.get();
    let bytes = run(project.clone()).unwrap();
    assert_eq!(
        super::cooperative::MANIFEST_ENCODER_CREATIONS.get() - creations_before,
        1,
        "full project compression must reuse its manifest encoder"
    );
    let canonical = encode_full_project_for_test(
        &project, &[], &artifact, &build.incremental, snapshot, &build.report.diagnostics,
    ).unwrap();
    assert_eq!(bytes, canonical, "direct final-buffer compression must preserve the container bytes");
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
    let encoded = encode_source_section(
        &entries,
        &function_indices,
        ProjectContainerKind::FullProject,
        None,
    )
    .unwrap();
    let mut cursor = 0;
    let section = read_section(&encoded, &mut cursor, encoded.len()).unwrap();

    assert_eq!(
        decode_source_section(&section, &functions).unwrap(),
        entries
    );
    assert_eq!(cursor, encoded.len());
}
