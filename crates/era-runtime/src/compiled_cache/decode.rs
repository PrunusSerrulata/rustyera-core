#[allow(clippy::wildcard_imports)]
use super::*;

#[cfg(test)]
pub(crate) fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<DecodedCompiledCache, String> {
    decode_with_progress(bytes, maximum_bytes, None)
}

pub(crate) fn decode_with_progress(
    bytes: &[u8],
    maximum_bytes: usize,
    progress: Option<&crate::ProjectProgressReporter>,
) -> Result<DecodedCompiledCache, String> {
    report_decode_stage(progress, crate::ProjectProgressStage::CacheParsing, 0);
    let sections = parse_cache_sections(bytes, maximum_bytes)?;
    report_decode_stage(progress, crate::ProjectProgressStage::CacheParsing, 1);
    report_decode_stage(progress, crate::ProjectProgressStage::CacheDecoding, 0);
    let parts = decode_cache_parts(&sections)?;
    report_decode_stage(progress, crate::ProjectProgressStage::CacheDecoding, 1);
    let artifact = BytecodeArtifact {
        manifest: parts.metadata.manifest,
        call_compatibility: parts.metadata.call_compatibility,
        project_data: parts.project_data,
        globals: parts.globals,
        native_imports: parts.metadata.native_imports,
        host_imports: parts.metadata.host_imports,
        functions: parts.functions,
        event_groups: parts.metadata.event_groups,
        source_map: SourceMap {
            sources: parts.sources,
            statement_fingerprints: parts.fingerprints,
            entries: parts.source_entries,
        },
    };
    report_decode_stage(progress, crate::ProjectProgressStage::CacheValidating, 0);
    let unvalidated = artifact.into_unvalidated();
    let context = ValidationContext::for_artifact(unvalidated.artifact());
    let validation = validate_bytecode(unvalidated, &context);
    let artifact = validation.value.ok_or_else(|| {
        validation.diagnostics.first().map_or_else(
            || "cached artifact failed validation".into(),
            |value| value.message.clone(),
        )
    })?;
    report_decode_stage(progress, crate::ProjectProgressStage::CacheValidating, 1);
    let incremental = IncrementalState::from_compact_cache_keys(
        artifact.artifact(),
        parts.incremental_cache_keys,
    )?;
    Ok(DecodedCompiledCache {
        key: sections.key,
        artifact,
        incremental,
        snapshot: parts.snapshot,
        diagnostics: parts.diagnostics,
    })
}

fn report_decode_stage(
    progress: Option<&crate::ProjectProgressReporter>,
    stage: crate::ProjectProgressStage,
    completed: u64,
) {
    if let Some(progress) = progress {
        progress.report(crate::ProjectProgress {
            stage,
            completed,
            total: 1,
        });
    }
}

/// Decode the identity and exact frontend-submitted manifest embedded in a project file.
///
/// This source-only projection reuses the runtime cache parser but deliberately avoids
/// decoding or validating bytecode sections that an extraction tool does not consume.
///
/// # Errors
///
/// Returns an error when the cache is over the caller's limit, corrupt, unsupported, or
/// does not contain a decodable project snapshot.
pub fn decode_project_file(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<DecodedProjectFile, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    require_full_project(&sections)?;
    let mut manifest =
        decode_manifest_section(&sections.manifest, sections.identity.project_revision)
            .map_err(ProjectFileError::from)?;
    let actual_identity = project_identity(&manifest);
    if actual_identity != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    Ok(DecodedProjectFile {
        identity: project_identity(&manifest),
        manifest,
    })
}

/// Decode a compact project-file manifest for frontend-owned resource and diagnostic I/O.
///
/// Non-resource payloads that are not referenced by a cached diagnostic are cleared. Their
/// original content hashes remain available for identity validation, while the full cache import
/// remains authoritative for runtime loading.
///
/// # Errors
///
/// Returns an error under the same conditions as [`decode_project_file`].
pub fn decode_project_file_frontend_manifest(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<DecodedProjectFile, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    require_full_project(&sections)?;
    let (manifest, diagnostics) = rayon::join(
        || decode_manifest_section(&sections.manifest, sections.identity.project_revision),
        || decode_section::<Vec<ProtocolDiagnostic>>(&sections.diagnostics),
    );
    let mut manifest = manifest.map_err(ProjectFileError::from)?;
    if project_identity(&manifest) != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    let identity = project_identity(&manifest);
    compact_frontend_manifest(&mut manifest, &diagnostics.map_err(ProjectFileError::from)?);
    Ok(DecodedProjectFile { identity, manifest })
}

/// Validate a project file and prepare one compact append-only configuration update.
///
/// The returned bytes contain only the journal record, not a regenerated project container.
/// Callers must truncate an interrupted trailing record to [`ProjectConfigurationUpdate::truncate_to`]
/// before appending. The embedded configuration is compared with `expected_digest` using
/// normalized LF line endings; an empty digest represents a missing `reraconfig.toml`.
///
/// # Errors
///
/// Returns an error when the project file or requested TOML is invalid, the transfer limit is
/// exceeded, or the optimistic-lock digest no longer matches and the requested contents have not
/// already been applied.
pub fn prepare_project_configuration_update(
    bytes: &[u8],
    maximum_bytes: usize,
    expected_digest: &[u8],
    contents: &str,
) -> Result<ProjectConfigurationUpdate, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    require_full_project(&sections)?;
    let mut manifest =
        decode_manifest_section(&sections.manifest, sections.identity.project_revision)
            .map_err(ProjectFileError::from)?;
    if project_identity(&manifest) != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    let current = configuration_digest(&manifest).map_err(ProjectFileError::from)?;
    let requested_source = era_config::normalize_line_endings(contents);
    let requested_digest = *blake3::hash(requested_source.as_bytes()).as_bytes();
    let expected_matches = match current {
        Some(digest) => expected_digest == digest.as_slice(),
        None => expected_digest.is_empty(),
    };
    if !expected_matches && current != Some(requested_digest) {
        return Err(ProjectFileError::from(
            "reraconfig.toml was modified by another process".to_owned(),
        ));
    }
    let (append, source_digest) =
        encode_record(current, contents).map_err(ProjectFileError::from)?;
    let append = if current == Some(source_digest) {
        Vec::new()
    } else {
        replace_configuration(&mut manifest, &requested_source, source_digest);
        append
    };
    if sections
        .configuration_journal
        .valid_end
        .checked_add(append.len())
        .is_none_or(|length| length > maximum_bytes)
    {
        return Err(ProjectFileError::from(
            "project configuration update exceeds the transfer limit".to_owned(),
        ));
    }
    Ok(ProjectConfigurationUpdate {
        truncate_to: u64::try_from(sections.configuration_journal.valid_end)
            .map_err(|_| ProjectFileError::from("project file is too large".to_owned()))?,
        append,
        identity: project_identity(&manifest),
    })
}

pub(super) fn compact_frontend_manifest(
    manifest: &mut ProjectManifest,
    diagnostics: &[ProtocolDiagnostic],
) {
    let diagnostic_sources = diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.source.as_ref())
        .map(|source| source.relative_path.to_lowercase())
        .collect::<BTreeSet<_>>();
    for file in &mut manifest.files {
        if file.content_hash.is_none() {
            let payload = match &file.payload {
                FilePayload::Utf8(text) => text.as_bytes(),
                FilePayload::Bytes(bytes) => bytes.as_slice(),
                FilePayload::IoError(_) | FilePayload::ExternalResource(_) => continue,
            };
            file.content_hash = Some(ProtocolBytes::new(
                blake3::hash(payload).as_bytes().to_vec(),
            ));
        }
        if file.category == FileCategory::Resource
            || diagnostic_sources.contains(&file.relative_path.to_lowercase())
        {
            continue;
        }
        match &mut file.payload {
            FilePayload::Utf8(text) => text.clear(),
            FilePayload::Bytes(bytes) => *bytes = ProtocolBytes::new(Vec::new()),
            FilePayload::IoError(error) => error.message.clear(),
            FilePayload::ExternalResource(_) => {}
        }
    }
}

pub(super) fn parse_cache_sections(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<CompiledCacheSections<'_>, String> {
    let ParsedContainerHeader {
        kind,
        version,
        identity,
        key,
        function_section_count,
        source_section_count,
        mut cursor,
    } = parse_container_header(bytes, maximum_bytes)?;
    let metadata = read_section(bytes, &mut cursor, bytes.len())?;
    let globals = read_section(bytes, &mut cursor, bytes.len())?;
    let incremental = read_section(bytes, &mut cursor, bytes.len())?;
    let project_data = read_section(bytes, &mut cursor, bytes.len())?;
    let sources = read_section(bytes, &mut cursor, bytes.len())?;
    let fingerprints = read_section(bytes, &mut cursor, bytes.len())?;
    let manifest = read_section(bytes, &mut cursor, bytes.len())?;
    let snapshot = read_section(bytes, &mut cursor, bytes.len())?;
    let diagnostics = read_section(bytes, &mut cursor, bytes.len())?;
    let functions = read_section_list(bytes, &mut cursor, function_section_count)?;
    let source_entries = read_section_list(bytes, &mut cursor, source_section_count)?;
    let journal_start = cursor
        .checked_add(32)
        .ok_or("compiled project cache digest offset overflows")?;
    let configuration_journal = parse_configuration_journal(bytes, cursor)?;
    if kind == ProjectContainerKind::CompiledCache && bytes.len() != journal_start {
        return Err("compiled project cache cannot contain a configuration journal".into());
    }
    let fixed_sections = [
        &metadata,
        &globals,
        &incremental,
        &project_data,
        &sources,
        &fingerprints,
        &manifest,
        &snapshot,
        &diagnostics,
    ];
    debug_assert_eq!(fixed_sections.len(), FIXED_SECTION_COUNT);
    let decoded_bytes = fixed_sections
        .into_iter()
        .chain(&functions)
        .chain(&source_entries)
        .try_fold(0_u64, |total, section| {
            total.checked_add(section.decoded_length)
        })
        .ok_or("compiled cache decoded length overflow")?;
    if decoded_bytes > MAXIMUM_DECODED_PAYLOAD_BYTES {
        return Err("compiled cache decoded sections exceed their limit".into());
    }
    Ok(CompiledCacheSections {
        kind,
        version,
        identity,
        key,
        metadata,
        globals,
        incremental,
        project_data,
        sources,
        fingerprints,
        manifest,
        snapshot,
        diagnostics,
        functions,
        source_entries,
        configuration_journal,
    })
}

struct ParsedContainerHeader {
    kind: ProjectContainerKind,
    version: u8,
    identity: ProjectIdentity,
    key: [u8; 32],
    function_section_count: usize,
    source_section_count: usize,
    cursor: usize,
}

fn parse_container_header(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<ParsedContainerHeader, String> {
    if bytes.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    let magic_length = PROJECT_MAGIC.len();
    let minimum =
        magic_length + 1 + 8 + 32 + 32 + 4 + 4 + FIXED_SECTION_COUNT * 16 + 32;
    if bytes.len() < minimum {
        return Err("project file has an invalid header".into());
    }
    let kind = match &bytes[..magic_length] {
        magic if magic == PROJECT_MAGIC => ProjectContainerKind::FullProject,
        magic if magic == CACHE_MAGIC => ProjectContainerKind::CompiledCache,
        _ => return Err("project file has an invalid header".into()),
    };
    let mut cursor = magic_length;
    let version = *bytes
        .get(cursor)
        .ok_or("project file version is truncated")?;
    cursor += 1;
    if !matches!(
        (kind, version),
        (
            ProjectContainerKind::FullProject,
            LEGACY_PROJECT_VERSION | PREVIOUS_PROJECT_VERSION | VERSION
        ) | (ProjectContainerKind::CompiledCache, VERSION)
    ) {
        return Err(format!("unsupported project file version {version:02x}"));
    }
    let project_revision = read_u64(bytes, &mut cursor)?;
    let source_digest = bytes
        .get(cursor..cursor + 32)
        .ok_or("project file source identity is truncated")?
        .to_vec();
    cursor += 32;
    let identity = ProjectIdentity {
        project_revision,
        source_digest: ProtocolBytes::new(source_digest),
    };
    let key: [u8; 32] = bytes
        .get(cursor..cursor + 32)
        .ok_or("compiled project cache key is truncated")?
        .try_into()
        .expect("32-byte slice");
    cursor += 32;
    let function_section_count = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| "compiled cache function section count is not addressable")?;
    let source_section_count = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| "compiled cache source section count is not addressable")?;
    if function_section_count > TARGET_PARALLEL_SECTIONS.saturating_mul(2)
        || source_section_count > TARGET_PARALLEL_SECTIONS
    {
        return Err("compiled project cache has too many parallel sections".into());
    }
    Ok(ParsedContainerHeader {
        kind,
        version,
        identity,
        key,
        function_section_count,
        source_section_count,
        cursor,
    })
}

fn read_section_list<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    count: usize,
) -> Result<Vec<EncodedSectionRef<'a>>, String> {
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        sections.push(read_section(bytes, cursor, bytes.len())?);
    }
    Ok(sections)
}

fn require_full_project(sections: &CompiledCacheSections<'_>) -> Result<(), ProjectFileError> {
    if sections.kind != ProjectContainerKind::FullProject {
        return Err(ProjectFileError::from(
            "compiled project caches are not portable project files".to_owned(),
        ));
    }
    Ok(())
}

fn parse_configuration_journal(
    bytes: &[u8],
    digest_offset: usize,
) -> Result<ConfigurationJournal<'_>, String> {
    let digest_end = digest_offset
        .checked_add(32)
        .ok_or("compiled project cache digest offset overflows")?;
    let digest = bytes
        .get(digest_offset..digest_end)
        .ok_or("compiled project cache digest is truncated")?;
    if blake3::hash(&bytes[..digest_offset]).as_bytes() != digest {
        return Err("compiled project cache digest mismatch".into());
    }
    parse_journal(bytes, digest_end)
}

#[derive(Clone, Copy, Default)]
pub(super) struct CacheDecodeDelays {
    pub(super) source_records: std::time::Duration,
    pub(super) source_entries: std::time::Duration,
    pub(super) independent: std::time::Duration,
}

struct IndependentCacheParts {
    metadata: CompiledCacheMetadata,
    globals: Vec<BytecodeGlobal>,
    incremental_cache_keys: Vec<Digest>,
    project_data: erabasic_data::ProjectData,
    fingerprints: Vec<Digest>,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
}

fn cache_decode_delay(delay: std::time::Duration) {
    #[cfg(test)]
    std::thread::sleep(delay);
    #[cfg(not(test))]
    debug_assert!(delay.is_zero());
}

pub(super) fn decode_cache_parts(
    sections: &CompiledCacheSections<'_>,
) -> Result<DecodedCacheParts, String> {
    decode_cache_parts_with_delays(sections, CacheDecodeDelays::default())
}

pub(super) fn decode_cache_parts_with_delays(
    sections: &CompiledCacheSections<'_>,
    delays: CacheDecodeDelays,
) -> Result<DecodedCacheParts, String> {
    let (manifest_and_sources, (functions_and_entries, independent)) = rayon::join(
        || decode_manifest_and_sources(sections, delays.source_records),
        || {
            rayon::join(
                || decode_functions_and_entries(sections, delays.source_entries),
                || decode_independent_cache_parts(sections, delays.independent),
            )
        },
    );
    let (manifest, sources) = manifest_and_sources?;
    let (functions, entries) = functions_and_entries?;
    let IndependentCacheParts {
        metadata,
        globals,
        incremental_cache_keys,
        project_data,
        fingerprints,
        snapshot,
        diagnostics,
    } = independent?;
    let snapshot = snapshot.into_snapshot(manifest)?;
    Ok(DecodedCacheParts {
        metadata,
        globals,
        incremental_cache_keys,
        project_data,
        sources,
        fingerprints,
        snapshot,
        diagnostics,
        functions,
        source_entries: entries,
    })
}

fn decode_manifest_and_sources(
    sections: &CompiledCacheSections<'_>,
    delay: std::time::Duration,
) -> Result<(ProjectManifest, Vec<SourceRecord>), String> {
    let mut manifest =
        decode_manifest_section(&sections.manifest, sections.identity.project_revision)
            .map_err(|error| format!("manifest section: {error}"))?;
    if project_identity(&manifest) != sections.identity {
        return Err("project file identity does not match its embedded manifest".into());
    }
    apply_journal(&mut manifest, &sections.configuration_journal)?;
    cache_decode_delay(delay);
    let sources =
        if sections.kind == ProjectContainerKind::CompiledCache && sections.version == VERSION {
            decode_compact_source_record_section(&sections.sources, &manifest)
        } else {
            decode_source_record_section(&sections.sources, &manifest)
        }
        .map_err(|error| format!("source-record section: {error}"))?;
    Ok((manifest, sources))
}

fn decode_functions_and_entries(
    sections: &CompiledCacheSections<'_>,
    delay: std::time::Duration,
) -> Result<(Vec<BytecodeFunction>, Vec<SourceMapEntry>), String> {
    let functions = decode_function_sections(&sections.functions)?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    cache_decode_delay(delay);
    let entries = decode_source_sections(&sections.source_entries, &functions)?
        .into_iter()
        .flatten()
        .collect();
    Ok((functions, entries))
}

fn decode_independent_cache_parts(
    sections: &CompiledCacheSections<'_>,
    delay: std::time::Duration,
) -> Result<IndependentCacheParts, String> {
    cache_decode_delay(delay);
    let ((metadata, globals), (diagnostics, snapshot)) = rayon::join(
        || {
            rayon::join(
                || decode_named_section("metadata", &sections.metadata),
                || decode_named_section("globals", &sections.globals),
            )
        },
        || {
            rayon::join(
                || decode_named_section("diagnostics", &sections.diagnostics),
                || decode_named_section("snapshot", &sections.snapshot),
            )
        },
    );
    let ((incremental_cache_keys, project_data), fingerprints) = rayon::join(
        || {
            rayon::join(
                || {
                    decode_incremental_section(&sections.incremental)
                        .map_err(|error| format!("incremental section: {error}"))
                },
                || decode_named_section("project-data", &sections.project_data),
            )
        },
        || {
            decode_digest_section(&sections.fingerprints)
                .map_err(|error| format!("fingerprint section: {error}"))
        },
    );
    Ok(IndependentCacheParts {
        metadata: metadata?,
        globals: globals?,
        incremental_cache_keys: incremental_cache_keys?,
        project_data: project_data?,
        fingerprints: fingerprints?,
        snapshot: snapshot?,
        diagnostics: diagnostics?,
    })
}

fn decode_named_section<T>(name: &str, section: &EncodedSectionRef<'_>) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    decode_section(section).map_err(|error| format!("{name} section: {error}"))
}

fn decode_function_sections(
    sections: &[EncodedSectionRef<'_>],
) -> Result<Vec<Vec<BytecodeFunction>>, String> {
    sections
        .par_iter()
        .enumerate()
        .map(|(index, section)| {
            decode_section::<Vec<BytecodeFunction>>(section)
                .map_err(|error| format!("function section {index}: {error}"))
        })
        .collect()
}

fn decode_source_sections(
    sections: &[EncodedSectionRef<'_>],
    functions: &[BytecodeFunction],
) -> Result<Vec<Vec<SourceMapEntry>>, String> {
    sections
        .par_iter()
        .enumerate()
        .map(|(index, section)| {
            decode_source_section(section, functions)
                .map_err(|error| format!("source-entry section {index}: {error}"))
        })
        .collect()
}
