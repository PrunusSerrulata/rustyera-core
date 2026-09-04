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
    let embedded_source_digest = sections.identity.source_digest.clone();
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
    // Vec capacity may grow to almost twice its current length when a chunk crosses an allocation
    // boundary, so bound retained memory rather than only the initialized compressed bytes.
    let retained_bound =
        stream::HEADER_BYTES + manifest_compressed_bytes * 2 + maximum_record_bytes * 2 + 13;
    assert!(maximum_retained <= retained_bound);
    assert_ne!(
        streamed.project.identity.source_digest,
        embedded_source_digest
    );
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
fn streamed_project_file_custom_decoded_budget_precedes_manifest_allocation() {
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
    let mut offset = stream::HEADER_BYTES;
    let mut decoded_through_manifest = 0_u64;
    for index in 0..=MANIFEST_SECTION_INDEX {
        decoded_through_manifest +=
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if index != MANIFEST_SECTION_INDEX {
            offset += 16
                + usize::try_from(u64::from_le_bytes(
                    bytes[offset + 8..offset + 16].try_into().unwrap(),
                ))
                .unwrap();
        }
    }
    let mut limited = ProjectFileStreamDecoder::new_with_decoded_limit(
        bytes.len(),
        bytes.len(),
        decoded_through_manifest - 1,
    )
    .unwrap();
    let error = limited.append(&bytes[..offset + 16]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("decoded sections exceed their limit")
    );
    assert_eq!(limited.retained_bytes(), stream::HEADER_BYTES);

    let mut normal = ProjectFileStreamDecoder::new_with_decoded_limit(
        bytes.len(),
        bytes.len(),
        128 * 1024 * 1024,
    )
    .unwrap();
    for chunk in bytes.chunks(17) {
        normal.append(chunk).unwrap();
    }
    assert_eq!(
        normal.finish().unwrap().project,
        decode_project_file(&bytes, bytes.len()).unwrap()
    );

    let mut attempted_relaxation = bytes.clone();
    attempted_relaxation[stream::HEADER_BYTES..stream::HEADER_BYTES + 8]
        .copy_from_slice(&(MAXIMUM_DECODED_PAYLOAD_BYTES + 1).to_le_bytes());
    let mut hard_limit =
        ProjectFileStreamDecoder::new_with_decoded_limit(bytes.len(), bytes.len(), u64::MAX)
            .unwrap();
    assert!(hard_limit.append(&attempted_relaxation).is_err());
    assert_eq!(hard_limit.retained_bytes(), stream::HEADER_BYTES);
    assert!(
        ProjectFileStreamDecoder::new_with_decoded_limit(bytes.len(), bytes.len() - 1, u64::MAX)
            .is_err()
    );
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
    let reexported = encode_compiled_cache_for_test(
        &decoded.snapshot.manifest,
        &[],
        &decoded.artifact,
        &decoded.incremental,
        &decoded.snapshot,
        &decoded.diagnostics,
    )
    .unwrap();
    let reloaded = decode(&reexported, 64 * 1024 * 1024).unwrap();
    assert_eq!(
        reloaded.artifact.artifact().source_map,
        decoded.artifact.artifact().source_map
    );
    assert_eq!(
        reloaded.artifact.artifact().manifest.artifact_id,
        decoded.artifact.artifact().manifest.artifact_id
    );
    let mut forged_manifest = decoded.snapshot.manifest.as_ref().clone();
    forged_manifest.files[0].content_hash = Some(ProtocolBytes::new(vec![0; 32]));
    assert!(
        encode_compiled_cache_for_test(
            &forged_manifest,
            &[],
            &decoded.artifact,
            &decoded.incremental,
            &decoded.snapshot,
            &decoded.diagnostics,
        )
        .is_err()
    );
    assert!(
        encode_full_project_for_test(
            &decoded.snapshot.manifest,
            &[],
            &decoded.artifact,
            &decoded.incremental,
            &decoded.snapshot,
            &decoded.diagnostics,
        )
        .is_err()
    );
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
    let mut prefix = &decoded[COMPACT_MANIFEST_SECTION_MAGIC.len()..];
    let _ = read_bytes(&mut prefix, 4096).unwrap();
    let policy_prefix = decoded.len() - prefix.len();
    let first_tags = policy_prefix + 1 + 1 + "main.erb".len();
    decoded[first_tags + 2] = 0;
    let compressed = zstd::bulk::compress(&decoded, CACHE_COMPRESSION_LEVEL).unwrap();
    let corrupt = EncodedSectionRef {
        decoded_length: decoded.len() as u64,
        compressed: &compressed,
    };
    assert!(
        decode_manifest_section(&corrupt, 1, VERSION)
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
        decode_manifest_section(&corrupt, 1, VERSION)
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
