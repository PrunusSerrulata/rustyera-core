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
    let first = encode_full_project_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    let second = encode_full_project_for_test(
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

// Synthesize the old source section while leaving executable sections opaque: legacy
// full projects support extraction only and must be recompiled by the current runtime.
fn profileless_project_fixture(bytes: &[u8], version: u8) -> Vec<u8> {
    project_manifest_fixture(bytes, version, None)
}

fn project_manifest_fixture(
    bytes: &[u8],
    version: u8,
    identity: Option<&erabasic_compat::CompatibilityIdentity>,
) -> Vec<u8> {
    let mut cursor = stream::HEADER_BYTES;
    for _ in 0..MANIFEST_SECTION_INDEX {
        read_section(bytes, &mut cursor, bytes.len()).unwrap();
    }
    let start = cursor;
    let section = read_section(bytes, &mut cursor, bytes.len()).unwrap();
    let raw = zstd::bulk::decompress(
        section.compressed,
        usize::try_from(section.decoded_length).unwrap(),
    )
    .unwrap();
    let mut remaining = &raw[4..];
    read_bytes(&mut remaining, 4096).unwrap();
    let legacy = encode_raw_section(PROJECT_COMPRESSION_LEVEL, None, |writer| {
        writer
            .write_all(if identity.is_some() {
                MANIFEST_SECTION_MAGIC
            } else {
                LEGACY_MANIFEST_SECTION_MAGIC
            })
            .map_err(|error| error.to_string())?;
        if let Some(identity) = identity {
            write_bytes(writer, &serde_json::to_vec(identity).unwrap())?;
        }
        writer
            .write_all(remaining)
            .map_err(|error| error.to_string())
    })
    .unwrap();
    let mut output = bytes[..start].to_vec();
    output[8] = version;
    output.extend_from_slice(&legacy);
    output.extend_from_slice(&bytes[cursor..bytes.len() - 32]);
    let digest = blake3::hash(&output);
    output.extend_from_slice(digest.as_bytes());
    output
}

#[test]
fn profile_identity_survives_cache_and_full_project_without_cross_profile_keys() {
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
    let mut project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let reference_key = project_key(&project_identity(&project), &[]);
    let source_digest = project_identity(&project).source_digest;
    project.compatibility =
        CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
    assert_eq!(project_identity(&project).source_digest, source_digest);
    assert_ne!(project_key(&project_identity(&project), &[]), reference_key);
    let snake_key = project_key(&project_identity(&project), &[]);
    for contract in [
        erabasic_compat::SQL_SERVICE_CONTRACT_NAME,
        erabasic_compat::SQL_LIMITS_CONTRACT_NAME,
        erabasic_compat::SCENE_CONTRACT_NAME,
        erabasic_compat::AUDIO_SERVICE_CONTRACT_NAME,
    ] {
        let mut different_service = project.clone();
        bump_compatibility_service_version(&mut different_service.compatibility, contract);
        assert_ne!(
            project_key(&project_identity(&different_service), &[]),
            snake_key,
            "{contract} must participate in the cache identity"
        );
    }
    project.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(
            "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n".into(),
        ),
        content_hash: None,
    });
    let build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let artifact = build.artifact.as_ref().unwrap();
    assert_eq!(
        artifact.artifact().manifest.compatibility,
        project.compatibility
    );
    let cache = encode_compiled_cache_for_test(
        &project,
        &[],
        artifact,
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let decoded = decode(&cache, cache.len()).unwrap();
    assert_eq!(
        decoded.artifact.artifact().manifest.compatibility,
        project.compatibility
    );
    let full = encode_full_project_for_test(
        &project,
        &[],
        artifact,
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let projected = decode_project_file_frontend_manifest(&full, full.len()).unwrap();
    assert_eq!(projected.identity, project_identity(&project));
    assert!(
        matches!(&projected.manifest.files[1].payload, FilePayload::Utf8(text) if text.contains("emuera.skia.snake"))
    );
    let mut stream = ProjectFileStreamDecoder::new(full.len(), full.len()).unwrap();
    for chunk in full.chunks(17) {
        stream.append(chunk).unwrap();
    }
    assert_eq!(
        stream.finish().unwrap().project.identity,
        project_identity(&project)
    );
    let mut old_snake = project.compatibility.clone();
    old_snake.semantic_version = 1;
    old_snake.policy_version = 1;
    assert_historical_profile_rejected(&full, &old_snake);
    let journal = encode_record(
        configuration_digest(&project).unwrap(),
        "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.em\"\n",
    )
    .unwrap()
    .0;
    let mut changed = full.clone();
    changed.extend_from_slice(&journal);
    assert!(decode_project_file(&changed, changed.len()).is_err());
}

fn assert_historical_profile_rejected(
    full: &[u8],
    identity: &erabasic_compat::CompatibilityIdentity,
) {
    let historical = project_manifest_fixture(full, PROFILED_PROJECT_VERSION, Some(identity));
    let error = decode_project_file(&historical, historical.len())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported compatibility identity"),
        "{error}"
    );
    let mut old_stream = ProjectFileStreamDecoder::new(historical.len(), historical.len()).unwrap();
    let result = (|| {
        for chunk in historical.chunks(13) {
            old_stream.append(chunk)?;
        }
        old_stream.finish()
    })();
    let error = result
        .expect_err("historical snake identity must be rejected")
        .to_string();
    assert!(
        error.contains("unsupported compatibility identity"),
        "{error}"
    );
}

fn bump_compatibility_service_version(
    identity: &mut erabasic_compat::CompatibilityIdentity,
    name: &str,
) {
    identity
        .services
        .iter_mut()
        .find(|service| service.name == name)
        .expect("compatibility identity carries the requested service contract")
        .version += 1;
}
