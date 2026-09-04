#[test]
fn compact_cache_rejects_configuration_journals() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let mut bytes = encode_compiled_cache_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    bytes.extend_from_slice(&encode_record(None, "[audio]\nvolume = 42\n").unwrap().0);
    assert!(
        decode(&bytes, bytes.len())
            .err()
            .unwrap()
            .contains("cannot contain a configuration journal")
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
    let base = encode_full_project_for_test(
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
    assert_eq!(cached.key, project_key(&original_identity, &[]));

    let current = blake3::hash(b"[audio]\nvolume = 42\n");
    let unchanged = prepare_project_configuration_update(
        &updated,
        usize::MAX,
        expected.as_bytes(),
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
    let base = encode_full_project_for_test(
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    let bytes = encode_full_project_for_test(
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        context: None,
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
        notification: era_runtime_protocol::DiagnosticNotification::default(),
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

fn project_fixture_version(bytes: &[u8], version: u8) -> Vec<u8> {
    let mut fixture = bytes.to_vec();
    fixture[8] = version;
    let digest_offset = fixture.len() - 32;
    let digest = blake3::hash(&fixture[..digest_offset]);
    fixture[digest_offset..].copy_from_slice(digest.as_bytes());
    fixture
}

fn assert_streamed_project_manifest(bytes: &[u8], chunk_size: usize, project: &ProjectManifest) {
    let mut decoder = ProjectFileStreamDecoder::new(bytes.len(), bytes.len()).unwrap();
    for chunk in bytes.chunks(chunk_size) {
        decoder.append(chunk).unwrap();
    }
    assert_eq!(&decoder.finish().unwrap().project.manifest, project);
}

#[test]
fn project_file_projection_honors_limits_and_version() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let mut bytes = encode_full_project_for_test(
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
    let legacy = profileless_project_fixture(&bytes, LEGACY_PROJECT_VERSION);
    assert_eq!(
        decode_project_file(&legacy, legacy.len()).unwrap().manifest,
        project
    );
    assert_streamed_project_manifest(&legacy, 1, &project);
    let previous = profileless_project_fixture(&bytes, PREVIOUS_PROJECT_VERSION);
    assert_eq!(
        decode_project_file(&previous, previous.len())
            .unwrap()
            .manifest,
        project
    );
    assert_streamed_project_manifest(&previous, 1, &project);
    assert_streamed_project_manifest(&bytes, 1, &project);
    // v12 remains a portable source container, while its compiled artifact is
    // intentionally obsolete under the Batch 2 data ABI.
    let data_v12 = project_fixture_version(&bytes, DATA_PROJECT_VERSION);
    assert_eq!(
        decode_project_file(&data_v12, data_v12.len())
            .unwrap()
            .manifest,
        project
    );
    assert!(
        decode(&data_v12, data_v12.len())
            .err()
            .expect("v12 compiled artifact must require a source rebuild")
            .contains("requires a source rebuild")
    );
    assert_streamed_project_manifest(&data_v12, 17, &project);
    // v9 has the same source-manifest representation but obsolete compiled semantics.
    let profiled = project_fixture_version(&bytes, PROFILED_PROJECT_VERSION);
    assert_eq!(
        decode_project_file(&profiled, profiled.len())
            .unwrap()
            .manifest,
        project
    );
    assert!(
        decode(&profiled, profiled.len())
            .err()
            .unwrap()
            .contains("requires a source rebuild")
    );
    assert_streamed_project_manifest(&profiled, 11, &project);
    let mut stale_cache = previous.clone();
    stale_cache[..8].copy_from_slice(b"RERACACH");
    let error = decode(&stale_cache, stale_cache.len())
        .err()
        .expect("a version 7 compiled cache must be rejected");
    assert!(error.contains("unsupported project file version 07"));
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
