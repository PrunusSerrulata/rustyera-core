#[allow(clippy::wildcard_imports)]
use super::*;

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
            if source_record_from_file(file)? != *source {
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

pub(super) fn write_bytes(writer: &mut dyn std::io::Write, bytes: &[u8]) -> Result<(), String> {
    write_varint(
        writer,
        u64::try_from(bytes.len()).map_err(|_| "compiled cache byte string is too large")?,
    )?;
    writer.write_all(bytes).map_err(|error| error.to_string())
}

pub(super) fn read_bytes(reader: &mut dyn std::io::Read, maximum: u64) -> Result<Vec<u8>, String> {
    let length = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| "compiled cache byte string is not addressable")?;
    if u64::try_from(length).unwrap_or(u64::MAX) > maximum {
        return Err("compiled cache byte string exceeds its section".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "compiled cache byte string allocation failed")?;
    bytes.resize(length, 0);
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub(super) fn read_count(
    reader: &mut dyn std::io::Read,
    maximum: u64,
    name: &str,
) -> Result<usize, String> {
    let count = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| format!("compiled cache {name} count is not addressable"))?;
    if u64::try_from(count).unwrap_or(u64::MAX) > maximum {
        return Err(format!("compiled cache {name} count is invalid"));
    }
    Ok(count)
}

pub(super) fn expect_magic(
    reader: &mut dyn std::io::Read,
    expected: [u8; 4],
    name: &str,
) -> Result<(), String> {
    let mut actual = [0_u8; 4];
    reader
        .read_exact(&mut actual)
        .map_err(|error| error.to_string())?;
    if actual != expected {
        return Err(format!(
            "compiled cache {name} section has an invalid header"
        ));
    }
    Ok(())
}

pub(super) fn decode_file_category(value: u8) -> Result<FileCategory, String> {
    match value {
        0 => Ok(FileCategory::Csv),
        1 => Ok(FileCategory::Erh),
        2 => Ok(FileCategory::Erb),
        3 => Ok(FileCategory::ResourceManifest),
        4 => Ok(FileCategory::Resource),
        5 => Ok(FileCategory::Configuration),
        6 => Ok(FileCategory::Als),
        7 => Ok(FileCategory::Erd),
        _ => Err("project manifest file category is invalid".into()),
    }
}

pub(super) fn encode_digest_section(
    digests: &[Digest],
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(DIGEST_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u64::try_from(digests.len())
                    .map_err(|_| "compiled cache digest count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;
        for digest in digests {
            if digest.0[16..].iter().any(|byte| *byte != 0) {
                return Err(
                    "statement fingerprint exceeds the project-file 128-bit identity".into(),
                );
            }
            writer
                .write_all(&digest.0[..16])
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

pub(super) fn decode_digest_section(
    section: &EncodedSectionRef<'_>,
) -> Result<Vec<Digest>, String> {
    let decoded_length = usize::try_from(section.decoded_length)
        .map_err(|_| "compiled cache digest section is not addressable")?;
    let decoded = zstd::bulk::decompress(section.compressed, decoded_length)
        .map_err(|error| error.to_string())?;
    if decoded.len() != decoded_length {
        return Err("compiled cache decoded digest section length differs".into());
    }
    if decoded.get(..DIGEST_SECTION_MAGIC.len()) != Some(DIGEST_SECTION_MAGIC) {
        return Err("compiled cache digest section has an invalid header".into());
    }
    let mut cursor = DIGEST_SECTION_MAGIC.len();
    let count = usize::try_from(read_u64(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache digest count is not addressable")?;
    let expected_length = cursor
        .checked_add(
            count
                .checked_mul(16)
                .ok_or("compiled cache digest section length overflow")?,
        )
        .ok_or("compiled cache digest section length overflow")?;
    if expected_length != decoded.len() {
        return Err("compiled cache digest section length is invalid".into());
    }
    let mut digests = Vec::new();
    digests
        .try_reserve_exact(count)
        .map_err(|_| "compiled cache digest allocation failed")?;
    while cursor < decoded.len() {
        let end = cursor + 16;
        let mut digest = [0_u8; 32];
        digest[..16].copy_from_slice(&decoded[cursor..end]);
        digests.push(Digest(digest));
        cursor = end;
    }
    Ok(digests)
}

/// Encode source locations without repeating a textual function key for every instruction range.
///
/// Source-map entries are already canonicalized by `(function, code_start, code_end)`. The cache
/// groups each contiguous function and delta-encodes code ranges while retaining every source and
/// origin field. This is deliberately a cache-local representation; the public bytecode and JSON
/// representations stay unchanged.
#[allow(clippy::too_many_lines)]
pub(super) fn encode_source_section(
    entries: &[SourceMapEntry],
    function_indices: &std::collections::BTreeMap<SymbolKey, usize>,
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(SOURCE_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u64::try_from(entries.len())
                    .map_err(|_| "compiled cache source entry count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u32::try_from(group_count)
                    .map_err(|_| "compiled cache source group count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;

        let mut group_start = 0;
        while group_start < entries.len() {
            let function = entries[group_start].function;
            let group_length =
                entries[group_start..].partition_point(|entry| entry.function == function);
            write_varint(
                writer,
                u64::try_from(
                    *function_indices
                        .get(&function)
                        .ok_or("source-map function is missing from the artifact")?,
                )
                .map_err(|_| "source-map function index is too large")?,
            )?;
            writer
                .write_all(
                    &u32::try_from(group_length)
                        .map_err(|_| "compiled cache source group is too large")?
                        .to_le_bytes(),
                )
                .map_err(|error| error.to_string())?;
            let mut previous_code_end = 0_u64;
            let mut previous_byte_start = 0_u64;
            for entry in &entries[group_start..group_start + group_length] {
                write_varint(
                    writer,
                    entry
                        .code_start
                        .checked_sub(previous_code_end)
                        .ok_or("source-map code ranges are not ordered")?,
                )?;
                write_varint(
                    writer,
                    entry
                        .code_end
                        .checked_sub(entry.code_start)
                        .ok_or("source-map code range is reversed")?,
                )?;
                write_varint(
                    writer,
                    encode_signed_delta(entry.byte_start, previous_byte_start)?,
                )?;
                write_varint(
                    writer,
                    entry
                        .byte_end
                        .checked_sub(entry.byte_start)
                        .ok_or("source-map byte range is reversed")?,
                )?;
                write_varint(writer, u64::from(entry.statement_fingerprint))?;
                write_varint(writer, u64::from(entry.source_index))?;
                match entry.origin_chain.as_deref() {
                    None => write_varint(writer, 0)?,
                    Some(origins) => {
                        write_varint(
                            writer,
                            u64::try_from(origins.len())
                                .map_err(|_| "source-map origin chain is too long")?
                                .checked_add(1)
                                .ok_or("source-map origin chain is too long")?,
                        )?;
                        for &(source_index, byte_start, byte_end) in origins {
                            write_varint(writer, u64::from(source_index))?;
                            write_varint(writer, byte_start)?;
                            write_varint(
                                writer,
                                byte_end
                                    .checked_sub(byte_start)
                                    .ok_or("source-map origin byte range is reversed")?,
                            )?;
                        }
                    }
                }
                previous_code_end = entry.code_end;
                previous_byte_start = entry.byte_start;
            }
            group_start += group_length;
        }
        Ok(())
    })
}

pub(super) fn decode_source_section(
    section: &EncodedSectionRef<'_>,
    functions: &[BytecodeFunction],
) -> Result<Vec<SourceMapEntry>, String> {
    let decoded_length = usize::try_from(section.decoded_length)
        .map_err(|_| "compiled cache source section is not addressable")?;
    let decoded = zstd::bulk::decompress(section.compressed, decoded_length)
        .map_err(|error| error.to_string())?;
    if decoded.len() != decoded_length {
        return Err("compiled cache decoded source section length differs".into());
    }
    if decoded.get(..SOURCE_SECTION_MAGIC.len()) != Some(SOURCE_SECTION_MAGIC) {
        return Err("compiled cache source section has an invalid header".into());
    }
    let mut cursor = SOURCE_SECTION_MAGIC.len();
    let entry_count = usize::try_from(read_u64(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache source entry count is not addressable")?;
    let group_count = usize::try_from(read_u32(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache source group count is not addressable")?;
    if (entry_count == 0) != (group_count == 0) || group_count > entry_count {
        return Err("compiled cache source group count is invalid".into());
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(entry_count)
        .map_err(|_| "compiled cache source entry allocation failed")?;
    for _ in 0..group_count {
        let function_index = usize::try_from(read_varint(&decoded, &mut cursor)?)
            .map_err(|_| "compiled cache source function index is not addressable")?;
        let function = functions
            .get(function_index)
            .ok_or("compiled cache source function index is out of range")?
            .key;
        let group_length = usize::try_from(read_u32(&decoded, &mut cursor)?)
            .map_err(|_| "compiled cache source group length is not addressable")?;
        if group_length == 0 || group_length > entry_count.saturating_sub(entries.len()) {
            return Err("compiled cache source group length is invalid".into());
        }
        let mut previous_code_end = 0_u64;
        let mut previous_byte_start = 0_u64;
        for _ in 0..group_length {
            let code_start = previous_code_end
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code offset overflow")?;
            let code_end = code_start
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code range overflow")?;
            let byte_start =
                decode_signed_delta(previous_byte_start, read_varint(&decoded, &mut cursor)?)?;
            let byte_end = byte_start
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source byte range overflow")?;
            let statement_fingerprint = u32::try_from(read_varint(&decoded, &mut cursor)?)
                .map_err(|_| "compiled cache statement fingerprint is out of range")?;
            let source_index = u32::try_from(read_varint(&decoded, &mut cursor)?)
                .map_err(|_| "compiled cache source index is out of range")?;
            let encoded_origin_count = read_varint(&decoded, &mut cursor)?;
            let origin_chain = if encoded_origin_count == 0 {
                None
            } else {
                let origin_count = usize::try_from(encoded_origin_count - 1)
                    .map_err(|_| "compiled cache source origin count is not addressable")?;
                if origin_count > decoded.len().saturating_sub(cursor) / 3 {
                    return Err("compiled cache source origin count is invalid".into());
                }
                let mut origins = Vec::new();
                origins
                    .try_reserve_exact(origin_count)
                    .map_err(|_| "compiled cache source origin allocation failed")?;
                for _ in 0..origin_count {
                    let source_index = u32::try_from(read_varint(&decoded, &mut cursor)?)
                        .map_err(|_| "compiled cache origin source index is out of range")?;
                    let byte_start = read_varint(&decoded, &mut cursor)?;
                    let byte_end = byte_start
                        .checked_add(read_varint(&decoded, &mut cursor)?)
                        .ok_or("compiled cache source origin byte range overflow")?;
                    origins.push((source_index, byte_start, byte_end));
                }
                Some(Box::new(origins))
            };
            entries.push(SourceMapEntry {
                function,
                code_start,
                code_end,
                byte_start,
                byte_end,
                statement_fingerprint,
                origin_chain,
                source_index,
            });
            previous_code_end = code_end;
            previous_byte_start = byte_start;
        }
    }
    if entries.len() != entry_count || cursor != decoded.len() {
        return Err("compiled cache source section has trailing or missing entries".into());
    }
    Ok(entries)
}

pub(super) fn encode_signed_delta(value: u64, previous: u64) -> Result<u64, String> {
    let delta = i128::from(value) - i128::from(previous);
    let delta = i64::try_from(delta).map_err(|_| "source-map byte delta is out of range")?;
    Ok((delta.cast_unsigned() << 1) ^ (delta >> 63).cast_unsigned())
}

pub(super) fn decode_signed_delta(previous: u64, encoded: u64) -> Result<u64, String> {
    let magnitude = i128::from(encoded >> 1);
    let delta = if encoded & 1 == 0 {
        magnitude
    } else {
        -magnitude - 1
    };
    u64::try_from(i128::from(previous) + delta)
        .map_err(|_| "compiled cache source byte delta overflows".into())
}
