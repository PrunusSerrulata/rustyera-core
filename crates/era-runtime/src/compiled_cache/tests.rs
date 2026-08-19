use std::fmt::Write as _;

use era_runtime_protocol::{
    ConfigurationClientProfile, ExternalResource, FileCategory, FilePayload, SubmittedFile,
};

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
    let mut project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    project.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8("[meta]\nschema_version = 2\n\n[text]\nfont_size = 21\n".into()),
        content_hash: None,
    });
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
    let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();
    let decoded_file = decode_project_file(&bytes, 64 * 1024 * 1024).unwrap();

    assert_eq!(&bytes[..8], b"RERAPROJ");
    assert_eq!(bytes[8], 8);
    assert_eq!(decoded.key, project_key(&project_identity(&project), &[]));
    assert_eq!(decoded_file.identity, project_identity(&project));
    assert_eq!(decoded_file.manifest, project);
    assert!(
        decoded
            .snapshot
            .editable_configuration
            .is_specified("FontSize")
    );
    assert_eq!(
        decoded.snapshot.client_configuration.get_code("FontSize"),
        decoded.snapshot.configuration.get_code("FontSize")
    );
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
    let mut revised = project.clone();
    revised.project_revision = 9;
    assert_eq!(
        project_key(&project_identity(&project), &[]),
        project_key(&project_identity(&revised), &[])
    );
    let mut changed = project.clone();
    changed.files[0].payload = FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n".into());
    assert_ne!(
        project_key(&project_identity(&project), &[]),
        project_key(&project_identity(&changed), &[])
    );
}

fn small_compiled_cache() -> Vec<u8> {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    encode_compiled_cache_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap()
}

#[test]
fn parallel_cache_decode_reports_failures_in_dependency_order() {
    static INVALID_SECTION: [u8; 1] = [0xff];
    let bytes = small_compiled_cache();
    let invalid = || EncodedSectionRef {
        decoded_length: 1,
        compressed: &INVALID_SECTION,
    };

    for _ in 0..16 {
        let mut sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
        sections.manifest = invalid();
        sections.metadata = invalid();
        let Err(error) = decode_cache_parts(&sections) else {
            panic!("corrupt manifest unexpectedly decoded");
        };
        assert!(
            error.starts_with("manifest section:"),
            "manifest dependency must win independently of task completion order"
        );

        let mut sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
        sections.sources = invalid();
        sections.metadata = invalid();
        let Err(error) = decode_cache_parts(&sections) else {
            panic!("corrupt source records unexpectedly decoded");
        };
        assert!(
            error.starts_with("source-record section:"),
            "source records must win over independent sections"
        );

        let mut sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
        assert!(!sections.functions.is_empty());
        sections.functions[0] = invalid();
        sections.metadata = invalid();
        let Err(error) = decode_cache_parts(&sections) else {
            panic!("corrupt function section unexpectedly decoded");
        };
        assert!(
            error.starts_with("function section 0:"),
            "function dependencies must win over independent sections"
        );
    }
}

#[test]
fn source_sections_decode_without_an_independent_section_barrier() {
    let bytes = small_compiled_cache();
    let sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(3)
        .build()
        .unwrap();
    let started = std::time::Instant::now();
    pool.install(|| {
        decode_cache_parts_with_delays(
            &sections,
            CacheDecodeDelays {
                source_records: std::time::Duration::from_millis(500),
                source_entries: std::time::Duration::from_millis(500),
                independent: std::time::Duration::from_millis(500),
            },
        )
        .unwrap();
    });
    assert!(
        started.elapsed() < std::time::Duration::from_millis(850),
        "source decoding waited for the independent section group"
    );
}

#[test]
fn cache_decode_reports_structured_stage_boundaries() {
    let bytes = small_compiled_cache();
    let reports = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&reports);
    let reporter = crate::ProjectProgressReporter::new(move |progress| {
        observed.lock().unwrap().push(progress);
    });

    decode_with_progress(&bytes, bytes.len(), Some(&reporter)).unwrap();

    let reports = reports.lock().unwrap();
    let expected = [
        crate::ProjectProgressStage::CacheParsing,
        crate::ProjectProgressStage::CacheDecoding,
        crate::ProjectProgressStage::CacheValidating,
    ];
    for stage in expected {
        let stage_reports = reports
            .iter()
            .filter(|progress| progress.stage == stage)
            .collect::<Vec<_>>();
        assert_eq!(stage_reports.len(), 2, "{stage:?}");
        assert_eq!(stage_reports[0].completed, 0);
        assert_eq!(stage_reports[1].completed, 1);
        assert_eq!(stage_reports[1].total, 1);
    }
}

#[test]
fn native_tui_and_cooperative_browser_caches_are_byte_identical() {
    let mut initial = manifest("@SYSTEM_TITLE\nPRINTL cache v1\nRETURN\n", 1);
    initial.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8("[meta]\nschema_version = 2\n[text]\nfont_size = 20\n".into()),
        content_hash: None,
    });
    let first = crate::project::build_project_with_extensions_and_progress(
        &initial,
        None,
        None,
        &[],
        ConfigurationClientProfile::Tui,
        None,
    );
    assert!(first.report.success, "{:?}", first.report.diagnostics);

    let mut reloaded = initial.clone();
    reloaded.project_revision = 2;
    reloaded.files[0].payload =
        FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL cache v2\nRETURN\n".into());
    let mut browser_cold = reloaded.clone();
    // A browser can reach the same source generation through a different number of reloads.
    browser_cold.project_revision = 9;
    let mut tui = crate::project::build_project_with_extensions_and_progress(
        &reloaded,
        Some(&first.incremental),
        first.artifact.as_ref().map(ValidatedArtifact::artifact),
        &[],
        ConfigurationClientProfile::Tui,
        None,
    );
    let mut browser = crate::project::build_project_with_extensions_and_progress(
        &browser_cold,
        None,
        None,
        &[],
        ConfigurationClientProfile::Browser,
        None,
    );
    assert!(tui.report.success, "{:?}", tui.report.diagnostics);
    assert!(browser.report.success, "{:?}", browser.report.diagnostics);
    tui.incremental.compact();
    browser.incremental.compact();

    let tui_snapshot = tui.snapshot.as_ref().unwrap();
    let native = encode_cancellable(
        Arc::clone(&tui_snapshot.manifest),
        Vec::new(),
        tui.artifact.unwrap(),
        Arc::new(tui.incremental),
        CompiledSnapshotMetadata::from(tui_snapshot),
        tui.report.diagnostics,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let browser_snapshot = browser.snapshot.as_ref().unwrap();
    let cooperative = encode_compiled_cache_for_test(
        &browser_snapshot.manifest,
        &[],
        browser.artifact.as_ref().unwrap(),
        &browser.incremental,
        browser_snapshot,
        &browser.report.diagnostics,
    )
    .unwrap();

    assert_eq!(native, cooperative);
    let decoded = decode(&native, native.len()).unwrap();
    assert_eq!(
        decoded.snapshot.configuration_profile,
        ConfigurationClientProfile::Reference
    );
    assert_eq!(
        decoded.snapshot.manifest.project_revision,
        COMPILED_CACHE_PROJECT_REVISION
    );
}

#[test]
fn single_and_multi_threaded_builds_are_byte_identical() {
    let mut source = "@SYSTEM_TITLE\nRETURN\n".to_owned();
    for index in 0..256 {
        writeln!(source, "@IDENTITY_{index}\nPRINTL {index}\nRETURN").unwrap();
    }
    let project = manifest(&source, 1);
    let build = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut build = crate::project::build_project(&project, None);
                assert!(build.report.success, "{:?}", build.report.diagnostics);
                build.incremental.compact();
                let artifact = build.artifact.as_ref().unwrap();
                let bytes = encode_compiled_cache_for_test(
                    &project,
                    &[],
                    artifact,
                    &build.incremental,
                    build.snapshot.as_ref().unwrap(),
                    &build.report.diagnostics,
                )
                .unwrap();
                (
                    artifact.artifact().manifest.program_version.execution_id,
                    artifact.artifact().manifest.artifact_id,
                    artifact.artifact().source_map.clone(),
                    build.report.diagnostics,
                    bytes,
                )
            })
    };

    assert_eq!(build(1), build(4));
}

#[test]
fn full_project_keeps_its_project_revision() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 7);
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

    let decoded = decode_project_file(&bytes, bytes.len()).unwrap();
    assert_eq!(decoded.identity.project_revision, 7);
    assert_eq!(decoded.manifest.project_revision, 7);
}

#[test]
fn streamed_project_file_decode_skips_compiled_sections_and_preserves_journal() {
    let project = manifest(
        &format!(";{}\n@SYSTEM_TITLE\nRETURN\n", "source".repeat(16_000)),
        7,
    );
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
    let (first_record, first_digest) = encode_record(None, "[audio]\nvolume = 10\n").unwrap();
    let (final_record, final_digest) =
        encode_record(Some(first_digest), "[audio]\nvolume = 42\n").unwrap();
    let (interrupted_record, _) =
        encode_record(Some(final_digest), "[audio]\nvolume = 80\n").unwrap();
    let sections = parse_cache_sections(&bytes, bytes.len()).unwrap();
    let embedded_identity = sections.identity.clone();
    let manifest_compressed_bytes = sections.manifest.compressed.len();
    bytes.extend_from_slice(&first_record);
    bytes.extend_from_slice(&final_record);
    bytes.extend_from_slice(&interrupted_record[..interrupted_record.len() / 2]);
    let expected = decode_project_file(&bytes, bytes.len()).unwrap();
    let mut decoder = ProjectFileStreamDecoder::new(bytes.len(), bytes.len()).unwrap();
    let mut maximum_retained = 0;
    for chunk in bytes.chunks(13) {
        decoder.append(chunk).unwrap();
        maximum_retained = maximum_retained.max(decoder.retained_bytes());
    }
    let streamed = decoder.finish().unwrap();

    assert_eq!(streamed.project, expected);
    assert_eq!(streamed.file_digest, *blake3::hash(&bytes).as_bytes());
    let maximum_record_bytes = first_record.len().max(final_record.len());
    let retained_bound =
        stream::HEADER_BYTES + manifest_compressed_bytes + maximum_record_bytes * 2 + 13;
    assert!(maximum_retained <= retained_bound);
    assert_ne!(streamed.project.identity, embedded_identity);
    assert!(streamed.project.manifest.files.iter().any(|file| {
        file.relative_path == "reraconfig.toml"
            && file.payload == FilePayload::Utf8("[audio]\nvolume = 42\n".into())
    }));
}

#[test]
fn streamed_project_file_decode_rejects_corrupt_incomplete_and_oversized_inputs() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
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

    let mut incomplete = ProjectFileStreamDecoder::new(bytes.len(), bytes.len()).unwrap();
    incomplete.append(&bytes[..bytes.len() - 1]).unwrap();
    assert!(incomplete.finish().is_err());

    let mut corrupt = bytes.clone();
    corrupt[bytes.len() - 33] ^= 1;
    let mut decoder = ProjectFileStreamDecoder::new(corrupt.len(), corrupt.len()).unwrap();
    assert!(decoder.append(&corrupt).is_err());

    let mut oversized_section = bytes.clone();
    let mut section_offset = stream::HEADER_BYTES;
    for _ in 0..MANIFEST_SECTION_INDEX {
        let compressed_length = usize::try_from(u64::from_le_bytes(
            oversized_section[section_offset + 8..section_offset + 16]
                .try_into()
                .unwrap(),
        ))
        .unwrap();
        section_offset += 16 + compressed_length;
    }
    oversized_section[section_offset + 8..section_offset + 16]
        .copy_from_slice(&u64::MAX.to_le_bytes());
    let mut decoder =
        ProjectFileStreamDecoder::new(oversized_section.len(), oversized_section.len()).unwrap();
    assert!(decoder.append(&oversized_section).is_err());
    assert_eq!(decoder.retained_bytes(), stream::HEADER_BYTES);

    let mut oversized_decoded_section = bytes.clone();
    oversized_decoded_section[stream::HEADER_BYTES..stream::HEADER_BYTES + 8]
        .copy_from_slice(&u64::MAX.to_le_bytes());
    let mut decoder = ProjectFileStreamDecoder::new(
        oversized_decoded_section.len(),
        oversized_decoded_section.len(),
    )
    .unwrap();
    assert!(decoder.append(&oversized_decoded_section).is_err());

    assert!(ProjectFileStreamDecoder::new(bytes.len(), bytes.len() - 1).is_err());
}

#[test]
fn project_file_cache_externalizes_resources_and_compacts_runtime_sources() {
    let mut project = manifest("@SYSTEM_TITLE\nPRINTL cached\nRETURN\n", 3);
    project.files.push(SubmittedFile {
        relative_path: "resources/title.bin".into(),
        category: FileCategory::Resource,
        payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3, 4])),
        content_hash: None,
    });
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

    let (cache, frontend) =
        decode_project_file_cache_with_progress(&bytes, bytes.len(), None).unwrap();
    let frontend_source = &frontend.manifest.files[0];
    let frontend_resource = &frontend.manifest.files[1];
    assert!(matches!(&frontend_source.payload, FilePayload::Utf8(value) if value.is_empty()));
    assert!(matches!(
        &frontend_resource.payload,
        FilePayload::Bytes(value) if value.as_slice() == [1, 2, 3, 4]
    ));
    let runtime_source = &cache.snapshot.manifest.files[0];
    let runtime_resource = &cache.snapshot.manifest.files[1];
    assert!(matches!(&runtime_source.payload, FilePayload::Utf8(value) if value.is_empty()));
    assert!(matches!(
        &runtime_resource.payload,
        FilePayload::ExternalResource(resource) if resource.byte_length == 4
    ));
    assert_eq!(project_identity(&frontend.manifest), frontend.identity);
    assert_eq!(
        project_identity(&cache.snapshot.manifest),
        frontend.identity
    );
}

#[test]
fn compact_cache_omits_source_and_binary_payloads_but_remains_loadable() {
    let mut project = manifest(
        &format!("@SYSTEM_TITLE\nPRINTL {}\nRETURN\n", "x".repeat(64_000)),
        1,
    );
    let resource = (0..4_000_u64)
        .flat_map(|index| blake3::hash(&index.to_le_bytes()).as_bytes().to_vec())
        .collect::<Vec<_>>();
    project.files.push(SubmittedFile {
        relative_path: "resources/title.png".into(),
        category: FileCategory::Resource,
        payload: FilePayload::Bytes(ProtocolBytes::new(resource)),
        content_hash: None,
    });
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let full = encode_full_project_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let compact = encode_compiled_cache_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    assert_eq!(&compact[..8], b"RERACACH");
    assert!(compact.len() < full.len());
    let decoded = decode(&compact, 64 * 1024 * 1024).unwrap();
    assert_eq!(
        decoded.snapshot.project_identity,
        build.snapshot.as_ref().unwrap().project_identity
    );
    assert!(decode_project_file(&compact, 64 * 1024 * 1024).is_err());
}

#[test]
fn compact_cache_encodes_omitted_external_resources_as_binary_payloads() {
    let mut project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let resource = b"external resource";
    let digest = blake3::hash(resource);
    project.files.push(SubmittedFile {
        relative_path: "resources/title.png".into(),
        category: FileCategory::Resource,
        payload: FilePayload::ExternalResource(ExternalResource {
            byte_length: resource.len() as u64,
            image_metadata: None,
        }),
        content_hash: Some(ProtocolBytes::new(digest.as_bytes().to_vec())),
    });
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();

    let compact = encode_compiled_cache_for_test(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    let decoded = decode(&compact, 64 * 1024 * 1024).unwrap();

    assert_eq!(
        decoded.snapshot.manifest.files[1].category,
        FileCategory::Resource
    );
    assert_eq!(
        decoded.snapshot.manifest.files[1]
            .content_hash
            .as_ref()
            .unwrap()
            .as_slice(),
        digest.as_bytes()
    );
    assert!(matches!(
        &decoded.snapshot.manifest.files[1].payload,
        FilePayload::Bytes(bytes) if bytes.as_slice().is_empty()
    ));
}

#[test]
fn compact_sections_reject_noncanonical_omission_hashes_and_source_metadata() {
    let source = "@SYSTEM_TITLE\nRETURN\n";
    let mut project = manifest(source, 1);
    project.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8("[audio]\nvolume = 100\n".into()),
        content_hash: None,
    });
    let manifest_section =
        encode_manifest_section(&project, ProjectContainerKind::CompiledCache, None).unwrap();
    let mut cursor = 0;
    let encoded = read_section(&manifest_section, &mut cursor, manifest_section.len()).unwrap();
    let mut decoded = zstd::bulk::decompress(
        encoded.compressed,
        usize::try_from(encoded.decoded_length).unwrap(),
    )
    .unwrap();
    let first_tags = COMPACT_MANIFEST_SECTION_MAGIC.len() + 1 + 1 + "main.erb".len();
    decoded[first_tags + 2] = 0;
    let compressed = zstd::bulk::compress(&decoded, CACHE_COMPRESSION_LEVEL).unwrap();
    let corrupt = EncodedSectionRef {
        decoded_length: decoded.len() as u64,
        compressed: &compressed,
    };
    assert!(
        decode_manifest_section(&corrupt, 1)
            .unwrap_err()
            .contains("omission policy")
    );

    let mut decoded = zstd::bulk::decompress(
        encoded.compressed,
        usize::try_from(encoded.decoded_length).unwrap(),
    )
    .unwrap();
    let second_path = first_tags + 3 + 32;
    let second_tags = second_path + 1 + "reraconfig.toml".len();
    decoded[second_tags + 3] ^= 1;
    let compressed = zstd::bulk::compress(&decoded, CACHE_COMPRESSION_LEVEL).unwrap();
    let corrupt = EncodedSectionRef {
        decoded_length: decoded.len() as u64,
        compressed: &compressed,
    };
    assert!(
        decode_manifest_section(&corrupt, 1)
            .unwrap_err()
            .contains("payload hash mismatch")
    );

    let source_record = source_record_from_file(&project.files[0]).unwrap();
    let encoded = encode_compact_source_record_section(
        &[source_record],
        &project,
        ProjectContainerKind::CompiledCache,
        None,
    )
    .unwrap();
    let mut cursor = 0;
    let section = read_section(&encoded, &mut cursor, encoded.len()).unwrap();
    let mut decoded = zstd::bulk::decompress(
        section.compressed,
        usize::try_from(section.decoded_length).unwrap(),
    )
    .unwrap();
    *decoded.last_mut().unwrap() = u8::MAX;
    let compressed = zstd::bulk::compress(&decoded, CACHE_COMPRESSION_LEVEL).unwrap();
    let corrupt = EncodedSectionRef {
        decoded_length: decoded.len() as u64,
        compressed: &compressed,
    };
    assert!(decode_compact_source_record_section(&corrupt, &project).is_err());
}

include!("tests_continued.rs");
