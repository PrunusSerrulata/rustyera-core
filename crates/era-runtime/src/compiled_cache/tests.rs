use std::fmt::Write as _;

use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};

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
    let decoded_manifest = decode_compiled_project_manifest(&bytes, 64 * 1024 * 1024).unwrap();

    assert_eq!(decoded.key, project_key(&project_identity(&project), &[]));
    assert_eq!(decoded_manifest, project);
    assert_eq!(decoded.diagnostics, build.report.diagnostics);
    assert_eq!(
        decoded.artifact.artifact(),
        build.artifact.as_ref().unwrap().artifact()
    );
    assert_eq!(
        decoded.artifact.artifact().manifest.artifact_id,
        build.artifact.unwrap().artifact().manifest.artifact_id
    );
    assert_eq!(
        project_key(&project_identity(&project), &[]),
        project_key(
            &project_identity(&manifest("@SYSTEM_TITLE\nRETURN\n", 9)),
            &[]
        )
    );
    assert_ne!(
        project_key(&project_identity(&project), &[]),
        project_key(
            &project_identity(&manifest("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n", 1)),
            &[]
        )
    );
}

#[test]
fn compiled_project_cache_rejects_corruption() {
    assert!(decode(b"not a compiled cache", 1024).is_err());
    assert!(decode_compiled_project_manifest(b"not a compiled cache", 1024).is_err());
}

#[test]
fn source_manifest_projection_honors_limits_and_cache_version() {
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

    assert!(decode_compiled_project_manifest(&bytes, bytes.len() - 1).is_err());
    bytes[8..12].copy_from_slice(&4_u32.to_le_bytes());
    let digest_offset = bytes.len() - 32;
    let digest = blake3::hash(&bytes[..digest_offset]);
    bytes[digest_offset..].copy_from_slice(digest.as_bytes());
    assert!(
        decode_compiled_project_manifest(&bytes, 64 * 1024 * 1024)
            .unwrap_err()
            .to_string()
            .contains("unsupported compiled project cache version 4")
    );
}

#[test]
fn compiled_project_cache_encoding_honors_cancellation() {
    let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
    let mut build = crate::project::build_project(&project, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let cancelled = AtomicBool::new(true);

    let error = encode_cancellable(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
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
    let encoded = encode_source_section(&entries, None).unwrap();
    let mut cursor = 0;
    let section = read_section(&encoded, &mut cursor, encoded.len()).unwrap();

    assert_eq!(decode_source_section(&section).unwrap(), entries);
    assert_eq!(cursor, encoded.len());
}

#[test]
fn sharded_binary_cache_bounds_small_project_parallel_overhead() {
    #[derive(Serialize)]
    struct V2PayloadRef<'a> {
        artifact: &'a BytecodeArtifact,
        incremental: &'a IncrementalState,
        snapshot: &'a NormalizedProjectSnapshot,
        diagnostics: &'a [ProtocolDiagnostic],
    }

    const MAXIMUM_PARALLEL_SECTION_OVERHEAD: usize = 16 * 1024;

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
    let payload = V2PayloadRef {
        artifact: build.artifact.as_ref().unwrap().artifact(),
        incremental: &build.incremental,
        snapshot: build.snapshot.as_ref().unwrap(),
        diagnostics: &build.report.diagnostics,
    };
    let encoder = zstd::stream::Encoder::new(Vec::new(), 7).unwrap();
    let mut writer = CountingWriter::new(encoder, None);
    serde_json::to_writer(&mut writer, &payload).unwrap();
    let v2_payload = writer.into_inner().finish().unwrap();
    let cache_bytes = encode(
        &project,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();

    assert!(
        cache_bytes.len() <= v2_payload.len() + MAXIMUM_PARALLEL_SECTION_OVERHEAD,
        "encoded={} v2={}",
        cache_bytes.len(),
        v2_payload.len()
    );
}
