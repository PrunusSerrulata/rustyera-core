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
