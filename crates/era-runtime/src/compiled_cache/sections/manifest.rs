pub(super) fn encode_diagnostic_templates(
    diagnostics: &[ProtocolDiagnostic],
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let templates = diagnostics
        .iter()
        .cloned()
        .map(|mut diagnostic| {
            crate::compatibility::clear_diagnostic_scope(&mut diagnostic);
            diagnostic
        })
        .collect::<Vec<_>>();
    encode_section(&templates, kind, cancelled)
}

pub(super) fn encode_section<T: Serialize + ?Sized>(
    value: &T,
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        rmp_serde::encode::write(writer, value).map_err(|error| error.to_string())
    })
}

pub(super) fn decode_manifest_section(
    section: &EncodedSectionRef<'_>,
    project_revision: u64,
    version: u8,
) -> Result<ProjectManifest, String> {
    decode_raw_section(section, |reader| {
        let mut magic = [0_u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|error| error.to_string())?;
        let compact = match &magic {
            value if value == MANIFEST_SECTION_MAGIC && version >= PROFILED_PROJECT_VERSION => {
                false
            }
            value
                if value == COMPACT_MANIFEST_SECTION_MAGIC
                    && version >= PROFILED_PROJECT_VERSION =>
            {
                true
            }
            value
                if value == LEGACY_MANIFEST_SECTION_MAGIC && version < PROFILED_PROJECT_VERSION =>
            {
                false
            }
            value
                if value == LEGACY_COMPACT_MANIFEST_SECTION_MAGIC
                    && version < PROFILED_PROJECT_VERSION =>
            {
                true
            }
            _ => return Err("project manifest has invalid magic".into()),
        };
        let compatibility = if version >= PROFILED_PROJECT_VERSION {
            let bytes = read_bytes(reader, 4096)?;
            let identity: erabasic_compat::CompatibilityIdentity =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            identity.validate().map_err(|error| error.to_string())?;
            identity
        } else {
            erabasic_compat::CompatibilityIdentity::default()
        };
        let count = read_count(reader, section.decoded_length, "project manifest file")?;
        let mut files = Vec::new();
        let mut paths = BTreeSet::new();
        files
            .try_reserve_exact(count)
            .map_err(|_| "project manifest allocation failed")?;
        for _ in 0..count {
            let relative_path = String::from_utf8(read_bytes(reader, section.decoded_length)?)
                .map_err(|_| "project manifest path is not UTF-8")?;
            let normalized_path =
                validate_relative_path(&relative_path).map_err(|error| error.to_string())?;
            if !paths.insert(normalized_path.to_lowercase()) {
                return Err("project manifest contains duplicate paths".into());
            }
            let mut tags = [0_u8; 3];
            reader
                .read_exact(&mut tags)
                .map_err(|error| error.to_string())?;
            let category = decode_file_category(tags[0])?;
            let (payload, content_hash) = if compact {
                decode_compact_manifest_payload(reader, section.decoded_length, category, tags)?
            } else {
                decode_full_manifest_payload(reader, section.decoded_length, tags)?
            };
            files.push(SubmittedFile {
                relative_path,
                category,
                payload,
                content_hash,
            });
        }
        Ok(ProjectManifest {
            project_revision,
            compatibility,
            files,
        })
    })
}

pub(super) fn decode_compact_manifest_payload(
    reader: &mut dyn std::io::Read,
    decoded_length: u64,
    category: FileCategory,
    tags: [u8; 3],
) -> Result<(FilePayload, Option<ProtocolBytes>), String> {
    let mut hash = [0_u8; blake3::OUT_LEN];
    reader
        .read_exact(&mut hash)
        .map_err(|error| error.to_string())?;
    let omitted = match tags[2] {
        0 => false,
        1 => true,
        _ => return Err("project manifest omission tag is invalid".into()),
    };
    let expected_omitted = !matches!(
        category,
        FileCategory::Configuration | FileCategory::ResourceManifest
    );
    if omitted != expected_omitted {
        return Err("project cache manifest violates its payload omission policy".into());
    }
    let expected_bytes = category == FileCategory::Resource;
    if (tags[1] == 1) != expected_bytes || tags[1] > 1 {
        return Err("project cache manifest payload type disagrees with its category".into());
    }
    let payload_bytes = if omitted {
        Vec::new()
    } else {
        read_bytes(reader, decoded_length)?
    };
    let payload = decode_manifest_payload(payload_bytes, tags[1])?;
    if !omitted && manifest_payload_hash(&payload).as_bytes() != &hash {
        return Err("project cache manifest payload hash mismatch".into());
    }
    Ok((payload, Some(ProtocolBytes::new(hash.to_vec()))))
}

pub(super) fn decode_full_manifest_payload(
    reader: &mut dyn std::io::Read,
    decoded_length: u64,
    tags: [u8; 3],
) -> Result<(FilePayload, Option<ProtocolBytes>), String> {
    let payload = decode_manifest_payload(read_bytes(reader, decoded_length)?, tags[2])?;
    let content_hash = match tags[1] {
        0 => None,
        1 => Some(ProtocolBytes::new(
            manifest_payload_hash(&payload).as_bytes().to_vec(),
        )),
        _ => return Err("project manifest hash-presence tag is invalid".into()),
    };
    Ok((payload, content_hash))
}

pub(super) fn decode_manifest_payload(bytes: Vec<u8>, tag: u8) -> Result<FilePayload, String> {
    match tag {
        0 => String::from_utf8(bytes)
            .map(FilePayload::Utf8)
            .map_err(|_| "project manifest text payload is not UTF-8".into()),
        1 => Ok(FilePayload::Bytes(ProtocolBytes::new(bytes))),
        _ => Err("project manifest payload tag is invalid".into()),
    }
}

pub(super) fn manifest_payload_hash(payload: &FilePayload) -> blake3::Hash {
    match payload {
        FilePayload::Utf8(text) => blake3::hash(text.as_bytes()),
        FilePayload::Bytes(bytes) => blake3::hash(bytes.as_slice()),
        FilePayload::ExternalResource(_) => {
            unreachable!("external resources are omitted from compact caches")
        }
        FilePayload::IoError(_) => unreachable!(),
    }
}

/// Compact caches retain validated source offsets alongside manifest hashes, but
/// intentionally omit source payloads. Re-exporting such a cache must compare the
/// retained identity rather than treating the omitted text as an empty source.
fn compact_source_matches_manifest(
    source: &SourceRecord,
    file: &SubmittedFile,
) -> Result<bool, String> {
    let omitted = !matches!(
        file.category,
        FileCategory::Configuration | FileCategory::ResourceManifest
    ) && matches!(&file.payload, FilePayload::Utf8(text) if text.is_empty())
        && file
            .content_hash
            .as_ref()
            .is_some_and(|hash| hash.as_slice() != blake3::hash(&[]).as_bytes());
    if omitted {
        return Ok(file
            .content_hash
            .as_ref()
            .is_some_and(|hash| hash.as_slice() == source.content_hash.0)
            && validate_relative_path(&file.relative_path).map_err(|error| error.to_string())?
                == source.relative_path);
    }
    Ok(source_record_from_file(file)? == *source)
}

pub(super) fn encode_compact_source_record_section(
    sources: &[SourceRecord],
    manifest: &ProjectManifest,
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let manifest_indices = manifest
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            validate_relative_path(&file.relative_path)
                .map(|path| (path, index))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let records = sources
        .iter()
        .map(|source| -> Result<(usize, &SourceRecord), String> {
            let index = *manifest_indices
                .get(&source.relative_path)
                .ok_or_else(|| "bytecode source is missing from the project manifest".to_owned())?;
            let file = &manifest.files[index];
            if !compact_source_matches_manifest(source, file)? {
                return Err("bytecode source differs from the project manifest".into());
            }
            Ok((index, source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(COMPACT_SOURCE_RECORD_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(records.len()).map_err(|_| "too many bytecode sources")?,
        )?;
        for &(index, source) in &records {
            write_varint(
                writer,
                u64::try_from(index).map_err(|_| "manifest file index is too large")?,
            )?;
            write_varint(writer, source.byte_len)?;
            write_varint(
                writer,
                u64::try_from(source.line_starts.len())
                    .map_err(|_| "source record has too many lines")?,
            )?;
            let mut previous = 0_u64;
            for &line_start in &source.line_starts {
                let delta = line_start
                    .checked_sub(previous)
                    .ok_or("source record line starts are not ordered")?;
                write_varint(writer, delta)?;
                previous = line_start;
            }
        }
        Ok(())
    })
}

pub(super) fn decode_compact_source_record_section(
    section: &EncodedSectionRef<'_>,
    manifest: &ProjectManifest,
) -> Result<Vec<SourceRecord>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(
            reader,
            *COMPACT_SOURCE_RECORD_SECTION_MAGIC,
            "compact source record",
        )?;
        let count = read_count(reader, section.decoded_length, "compact source record")?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(count)
            .map_err(|_| "compact source record allocation failed")?;
        for _ in 0..count {
            let index = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "manifest source index is not addressable")?;
            let file = manifest
                .files
                .get(index)
                .ok_or("manifest source index is out of range")?;
            if !matches!(
                file.category,
                FileCategory::Csv
                    | FileCategory::Erh
                    | FileCategory::Erb
                    | FileCategory::Als
                    | FileCategory::Erd
            ) {
                return Err("compact source record refers to a non-source manifest file".into());
            }
            let hash: [u8; 32] = file
                .content_hash
                .as_ref()
                .ok_or("compact source record is missing its manifest hash")?
                .as_slice()
                .try_into()
                .map_err(|_| "compact source record manifest hash is not 32 bytes")?;
            let byte_len = read_stream_varint(reader)?;
            let line_count = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "compact source line count is not addressable")?;
            if line_count == 0
                || u64::try_from(line_count).unwrap_or(u64::MAX) > byte_len.saturating_add(1)
            {
                return Err("compact source line count is invalid".into());
            }
            let mut line_starts = Vec::new();
            line_starts
                .try_reserve_exact(line_count)
                .map_err(|_| "compact source line allocation failed")?;
            let mut previous = 0_u64;
            for line_index in 0..line_count {
                let delta = read_stream_varint(reader)?;
                if (line_index == 0 && delta != 0) || (line_index != 0 && delta == 0) {
                    return Err("compact source line deltas are invalid".into());
                }
                previous = previous
                    .checked_add(delta)
                    .ok_or("compact source line delta overflows")?;
                if previous > byte_len {
                    return Err("compact source line start exceeds its byte length".into());
                }
                line_starts.push(previous);
            }
            sources.push(SourceRecord {
                relative_path: validate_relative_path(&file.relative_path)
                    .map_err(|error| error.to_string())?,
                content_hash: Digest(hash),
                byte_len,
                line_starts,
            });
        }
        Ok(sources)
    })
}

pub(super) fn encode_source_record_section(
    sources: &[SourceRecord],
    manifest: &ProjectManifest,
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let manifest_indices = manifest
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            validate_relative_path(&file.relative_path)
                .map(|path| (path, index))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let indices = sources
        .iter()
        .map(|source| -> Result<usize, String> {
            let index = *manifest_indices
                .get(&source.relative_path)
                .ok_or_else(|| "bytecode source is missing from the project manifest".to_owned())?;
            let file = &manifest.files[index];
            if source_record_from_file(file)? != *source {
                return Err("bytecode source differs from the project manifest".into());
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(SOURCE_RECORD_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(indices.len()).map_err(|_| "too many bytecode sources")?,
        )?;
        for &index in &indices {
            write_varint(
                writer,
                u64::try_from(index).map_err(|_| "manifest file index is too large")?,
            )?;
        }
        Ok(())
    })
}

pub(super) fn decode_source_record_section(
    section: &EncodedSectionRef<'_>,
    manifest: &ProjectManifest,
) -> Result<Vec<SourceRecord>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(reader, *SOURCE_RECORD_SECTION_MAGIC, "source record")?;
        let count = read_count(reader, section.decoded_length, "source record")?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(count)
            .map_err(|_| "source record allocation failed")?;
        for _ in 0..count {
            let index = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "manifest source index is not addressable")?;
            let file = manifest
                .files
                .get(index)
                .ok_or("manifest source index is out of range")?;
            sources.push(source_record_from_file(file)?);
        }
        Ok(sources)
    })
}

pub(super) fn source_record_from_file(file: &SubmittedFile) -> Result<SourceRecord, String> {
    let FilePayload::Utf8(text) = &file.payload else {
        return Err("bytecode source does not refer to a text manifest file".into());
    };
    let mut line_starts =
        Vec::with_capacity(text.bytes().filter(|byte| *byte == b'\n').count() + 1);
    line_starts.push(0);
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .map(|(index, _)| u64::try_from(index + 1).unwrap_or(u64::MAX)),
    );
    let content_hash = file.content_hash.as_ref().map_or_else(
        || *blake3::hash(text.as_bytes()).as_bytes(),
        |hash| {
            hash.as_slice()
                .try_into()
                .unwrap_or_else(|_| *blake3::hash(text.as_bytes()).as_bytes())
        },
    );
    Ok(SourceRecord {
        relative_path: validate_relative_path(&file.relative_path)
            .map_err(|error| error.to_string())?,
        content_hash: Digest(content_hash),
        byte_len: u64::try_from(text.len()).map_err(|_| "source text is too large")?,
        line_starts,
    })
}

pub(super) fn encode_incremental_section(
    cache_keys: &[Digest],
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(INCREMENTAL_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(cache_keys.len()).map_err(|_| "too many incremental cache keys")?,
        )?;
        for key in cache_keys {
            writer
                .write_all(&key.0)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

pub(super) fn decode_incremental_section(
    section: &EncodedSectionRef<'_>,
) -> Result<Vec<Digest>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(reader, *INCREMENTAL_SECTION_MAGIC, "incremental cache")?;
        let count = read_count(reader, section.decoded_length / 32, "incremental cache key")?;
        let mut keys = Vec::new();
        keys.try_reserve_exact(count)
            .map_err(|_| "incremental cache allocation failed")?;
        for _ in 0..count {
            let mut key = [0_u8; 32];
            reader
                .read_exact(&mut key)
                .map_err(|error| error.to_string())?;
            keys.push(Digest(key));
        }
        Ok(keys)
    })
}

